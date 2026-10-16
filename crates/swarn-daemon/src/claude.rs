//! Claude Code driver (§4.2): the user's own `claude`, unmodified, over stream-json on stdio.
//! Flags and message shapes were checked against Claude Code 2.1.300; see docs/cli-behaviour.md.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::params;
use serde_json::{Value, json};
use swarn_core::schema::{Decision, EventKind, Session};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::db::Db;
use crate::env::AgentEnv;
use crate::events::EventLog;

/// `--permission-prompt-tool stdio` is what routes prompts to us. Without it, `manual` mode denies
/// on its own and the agent gives up (measured; the `initialize` handshake alone does not help).
const ARGS: &[&str] = &[
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--include-partial-messages",
    "--permission-mode",
    "manual",
    "--permission-prompt-tool",
    "stdio",
];

/// Policy default (§2.6) until `policy.toml` exists. Claude itself never times a prompt out.
const APPROVAL_EXPIRY: Duration = Duration::from_secs(30 * 60);

struct Pending {
    claude_request_id: String,
    input: Value,
    suggestions: Value,
}

struct Live {
    stdin: mpsc::UnboundedSender<String>,
    pending: HashMap<String, Pending>,
    /// Text the user sent while a permission was open. Delivered once nothing is pending (§2.6).
    queued: Vec<String>,
    /// Set by `cancel`. Claude ends an interrupted turn with an error `result`, which here means
    /// "stopped as asked", not a failure.
    cancelling: bool,
}

/// Running sessions. ponytail: one std Mutex over a small map, held only for map edits and never
/// across an await; a session_manager task would be the upgrade if it ever shows in a profile.
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
        workspace_id: String,
        worktree: String,
        prompt: String,
        model: Option<String>,
    ) -> Result<Session, String> {
        // SWARN_CLAUDE_BIN pins a specific binary (tests use a scripted fake); otherwise the
        // user's own `claude` from the login shell's PATH.
        let claude = std::env::var_os("SWARN_CLAUDE_BIN")
            .map(std::path::PathBuf::from)
            .or_else(|| self.env.which("claude"))
            .ok_or("claude was not found on your login shell's PATH")?;
        let id = ulid::Ulid::generate().to_string();
        let row_id = id.clone();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO session (id, workspace_id, agent, tier, status, started_at, last_change_at)
                     VALUES (?1, ?2, 'claude-code', 'native', 'starting',
                             strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                    params![row_id, workspace_id],
                )
            })
            .await
            .map_err(|e| e.to_string())?;

        let mut child = Command::new(claude)
            .args(ARGS)
            .args(model.iter().flat_map(|m| ["--model", m.as_str()]))
            .current_dir(&worktree)
            .env_clear()
            .envs(&self.env.vars)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("starting claude: {e}"))?;
        let pid = child.id();
        let row_id = id.clone();
        let _ = self
            .db
            .call(move |conn| {
                conn.execute(
                    "UPDATE session SET pid = ?2 WHERE id = ?1",
                    params![row_id, pid],
                )
            })
            .await;

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
                eprintln!("claude[{tag}]: {line}");
            }
        });

        self.live.lock().unwrap().insert(
            id.clone(),
            Live {
                stdin: stdin_tx,
                pending: HashMap::new(),
                queued: Vec::new(),
                cancelling: false,
            },
        );
        self.send_prompt(&id, prompt).await;

        let stdout = child.stdout.take().expect("piped");
        let this = Arc::clone(self);
        let session_id = id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut started = false;
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                for action in translate(&msg) {
                    this.apply(&session_id, action, &mut started).await;
                }
            }
            let status = child.wait().await;
            this.live.lock().unwrap().remove(&session_id);
            let code = status.ok().and_then(|s| s.code());
            if code != Some(0) {
                this.log
                    .append(
                        &session_id,
                        EventKind::Error,
                        json!({ "message": format!("claude exited with {code:?}") }),
                        0,
                    )
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

    async fn apply(self: &Arc<Self>, session_id: &str, action: Action, started: &mut bool) {
        match action {
            Action::AgentSession {
                claude_session_id,
                payload,
            } => {
                if std::mem::replace(started, true) {
                    return; // Claude repeats `init` on every turn; the session started once.
                }
                let (id, agent_id) = (session_id.to_owned(), claude_session_id);
                let _ = self
                    .db
                    .call(move |conn| {
                        conn.execute(
                            "UPDATE session SET acp_session = ?2 WHERE id = ?1",
                            params![id, agent_id],
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
                let (kind, payload) =
                    if kind == EventKind::Error && self.take_cancelling(session_id) {
                        (
                            EventKind::AgentMessageEnd,
                            json!({ "stop_reason": "cancelled" }),
                        )
                    } else {
                        (kind, payload)
                    };
                self.log
                    .append(session_id, kind, payload, self.pending_count(session_id))
                    .await;
            }
            Action::Permission {
                claude_request_id,
                tool,
                summary,
                input,
                suggestions,
            } => {
                self.open_permission(
                    session_id,
                    claude_request_id,
                    tool,
                    summary,
                    input,
                    suggestions,
                )
                .await;
            }
        }
    }

    async fn open_permission(
        self: &Arc<Self>,
        session_id: &str,
        claude_request_id: String,
        tool: String,
        summary: String,
        input: Value,
        suggestions: Value,
    ) {
        let request_id = ulid::Ulid::generate().to_string();
        let nonce = random_hex();
        let risk = risk(&tool);
        let (rid, sid, t, n) = (
            request_id.clone(),
            session_id.to_owned(),
            tool.clone(),
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
                        t,
                        risk,
                        n,
                        format!("+{} seconds", APPROVAL_EXPIRY.as_secs())
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
            session.pending.insert(
                request_id.clone(),
                Pending {
                    claude_request_id,
                    input: input.clone(),
                    suggestions,
                },
            );
            session.pending.len()
        };
        let payload = json!({
            "request_id": request_id, "session_id": session_id, "tool": tool, "summary": summary,
            "detail": input, "risk": risk, "nonce": nonce, "expires_at": expires_at,
        });
        self.log
            .append(session_id, EventKind::PermissionRequest, payload, pending)
            .await;

        let this = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(APPROVAL_EXPIRY).await;
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
        let (session_id, pending, remaining, stdin, flush) = {
            let mut live = self.live.lock().unwrap();
            let (session_id, session) = live
                .iter_mut()
                .find(|(_, s)| s.pending.contains_key(request_id))
                .ok_or_else(|| format!("no open permission request {request_id}"))?;
            let pending = session.pending.remove(request_id).expect("found above");
            let remaining = session.pending.len();
            let flush = if remaining == 0 {
                std::mem::take(&mut session.queued)
            } else {
                Vec::new()
            };
            (
                session_id.clone(),
                pending,
                remaining,
                session.stdin.clone(),
                flush,
            )
        };

        let response = match decision {
            Decision::AllowOnce => json!({ "behavior": "allow", "updatedInput": pending.input }),
            Decision::AllowSession => json!({
                "behavior": "allow", "updatedInput": pending.input, "updatedPermissions": pending.suggestions,
            }),
            Decision::Deny => json!({
                "behavior": "deny",
                "message": if reason == Some("expired") { "The approval request expired." } else { "Denied in Swarn." },
            }),
        };
        let line = json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": pending.claude_request_id, "response": response },
        });
        let _ = stdin.send(line.to_string());

        let decision_str = serde_json::to_value(decision).expect("Decision serialises");
        let (rid, d) = (
            request_id.to_owned(),
            decision_str.as_str().unwrap_or("deny").to_owned(),
        );
        let _ = self
            .db
            .call(move |conn| {
                conn.execute(
                    "UPDATE approval SET resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), decision = ?2, device_id = 'local'
                     WHERE request_id = ?1",
                    params![rid, d],
                )
            })
            .await;
        let mut payload =
            json!({ "request_id": request_id, "decision": decision_str, "device_id": "local" });
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
        for text in flush {
            self.send_prompt(&session_id, text).await;
        }
        Ok(())
    }

    /// Sends a follow-up, or holds it while a permission is open.
    pub async fn prompt(&self, session_id: &str, text: String) -> Result<&'static str, String> {
        {
            let mut live = self.live.lock().unwrap();
            let session = live
                .get_mut(session_id)
                .ok_or_else(|| format!("session {session_id} is not running"))?;
            if !session.pending.is_empty() {
                session.queued.push(text);
                return Ok("queued");
            }
        }
        self.send_prompt(session_id, text).await;
        Ok("sent")
    }

    async fn send_prompt(&self, session_id: &str, text: String) {
        let line = json!({ "type": "user", "message": { "role": "user", "content": text } });
        let sent = self
            .live
            .lock()
            .unwrap()
            .get(session_id)
            .map(|s| s.stdin.send(line.to_string()));
        if sent.is_some() {
            self.log
                .append(
                    session_id,
                    EventKind::UserMessage,
                    json!({ "text": text }),
                    self.pending_count(session_id),
                )
                .await;
        }
    }

    /// Stops the current turn. Open permissions are denied first so Claude is not left waiting.
    pub async fn cancel(&self, session_id: &str) -> Result<(), String> {
        let (stdin, open): (_, Vec<String>) = {
            let mut live = self.live.lock().unwrap();
            let session = live
                .get_mut(session_id)
                .ok_or_else(|| format!("session {session_id} is not running"))?;
            session.cancelling = true;
            (
                session.stdin.clone(),
                session.pending.keys().cloned().collect(),
            )
        };
        for request_id in open {
            let _ = self
                .resolve(&request_id, Decision::Deny, Some("cancelled"))
                .await;
        }
        let interrupt = json!({
            "type": "control_request", "request_id": ulid::Ulid::generate().to_string(),
            "request": { "subtype": "interrupt" },
        });
        stdin
            .send(interrupt.to_string())
            .map_err(|_| "session already ended".to_owned())
    }

    fn take_cancelling(&self, session_id: &str) -> bool {
        let mut live = self.live.lock().unwrap();
        live.get_mut(session_id)
            .is_some_and(|s| std::mem::replace(&mut s.cancelling, false))
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

#[derive(Debug, PartialEq)]
pub enum Action {
    AgentSession {
        claude_session_id: String,
        payload: Value,
    },
    Emit(EventKind, Value),
    Permission {
        claude_request_id: String,
        tool: String,
        summary: String,
        input: Value,
        suggestions: Value,
    },
}

/// One stream-json line in, Swarn events out. Pure, so recorded transcripts can be replayed against
/// it (§9.2). Anything not listed (hooks, rate limits, status pings, summaries) is dropped.
pub fn translate(msg: &Value) -> Vec<Action> {
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    match (msg["type"].as_str(), msg["subtype"].as_str()) {
        (Some("system"), Some("init")) => vec![Action::AgentSession {
            claude_session_id: s(&msg["session_id"]),
            payload: json!({
                "agent": "claude-code", "model": msg["model"], "version": msg["claude_code_version"],
                "permission_mode": msg["permissionMode"],
            }),
        }],
        (Some("stream_event"), _) if msg["event"]["type"] == "content_block_delta" => {
            let delta = &msg["event"]["delta"];
            match delta["type"].as_str() {
                Some("text_delta") => {
                    vec![Action::Emit(EventKind::AgentMessageDelta, json!({ "text": delta["text"], "index": msg["event"]["index"] }))]
                }
                Some("thinking_delta") if !s(&delta["thinking"]).is_empty() => {
                    vec![Action::Emit(EventKind::ThoughtDelta, json!({ "text": delta["thinking"] }))]
                }
                _ => vec![],
            }
        }
        (Some("assistant"), _) => blocks(msg)
            .filter(|b| b["type"] == "tool_use")
            .flat_map(|b| {
                let mut out = vec![Action::Emit(
                    EventKind::ToolCall,
                    json!({ "call_id": b["id"], "name": b["name"], "input": b["input"], "status": "pending" }),
                )];
                if b["name"] == "TodoWrite" {
                    out.push(Action::Emit(EventKind::Plan, json!({ "entries": b["input"]["todos"] })));
                }
                out
            })
            .collect(),
        (Some("user"), _) => blocks(msg)
            .filter(|b| b["type"] == "tool_result")
            .map(|b| {
                let status = if b["is_error"] == true { "failed" } else { "completed" };
                Action::Emit(EventKind::ToolCallUpdate, json!({ "call_id": b["tool_use_id"], "status": status, "content": b["content"] }))
            })
            .collect(),
        (Some("control_request"), _) if msg["request"]["subtype"] == "can_use_tool" => {
            let r = &msg["request"];
            let tool = s(&r["tool_name"]);
            let display = r["display_name"].as_str().unwrap_or(&tool).to_owned();
            let description = s(&r["description"]);
            let summary = if description.is_empty() { display } else { format!("{display}: {description}") };
            vec![Action::Permission {
                claude_request_id: s(&msg["request_id"]),
                tool,
                summary,
                input: r["input"].clone(),
                suggestions: r.get("permission_suggestions").cloned().unwrap_or(json!([])),
            }]
        }
        (Some("result"), subtype) => {
            let u = &msg["usage"];
            let input = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
                .iter()
                .filter_map(|k| u[k].as_u64())
                .sum::<u64>();
            let usage = Action::Emit(
                EventKind::Usage,
                json!({ "input_tokens": input, "output_tokens": u["output_tokens"], "cost_usd": msg["total_cost_usd"] }),
            );
            if subtype == Some("success") && msg["is_error"] != true {
                vec![usage, Action::Emit(EventKind::AgentMessageEnd, json!({ "stop_reason": msg["stop_reason"] }))]
            } else {
                let message = msg["result"].as_str().map(str::to_owned).unwrap_or_else(|| s(&msg["subtype"]));
                vec![usage, Action::Emit(EventKind::Error, json!({ "message": message }))]
            }
        }
        _ => vec![],
    }
}

fn blocks(msg: &Value) -> impl Iterator<Item = &Value> {
    msg["message"]["content"].as_array().into_iter().flatten()
}

/// §2.6 risk classes, conservative: anything unrecognised counts as exec.
fn risk(tool: &str) -> &'static str {
    match tool {
        "Read" | "Glob" | "Grep" | "LS" => "read",
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => "write",
        "WebFetch" | "WebSearch" => "network",
        _ => "exec",
    }
}

fn random_hex() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .expect("/dev/urandom is readable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed and anonymised from a real Claude Code 2.1.300 run (docs/cli-behaviour.md).
    const TRANSCRIPT: &str = include_str!("../tests/fixtures/claude-2.1.300.jsonl");

    #[test]
    fn recorded_transcript_maps_to_the_expected_events() {
        let actions: Vec<Action> = TRANSCRIPT
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .flat_map(|m: Value| translate(&m))
            .collect();
        let kinds: Vec<String> = actions
            .iter()
            .map(|a| match a {
                Action::AgentSession { .. } => "agent_session".to_owned(),
                Action::Permission { summary, .. } => format!("permission: {summary}"),
                Action::Emit(kind, _) => serde_json::to_value(kind)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "agent_session",
                "tool_call",
                "permission: Write: hello.txt",
                "tool_call_update",
                "agent_message_delta",
                "agent_message_delta",
                "usage",
                "agent_message_end",
            ]
        );
        let Action::Permission {
            claude_request_id,
            input,
            ..
        } = &actions[2]
        else {
            unreachable!()
        };
        assert_eq!(claude_request_id, "3545d1d7-5533-4ce9-bf73-496bf503ed5c");
        assert_eq!(input["content"], "hi");
    }

    #[test]
    fn unknown_tools_are_treated_as_exec() {
        assert_eq!(risk("Write"), "write");
        assert_eq!(risk("SomeNewTool"), "exec");
    }
}
