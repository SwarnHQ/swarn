//! Paired devices (§2.5–2.7): the pairing window, the device listener and each device's session.
//!
//! One TCP port serves two things: `GET /healthz`, the only unauthenticated endpoint, and a
//! WebSocket carrying CBOR `SealedFrame`s. Every frame is sealed to the receiver's static key and
//! signed by its sender, so someone on the same Wi-Fi sees only sizes and timing (§1.2).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use futures_util::{SinkExt, StreamExt};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use swarn_core::crypto::{self, Purpose};
use swarn_core::local_api::DaemonInfo;
use swarn_core::schema::{
    DaemonMessage, DeviceMessage, Event, EventKind, PairAck, PairRequest, SCHEMA_MAX, SCHEMA_MIN,
    Scope, Wire,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::db::Db;
use crate::events::EventLog;
use crate::identity::Identity;
use crate::sessions::Sessions;

/// §2.5: a pairing token is good for 120 seconds and one use.
const PAIR_WINDOW: Duration = Duration::from_secs(120);
const STRANGER_GRACE: Duration = Duration::from_secs(30);
const MAX_FRAME: usize = 300 * 1024;

/// §2.1 ceiling: at most this often per session, streamed text is batched into one frame.
const DELTA_BATCH: Duration = Duration::from_millis(100);

pub struct Devices {
    pub db: Db,
    pub log: EventLog,
    pub sessions: Arc<Sessions>,
    pub identity: Arc<Identity>,
    pub info: DaemonInfo,
    windows: Mutex<HashMap<String, Instant>>,
    /// Open connections by device id, so revoking a device also hangs up on it.
    connected: Mutex<HashMap<String, tokio::task::AbortHandle>>,
    /// Outgoing frame counter: starts at the wall clock in microseconds, so it keeps rising across
    /// daemon restarts and a device can drop anything it has seen before.
    ctr: AtomicU64,
}

#[derive(Clone)]
struct Device {
    id: String,
    x25519: [u8; 32],
}

impl Devices {
    pub fn new(
        db: Db,
        log: EventLog,
        sessions: Arc<Sessions>,
        identity: Arc<Identity>,
        info: DaemonInfo,
    ) -> Arc<Self> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        Arc::new(Self {
            db,
            log,
            sessions,
            identity,
            info,
            windows: Mutex::new(HashMap::new()),
            connected: Mutex::new(HashMap::new()),
            ctr: AtomicU64::new(now),
        })
    }

    /// Opens a pairing window and returns the `swarn://pair` URL the QR code carries (§2.5).
    pub fn pair_window(
        &self,
        candidates: &[swarn_core::schema::Candidate],
        gateway: &str,
    ) -> Value {
        let pw = B64.encode(crypto::random_bytes::<16>());
        let mut windows = self.windows.lock().unwrap();
        windows.retain(|_, expiry| *expiry > Instant::now());
        windows.insert(pw.clone(), Instant::now() + PAIR_WINDOW);
        drop(windows);
        // Compact on purpose: every byte makes the QR code denser and harder to scan.
        let c = candidates
            .iter()
            .map(|c| format!("{}~{}~{}", c.kind, c.host, c.port))
            .collect::<Vec<_>>()
            .join(",");
        let url = format!(
            "swarn://pair?v=1&m={}&n={}&sk={}&vk={}&pw={}&c={}&g={}",
            self.info.machine_id,
            percent(&self.info.machine_name),
            B64.encode(self.identity.x25519_public),
            B64.encode(self.identity.signing.public()),
            pw,
            percent(&c),
            percent(gateway),
        );
        json!({ "url": url, "expires_in_secs": PAIR_WINDOW.as_secs(), "candidates": candidates })
    }

    pub async fn list(&self) -> Result<Value, String> {
        let connected: Vec<String> = self.connected.lock().unwrap().keys().cloned().collect();
        self.db
            .call(move |conn| {
                conn.prepare("SELECT id, name, platform, paired_at, revoked_at FROM device ORDER BY paired_at")?
                    .query_map([], |r| {
                        let id: String = r.get(0)?;
                        Ok(json!({
                            "connected": connected.contains(&id),
                            "id": id, "name": r.get::<_, String>(1)?, "platform": r.get::<_, String>(2)?,
                            "paired_at": r.get::<_, String>(3)?, "revoked_at": r.get::<_, Option<String>>(4)?,
                        }))
                    })?
                    .collect::<rusqlite::Result<Vec<Value>>>()
            })
            .await
            .map(Value::Array)
            .map_err(|e| e.to_string())
    }

    /// Revoking deletes nothing already on the phone (§2.5), but the device can't connect, approve
    /// or receive anything new.
    pub async fn revoke(&self, device_id: String) -> Result<(), String> {
        if let Some(conn) = self.connected.lock().unwrap().remove(&device_id) {
            conn.abort();
        }
        let id = device_id.clone();
        let changed = self
            .db
            .call(move |conn| {
                conn.execute(
                    "UPDATE device SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND revoked_at IS NULL",
                    [id],
                )
            })
            .await
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            Err(format!("no active device {device_id}"))
        } else {
            Ok(())
        }
    }

    pub async fn listen(self: Arc<Self>, listener: TcpListener) {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let this = Arc::clone(&self);
            // The task learns its own abort handle, so revoking the device can hang up on it.
            let (handle_tx, handle_rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let handle = handle_rx.await.ok();
                if let Err(e) = this.connection(stream, handle).await {
                    eprintln!("swarnd: device connection: {e}");
                }
            });
            let _ = handle_tx.send(task.abort_handle());
        }
    }

    async fn connection(
        self: Arc<Self>,
        stream: TcpStream,
        handle: Option<tokio::task::AbortHandle>,
    ) -> Result<(), String> {
        let mut head = [0u8; 12];
        let n = stream.peek(&mut head).await.map_err(|e| e.to_string())?;
        if &head[..n] == b"GET /healthz" {
            return self.healthz(stream).await;
        }
        // The largest padded frame is 256 KiB; anything bigger is not ours, and a stranger
        // shouldn't get to make the daemon buffer tungstenite's 64 MiB default.
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        let ws = tokio_tungstenite::accept_async_with_config(stream, Some(config))
            .await
            .map_err(|e| e.to_string())?;
        // A connection has this long to pair or prove it's a known device, then it's dropped.
        let deadline = tokio::time::Instant::now() + STRANGER_GRACE;
        let (mut sink, mut source) = ws.split();

        // One writer per connection; everything outgoing goes through it, sealed to the device.
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<(Device, DaemonMessage)>();
        let this = Arc::clone(&self);
        let _writer = AbortOnDrop(tokio::spawn(async move {
            while let Some((device, msg)) = out_rx.recv().await {
                let ctr = this.ctr.fetch_add(1, Ordering::SeqCst) + 1;
                let Ok(plain) = serde_json::to_vec(&Wire { ctr, msg }) else {
                    continue;
                };
                let frame = crypto::seal(
                    &plain,
                    &device.x25519,
                    Purpose::Session,
                    &this.identity.signing,
                );
                let Ok(bytes) = frame.and_then(|f| crypto::encode(&f)) else {
                    continue;
                };
                if sink.send(Message::binary(bytes)).await.is_err() {
                    break;
                }
            }
        }));

        let mut device: Option<Device> = None;
        let mut stream_task: Option<AbortOnDrop> = None;
        loop {
            let next = if device.is_some() {
                source.next().await
            } else {
                tokio::time::timeout_at(deadline, source.next())
                    .await
                    .ok()
                    .flatten()
            };
            let Some(Ok(message)) = next else { break };
            let Message::Binary(bytes) = message else {
                continue;
            };
            let Ok(frame) = crypto::decode(&bytes) else {
                continue;
            };
            let Ok((from, plain)) =
                crypto::open(&frame, &self.identity.x25519_secret, Purpose::Session)
            else {
                continue;
            };
            let Ok(wire) = serde_json::from_slice::<Wire<DeviceMessage>>(&plain) else {
                continue;
            };

            let known = self.device_by_key(from.clone()).await;
            match (known, wire.msg) {
                (None, DeviceMessage::PairRequest(request)) => {
                    match self.pair(&from, request).await {
                        Ok(paired) => {
                            let ack = PairAck {
                                machine_id: self.info.machine_id.clone(),
                                machine_name: self.info.machine_name.clone(),
                                schema_min: SCHEMA_MIN,
                                schema_max: SCHEMA_MAX,
                            };
                            let _ = out_tx.send((paired.clone(), DaemonMessage::PairAck(ack)));
                            self.accept_counter(&paired.id, wire.ctr).await;
                            self.register(&paired.id, &handle);
                            device = Some(paired);
                        }
                        Err(e) => eprintln!("swarnd: pairing refused: {e}"),
                    }
                }
                // Strangers get no answer at all.
                (None, _) => {}
                (Some((found, p256)), msg) => {
                    if device.as_ref().is_some_and(|d| d.id != found.id) {
                        break; // one connection, one device
                    }
                    if !self.accept_counter(&found.id, wire.ctr).await {
                        continue; // a replayed or reordered frame
                    }
                    if device.is_none() {
                        self.register(&found.id, &handle);
                        device = Some(found.clone());
                    }
                    let reply = |msg| {
                        let _ = out_tx.send((found.clone(), msg));
                    };
                    match msg {
                        DeviceMessage::Hello {
                            schema_min,
                            schema_max,
                            ..
                        } => reply(hello(&self.info, schema_min, schema_max)),
                        DeviceMessage::Subscribe { scope, after_seq } => {
                            drop(stream_task.take()); // stop the previous subscription first
                            let (this, tx, d) = (Arc::clone(&self), out_tx.clone(), found.clone());
                            stream_task = Some(AbortOnDrop(tokio::spawn(async move {
                                this.stream(d, tx, scope, after_seq).await
                            })));
                        }
                        DeviceMessage::Prompt {
                            session_id, text, ..
                        } => {
                            if let Err(e) = self.sessions.prompt(&session_id, text, None).await {
                                reply(error("prompt_failed", &e));
                            }
                        }
                        DeviceMessage::Approve(approval) => {
                            if let Err(e) = self
                                .sessions
                                .resolve_signed(&approval, &found.id, &p256)
                                .await
                            {
                                reply(error("approval_rejected", &e));
                            }
                        }
                        DeviceMessage::Cancel { session_id } => {
                            if let Err(e) = self.sessions.cancel(&session_id).await {
                                reply(error("cancel_failed", &e));
                            }
                        }
                        DeviceMessage::CreateWorkspace { .. } => reply(error(
                            "not_supported",
                            "Creating workspaces from the phone comes in v0.2",
                        )),
                        DeviceMessage::Ack { .. } => {}
                        DeviceMessage::PairRequest(_) => {
                            reply(error("already_paired", "This device is already paired"))
                        }
                    }
                }
            }
        }

        drop(stream_task);
        if let Some(d) = device {
            self.connected.lock().unwrap().remove(&d.id);
        }
        Ok(())
    }

    fn register(&self, device_id: &str, handle: &Option<tokio::task::AbortHandle>) {
        if let Some(handle) = handle {
            self.connected
                .lock()
                .unwrap()
                .insert(device_id.to_owned(), handle.clone());
        }
    }

    async fn healthz(&self, mut stream: TcpStream) -> Result<(), String> {
        let mut request = [0u8; 1024];
        let _ = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut request)).await;
        let body =
            json!({ "machine_id": self.info.machine_id, "schema": [SCHEMA_MIN, SCHEMA_MAX] })
                .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .map_err(|e| e.to_string())
    }

    /// §2.5 step 4: the token must be open, unexpired and unused, and the P-256 key must sign the
    /// proof. The token burns on success.
    async fn pair(&self, from: &[u8], request: PairRequest) -> Result<Device, String> {
        let p256 = B64.decode(&request.p256_pub).map_err(|_| "bad p256_pub")?;
        if p256 != from {
            return Err("the frame is not signed by the key being paired".into());
        }
        let x25519: [u8; 32] = B64
            .decode(&request.x25519_pub)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or("bad x25519_pub")?;
        let proof = B64.decode(&request.proof).map_err(|_| "bad proof")?;
        crypto::verify(
            &p256,
            &crypto::pairing_signing_input(&request.pw, &request.device_id, &request.x25519_pub),
            &proof,
        )
        .map_err(|_| "the pairing proof does not verify")?;
        {
            let mut windows = self.windows.lock().unwrap();
            match windows.remove(&request.pw) {
                Some(expiry) if expiry > Instant::now() => {}
                _ => return Err("the pairing window is closed or was already used".into()),
            }
        }
        let device = Device {
            id: request.device_id.clone(),
            x25519,
        };
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO device (id, name, platform, x25519_pub, p256_pub, push_token, paired_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                     ON CONFLICT(id) DO UPDATE SET name = ?2, platform = ?3, x25519_pub = ?4, p256_pub = ?5,
                       push_token = ?6, paired_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), revoked_at = NULL, last_ctr = 0",
                    params![request.device_id, request.name, request.platform, x25519.to_vec(), p256, request.push_token],
                )
            })
            .await
            .map_err(|e| e.to_string())?;
        Ok(device)
    }

    /// A known, unrevoked device by its signing key, with that key.
    async fn device_by_key(&self, p256: Vec<u8>) -> Option<(Device, Vec<u8>)> {
        let key = p256.clone();
        let row: Option<(String, Vec<u8>)> = self
            .db
            .call(move |conn| {
                conn.query_row(
                    "SELECT id, x25519_pub FROM device WHERE p256_pub = ?1 AND revoked_at IS NULL",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
            })
            .await
            .ok()
            .flatten();
        let (id, x25519) = row?;
        Some((
            Device {
                id,
                x25519: x25519.try_into().ok()?,
            },
            p256,
        ))
    }

    /// True if `ctr` is above every counter this device has used before, and records it. Stored,
    /// so a frame captured before a daemon restart can't be replayed after one.
    async fn accept_counter(&self, device_id: &str, ctr: u64) -> bool {
        let id = device_id.to_owned();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE device SET last_ctr = ?2 WHERE id = ?1 AND last_ctr < ?2",
                    params![id, ctr as i64],
                )
            })
            .await
            .is_ok_and(|changed| changed == 1)
    }

    /// Snapshot (when asked from the start), then the backlog, then live events, with streamed
    /// text batched per session at most every 100 ms (§2.1).
    async fn stream(
        self: Arc<Self>,
        device: Device,
        tx: mpsc::UnboundedSender<(Device, DaemonMessage)>,
        scope: Scope,
        after_seq: u64,
    ) {
        let wanted = |e: &Event| match &scope {
            Scope::All => true,
            Scope::Sessions(ids) => ids.contains(&e.session_id),
        };
        let send = |msg: DaemonMessage| tx.send((device.clone(), msg)).is_ok();
        let mut live = self.log.subscribe();
        let mut last = after_seq;
        if after_seq == 0 {
            let head = self.log.head().await;
            let (workspaces, sessions) = (
                crate::workspace::list(&self.db).await.unwrap_or_default(),
                self.sessions.list().await.unwrap_or_default(),
            );
            if !send(DaemonMessage::Snapshot {
                workspaces,
                sessions,
                seq: head,
            }) {
                return;
            }
            last = head;
        }
        let mut batch: HashMap<String, Event> = HashMap::new();
        let mut tick = tokio::time::interval(DELTA_BATCH);
        loop {
            for event in self.log.since(last).await.unwrap_or_default() {
                last = event.seq;
                if wanted(&event) && !forward(&mut batch, event, &send) {
                    return;
                }
            }
            loop {
                tokio::select! {
                    event = live.recv() => match event {
                        Ok(event) if event.seq <= last => {}
                        Ok(event) => {
                            last = event.seq;
                            if wanted(&event) && !forward(&mut batch, event, &send) {
                                return;
                            }
                        }
                        Err(RecvError::Lagged(_)) => break,
                        Err(RecvError::Closed) => return,
                    },
                    _ = tick.tick() => {
                        for (_, event) in batch.drain() {
                            if !send(DaemonMessage::Event(event)) {
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Aborts its task when dropped. A connection's helper tasks die with it, including when revoking
/// aborts the connection mid-await, so the socket closes at once.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Streamed text waits in `batch`, merged per session; any other event for that session flushes
/// it first, so order within a session never changes.
fn forward(
    batch: &mut HashMap<String, Event>,
    event: Event,
    send: &impl Fn(DaemonMessage) -> bool,
) -> bool {
    if event.kind == EventKind::AgentMessageDelta {
        match batch.get_mut(&event.session_id) {
            Some(pending) => {
                let more = event.payload["text"].as_str().unwrap_or_default();
                let text = format!(
                    "{}{more}",
                    pending.payload["text"].as_str().unwrap_or_default()
                );
                pending.payload["text"] = json!(text);
                pending.seq = event.seq;
            }
            None => {
                batch.insert(event.session_id.clone(), event);
            }
        }
        return true;
    }
    if let Some(pending) = batch.remove(&event.session_id)
        && !send(DaemonMessage::Event(pending))
    {
        return false;
    }
    send(DaemonMessage::Event(event))
}

/// §2.8: pick the highest schema both sides speak, or say exactly who has to update.
fn hello(info: &DaemonInfo, device_min: u32, device_max: u32) -> DaemonMessage {
    if device_max < SCHEMA_MIN {
        return error("schema_too_old", "Update Swarn on this phone");
    }
    if device_min > SCHEMA_MAX {
        return error("schema_too_new", "This Mac needs an update");
    }
    DaemonMessage::HelloAck {
        schema: device_max.min(SCHEMA_MAX),
        machine_id: info.machine_id.clone(),
        machine_name: info.machine_name.clone(),
        caps: vec![],
    }
}

fn error(code: &str, message: &str) -> DaemonMessage {
    DaemonMessage::Error {
        code: code.into(),
        message: message.into(),
        retryable: false,
    }
}

fn percent(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u64, session: &str, kind: EventKind, text: &str) -> Event {
        Event {
            seq,
            session_id: session.into(),
            kind,
            payload: json!({ "text": text }),
            ts: String::new(),
        }
    }

    #[test]
    fn deltas_merge_per_session_and_never_overtake_other_events() {
        let sent = std::cell::RefCell::new(Vec::new());
        let send = |m: DaemonMessage| {
            if let DaemonMessage::Event(e) = m {
                sent.borrow_mut().push((
                    e.seq,
                    e.payload["text"].as_str().unwrap_or_default().to_owned(),
                ));
            }
            true
        };
        let mut batch = HashMap::new();
        forward(
            &mut batch,
            ev(1, "a", EventKind::AgentMessageDelta, "Hel"),
            &send,
        );
        forward(
            &mut batch,
            ev(2, "b", EventKind::AgentMessageDelta, "other"),
            &send,
        );
        forward(
            &mut batch,
            ev(3, "a", EventKind::AgentMessageDelta, "lo"),
            &send,
        );
        forward(&mut batch, ev(4, "a", EventKind::ToolCall, ""), &send);
        assert_eq!(
            *sent.borrow(),
            [(3, "Hello".to_owned()), (4, String::new())]
        );
        assert_eq!(batch.len(), 1, "session b's text still waits for the tick");
    }

    #[test]
    fn schema_negotiation() {
        let info = DaemonInfo {
            machine_id: "m".into(),
            machine_name: "Mac".into(),
            version: "0".into(),
            schema_min: 1,
            schema_max: 1,
            device_port: None,
        };
        assert!(matches!(
            hello(&info, 1, 3),
            DaemonMessage::HelloAck { schema: 1, .. }
        ));
        assert!(
            matches!(hello(&info, 2, 3), DaemonMessage::Error { code, .. } if code == "schema_too_new")
        );
    }

    #[test]
    fn percent_encoding_keeps_urls_parseable() {
        assert_eq!(percent("Seb's Mac mini"), "Seb%27s%20Mac%20mini");
    }
}
