//! Reviewing a workspace: its diff, and opening a pull request with `gh` (§4.4, §5.3).

use tokio::process::Command;

use swarn_core::schema::Workspace;

use crate::env::AgentEnv;
use crate::workspace::run_git;

/// Everything the workspace changed since it branched: commits, edits and new files, as one
/// unified diff. Measured from the merge-base, so later commits on the base branch don't show up
/// as reversed changes.
pub async fn diff(ws: &Workspace) -> Result<String, String> {
    let wt = ws.worktree_path.as_str();
    let base = run_git(wt, &["merge-base", "HEAD", &ws.base_ref]).await?;
    let mut out = run_git(wt, &["diff", "--no-color", "--no-ext-diff", &base]).await?;

    // New files the agent hasn't staged are invisible to `git diff`; diff each against nothing.
    let untracked = run_git(wt, &["ls-files", "--others", "--exclude-standard"]).await?;
    for file in untracked.lines().filter(|l| !l.is_empty()) {
        let (_, text) = git_raw(
            wt,
            &["diff", "--no-color", "--no-index", "--", "/dev/null", file],
        )
        .await?;
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(text.trim_end());
    }
    Ok(out)
}

/// Commits anything uncommitted, pushes `swarn/<name>`, and opens the pull request against the
/// workspace's base branch. Returns the PR's URL. Each precondition fails with a message that says
/// what to do, rather than a raw subprocess error (§4.4).
pub async fn create_pr(
    env: &AgentEnv,
    ws: &Workspace,
    title: &str,
    body: &str,
) -> Result<String, String> {
    let wt = ws.worktree_path.as_str();
    let gh = env
        .which("gh")
        .ok_or("The GitHub CLI isn't installed. Install it with `brew install gh`, then run `gh auth login`.")?;
    let signed_in = Command::new(&gh)
        .args(["auth", "status"])
        .current_dir(wt)
        .env_clear()
        .envs(&env.vars)
        .output()
        .await
        .map_err(|e| format!("running gh: {e}"))?;
    if !signed_in.status.success() {
        return Err(
            "The GitHub CLI isn't signed in. Run `gh auth login` in a terminal, then try again."
                .into(),
        );
    }
    run_git(wt, &["remote", "get-url", "origin"])
        .await
        .map_err(|_| "This repository has no `origin` remote to push to.".to_owned())?;

    if !run_git(wt, &["status", "--porcelain"]).await?.is_empty() {
        run_git(wt, &["add", "-A"]).await?;
        run_git(wt, &["commit", "-q", "-m", title]).await?;
    }
    let base = run_git(wt, &["merge-base", "HEAD", &ws.base_ref]).await?;
    let ahead = run_git(wt, &["rev-list", "--count", &format!("{base}..HEAD")]).await?;
    if ahead == "0" {
        return Err("Nothing to open a pull request for: the branch has no changes yet.".into());
    }
    run_git(wt, &["push", "-q", "-u", "origin", &ws.branch]).await?;

    let created = Command::new(&gh)
        .args([
            "pr",
            "create",
            "--base",
            &ws.base_ref,
            "--head",
            &ws.branch,
            "--title",
            title,
            "--body",
            body,
        ])
        .current_dir(wt)
        .env_clear()
        .envs(&env.vars)
        .output()
        .await
        .map_err(|e| format!("running gh: {e}"))?;
    let stdout = String::from_utf8_lossy(&created.stdout);
    if !created.status.success() {
        return Err(String::from_utf8_lossy(&created.stderr).trim().to_owned());
    }
    Ok(stdout
        .lines()
        .rev()
        .find(|l| l.starts_with("https://"))
        .unwrap_or(stdout.trim())
        .to_owned())
}

/// `git` without treating a non-zero exit as failure: `diff --no-index` exits 1 when the files
/// differ, which is the normal case here.
async fn git_raw(dir: &str, args: &[&str]) -> Result<(i32, String), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("running git: {e}"))?;
    let code = out.status.code().unwrap_or(-1);
    if code > 1 {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok((code, String::from_utf8_lossy(&out.stdout).into_owned()))
}
