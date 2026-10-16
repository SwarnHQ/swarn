//! Performance harness (PRD §9.3). Starts a daemon on a temp home with N fake agents streaming at
//! a fixed rate, opens the desktop in bench mode, samples both processes' RSS, and prints one JSON
//! result with the PRD's release budgets next to the numbers.
//!
//! Build everything in one profile first:
//!   cargo build --release -p swarn-daemon -p swarn-bench -p swarn-desktop --features swarn-desktop/bench
//! then run: target/release/swarn-bench [--agents 10] [--rate 50] [--seconds 30]
//!
//! Frame times need the window on screen: a locked or hidden Mac draws no frames.

use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use swarn_core::local_api::{DaemonInfo, call};

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str, default: f64| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let (agents, rate, seconds) = (
        flag("--agents", 10.0) as usize,
        flag("--rate", 50.0),
        flag("--seconds", 30.0),
    );

    let bin = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    for name in ["swarnd", "swarn", "swarn-fake-agent"] {
        if !bin.join(name).exists() {
            eprintln!(
                "missing {}; build all three in the same profile first (see the top of main.rs)",
                bin.join(name).display()
            );
            std::process::exit(2);
        }
    }

    let home = tempfile::tempdir().unwrap();
    // SAFETY: still single-threaded here; the client calls below read it.
    unsafe { std::env::set_var("SWARN_HOME", home.path()) };
    let daemon = Killed(
        Command::new(bin.join("swarnd"))
            .env("SWARN_CLAUDE_BIN", bin.join("swarn-fake-agent"))
            .env("FAKE_RATE", rate.to_string())
            .env("FAKE_SECONDS", seconds.to_string())
            .spawn()
            .unwrap(),
    );
    wait(|| call::<DaemonInfo>("daemon.info", json!({})).is_ok());

    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    git(
        repo.path(),
        &[
            "-c",
            "user.name=b",
            "-c",
            "user.email=b@b",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    let repo_id = call::<Value>("repo.add", json!({ "path": repo.path() })).unwrap()["id"].clone();
    let workspaces: Vec<Value> = (0..agents)
        .map(|i| {
            call::<Value>(
                "workspace.create",
                json!({ "repo_id": repo_id, "name": format!("bench-{i}") }),
            )
            .unwrap()
        })
        .collect();

    let report = home.path().join("desktop.json");
    let desktop = Killed(
        Command::new(bin.join("swarn"))
            .env("SWARN_BENCH_OUT", &report)
            .env("SWARN_BENCH_SECONDS", (seconds + 5.0).to_string())
            .spawn()
            .unwrap(),
    );
    std::thread::sleep(Duration::from_secs(3));
    for ws in &workspaces {
        let _: Value = call(
            "session.start",
            json!({ "workspace_id": ws["id"], "prompt": "go" }),
        )
        .unwrap();
    }

    let (mut daemon_peak, mut desktop_peak, mut total_peak) = (0u64, 0u64, 0u64);
    let deadline = Instant::now() + Duration::from_secs_f64(seconds + 10.0);
    while Instant::now() < deadline && !report.exists() {
        let (d, w) = (rss_kb(daemon.0.id()), rss_kb(desktop.0.id()));
        daemon_peak = daemon_peak.max(d);
        desktop_peak = desktop_peak.max(w);
        total_peak = total_peak.max(d + w);
        std::thread::sleep(Duration::from_millis(500));
    }
    std::thread::sleep(Duration::from_secs(1));
    let desktop_stats: Value = std::fs::read_to_string(&report)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({ "error": "the desktop did not write its report" }));

    let mb = |kb: u64| (kb as f64 / 1024.0 * 10.0).round() / 10.0;
    let p99 = desktop_stats["draw_ms"]["p99"].as_f64();
    let cold = desktop_stats["cold_start_ms"].as_f64();
    let result = json!({
        "agents": agents, "rate_per_agent": rate, "seconds": seconds,
        "rss_mb": { "daemon_peak": mb(daemon_peak), "desktop_peak": mb(desktop_peak), "together_peak": mb(total_peak) },
        "desktop": desktop_stats,
        "budgets": {
            "rss_under_250_mb": mb(total_peak) < 250.0,
            "p99_frame_under_8_3_ms": p99.map(|p| p < 8.3),
            "cold_start_under_400_ms": cold.map(|c| c < 400.0),
        },
    });
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

fn rss_kb(pid: u32) -> u64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output();
    out.ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

fn git(dir: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap()
            .success(),
        "git {args:?}"
    );
}

fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the daemon"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
