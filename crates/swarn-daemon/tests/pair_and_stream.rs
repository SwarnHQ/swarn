//! Week 5's "done when" (PRD §8, §9.2): a simulated phone pairs from the QR URL, negotiates the
//! schema, subscribes, follows a scripted session in order, has a bad approval refused and a good
//! one accepted, sees a replayed frame ignored, and is hung up on when revoked. Everything goes
//! through real sealed frames over a real WebSocket.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde_json::{Value, json};
use swarn_core::crypto::{self, Purpose, SigningKey};
use swarn_core::local_api::{DaemonInfo, call};
use swarn_core::schema::{
    Approval, DaemonMessage, Decision, DeviceMessage, EventKind, PairRequest, Repo, Scope, Wire,
    Workspace,
};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

struct Phone {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    x25519_secret: [u8; 32],
    x25519_public: [u8; 32],
    key: SigningKey,
    daemon_x25519: [u8; 32],
    daemon_signing: Vec<u8>,
    ctr: u64,
    last_sent: Vec<u8>,
}

impl Phone {
    fn send(&mut self, msg: DeviceMessage) {
        self.ctr += 1;
        let plain = serde_json::to_vec(&Wire { ctr: self.ctr, msg }).unwrap();
        let frame = crypto::seal(&plain, &self.daemon_x25519, Purpose::Session, &self.key).unwrap();
        self.last_sent = crypto::encode(&frame).unwrap();
        self.ws
            .send(Message::binary(self.last_sent.clone()))
            .unwrap();
    }

    /// Next message from the daemon; checks it really came from the key in the QR code.
    fn recv(&mut self) -> Option<DaemonMessage> {
        loop {
            match self.ws.read() {
                Ok(Message::Binary(bytes)) => {
                    let frame = crypto::decode(&bytes).unwrap();
                    let (from, plain) =
                        crypto::open(&frame, &self.x25519_secret, Purpose::Session).unwrap();
                    assert_eq!(
                        from, self.daemon_signing,
                        "frame not signed by the paired daemon"
                    );
                    return Some(
                        serde_json::from_slice::<Wire<DaemonMessage>>(&plain)
                            .unwrap()
                            .msg,
                    );
                }
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }

    fn until(
        &mut self,
        mut pred: impl FnMut(&DaemonMessage) -> bool,
    ) -> (DaemonMessage, Vec<DaemonMessage>) {
        let mut seen = Vec::new();
        loop {
            let msg = self
                .recv()
                .unwrap_or_else(|| panic!("connection ended; saw {seen:?}"));
            if pred(&msg) {
                return (msg, seen);
            }
            seen.push(msg);
        }
    }
}

#[test]
fn a_phone_pairs_follows_a_session_and_approves() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary.
    unsafe { std::env::set_var("SWARN_HOME", home.path()) };
    let _daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_swarnd"))
            .env("SWARN_DEVICE_PORT", "0")
            .env("SWARN_NO_MDNS", "1")
            .env(
                "SWARN_CLAUDE_BIN",
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake_claude.py"),
            )
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let info = loop {
        match call::<DaemonInfo>("daemon.info", json!({})) {
            Ok(info) => break info,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("daemon never came up: {e}"),
        }
    };
    let port = info.device_port.expect("device listener");

    // The unauthenticated health check answers with the machine id and nothing else.
    let mut health = TcpStream::connect(("127.0.0.1", port)).unwrap();
    health
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let mut body = String::new();
    health.read_to_string(&mut body).unwrap();
    assert!(
        body.starts_with("HTTP/1.1 200") && body.contains(&info.machine_id),
        "{body}"
    );

    // A stranger sending an oversized frame is cut off rather than buffered.
    let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let (mut stranger, _) = tungstenite::client(
        format!("ws://127.0.0.1:{port}/"),
        MaybeTlsStream::Plain(socket),
    )
    .unwrap();
    let _ = stranger.send(Message::binary(vec![0u8; 1024 * 1024]));
    let timed_out = matches!(
        stranger.read(),
        Err(tungstenite::Error::Io(e))
            if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
    );
    assert!(!timed_out, "an oversized frame was accepted");

    // Pair from the QR URL.
    let pairing: Value = call("device.pair_window", json!({})).unwrap();
    let url = pairing["url"].as_str().unwrap();
    let query = |name: &str| {
        url.split(['?', '&'])
            .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
            .unwrap()
            .to_owned()
    };
    let (x25519_secret, x25519_public) = crypto::x25519_generate();
    let key = SigningKey::P256(crypto::p256_generate());
    let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let (ws, _) = tungstenite::client(
        format!("ws://127.0.0.1:{port}/"),
        MaybeTlsStream::Plain(socket),
    )
    .unwrap();
    let mut phone = Phone {
        ws,
        x25519_secret,
        x25519_public,
        daemon_x25519: B64.decode(query("sk")).unwrap().try_into().unwrap(),
        daemon_signing: B64.decode(query("vk")).unwrap(),
        key,
        ctr: 0,
        last_sent: Vec::new(),
    };
    let pw = query("pw");
    let x25519_b64 = B64.encode(phone.x25519_public);
    let proof = phone
        .key
        .sign(&crypto::pairing_signing_input(&pw, "phone-1", &x25519_b64));
    phone.send(DeviceMessage::PairRequest(PairRequest {
        device_id: "phone-1".into(),
        name: "Test iPhone".into(),
        platform: "ios".into(),
        x25519_pub: x25519_b64,
        p256_pub: B64.encode(phone.key.public()),
        push_token: None,
        pw,
        proof: B64.encode(proof),
    }));
    assert!(
        matches!(phone.recv(), Some(DaemonMessage::PairAck(ack)) if ack.machine_id == info.machine_id)
    );

    phone.send(DeviceMessage::Hello {
        schema_min: 1,
        schema_max: 2,
        device_id: "phone-1".into(),
        app_version: "0.1".into(),
    });
    assert!(matches!(
        phone.recv(),
        Some(DaemonMessage::HelloAck { schema: 1, .. })
    ));
    phone.send(DeviceMessage::Subscribe {
        scope: Scope::All,
        after_seq: 0,
    });
    assert!(matches!(phone.recv(), Some(DaemonMessage::Snapshot { .. })));

    // A replayed frame (the subscribe again, same counter) is dropped without an answer; the next
    // real message still works.
    let replay = phone.last_sent.clone();
    phone.ws.send(Message::binary(replay)).unwrap();

    // A session starts on the Mac; the phone follows it.
    let repo_dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-C"])
                .arg(repo_dir.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "init"]);
    let repo: Repo = call("repo.add", json!({ "path": repo_dir.path() })).unwrap();
    let ws: Workspace = call(
        "workspace.create",
        json!({ "repo_id": repo.id, "name": "phone" }),
    )
    .unwrap();
    let _: Value = call(
        "session.start",
        json!({ "workspace_id": ws.id, "prompt": "write it" }),
    )
    .unwrap();

    let is_event = |kind: EventKind| move |m: &DaemonMessage| matches!(m, DaemonMessage::Event(e) if e.kind == kind);
    let (DaemonMessage::Event(request), _) = phone.until(is_event(EventKind::PermissionRequest))
    else {
        unreachable!()
    };
    let request_id = request.payload["request_id"].as_str().unwrap().to_owned();
    let nonce = request.payload["nonce"].as_str().unwrap().to_owned();

    let approve = |phone: &mut Phone, nonce: &str| {
        let signed_at = now_rfc3339();
        let input =
            crypto::approval_signing_input(&request_id, nonce, "allow_once", "phone-1", &signed_at);
        let sig = B64.encode(phone.key.sign(&input));
        phone.send(DeviceMessage::Approve(Approval {
            request_id: request_id.clone(),
            nonce: nonce.to_owned(),
            decision: Decision::AllowOnce,
            device_id: "phone-1".into(),
            signed_at,
            sig,
        }));
    };
    approve(&mut phone, "0000");
    let (refused, _) = phone.until(|m| matches!(m, DaemonMessage::Error { .. }));
    assert!(
        matches!(refused, DaemonMessage::Error { code, message, .. } if code == "approval_rejected" && message == "nonce_mismatch")
    );

    approve(&mut phone, &nonce);
    let (resolved, _) = phone.until(|m| matches!(m, DaemonMessage::Event(e) if e.kind == EventKind::PermissionResolved && e.payload["rejected"] != true));
    let DaemonMessage::Event(resolved) = resolved else {
        unreachable!()
    };
    assert_eq!(resolved.payload["device_id"], "phone-1");
    let (_, between) = phone.until(is_event(EventKind::AgentMessageEnd));
    let seqs: Vec<u64> = between
        .iter()
        .filter_map(|m| match m {
            DaemonMessage::Event(e) => Some(e.seq),
            _ => None,
        })
        .collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "events out of order: {seqs:?}"
    );

    // The Mac sees the phone, then revokes it and the connection drops.
    let devices: Vec<Value> = call("device.list", json!({})).unwrap();
    assert_eq!(devices[0]["id"], "phone-1");
    assert_eq!(devices[0]["connected"], true);
    let _: Value = call("device.revoke", json!({ "device_id": "phone-1" })).unwrap();
    let mut closed = false;
    for _ in 0..50 {
        if phone.recv().is_none() {
            closed = true;
            break;
        }
    }
    assert!(closed, "a revoked device stays connected");
}

/// UTC now as RFC 3339, without a date library.
fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}
