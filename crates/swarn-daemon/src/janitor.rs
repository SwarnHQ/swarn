//! Retention (PRD §3.3). Active workspaces keep every event, because a truncated transcript is a
//! silent lie about what an agent did. Archived ones keep 30 days. Past the size cap, the oldest
//! archived workspaces go first; if only active logs are left, the janitor warns and stops.
//! Runs at startup and then hourly, one transaction per workspace.

use std::time::Duration;

use rusqlite::{Connection, params};

use crate::db::Db;

const KEEP_ARCHIVED_DAYS: u32 = 30;

/// 2 GB unless SWARN_RETENTION_CAP_MB says otherwise.
fn cap_bytes() -> u64 {
    std::env::var("SWARN_RETENTION_CAP_MB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2048)
        * 1024
        * 1024
}

pub async fn run(db: Db) {
    loop {
        let cap = cap_bytes();
        match db.call(move |conn| prune(conn, cap)).await {
            Ok(report) if report.over_cap_with_only_active => {
                eprintln!(
                    "swarnd: the event log is over its size cap, but only active workspaces are left; keeping them"
                )
            }
            Ok(_) => {}
            Err(e) => eprintln!("swarnd: retention failed: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub workspaces_pruned: usize,
    pub over_cap_with_only_active: bool,
}

pub fn prune(conn: &mut Connection, cap: u64) -> rusqlite::Result<Report> {
    let mut report = Report::default();
    // Archived longer than the grace period.
    let stale: Vec<String> = conn
        .prepare(
            "SELECT id FROM workspace WHERE state = 'archived'
             AND archived_at < strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?1) ORDER BY archived_at",
        )?
        .query_map([format!("-{KEEP_ARCHIVED_DAYS} days")], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in stale {
        report.workspaces_pruned += drop_events(conn, &id)?;
    }

    // Over the cap: the oldest archived workspaces that still have events, one at a time.
    while used_bytes(conn)? > cap {
        let next: Option<String> = conn
            .query_row(
                "SELECT w.id FROM workspace w WHERE w.state = 'archived'
                 AND EXISTS (SELECT 1 FROM event e JOIN session s ON s.id = e.session_id WHERE s.workspace_id = w.id)
                 ORDER BY w.archived_at LIMIT 1",
                [],
                |r| r.get(0),
            )
            .ok();
        match next {
            Some(id) => report.workspaces_pruned += drop_events(conn, &id)?,
            None => {
                report.over_cap_with_only_active = true;
                break;
            }
        }
    }
    Ok(report)
}

/// Deletes one workspace's events in its own transaction. Returns 1 if anything went.
fn drop_events(conn: &mut Connection, workspace_id: &str) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let deleted = tx.execute(
        "DELETE FROM event WHERE session_id IN (SELECT id FROM session WHERE workspace_id = ?1)",
        params![workspace_id],
    )?;
    tx.commit()?;
    Ok(usize::from(deleted > 0))
}

/// Bytes the database actually uses: freed pages are reused, so they don't count against the cap.
fn used_bytes(conn: &Connection) -> rusqlite::Result<u64> {
    let pragma = |name: &str| conn.pragma_query_value(None, name, |r| r.get::<_, i64>(0));
    Ok(((pragma("page_count")? - pragma("freelist_count")?) * pragma("page_size")?) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO repo (id, path, default_branch, created_at) VALUES ('r', '/r', 'main', '2026-01-01');
             INSERT INTO workspace (id, repo_id, name, branch, base_ref, worktree_path, state, created_at, archived_at) VALUES
               ('old', 'r', 'old', 'b1', 'main', '/w1', 'archived', '2026-01-01', strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-40 days')),
               ('recent', 'r', 'recent', 'b2', 'main', '/w2', 'archived', '2026-01-01', strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-2 days')),
               ('live', 'r', 'live', 'b3', 'main', '/w3', 'ready', '2026-01-01', NULL);
             INSERT INTO session (id, workspace_id, agent, tier, status, started_at, last_change_at) VALUES
               ('s-old', 'old', 'codex', 'native', 'exited', 'x', 'x'),
               ('s-recent', 'recent', 'codex', 'native', 'exited', 'x', 'x'),
               ('s-live', 'live', 'codex', 'native', 'working', 'x', 'x');",
        )
        .unwrap();
        for session in ["s-old", "s-recent", "s-live"] {
            for _ in 0..50 {
                conn.execute(
                    "INSERT INTO event (session_id, kind, payload, ts) VALUES (?1, 'agent_message_delta', ?2, 'x')",
                    params![session, "x".repeat(2000)],
                )
                .unwrap();
            }
        }
        conn
    }

    fn events(conn: &Connection, session: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM event WHERE session_id = ?1",
            [session],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn archived_past_thirty_days_go_and_active_logs_never_do() {
        let mut conn = seeded();
        let report = prune(&mut conn, u64::MAX).unwrap();
        assert_eq!(
            report,
            Report {
                workspaces_pruned: 1,
                over_cap_with_only_active: false
            }
        );
        assert_eq!(
            (
                events(&conn, "s-old"),
                events(&conn, "s-recent"),
                events(&conn, "s-live")
            ),
            (0, 50, 50)
        );
    }

    #[test]
    fn over_the_cap_recent_archives_go_next_then_it_warns() {
        let mut conn = seeded();
        let report = prune(&mut conn, 1).unwrap();
        assert_eq!(
            report,
            Report {
                workspaces_pruned: 2,
                over_cap_with_only_active: true
            }
        );
        assert_eq!(
            events(&conn, "s-live"),
            50,
            "an active transcript is never truncated"
        );
    }
}
