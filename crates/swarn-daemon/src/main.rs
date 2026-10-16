//! `swarnd`: owns agents, worktrees, the event log and pairings. The desktop only renders what
//! this process says.
//!
//! `swarnd`                       run the daemon in the foreground
//! `swarnd call <method> [json]`  send one JSON-RPC request to a running daemon and print the result
//! `swarnd watch [after-seq]`     print the live transcript of every session

mod api;
mod claude;
mod codex;
mod db;
mod devices;
mod discovery;
mod env;
mod events;
mod identity;
mod policy;
mod review;
mod sessions;
mod setup;
mod workspace;

use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::process::ExitCode;

use std::io::Write;

use serde_json::Value;
use swarn_core::local_api::{self, swarn_home};
use swarn_core::schema::{Event, EventKind};
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run(),
        Some("call") if args.len() >= 2 => call(&args[1], args.get(2).map(String::as_str)),
        Some("watch") => watch(args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0)),
        _ => {
            eprintln!("usage: swarnd [call <method> [params-json] | watch [after-seq]]");
            ExitCode::from(2)
        }
    }
}

fn call(method: &str, params: Option<&str>) -> ExitCode {
    let params = match params.map(serde_json::from_str).transpose() {
        Ok(p) => p.unwrap_or(Value::Object(Default::default())),
        Err(e) => {
            eprintln!("params are not JSON: {e}");
            return ExitCode::from(2);
        }
    };
    match local_api::call::<Value>(method, params) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("Value serialises")
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The week 2 terminal client: agent text streams inline, everything else gets its own line.
fn watch(after_seq: u64) -> ExitCode {
    let mut mid_line = false;
    let result = local_api::subscribe(after_seq, |event, _live| {
        print_event(&event, &mut mid_line);
        true
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn print_event(event: &Event, mid_line: &mut bool) {
    let p = &event.payload;
    let tag = &event.session_id[event.session_id.len().saturating_sub(6)..];
    let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let line = match event.kind {
        EventKind::AgentMessageDelta => {
            print!("{}", text(&p["text"]));
            let _ = std::io::stdout().flush();
            *mid_line = true;
            return;
        }
        EventKind::UserMessage => format!("[{tag}] > {}", text(&p["text"])),
        EventKind::SessionStarted => format!(
            "[{tag}] session started ({} {})",
            text(&p["model"]),
            text(&p["version"])
        ),
        EventKind::ToolCall => format!("[{tag}] tool {} {}", text(&p["name"]), p["input"]),
        EventKind::ToolCallUpdate => format!("[{tag}] tool {}", text(&p["status"])),
        EventKind::PermissionRequest => format!(
            "[{tag}] NEEDS APPROVAL ({}) {}\n    swarnd call approval.resolve '{{\"request_id\":\"{}\",\"decision\":\"allow_once\"}}'",
            text(&p["risk"]),
            text(&p["summary"]),
            text(&p["request_id"]),
        ),
        EventKind::PermissionResolved => {
            let reason = p["reason"]
                .as_str()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            format!("[{tag}] approval {}{reason}", text(&p["decision"]))
        }
        EventKind::StatusChanged => format!("[{tag}] status: {}", text(&p["status"])),
        EventKind::Usage => {
            let cost = p["cost_usd"]
                .as_f64()
                .map(|c| format!(", ${c:.4}"))
                .unwrap_or_default();
            format!(
                "[{tag}] usage: {} in, {} out{cost}",
                p["input_tokens"], p["output_tokens"]
            )
        }
        EventKind::Error => format!("[{tag}] error: {}", text(&p["message"])),
        EventKind::SessionEnded => format!("[{tag}] session ended ({})", p["exit_code"]),
        EventKind::AgentMessageEnd
        | EventKind::ThoughtDelta
        | EventKind::Plan
        | EventKind::DiffStat
        | EventKind::Unknown => {
            return;
        }
    };
    if std::mem::take(mid_line) {
        println!();
    }
    println!("{line}");
}

#[tokio::main]
async fn run() -> ExitCode {
    match serve().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("swarnd: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let home = swarn_home();
    // 0700 on the directory protects the socket even in the instant before its own chmod.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&home)?;

    let socket = local_api::socket_path();
    if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        return Err(format!(
            "another swarnd is already listening on {}",
            socket.display()
        )
        .into());
    }
    let _ = std::fs::remove_file(&socket); // stale, from a daemon that died without cleaning up

    let db = db::Db::open(&home.join("state.db"))?;
    // Agent processes die with the daemon that owned them, so no session survives a restart.
    db.call(|conn| {
        conn.execute(
            "UPDATE session SET status = 'exited', ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE status IN ('starting', 'working', 'needs_input', 'done')",
            [],
        )
    })
    .await?;
    let mut info = api::machine_info(&db).await?;
    let agent_env = env::AgentEnv::capture().await;
    let log = events::EventLog::new(db.clone());
    let sessions = sessions::Sessions::new(db.clone(), log.clone(), agent_env.clone());
    let identity = std::sync::Arc::new(identity::Identity::load_or_create(&home)?);

    // Phones connect here (§2.7). SWARN_DEVICE_PORT=0 picks a free port (tests). A port already
    // in use (a second daemon) leaves this one without a device listener rather than not running.
    let port: u16 = std::env::var("SWARN_DEVICE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8787);
    let device_listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
        Ok(listener) => {
            info.device_port = listener.local_addr().ok().map(|a| a.port());
            Some(listener)
        }
        Err(e) => {
            eprintln!("swarnd: no device listener on port {port}: {e}");
            None
        }
    };
    let mdns = info
        .device_port
        .and_then(|port| discovery::register_mdns(&info.machine_name, port));
    let devices = devices::Devices::new(
        db.clone(),
        log.clone(),
        sessions.clone(),
        identity,
        info.clone(),
    );
    if let Some(listener) = device_listener {
        tokio::spawn(devices.clone().listen(listener));
    }
    let api = api::Api {
        db,
        info,
        home: home.clone(),
        log,
        sessions,
        env: agent_env,
        devices,
        mdns_instance: mdns.as_ref().map(|(_, instance)| instance.clone()),
    };

    tokio::spawn(policy::keep_awake(api.db.clone()));

    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    eprintln!(
        "swarnd {} listening on {}",
        api.info.version,
        socket.display()
    );

    let mut terminate = signal(SignalKind::terminate())?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let api = api.clone();
                tokio::spawn(async move {
                    if let Err(e) = api.serve(stream).await {
                        eprintln!("swarnd: client connection: {e}");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = terminate.recv() => break,
        }
    }
    std::fs::remove_file(&socket)?;
    Ok(())
}
