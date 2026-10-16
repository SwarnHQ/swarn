//! Loading and saving the policy file, and keeping the Mac awake while agents work.

use std::path::PathBuf;
use std::time::Duration;

use swarn_core::local_api::Policy;
use tokio::process::{Child, Command};

use crate::db::Db;

/// `$SWARN_CONFIG_DIR/policy.toml`, else `~/.config/swarn/policy.toml` (PRD §2.6).
pub fn path() -> PathBuf {
    std::env::var_os("SWARN_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::home_dir()
                .expect("HOME is set")
                .join(".config/swarn")
        })
        .join("policy.toml")
}

/// Read fresh each time it matters, so a hand edit takes effect on the next session. A missing
/// file means defaults; a broken one also means defaults, with a warning, never a refusal to run.
pub fn load() -> Policy {
    match std::fs::read_to_string(path()) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("swarnd: ignoring {}: {e}", path().display());
            Policy::default()
        }),
        Err(_) => Policy::default(),
    }
}

/// ponytail: rewrites the whole file, so keys this version does not know (the PRD's
/// `require_biometric_for`, `auto_deny`) are dropped; switch to toml_edit when those land.
pub fn save(policy: &Policy) -> Result<(), String> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(policy).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// Holds an idle-sleep assertion (`caffeinate -i`, tied to this process with `-w`) while any session
/// is starting, working or waiting on the user, and the policy allows it. A closed lid on battery
/// still sleeps; onboarding says so (§5.5 step 9).
/// ponytail: checks every 10 s; idle sleep takes minutes, so that is plenty.
pub async fn keep_awake(db: Db) {
    let mut caffeinate: Option<Child> = None;
    loop {
        let active: i64 = db
            .call(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM session WHERE status IN ('starting', 'working', 'needs_input')",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap_or(0);
        let wanted = active > 0 && load().power.keep_awake;
        match (wanted, caffeinate.is_some()) {
            (true, false) => {
                caffeinate = Command::new("/usr/bin/caffeinate")
                    .args(["-i", "-w", &std::process::id().to_string()])
                    .kill_on_drop(true)
                    .spawn()
                    .ok();
            }
            (false, true) => caffeinate = None, // dropping it kills it
            _ => {}
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swarn_core::local_api::ApprovalMode;

    #[test]
    fn missing_keys_take_defaults_and_unknown_ones_are_ignored() {
        let policy: Policy = toml::from_str(
            "[approvals]\nmode = \"allow_edits\"\nrequire_biometric_for = [\"exec\"]\n",
        )
        .unwrap();
        assert_eq!(policy.approvals.mode, ApprovalMode::AllowEdits);
        assert_eq!(policy.approvals.default_expiry_secs, 1800);
        assert!(policy.power.keep_awake);
        assert_eq!(toml::from_str::<Policy>("").unwrap(), Policy::default());
    }
}
