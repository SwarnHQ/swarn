//! The event log: append, derive status, broadcast. All three happen inside one job on the database
//! thread, so `seq` order, status and what subscribers see can never disagree (§4.1).

use rusqlite::params;
use serde_json::{Value, json};
use swarn_core::schema::{Event, EventKind, Status, next_status};
use tokio::sync::broadcast;

use crate::db::Db;

#[derive(Clone)]
pub struct EventLog {
    db: Db,
    tx: broadcast::Sender<Event>,
}

impl EventLog {
    pub fn new(db: Db) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { db, tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Writes the event, moves the session's status if the event changes it (writing a
    /// `status_changed` event too), and broadcasts both. `pending` is the number of permission
    /// requests still open for the session after this event.
    pub async fn append(&self, session_id: &str, kind: EventKind, payload: Value, pending: usize) {
        let tx = self.tx.clone();
        let session_id = session_id.to_owned();
        let result = self
            .db
            .call(move |conn| {
                let event = insert(conn, &session_id, kind, &payload)?;
                let _ = tx.send(event);

                let current: String =
                    conn.query_row("SELECT status FROM session WHERE id = ?1", [&session_id], |r| r.get(0))?;
                let current: Status = serde_json::from_value(json!(current)).unwrap_or(Status::Starting);
                let next = next_status(current, kind, pending);
                if next != current {
                    let status = status_str(next);
                    conn.execute(
                        "UPDATE session SET status = ?2, last_change_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                         ended_at = CASE WHEN ?2 IN ('exited', 'failed') THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') ELSE ended_at END
                         WHERE id = ?1",
                        params![session_id, status],
                    )?;
                    let changed =
                        insert(conn, &session_id, EventKind::StatusChanged, &json!({ "status": status }))?;
                    let _ = tx.send(changed);
                }
                Ok(())
            })
            .await;
        if let Err(e) = result {
            eprintln!("swarnd: event log write failed: {e}");
        }
    }

    pub async fn since(&self, after_seq: u64) -> rusqlite::Result<Vec<Event>> {
        self.db
            .call(move |conn| {
                conn.prepare(
                    "SELECT seq, session_id, kind, payload, ts FROM event WHERE seq > ?1 ORDER BY seq",
                )?
                .query_map([after_seq as i64], |r| {
                    let kind: String = r.get(2)?;
                    let payload: String = r.get(3)?;
                    Ok(Event {
                        seq: r.get::<_, i64>(0)? as u64,
                        session_id: r.get(1)?,
                        kind: serde_json::from_value(json!(kind)).unwrap_or(EventKind::Unknown),
                        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                        ts: r.get(4)?,
                    })
                })?
                .collect()
            })
            .await
    }
}

fn insert(
    conn: &rusqlite::Connection,
    session_id: &str,
    kind: EventKind,
    payload: &Value,
) -> rusqlite::Result<Event> {
    let kind_str = serde_json::to_value(kind)
        .expect("EventKind serialises")
        .as_str()
        .unwrap_or("unknown")
        .to_owned();
    conn.query_row(
        "INSERT INTO event (session_id, kind, payload, ts) VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         RETURNING seq, ts",
        params![session_id, kind_str, payload.to_string()],
        |r| {
            Ok(Event { seq: r.get::<_, i64>(0)? as u64, session_id: session_id.to_owned(), kind, payload: payload.clone(), ts: r.get(1)? })
        },
    )
}

pub fn status_str(status: Status) -> &'static str {
    match status {
        Status::Starting => "starting",
        Status::Working => "working",
        Status::NeedsInput => "needs_input",
        Status::Done => "done",
        Status::Failed => "failed",
        Status::Exited => "exited",
    }
}
