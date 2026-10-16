//! A stand-in for the iPhone app, for working on pairing and approvals before the real one exists.
//! Opens a pairing window on the local daemon (or takes a swarn://pair URL), pairs over a real
//! sealed WebSocket, then prints every event. With --approve it answers each permission request
//! with a signed "allow once".
//!
//!   SWARN_HOME=… swarn-fake-phone [--approve] [--name "Test iPhone"] [swarn://pair?…]

use std::net::TcpStream;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde_json::{Value, json};
use swarn_core::crypto::{self, Purpose, SigningKey};
use swarn_core::local_api::call;
use swarn_core::schema::{
    Approval, DaemonMessage, Decision, DeviceMessage, EventKind, PairRequest, Scope, Wire,
};
use tungstenite::{Message, stream::MaybeTlsStream};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let approve = args.iter().any(|a| a == "--approve");
    let name = args
        .iter()
        .position(|a| a == "--name")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or("Fake iPhone".into());
    let url = match args.iter().find(|a| a.starts_with("swarn://")) {
        Some(url) => url.clone(),
        None => call::<Value>("device.pair_window", json!({})).expect("daemon reachable")["url"]
            .as_str()
            .unwrap()
            .to_owned(),
    };
    let query = |key: &str| {
        url.split(['?', '&'])
            .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
            .map(percent_decode)
            .unwrap_or_default()
    };
    // The QR's candidates are kind~host~port; a fake phone on the same Mac takes the port and uses
    // localhost.
    let port = query("c")
        .split(',')
        .filter_map(|c| c.rsplit('~').next()?.parse::<u16>().ok())
        .next()
        .unwrap_or(8787);
    let daemon_x25519: [u8; 32] = B64.decode(query("sk")).unwrap().try_into().unwrap();
    let daemon_signing = B64.decode(query("vk")).unwrap();

    let (secret, public) = crypto::x25519_generate();
    let key = SigningKey::P256(crypto::p256_generate());
    let device_id = format!("fake-{}", B64.encode(crypto::random_bytes::<6>()));
    let socket = TcpStream::connect(("127.0.0.1", port)).expect("device listener reachable");
    let (mut ws, _) = tungstenite::client(
        format!("ws://127.0.0.1:{port}/"),
        MaybeTlsStream::Plain(socket),
    )
    .unwrap();
    let mut ctr = 0u64;
    let mut send = |ws: &mut tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
                    msg: DeviceMessage| {
        ctr += 1;
        let plain = serde_json::to_vec(&Wire { ctr, msg }).unwrap();
        let frame = crypto::seal(&plain, &daemon_x25519, Purpose::Session, &key).unwrap();
        ws.send(Message::binary(crypto::encode(&frame).unwrap()))
            .unwrap();
    };

    let x25519_b64 = B64.encode(public);
    let pw = query("pw");
    let proof = key.sign(&crypto::pairing_signing_input(&pw, &device_id, &x25519_b64));
    send(
        &mut ws,
        DeviceMessage::PairRequest(PairRequest {
            device_id: device_id.clone(),
            name: name.clone(),
            platform: "ios".into(),
            x25519_pub: x25519_b64,
            p256_pub: B64.encode(key.public()),
            push_token: None,
            pw,
            proof: B64.encode(proof),
        }),
    );
    send(
        &mut ws,
        DeviceMessage::Hello {
            schema_min: 1,
            schema_max: 1,
            device_id: device_id.clone(),
            app_version: "fake".into(),
        },
    );
    send(
        &mut ws,
        DeviceMessage::Subscribe {
            scope: Scope::All,
            after_seq: 0,
        },
    );

    loop {
        let Ok(Message::Binary(bytes)) = ws.read() else {
            println!("disconnected");
            return;
        };
        let frame = crypto::decode(&bytes).unwrap();
        let (from, plain) = crypto::open(&frame, &secret, Purpose::Session).unwrap();
        assert_eq!(
            from, daemon_signing,
            "frame not signed by the daemon from the QR code"
        );
        let msg = serde_json::from_slice::<Wire<DaemonMessage>>(&plain)
            .unwrap()
            .msg;
        match &msg {
            DaemonMessage::PairAck(ack) => {
                println!("paired with {} as {name} ({device_id})", ack.machine_name)
            }
            DaemonMessage::Event(e) if e.kind == EventKind::AgentMessageDelta => {
                println!(
                    "[{}] text: {}",
                    e.seq,
                    e.payload["text"]
                        .as_str()
                        .unwrap_or_default()
                        .replace('\n', " ")
                )
            }
            DaemonMessage::Event(e) => println!("[{}] {:?} {}", e.seq, e.kind, e.payload),
            other => println!("{other:?}"),
        }
        if let DaemonMessage::Event(e) = msg
            && approve
            && e.kind == EventKind::PermissionRequest
        {
            let (request_id, nonce) = (
                e.payload["request_id"].as_str().unwrap(),
                e.payload["nonce"].as_str().unwrap(),
            );
            let signed_at = now_rfc3339();
            let input = crypto::approval_signing_input(
                request_id,
                nonce,
                "allow_once",
                &device_id,
                &signed_at,
            );
            let sig = B64.encode(key.sign(&input));
            send(
                &mut ws,
                DeviceMessage::Approve(Approval {
                    request_id: request_id.into(),
                    nonce: nonce.into(),
                    decision: Decision::AllowOnce,
                    device_id: device_id.clone(),
                    signed_at,
                    sig,
                }),
            );
            println!("approved {request_id}");
        }
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// UTC now as RFC 3339, without a date library (Howard Hinnant's civil_from_days).
fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
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
