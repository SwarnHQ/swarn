//! Running agent sessions (§4.2). The registry owns each agent process, its pending permissions
//! and any text waiting to be sent; the per-agent protocol lives in `claude.rs` and `codex.rs`.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde_json::{Value, json};
use swarn_core::crypto;
use swarn_core::local_api::ApprovalMode;
use swarn_core::schema::{Approval, Decision, EventKind, Session};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::claude::Claude;
use crate::codex::Codex;
use crate::db::Db;
use crate::env::AgentEnv;
use crate::events::EventLog;

/// What a driver wants done after reading one line from its agent.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// The agent's own session or thread id, kept for resume.
    AgentSession {
        id: String,
        payload: Value,
    },
    Emit(EventKind, Value),
    Permission(Box<Ask>),
    /// A protocol line to write back, such as the Codex handshake.
    Send(String),
    /// The agent can take the next prompt.
    Idle,
}

#[cfg(test)]
impl Action {
    /// A one-word name per action, for the transcript contract tests.
    pub fn describe(&self) -> String {
        match self {
            Action::AgentSession { .. } => "agent_session".to_owned(),
            Action::Permission(ask) => format!("permission: {}", ask.summary),
            Action::Emit(kind, _) => serde_json::to_value(kind)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned(),
            Action::Send(_) => "send".to_owned(),
            Action::Idle => "idle".to_owned(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Ask {
    pub reply: Reply,
    pub tool: String,
    pub summary: String,
    pub risk: &'static str,
    pub detail: Value,
}

/// What the driver needs to answer a permission request later.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Claude {
        request_id: String,
        input: Value,
        suggestions: Value,
    },
    Codex {
        id: Value,
    },
}

pub enum Driver {
    Claude(Claude),
    Codex(Box<Codex>),
}

impl Driver {
    pub fn for_agent(agent: &str, mode: ApprovalMode) -> Option<Self> {
        match agent {
            "claude-code" => {
                let mut claude = Claude::default();
                claude.mode = mode;
                Some(Self::Claude(claude))
            }
            "codex" => {
                let mut codex = Codex::default();
                codex.mode = mode;
                Some(Self::Codex(Box::new(codex)))
            }
            _ => None,
        }
    }

    fn agent(&self) -> &'static str {
        match self {
            Self::Claude(_) => "claude-code",
            Self::Codex(_) => "codex",
        }
    }

    /// `SWARN_CLAUDE_BIN` / `SWARN_CODEX_BIN` pin a specific binary (tests use scripted fakes);
    /// otherwise the user's own CLI from the login shell's PATH.
    fn binary(&self, env: &AgentEnv) -> Option<PathBuf> {
        let (var, program) = match self {
            Self::Claude(_) => ("SWARN_CLAUDE_BIN", "claude"),
            Self::Codex(_) => ("SWARN_CODEX_BIN", "codex"),
        };
        std::env::var_os(var)
            .map(PathBuf::from)
            .or_else(|| env.which(program))
    }

    fn args(&self, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
        match self {
            Self::Claude(c) => c.args(model, effort),
            Self::Codex(_) => vec!["app-server".into()],
        }
    }

    fn start(&mut self, cwd: &str, model: Option<String>, effort: Option<String>) -> Vec<String> {
        match self {
            Self::Claude(_) => vec![],
            Self::Codex(c) => c.start(cwd, model, effort),
        }
    }

    fn ready(&self) -> bool {
        match self {
            Self::Claude(_) => true,
            Self::Codex(c) => c.ready(),
        }
    }

    fn prompt(&mut self, text: &str) -> String {
        match self {
            Self::Claude(_) => Claude::prompt(text),
            Self::Codex(c) => c.prompt(text),
        }
    }

    fn answer(&mut self, reply: &Reply, decision: Decision, reason: Option<&str>) -> String {
        match (self, reply) {
            (
                Self::Claude(_),
                Reply::Claude {
                    request_id,
                    input,
                    suggestions,
                },
            ) => Claude::answer(request_id, input, suggestions, decision, reason),
            (Self::Codex(_), Reply::Codex { id }) => Codex::answer(id, decision, reason),
            _ => unreachable!("a reply always comes from its own driver"),
        }
    }

    fn interrupt(&mut self) -> Option<String> {
        match self {
            Self::Claude(c) => Some(c.interrupt()),
            Self::Codex(c) => c.interrupt(),
        }
    }

    fn translate(&mut self, msg: &Value) -> Vec<Action> {
        match self {
            Self::Claude(c) => c.translate(msg),
            Self::Codex(c) => c.translate(msg),
        }
    }
}

struct Live {
    driver: Driver,
    stdin: mpsc::UnboundedSender<String>,
    pending: HashMap<String, Reply>,
    /// Text that has to wait: a permission is open (§2.6), or the agent is mid-turn and cannot
    /// take another prompt yet (Codex), or it is still starting.
    queued: VecDeque<String>,
}

/// ponytail: one std Mutex over a small map, held only for map edits and never across an await;
/// a session_manager task would be the upgrade if it ever shows in a profile.
pub struct Sessions {
    live: Mutex<HashMap<String, Live>>,
    db: Db,
    log: EventLog,
    env: AgentEnv,
}

impl Sessions {
    pub fn new(db: Db, log: EventLog, env: AgentEnv) -> Arc<Self> {
        Arc::new(Self {
            live: Mutex::new(HashMap::new()),
            db,
            log,
            env,
        })
    }

    pub async fn start(
        self: &Arc<Self>,
        mut driver: Driver,
        workspace_id: String,
        worktree: String,
        prompt: String,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<Session, String> {
        let agent = driver.agent();
        let binary = driver
            .binary(&self.env)
            .ok_or_else(|| format!("{agent} was not found on your login shell's PATH"))?;
        // Spawn before the row exists, so a binary that won't start leaves no session stuck in
        // "starting".
        let mut child = Command::new(binary)
            .args(driver.args(model.as_deref(), effort.as_deref()))
            .current_dir(&worktree)
            .env_clear()
            .envs(&self.env.vars)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                format!("Couldn't start {agent}: {e}. Setup & checks shows what's missing.")
            })?;
        let pid = child.id();
        let id = ulid::Ulid::generate().to_string();
        let row_id = id.clone();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO session (id, workspace_id, agent, tier, status, pid, started_at, last_change_at)
                     VALUES (?1, ?2, ?3, 'native', 'starting', ?4,
                             strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                    params![row_id, workspace_id, agent, pid],
                )
            })
            .await
            .map_err(|e| e.to_string())?;

        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        let mut stdin = child.stdin.take().expect("piped");
        tokio::spawn(async move {
            while let Some(mut line) = stdin_rx.recv().await {
                line.push('\n');
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });
        let stderr = child.stderr.take().expect("piped");
        let tag = id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                eprintln!("{agent}[{tag}]: {line}");
            }
        });

        for line in driver.start(&worktree, model, effort) {
            let _ = stdin_tx.send(line);
        }
        let live = Live {
            driver,
            stdin: stdin_tx,
            pending: HashMap::new(),
            queued: VecDeque::from([prompt]),
        };
        self.live.lock().unwrap().insert(id.clone(), live);
        self.flush(&id).await;

        let stdout = child.stdout.take().expect("piped");
        let this = Arc::clone(self);
        let session_id = id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut recorded = false;
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let actions = match this.live.lock().unwrap().get_mut(&session_id) {
                    Some(live) => live.driver.translate(&msg),
                    None => break,
                };
                for action in actions {
                    this.apply(&session_id, action, &mut recorded).await;
                }
            }
            let status = child.wait().await;
            this.live.lock().unwrap().remove(&session_id);
            let code = status.ok().and_then(|s| s.code());
            if code != Some(0) {
                let message = json!({ "message": format!("{agent} exited with {code:?}") });
                this.log
                    .append(&session_id, EventKind::Error, message, 0)
                    .await;
            }
            this.log
                .append(
                    &session_id,
                    EventKind::SessionEnded,
                    json!({ "exit_code": code }),
                    0,
                )
                .await;
        });

        self.get(&id).await
    }

    async fn apply(self: &Arc<Self>, session_id: &str, action: Action, recorded: &mut bool) {
        match action {
            Action::AgentSession { id, payload } => {
                if std::mem::replace(recorded, true) {
                    return;
                }
                let sid = session_id.to_owned();
                let _ = self
                    .db
                    .call(move |conn| {
                        conn.execute(
                            "UPDATE session SET acp_session = ?2 WHERE id = ?1",
                            params![sid, id],
                        )
                    })
                    .await;
                self.log
                    .append(
                        session_id,
                        EventKind::SessionStarted,
                        payload,
                        self.pending_count(session_id),
                    )
                    .await;
            }
            Action::Emit(kind, payload) => {
                self.log
                    .append(session_id, kind, payload, self.pending_count(session_id))
                    .await;
            }
            Action::Permission(ask) => self.open_permission(session_id, *ask).await,
            Action::Send(line) => {
                if let Some(live) = self.live.lock().unwrap().get(session_id) {
                    let _ = live.stdin.send(line);
                }
            }
            Action::Idle => self.flush(session_id).await,
        }
    }

    async fn open_permission(self: &Arc<Self>, session_id: &str, ask: Ask) {
        // Neither CLI times a prompt out on its own, so the expiry is ours (§2.6).
        let expiry = Duration::from_secs(crate::policy::load().approvals.default_expiry_secs);
        let request_id = ulid::Ulid::generate().to_string();
        let nonce = random_hex();
        let (rid, sid, tool, risk, n) = (
            request_id.clone(),
            session_id.to_owned(),
            ask.tool.clone(),
            ask.risk,
            nonce.clone(),
        );
        let expires_at: String = match self
            .db
            .call(move |conn| {
                conn.query_row(
                    "INSERT INTO approval (request_id, session_id, tool, risk, nonce, expires_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?6))
                     RETURNING expires_at",
                    params![
                        rid,
                        sid,
                        tool,
                        risk,
                        n,
                        format!("+{} seconds", expiry.as_secs())
                    ],
                    |r| r.get(0),
                )
            })
            .await
        {
            Ok(at) => at,
            Err(e) => {
                eprintln!("swarnd: could not record approval: {e}");
                return;
            }
        };

        let pending = {
            let mut live = self.live.lock().unwrap();
            let Some(session) = live.get_mut(session_id) else {
                return;
            };
            session.pending.insert(request_id.clone(), ask.reply);
            session.pending.len()
        };
        let payload = json!({
            "request_id": request_id, "session_id": session_id, "tool": ask.tool, "summary": ask.summary,
            "detail": ask.detail, "risk": ask.risk, "nonce": nonce, "expires_at": expires_at,
        });
        self.log
            .append(session_id, EventKind::PermissionRequest, payload, pending)
            .await;

        let this = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(expiry).await;
            let _ = this
                .resolve(&request_id, Decision::Deny, Some("expired"))
                .await;
        });
    }

    /// Answers a pending permission. Unknown or already-resolved ids are refused, which is also what
    /// stops a replayed approval (§2.6). `reason` is set for daemon-made decisions such as expiry.
    pub async fn resolve(
        &self,
        request_id: &str,
        decision: Decision,
        reason: Option<&str>,
    ) -> Result<(), String> {
        self.resolve_as(request_id, decision, reason, "local").await
    }

    /// `device` is who decided: "local" for the desktop and daemon, else a paired device's id.
    async fn resolve_as(
        &self,
        request_id: &str,
        decision: Decision,
        reason: Option<&str>,
        device: &str,
    ) -> Result<(), String> {
        let (session_id, remaining) = {
            let mut live = self.live.lock().unwrap();
            let (session_id, session) = live
                .iter_mut()
                .find(|(_, s)| s.pending.contains_key(request_id))
                .ok_or_else(|| format!("no open permission request {request_id}"))?;
            let reply = session.pending.remove(request_id).expect("found above");
            let line = session.driver.answer(&reply, decision, reason);
            let _ = session.stdin.send(line);
            (session_id.clone(), session.pending.len())
        };

        let decision_str = serde_json::to_value(decision).expect("Decision serialises");
        let (rid, d, who) = (
            request_id.to_owned(),
            decision_str.as_str().unwrap_or("deny").to_owned(),
            device.to_owned(),
        );
        let _ = self
            .db
            .call(move |conn| {
                conn.execute(
                    "UPDATE approval SET resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), decision = ?2, device_id = ?3
                     WHERE request_id = ?1",
                    params![rid, d, who],
                )
            })
            .await;
        let mut payload =
            json!({ "request_id": request_id, "decision": decision_str, "device_id": device });
        if let Some(reason) = reason {
            payload["reason"] = json!(reason);
        }
        self.log
            .append(
                &session_id,
                EventKind::PermissionResolved,
                payload,
                remaining,
            )
            .await;
        self.flush(&session_id).await;
        Ok(())
    }

    /// An approval from a paired device (§2.6). Refused when the request is unknown, already
    /// answered, the nonce or device doesn't match, the signature doesn't verify against the
    /// device's stored P-256 key, it's signed more than 60 s in the future, or the request expired.
    /// Every refusal of a real request is logged as `permission_resolved` with `rejected: true` and
    /// the reason; nothing reaches the agent.
    pub async fn resolve_signed(
        &self,
        approval: &Approval,
        device_id: &str,
        p256: &[u8],
    ) -> Result<(), String> {
        let (rid, signed_at) = (approval.request_id.clone(), approval.signed_at.clone());
        let row: Option<(String, String, Option<String>, bool, bool)> = self
            .db
            .call(move |conn| {
                conn.query_row(
                    "SELECT session_id, nonce, resolved_at,
                            COALESCE(CAST(strftime('%s', ?2) AS INTEGER) > CAST(strftime('%s', 'now', '+60 seconds') AS INTEGER), 1),
                            CAST(strftime('%s', 'now') AS INTEGER) > CAST(strftime('%s', expires_at) AS INTEGER)
                     FROM approval WHERE request_id = ?1",
                    params![rid, signed_at],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()
            })
            .await
            .map_err(|e| e.to_string())?;
        let Some((session_id, nonce, resolved_at, future, expired)) = row else {
            return Err("unknown_request".into());
        };
        let decision = serde_json::to_value(approval.decision).expect("Decision serialises");
        let input = crypto::approval_signing_input(
            &approval.request_id,
            &approval.nonce,
            decision.as_str().unwrap_or_default(),
            &approval.device_id,
            &approval.signed_at,
        );
        let signature = B64.decode(&approval.sig).unwrap_or_default();
        let refusal = if resolved_at.is_some() {
            Some("already_resolved")
        } else if approval.nonce != nonce {
            Some("nonce_mismatch")
        } else if approval.device_id != device_id {
            Some("wrong_device")
        } else if crypto::verify(p256, &input, &signature).is_err() {
            Some("bad_signature")
        } else if future {
            Some("signed_in_future")
        } else if expired {
            Some("expired")
        } else {
            None
        };
        match refusal {
            None => {
                self.resolve_as(&approval.request_id, approval.decision, None, device_id)
                    .await
            }
            Some(reason) => {
                let payload = json!({
                    "request_id": approval.request_id, "decision": decision, "device_id": device_id,
                    "rejected": true, "reason": reason,
                });
                let pending = self.pending_count(&session_id);
                self.log
                    .append(&session_id, EventKind::PermissionResolved, payload, pending)
                    .await;
                Err(reason.to_owned())
            }
        }
    }

    /// Sends a follow-up now, or holds it until the agent can take it.
    pub async fn prompt(
        &self,
        session_id: &str,
        text: String,
        effort: Option<String>,
    ) -> Result<&'static str, String> {
        let delivery = {
            let mut live = self.live.lock().unwrap();
            let session = live
                .get_mut(session_id)
                .ok_or_else(|| format!("session {session_id} is not running"))?;
            if let (Driver::Codex(codex), Some(effort)) = (&mut session.driver, effort) {
                codex.effort = Some(effort);
            }
            let held = !session.pending.is_empty()
                || !session.driver.ready()
                || !session.queued.is_empty();
            session.queued.push_back(text);
            if held { "queued" } else { "sent" }
        };
        self.flush(session_id).await;
        Ok(delivery)
    }

    /// Sends queued text for as long as the agent can take it. Claude takes everything at once;
    /// Codex takes one prompt per turn and says `Idle` when it can take the next.
    async fn flush(&self, session_id: &str) {
        loop {
            let text = {
                let mut live = self.live.lock().unwrap();
                let Some(session) = live.get_mut(session_id) else {
                    return;
                };
                if !session.pending.is_empty() || !session.driver.ready() {
                    return;
                }
                let Some(text) = session.queued.pop_front() else {
                    return;
                };
                let line = session.driver.prompt(&text);
                let _ = session.stdin.send(line);
                text
            };
            self.log
                .append(
                    session_id,
                    EventKind::UserMessage,
                    json!({ "text": text }),
                    0,
                )
                .await;
        }
    }

    /// Stops the current turn. Open permissions are cancelled first so the agent is not left waiting.
    pub async fn cancel(&self, session_id: &str) -> Result<(), String> {
        let open: Vec<String> = {
            let live = self.live.lock().unwrap();
            let session = live
                .get(session_id)
                .ok_or_else(|| format!("session {session_id} is not running"))?;
            session.pending.keys().cloned().collect()
        };
        for request_id in open {
            let _ = self
                .resolve(&request_id, Decision::Deny, Some("cancelled"))
                .await;
        }
        let mut live = self.live.lock().unwrap();
        let session = live.get_mut(session_id).ok_or("session already ended")?;
        session.queued.clear();
        if let Some(line) = session.driver.interrupt() {
            session
                .stdin
                .send(line)
                .map_err(|_| "session already ended".to_owned())?;
        }
        Ok(())
    }

    fn pending_count(&self, session_id: &str) -> usize {
        self.live
            .lock()
            .unwrap()
            .get(session_id)
            .map_or(0, |s| s.pending.len())
    }

    pub async fn get(&self, id: &str) -> Result<Session, String> {
        let id = id.to_owned();
        self.db
            .call(move |conn| {
                conn.query_row(
                    &format!("{SESSION_SELECT} WHERE id = ?1"),
                    [id],
                    session_row,
                )
            })
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn list(&self) -> Result<Vec<Session>, String> {
        self.db
            .call(|conn| {
                conn.prepare(&format!("{SESSION_SELECT} ORDER BY started_at"))?
                    .query_map([], session_row)?
                    .collect()
            })
            .await
            .map_err(|e| e.to_string())
    }
}

const SESSION_SELECT: &str = "SELECT id, workspace_id, agent, tier, status, started_at, ended_at, last_change_at FROM session";

fn session_row(r: &rusqlite::Row) -> rusqlite::Result<Session> {
    let status: String = r.get(4)?;
    Ok(Session {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        agent: r.get(2)?,
        tier: r.get(3)?,
        status: serde_json::from_value(json!(status)).unwrap_or(swarn_core::schema::Status::Exited),
        started_at: r.get(5)?,
        ended_at: r.get(6)?,
        last_change_at: r.get(7)?,
    })
}

fn random_hex() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .expect("/dev/urandom is readable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
