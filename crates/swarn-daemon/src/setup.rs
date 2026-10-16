//! First-run checks (PRD §5.5 steps 2 to 4), run inside the daemon because its environment, not
//! the app's, is the one agents will run in. Each check says what is wrong and what to do.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Command;

use crate::env::AgentEnv;

pub async fn check(env: &AgentEnv) -> Value {
    let (git, gh, claude, codex) = tokio::join!(git(env), gh(env), claude(env), codex(env));
    let mut warnings = Vec::new();
    if !env.captured {
        warnings.push("Could not read your login shell's environment, so tools installed through it may be missing.".to_owned());
    }
    if env.vars.contains_key("ANTHROPIC_API_KEY") {
        warnings.push(
            "ANTHROPIC_API_KEY is set in your shell. Claude Code will bill that key instead of your Claude subscription.".to_owned(),
        );
    }
    json!({ "git": git, "gh": gh, "agents": [claude, codex], "warnings": warnings })
}

async fn git(env: &AgentEnv) -> Value {
    match run(env, "git", &["--version"]).await {
        Ok(out) => json!({ "ok": true, "detail": out }),
        // An unaccepted Xcode license breaks git, and with it gh, without saying so elsewhere.
        Err(e) if e.contains("license") => json!({
            "ok": false,
            "detail": "The Xcode license hasn't been accepted. Run `sudo xcodebuild -license` in a terminal.",
        }),
        Err(e) => json!({ "ok": false, "detail": e }),
    }
}

async fn gh(env: &AgentEnv) -> Value {
    if env.which("gh").is_none() {
        return json!({ "installed": false, "signed_in": false,
            "detail": "Not installed. Only needed to open pull requests: `brew install gh`." });
    }
    match run(env, "gh", &["auth", "status"]).await {
        Ok(_) => json!({ "installed": true, "signed_in": true, "detail": "Signed in" }),
        Err(_) if env.vars.contains_key("GH_TOKEN") => {
            json!({ "installed": true, "signed_in": true, "detail": "Using GH_TOKEN" })
        }
        Err(_) => {
            json!({ "installed": true, "signed_in": false, "detail": "Not signed in", "sign_in": "gh auth login" })
        }
    }
}

async fn claude(env: &AgentEnv) -> Value {
    let mut out =
        json!({ "agent": "claude-code", "name": "Claude Code", "sign_in": "claude /login" });
    let Some(path) = env.which("claude") else {
        out["installed"] = json!(false);
        out["detail"] = json!("Not installed. See https://claude.com/claude-code to install it.");
        return out;
    };
    out["installed"] = json!(true);
    out["path"] = json!(path);
    out["version"] = json!(
        run(env, "claude", &["--version"])
            .await
            .ok()
            .map(|v| first_word(&v))
    );
    let status = run(env, "claude", &["auth", "status"])
        .await
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let signed_in = status.as_ref().is_some_and(|s| s["loggedIn"] == true);
    out["signed_in"] = json!(signed_in);
    out["detail"] = json!(match &status {
        Some(s) if signed_in => {
            let method = if s["authMethod"] == "claude.ai" {
                "Claude account"
            } else {
                "API key"
            };
            match s["subscriptionType"].as_str() {
                Some(plan) => format!("Signed in with your {method} ({plan})"),
                None => format!("Signed in with your {method}"),
            }
        }
        _ => "Not signed in".to_owned(),
    });
    out
}

async fn codex(env: &AgentEnv) -> Value {
    let mut out = json!({ "agent": "codex", "name": "Codex", "sign_in": "codex login" });
    let Some(path) = env.which("codex") else {
        out["installed"] = json!(false);
        out["detail"] = json!(
            "Not installed. `brew install codex` or see https://developers.openai.com/codex."
        );
        return out;
    };
    out["installed"] = json!(true);
    out["path"] = json!(path);
    out["version"] = json!(
        run(env, "codex", &["--version"]).await.ok().map(|v| v
            .rsplit(' ')
            .next()
            .unwrap_or_default()
            .to_owned())
    );
    match run(env, "codex", &["login", "status"]).await {
        Ok(detail) => {
            out["signed_in"] = json!(true);
            out["detail"] = json!(detail.lines().next().unwrap_or("Signed in"));
        }
        Err(_) => {
            out["signed_in"] = json!(false);
            out["detail"] = json!("Not signed in");
        }
    }
    out
}

fn first_word(s: &str) -> String {
    s.split_whitespace().next().unwrap_or_default().to_owned()
}

/// Runs a tool from the captured PATH with the captured environment, 10 second budget. Output is
/// stdout (stderr if stdout is empty), or both together when the tool fails.
async fn run(env: &AgentEnv, program: &str, args: &[&str]) -> Result<String, String> {
    let path = env
        .which(program)
        .ok_or_else(|| format!("{program} not found"))?;
    let child = Command::new(path)
        .args(args)
        .env_clear()
        .envs(&env.vars)
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(10), child)
        .await
        .map_err(|_| format!("{program} did not answer within 10 seconds"))?
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if out.status.success() {
        // Some tools (codex login status) report on stderr even when all is well.
        Ok(if stdout.is_empty() {
            String::from_utf8_lossy(&out.stderr).trim().to_owned()
        } else {
            stdout
        })
    } else {
        Err(
            format!("{stdout} {}", String::from_utf8_lossy(&out.stderr).trim())
                .trim()
                .to_owned(),
        )
    }
}
