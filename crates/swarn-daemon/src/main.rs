//! `swarnd`: owns agents, worktrees, the event log and pairings. The desktop only renders what
//! this process says.
//!
//! `swarnd`                       run the daemon in the foreground
//! `swarnd call <method> [json]`  send one JSON-RPC request to a running daemon and print the result

mod api;
mod db;

use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::process::ExitCode;

use serde_json::Value;
use swarn_core::local_api::{self, swarn_home};
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run(),
        Some("call") if args.len() >= 2 => call(&args[1], args.get(2).map(String::as_str)),
        _ => {
            eprintln!("usage: swarnd [call <method> [params-json]]");
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
    let info = api::machine_info(&db).await?;
    let api = api::Api { db, info };

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
