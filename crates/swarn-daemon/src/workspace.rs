//! Workspaces: one git worktree and branch per task (§4.3). A failure leaves a row in `failed` and a
//! directory the user can remove, never a half-state the next operation trips over.

use std::path::Path;

use rusqlite::{OptionalExtension, params};
use swarn_core::schema::{Repo, Workspace, WorkspaceState};
use tokio::process::Command;

use crate::db::Db;

/// Used when a repo has no `.worktreeinclude`: the files people most often need in a fresh worktree.
const DEFAULT_INCLUDE: &str = ".env*";

pub async fn create(
    db: &Db,
    home: &Path,
    repo: Repo,
    name: String,
    base_ref: Option<String>,
) -> Result<Workspace, String> {
    let valid = !name.is_empty()
        && name.len() <= 48
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-');
    if !valid {
        return Err(format!(
            "workspace name {name:?} must be lowercase letters, digits and dashes"
        ));
    }

    // Checked up front so the answer is a sentence, not git's "fatal:" from halfway through.
    let branch_ref = format!("refs/heads/swarn/{name}");
    if run_git(
        &repo.path,
        &["rev-parse", "--verify", "--quiet", &branch_ref],
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "The branch swarn/{name} already exists, maybe from an archived workspace. Pick another name."
        ));
    }

    let repo_dir = Path::new(&repo.path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // The ULID's last six characters are random, so two repos with the same folder name never collide.
    let repo_key = format!(
        "{repo_dir}-{}",
        repo.id[repo.id.len() - 6..].to_ascii_lowercase()
    );
    let path = home.join("worktrees").join(repo_key).join(&name);
    let workspace = Workspace {
        id: ulid::Ulid::generate().to_string(),
        repo_id: repo.id.clone(),
        branch: format!("swarn/{name}"),
        base_ref: base_ref.unwrap_or_else(|| repo.default_branch.clone()),
        worktree_path: path.to_string_lossy().into_owned(),
        state: WorkspaceState::Creating,
        created_at: String::new(),
        archived_at: None,
        name,
    };

    let row = workspace.clone();
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO workspace (id, repo_id, name, branch, base_ref, worktree_path, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'creating', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![row.id, row.repo_id, row.name, row.branch, row.base_ref, row.worktree_path],
        )
    })
    .await
    .map_err(|e| e.to_string())?;

    let outcome = async {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| (false, e.to_string()))?;
        }
        run_git(
            &repo.path,
            &[
                "worktree",
                "add",
                "-b",
                &workspace.branch,
                &workspace.worktree_path,
                &workspace.base_ref,
            ],
        )
        .await
        .map_err(|e| (false, e))?;
        copy_included(Path::new(&repo.path), &path)
            .await
            .map_err(|e| (true, e))
    }
    .await;

    // No worktree on disk means nothing to show or clean up later, so the row goes too.
    let sql = match &outcome {
        Ok(_) => "UPDATE workspace SET state = 'ready' WHERE id = ?1",
        Err((true, _)) => "UPDATE workspace SET state = 'failed' WHERE id = ?1",
        Err((false, _)) => "DELETE FROM workspace WHERE id = ?1",
    };
    let id = workspace.id.clone();
    db.call(move |conn| conn.execute(sql, [id]))
        .await
        .map_err(|e| e.to_string())?;
    outcome.map_err(|(_, e)| e)?;
    // ponytail: no setup script or port block yet; both need repo configuration, which lands with first run (§5.5 step 6).
    get(db, &workspace.id)
        .await?
        .ok_or_else(|| "workspace vanished".into())
}

/// Copies gitignored files the worktree needs, matched by `.worktreeinclude` (gitignore syntax,
/// the same file Claude Code reads) or `.env*` when the repo has none. Returns how many were copied.
async fn copy_included(repo: &Path, worktree: &Path) -> Result<usize, String> {
    let include_file = repo.join(".worktreeinclude");
    let pattern_arg = if include_file.is_file() {
        format!("--exclude-from={}", include_file.display())
    } else {
        format!("--exclude={DEFAULT_INCLUDE}")
    };
    // Untracked files that match the patterns. Tracked ones are already in the new worktree.
    let listed = run_git(repo, &["ls-files", "--others", "--ignored", &pattern_arg]).await?;
    let mut copied = 0;
    for rel in listed.lines().filter(|l| !l.is_empty()) {
        let to = worktree.join(rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(repo.join(rel), &to).map_err(|e| format!("copying {rel}: {e}"))?;
        copied += 1;
    }
    Ok(copied)
}

/// Removes the worktree, then the branch only if it is merged. Unmerged branches stay, so nothing a
/// user might still want is ever deleted by archiving.
pub async fn archive(db: &Db, workspace_id: String, force: bool) -> Result<Workspace, String> {
    let ws = get(db, &workspace_id)
        .await?
        .ok_or_else(|| format!("no workspace {workspace_id}"))?;
    let repo_path: String = {
        let repo_id = ws.repo_id.clone();
        db.call(move |conn| {
            conn.query_row("SELECT path FROM repo WHERE id = ?1", [repo_id], |r| {
                r.get(0)
            })
        })
        .await
        .map_err(|e| e.to_string())?
    };
    if Path::new(&ws.worktree_path).exists() {
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&ws.worktree_path);
        run_git(&repo_path, &args)
            .await
            .map_err(|e| format!("{e} (pass \"force\": true to discard changes)"))?;
    }
    let _ = run_git(&repo_path, &["branch", "-d", &ws.branch]).await; // fails, harmlessly, when unmerged

    let id = workspace_id.clone();
    db.call(move |conn| {
        conn.execute(
            "UPDATE workspace SET state = 'archived', archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            [id],
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    get(db, &workspace_id)
        .await?
        .ok_or_else(|| "workspace vanished".into())
}

pub async fn list(db: &Db) -> Result<Vec<Workspace>, String> {
    db.call(|conn| {
        conn.prepare(&format!(
            "{SELECT} WHERE state != 'archived' ORDER BY created_at"
        ))?
        .query_map([], row)?
        .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

pub async fn get(db: &Db, id: &str) -> Result<Option<Workspace>, String> {
    let id = id.to_owned();
    db.call(move |conn| {
        conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], row)
            .optional()
    })
    .await
    .map_err(|e| e.to_string())
}

const SELECT: &str = "SELECT id, repo_id, name, branch, base_ref, worktree_path, state, created_at, archived_at FROM workspace";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Workspace> {
    let state: String = r.get(6)?;
    Ok(Workspace {
        id: r.get(0)?,
        repo_id: r.get(1)?,
        name: r.get(2)?,
        branch: r.get(3)?,
        base_ref: r.get(4)?,
        worktree_path: r.get(5)?,
        state: serde_json::from_value(serde_json::json!(state)).unwrap_or(WorkspaceState::Failed),
        created_at: r.get(7)?,
        archived_at: r.get(8)?,
    })
}

/// Trimmed stdout of `git -C <dir> ...`, or git's own error message.
pub async fn run_git(dir: impl AsRef<Path>, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir.as_ref())
        .args(args)
        .output()
        .await
        .map_err(|e| format!("running git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}
