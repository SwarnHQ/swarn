//! Codex driver (§4.2): the user's own `codex app-server`, JSON-RPC over stdio, one JSON object per
//! line and no `jsonrpc` field. Shapes come from `codex app-server generate-ts` for Codex CLI 0.147.0
//! and were checked against a real run; see docs/cli-behaviour.md.

use std::collections::HashMap;

use serde_json::{Value, json};
use swarn_core::local_api::ApprovalMode;
use swarn_core::schema::{Decision, EventKind};

use crate::sessions::{Action, Ask, Reply};

const INITIALIZE: u64 = 1;
const THREAD_START: u64 = 2;

#[derive(Default)]
pub struct Codex {
    pub mode: ApprovalMode,
    next_id: u64,
    cwd: String,
    model: Option<String>,
    version: Option<String>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    /// A turn is running, so the next prompt waits for `turn/completed`.
    in_turn: bool,
    /// Items by id. Approval requests only name the item, so the card's detail comes from here.
    items: HashMap<String, Value>,
}

impl Codex {
    pub fn start(&mut self, cwd: &str, model: Option<String>) -> Vec<String> {
        self.cwd = cwd.to_owned();
        self.model = model;
        self.next_id = THREAD_START + 1;
        let client =
            json!({ "name": "swarn", "title": "Swarn", "version": env!("CARGO_PKG_VERSION") });
        vec![json!({ "id": INITIALIZE, "method": "initialize", "params": { "clientInfo": client, "capabilities": null } }).to_string()]
    }

    pub fn ready(&self) -> bool {
        self.thread_id.is_some() && !self.in_turn
    }

    pub fn prompt(&mut self, text: &str) -> String {
        self.in_turn = true;
        let input = [json!({ "type": "text", "text": text, "text_elements": [] })];
        self.request(
            "turn/start",
            json!({ "threadId": self.thread_id, "input": input }),
        )
    }

    /// `decline` lets the agent carry on without the action; `cancel` also stops the turn, which is
    /// what a cancelled session wants.
    pub fn answer(id: &Value, decision: Decision, reason: Option<&str>) -> String {
        let decision = match (decision, reason) {
            (Decision::AllowOnce, _) => "accept",
            (Decision::AllowSession, _) => "acceptForSession",
            (Decision::Deny, Some("cancelled")) => "cancel",
            (Decision::Deny, _) => "decline",
        };
        json!({ "id": id, "result": { "decision": decision } }).to_string()
    }

    pub fn interrupt(&mut self) -> Option<String> {
        let turn_id = self.turn_id.clone()?;
        Some(self.request(
            "turn/interrupt",
            json!({ "threadId": self.thread_id, "turnId": turn_id }),
        ))
    }

    fn request(&mut self, method: &str, params: Value) -> String {
        self.next_id += 1;
        json!({ "id": self.next_id, "method": method, "params": params }).to_string()
    }

    pub fn translate(&mut self, msg: &Value) -> Vec<Action> {
        let p = &msg["params"];
        match (msg.get("id"), msg["method"].as_str()) {
            (Some(id), None) => self.response(id, msg),
            (Some(id), Some(method)) => self.server_request(id.clone(), method, p),
            (None, Some(method)) => self.notification(method, p),
            (None, None) => vec![],
        }
    }

    fn response(&mut self, id: &Value, msg: &Value) -> Vec<Action> {
        if let Some(error) = msg.get("error") {
            self.in_turn = false;
            let message = error["message"]
                .as_str()
                .unwrap_or("Codex refused the request");
            return vec![
                Action::Emit(EventKind::Error, json!({ "message": clean_error(message) })),
                Action::Idle,
            ];
        }
        match id.as_u64() {
            Some(INITIALIZE) => {
                // "swarn/0.147.0 (Mac OS …)": the version after the originator.
                let agent = msg["result"]["userAgent"].as_str().unwrap_or_default();
                self.version = agent.split([' ', '/']).nth(1).map(str::to_owned);
                // read-only makes every write ask; workspace-write lets edits through and still
                // asks before anything that needs more than the worktree.
                let sandbox = match self.mode {
                    ApprovalMode::Ask => "read-only",
                    ApprovalMode::AllowEdits => "workspace-write",
                };
                let mut params =
                    json!({ "cwd": self.cwd, "approvalPolicy": "on-request", "sandbox": sandbox });
                if let Some(model) = &self.model {
                    params["model"] = json!(model);
                }
                vec![
                    Action::Send(json!({ "method": "initialized" }).to_string()),
                    Action::Send(
                        json!({ "id": THREAD_START, "method": "thread/start", "params": params })
                            .to_string(),
                    ),
                ]
            }
            Some(THREAD_START) => {
                let result = &msg["result"];
                let thread_id = result["thread"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                self.thread_id = Some(thread_id.clone());
                let payload =
                    json!({ "agent": "codex", "model": result["model"], "version": self.version });
                vec![
                    Action::AgentSession {
                        id: thread_id,
                        payload,
                    },
                    Action::Idle,
                ]
            }
            _ => vec![],
        }
    }

    /// Approvals become permission requests. Anything else Codex asks the client for is refused
    /// with a JSON-RPC error, so the agent never hangs on a question Swarn cannot show yet.
    fn server_request(&mut self, id: Value, method: &str, p: &Value) -> Vec<Action> {
        let reply = Reply::Codex { id: id.clone() };
        match method {
            "item/commandExecution/requestApproval" => {
                let command = p["command"]
                    .as_str()
                    .or_else(|| self.items.get(p["itemId"].as_str().unwrap_or_default())?["command"].as_str())
                    .unwrap_or("a command")
                    .to_owned();
                let risk = if p["networkApprovalContext"].is_null() { "exec" } else { "network" };
                vec![Action::Permission(Box::new(Ask {
                    reply,
                    tool: "Shell".into(),
                    summary: format!("Run: {command}"),
                    risk,
                    detail: json!({ "command": command, "cwd": p["cwd"], "reason": p["reason"] }),
                }))]
            }
            "item/fileChange/requestApproval" => {
                let changes = self.items.get(p["itemId"].as_str().unwrap_or_default()).map(|i| i["changes"].clone());
                let names: Vec<String> = changes
                    .iter()
                    .flat_map(|c| c.as_array().into_iter().flatten())
                    .filter_map(|c| c["path"].as_str())
                    .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
                    .collect();
                let summary =
                    if names.is_empty() { "Edit files".to_owned() } else { format!("Edit: {}", names.join(", ")) };
                vec![Action::Permission(Box::new(Ask {
                    reply,
                    tool: "Edit".into(),
                    summary,
                    risk: "write",
                    detail: json!({ "changes": changes, "reason": p["reason"] }),
                }))]
            }
            other => vec![
                Action::Send(
                    json!({ "id": id, "error": { "code": -32601, "message": "Swarn does not handle this request yet" } })
                        .to_string(),
                ),
                Action::Emit(
                    EventKind::Error,
                    json!({ "message": format!("Codex asked for {other}, which Swarn does not support yet") }),
                ),
            ],
        }
    }

    fn notification(&mut self, method: &str, p: &Value) -> Vec<Action> {
        match method {
            "turn/started" => {
                self.turn_id = p["turn"]["id"].as_str().map(str::to_owned);
                self.in_turn = true;
                vec![]
            }
            "item/agentMessage/delta" => vec![Action::Emit(
                EventKind::AgentMessageDelta,
                json!({ "text": p["delta"] }),
            )],
            "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => {
                vec![Action::Emit(
                    EventKind::ThoughtDelta,
                    json!({ "text": p["delta"] }),
                )]
            }
            "item/started" => {
                let item = &p["item"];
                let id = item["id"].as_str().unwrap_or_default().to_owned();
                self.items.insert(id.clone(), item.clone());
                let (name, input) = match item["type"].as_str() {
                    Some("commandExecution") => ("Shell", json!({ "command": item["command"] })),
                    Some("fileChange") => {
                        let first = item["changes"][0]["path"].clone();
                        (
                            "Edit",
                            json!({ "file_path": first, "changes": item["changes"] }),
                        )
                    }
                    Some("mcpToolCall") => (
                        "Tool",
                        json!({ "tool": item["tool"], "server": item["server"] }),
                    ),
                    Some("webSearch") => ("WebSearch", json!({ "query": item["query"] })),
                    _ => return vec![],
                };
                vec![Action::Emit(
                    EventKind::ToolCall,
                    json!({ "call_id": id, "name": name, "input": input, "status": "pending" }),
                )]
            }
            "item/completed" => {
                let item = &p["item"];
                match item["type"].as_str() {
                    Some("commandExecution" | "fileChange" | "mcpToolCall" | "webSearch") => {
                        let status = if item["status"] == "completed" {
                            "completed"
                        } else {
                            "failed"
                        };
                        let content = item.get("aggregatedOutput").cloned().unwrap_or(Value::Null);
                        vec![Action::Emit(
                            EventKind::ToolCallUpdate,
                            json!({ "call_id": item["id"], "status": status, "content": content }),
                        )]
                    }
                    // Codex sends several messages per turn (commentary, then the answer); keep
                    // them apart as paragraphs.
                    Some("agentMessage") => vec![Action::Emit(
                        EventKind::AgentMessageDelta,
                        json!({ "text": "\n\n" }),
                    )],
                    _ => vec![],
                }
            }
            "turn/plan/updated" => vec![Action::Emit(
                EventKind::Plan,
                json!({ "entries": p["plan"] }),
            )],
            "thread/tokenUsage/updated" => {
                let usage = &p["tokenUsage"];
                let mut payload = json!({
                    "input_tokens": usage["total"]["inputTokens"], "output_tokens": usage["total"]["outputTokens"],
                });
                if let (Some(last), Some(window)) = (
                    usage["last"]["totalTokens"].as_f64(),
                    usage["modelContextWindow"].as_f64(),
                ) && window > 0.0
                {
                    payload["context_pct"] = json!((last * 100.0 / window).round());
                }
                vec![Action::Emit(EventKind::Usage, payload)]
            }
            "turn/diff/updated" => vec![Action::Emit(
                EventKind::DiffStat,
                diff_stat(p["diff"].as_str().unwrap_or_default()),
            )],
            "turn/completed" => {
                self.in_turn = false;
                self.turn_id = None;
                let turn = &p["turn"];
                let end = match turn["status"].as_str() {
                    Some("interrupted") => Action::Emit(
                        EventKind::AgentMessageEnd,
                        json!({ "stop_reason": "cancelled" }),
                    ),
                    Some("failed") => {
                        let message = turn["error"]["message"]
                            .as_str()
                            .unwrap_or("The turn failed");
                        Action::Emit(EventKind::Error, json!({ "message": clean_error(message) }))
                    }
                    _ => Action::Emit(
                        EventKind::AgentMessageEnd,
                        json!({ "stop_reason": "end_turn" }),
                    ),
                };
                vec![end, Action::Idle]
            }
            _ => vec![],
        }
    }
}

/// Codex often wraps the provider's JSON error inside its message; show the human part.
fn clean_error(message: &str) -> String {
    serde_json::from_str::<Value>(message)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| message.to_owned())
}

/// Files touched and lines added and removed, from a unified diff.
fn diff_stat(diff: &str) -> Value {
    let files = diff
        .lines()
        .filter(|l| l.starts_with("diff --git "))
        .count();
    let added = diff
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .count();
    let removed = diff
        .lines()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
        .count();
    json!({ "files": files, "additions": added, "deletions": removed })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed and anonymised from a real Codex CLI 0.147.0 run (docs/cli-behaviour.md).
    const TRANSCRIPT: &str = include_str!("../tests/fixtures/codex-0.147.0.jsonl");

    #[test]
    fn recorded_transcript_maps_to_the_expected_actions() {
        let mut codex = Codex::default();
        let first = codex.start("/work/repo", Some("gpt-5.6-luna".into()));
        assert!(first[0].contains("\"initialize\""));
        assert!(!codex.ready(), "no prompt before the thread exists");

        let mut actions = Vec::new();
        for line in TRANSCRIPT.lines() {
            actions.extend(codex.translate(&serde_json::from_str(line).unwrap()));
        }
        assert!(
            codex.ready(),
            "the turn completed, so the next prompt can go"
        );
        assert!(codex.prompt("next").contains("turn/start"));
        let kinds: Vec<String> = actions.iter().map(Action::describe).collect();
        assert_eq!(
            kinds,
            [
                "send",
                "send",
                "agent_session",
                "idle",
                "agent_message_delta",
                "agent_message_delta",
                "tool_call",
                "permission: Edit: hello.txt",
                "tool_call_update",
                "diff_stat",
                "usage",
                "agent_message_end",
                "idle",
            ]
        );
        let Action::Permission(ask) = &actions[7] else {
            unreachable!()
        };
        let Ask {
            reply: Reply::Codex { id },
            risk,
            detail,
            ..
        } = ask.as_ref()
        else {
            unreachable!()
        };
        assert_eq!(id, &json!(0));
        assert_eq!(*risk, "write");
        assert_eq!(detail["changes"][0]["diff"], "hi\n");
        assert_eq!(
            actions[9],
            Action::Emit(
                EventKind::DiffStat,
                json!({"files": 1, "additions": 1, "deletions": 0})
            )
        );
        assert!(Codex::answer(&json!(0), Decision::AllowOnce, None).contains("\"accept\""));
        assert!(Codex::answer(&json!(0), Decision::Deny, Some("expired")).contains("\"decline\""));
    }

    #[test]
    fn provider_errors_are_unwrapped() {
        let raw =
            r#"{"type":"error","status":400,"error":{"message":"The model is not supported."}}"#;
        assert_eq!(clean_error(raw), "The model is not supported.");
        assert_eq!(clean_error("plain"), "plain");
    }
}
