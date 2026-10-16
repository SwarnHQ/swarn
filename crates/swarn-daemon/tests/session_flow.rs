//! A whole session through the real daemon and socket, with a scripted fake `claude`: permission
//! request, a follow-up held while it is open, local approval, replay refused, and the order of
//! events a subscriber sees.

use std::process::{Child, Command};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use swarn_core::local_api::{CallError, DaemonInfo, call, subscribe};
use swarn_core::schema::{Event, EventKind, Repo, Session, Status, Workspace};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[test]
fn approval_unblocks_the_agent_and_queued_text_follows_it() {
    let home = tempfile::tempdir().unwrap();
    let fake = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake_claude.py");
    // SAFETY: the only test in this binary, so nothing else reads the environment concurrently.
    unsafe { std::env::set_var("SWARN_HOME", home.path()) };
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_swarnd"))
            .env("SWARN_CLAUDE_BIN", fake)
            .spawn()
            .unwrap(),
    );
    wait_for(|| call::<DaemonInfo>("daemon.info", json!({})).is_ok());

    let repo_dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-C"])
            .arg(repo_dir.path())
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "init"]);
    let repo: Repo = call("repo.add", json!({ "path": repo_dir.path() })).unwrap();
    let ws: Workspace = call(
        "workspace.create",
        json!({ "repo_id": repo.id, "name": "flow" }),
    )
    .unwrap();

    let (tx, events) = mpsc::channel::<Event>();
    std::thread::spawn(move || subscribe(0, |e| tx.send(e).is_ok()));

    let session: Session = call(
        "session.start",
        json!({ "workspace_id": ws.id, "prompt": "write it" }),
    )
    .unwrap();
    let mut seen = Vec::new();
    let request = next_of(&events, &mut seen, EventKind::PermissionRequest);
    let request_id = request.payload["request_id"].as_str().unwrap().to_owned();
    assert_eq!(request.payload["risk"], "write");
    assert_eq!(status(&session.id), Status::NeedsInput);

    let held: Value = call(
        "session.prompt",
        json!({ "session_id": session.id, "text": "and then?" }),
    )
    .unwrap();
    assert_eq!(
        held["delivery"], "queued",
        "text waits while a permission is open"
    );

    let _: Value = call(
        "approval.resolve",
        json!({ "request_id": request_id, "decision": "allow_once" }),
    )
    .unwrap();
    match call::<Value>(
        "approval.resolve",
        json!({ "request_id": request_id, "decision": "allow_once" }),
    ) {
        Err(CallError::Rpc(_)) => {}
        other => panic!("a replayed approval must be refused, got {other:?}"),
    }

    // The queued text goes out only after the decision, and the agent answers it.
    let echo = loop {
        let e = next_of(&events, &mut seen, EventKind::AgentMessageDelta);
        if e.payload["text"] == "echo: and then?" {
            break e;
        }
    };
    let position = |kind: EventKind, pred: &dyn Fn(&Event) -> bool| {
        seen.iter().position(|e| e.kind == kind && pred(e)).unwrap()
    };
    let resolved = position(EventKind::PermissionResolved, &|_| true);
    let queued = position(EventKind::UserMessage, &|e| {
        e.payload["text"] == "and then?"
    });
    assert!(
        resolved < queued,
        "queued text was sent before the decision"
    );
    assert!(
        seen.windows(2).all(|w| w[0].seq < w[1].seq),
        "events arrive in seq order"
    );
    assert!(echo.seq > seen[queued].seq);

    next_of(&events, &mut seen, EventKind::AgentMessageEnd);
    wait_for(|| status(&session.id) == Status::Done);
}

fn status(session_id: &str) -> Status {
    let sessions: Vec<Session> = call("session.list", json!({})).unwrap();
    sessions
        .into_iter()
        .find(|s| s.id == session_id)
        .unwrap()
        .status
}

/// Next event of `kind`, keeping everything received on the way in `seen`.
fn next_of(events: &mpsc::Receiver<Event>, seen: &mut Vec<Event>, kind: EventKind) -> Event {
    loop {
        let e = events
            .recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|_| {
                panic!(
                    "no {kind:?} event; saw {:?}",
                    seen.iter().map(|e| e.kind).collect::<Vec<_>>()
                )
            });
        seen.push(e.clone());
        if e.kind == kind {
            return e;
        }
    }
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(50));
    }
}
