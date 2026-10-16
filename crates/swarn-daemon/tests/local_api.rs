//! Starts a real `swarnd` in a temp SWARN_HOME and talks to it over the socket, the way the desktop
//! does.

use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use swarn_core::local_api::{CallError, DaemonInfo, RpcError, call, socket_path};
use swarn_core::schema::{Repo, Workspace};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[test]
fn desktop_can_list_what_was_added() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary, so nothing else reads the environment concurrently.
    unsafe { std::env::set_var("SWARN_HOME", home.path()) };
    let _daemon = Daemon(Command::new(env!("CARGO_BIN_EXE_swarnd")).spawn().unwrap());

    let deadline = Instant::now() + Duration::from_secs(10);
    let info = loop {
        match call::<DaemonInfo>("daemon.info", json!({})) {
            Ok(info) => break info,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("daemon never came up: {e}"),
        }
    };
    assert_eq!((info.schema_min, info.schema_max), (1, 1));
    assert_eq!(
        std::fs::metadata(socket_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let repo_dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(repo_dir.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q", "-b", "trunk"]);
    let path = repo_dir.path().to_str().unwrap();

    let added: Repo = call("repo.add", json!({ "path": path })).unwrap();
    assert_eq!(added.default_branch, "trunk");
    let again: Repo = call("repo.add", json!({ "path": path })).unwrap();
    assert_eq!(again.id, added.id, "adding twice returns the same repo");

    let listed: Vec<Repo> = call("repo.list", json!({})).unwrap();
    assert_eq!(listed, vec![added]);

    let not_a_repo = tempfile::tempdir().unwrap();
    match call::<Value>("repo.add", json!({ "path": not_a_repo.path() })) {
        Err(CallError::Rpc(e)) => assert_eq!(e.code, RpcError::DAEMON),
        other => panic!("expected a daemon error, got {other:?}"),
    }
    match call::<Value>("no.such.method", json!({})) {
        Err(CallError::Rpc(e)) => assert_eq!(e.code, RpcError::METHOD_NOT_FOUND),
        other => panic!("expected method not found, got {other:?}"),
    }
    // A workspace's diff covers both a committed edit and a new file nobody staged.
    std::fs::write(repo_dir.path().join("a.txt"), "one\n").unwrap();
    git(&["add", "a.txt"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "-m",
        "init",
    ]);
    let ws: Workspace = call(
        "workspace.create",
        json!({ "repo_id": listed[0].id, "name": "review" }),
    )
    .unwrap();
    let wt = std::path::Path::new(&ws.worktree_path);
    std::fs::write(wt.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(wt.join("new.txt"), "fresh\n").unwrap();
    let diff: Value = call("diff.get", json!({ "workspace_id": ws.id })).unwrap();
    let diff = diff["diff"].as_str().unwrap();
    assert!(diff.contains("+two"), "{diff}");
    assert!(
        diff.contains("+fresh") && diff.contains("new.txt"),
        "{diff}"
    );

    // Reusing a name is refused in plain words and leaves no half-made workspace behind.
    match call::<Value>(
        "workspace.create",
        json!({ "repo_id": listed[0].id, "name": "review" }),
    ) {
        Err(CallError::Rpc(e)) => assert!(e.message.contains("already exists"), "{}", e.message),
        other => panic!("expected a refusal, got {other:?}"),
    }
    let all: Vec<Workspace> = call("workspace.list", json!({})).unwrap();
    assert_eq!(all.len(), 1);
}
