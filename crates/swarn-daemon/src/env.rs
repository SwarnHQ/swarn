//! The environment agents run in. A launch agent starts with `PATH=/usr/bin:/bin:/usr/sbin:/sbin`,
//! where none of `claude`, `codex`, `node` or `gh` live, so the daemon asks the user's login shell
//! once and uses its answer for every child (PRD §5.5 step 2, measured in docs/cli-behaviour.md).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

const MARKER: &[u8] = b"\n__SWARN_ENV__\n";

#[derive(Clone, Debug)]
pub struct AgentEnv {
    pub vars: HashMap<String, String>,
    /// False when the login shell could not be read and the daemon's own environment stands in.
    pub captured: bool,
}

impl AgentEnv {
    /// `$SHELL -l -i -c env -0`, 5 second budget. Falls back to the daemon's own environment, which
    /// is what a terminal-started daemon already has.
    pub async fn capture() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let script = r#"printf '\n__SWARN_ENV__\n'; /usr/bin/env -0"#;
        let run = Command::new(&shell)
            .args(["-l", "-i", "-c", script])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output();
        let captured = match tokio::time::timeout(Duration::from_secs(5), run).await {
            Ok(Ok(out)) if out.status.success() => parse(&out.stdout),
            _ => None,
        };
        let ok = captured.is_some();
        let vars = captured.unwrap_or_else(|| {
            eprintln!("swarnd: could not read the login shell environment; using the daemon's own");
            std::env::vars().collect()
        });
        Self { vars, captured: ok }
    }

    /// First match for `program` on the captured `PATH`.
    pub fn which(&self, program: &str) -> Option<PathBuf> {
        let path = self.vars.get("PATH")?;
        std::env::split_paths(path)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    }
}

/// Everything after the marker, so whatever an interactive shell prints first is ignored.
fn parse(stdout: &[u8]) -> Option<HashMap<String, String>> {
    let start = stdout.windows(MARKER.len()).position(|w| w == MARKER)? + MARKER.len();
    let vars: HashMap<String, String> = stdout[start..]
        .split(|b| *b == 0)
        .filter_map(|entry| {
            let entry = String::from_utf8_lossy(entry);
            let (key, value) = entry.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect();
    vars.contains_key("PATH").then_some(vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_shell_noise_before_the_marker() {
        let out = b"Welcome to oh-my-zsh\n__SWARN_ENV__\nPATH=/a:/b\0HOME=/h\0EMPTY=\0";
        let vars = parse(out).unwrap();
        assert_eq!(vars["PATH"], "/a:/b");
        assert_eq!(vars["EMPTY"], "");
        assert!(!vars.contains_key("Welcome to oh-my-zsh"));
    }

    #[test]
    fn no_marker_or_no_path_means_no_capture() {
        assert!(parse(b"PATH=/a\0").is_none());
        assert!(parse(b"\n__SWARN_ENV__\nHOME=/h\0").is_none());
    }
}
