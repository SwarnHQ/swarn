//! Claude Code driver (§4.2): the user's own `claude`, unmodified, over stream-json on stdio.
//! Flags and message shapes were checked against Claude Code 2.1.300; see docs/cli-behaviour.md.

use serde_json::{Value, json};
use swarn_core::local_api::ApprovalMode;
use swarn_core::schema::{Decision, EventKind};

use crate::sessions::{Action, Ask, Reply};

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
    "--permission-prompt-tool",
    "stdio",
];

#[derive(Default)]
pub struct Claude {
    /// `manual` asks before writes and commands; `acceptEdits` only before commands.
    pub mode: ApprovalMode,
    /// Claude repeats `init` on every turn; the session started once.
    started: bool,
    /// Set by `interrupt`. Claude ends an interrupted turn with an error `result`, which here
    /// means "stopped as asked", not a failure.
    cancelling: bool,
}

impl Claude {
    pub fn args(&self, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
        let mut args: Vec<String> = ARGS.iter().map(|a| (*a).to_owned()).collect();
        let mode = match self.mode {
            ApprovalMode::Ask => "manual",
            ApprovalMode::AllowEdits => "acceptEdits",
        };
        args.extend(["--permission-mode".to_owned(), mode.to_owned()]);
        if let Some(model) = model {
            args.extend(["--model".to_owned(), model.to_owned()]);
        }
        if let Some(effort) = effort {
            args.extend(["--effort".to_owned(), effort.to_owned()]);
        }
        args
    }

    pub fn prompt(text: &str) -> String {
        json!({ "type": "user", "message": { "role": "user", "content": text } }).to_string()
    }

    pub fn answer(
        request_id: &str,
        input: &Value,
        suggestions: &Value,
        decision: Decision,
        reason: Option<&str>,
    ) -> String {
        let response = match decision {
            Decision::AllowOnce => json!({ "behavior": "allow", "updatedInput": input }),
            Decision::AllowSession => {
                json!({ "behavior": "allow", "updatedInput": input, "updatedPermissions": suggestions })
            }
            Decision::Deny => json!({
                "behavior": "deny",
                "message": if reason == Some("expired") { "The approval request expired." } else { "Denied in Swarn." },
            }),
        };
        json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": request_id, "response": response },
        })
        .to_string()
    }

    pub fn interrupt(&mut self) -> String {
        self.cancelling = true;
        json!({
            "type": "control_request",
            "request_id": ulid::Ulid::generate().to_string(),
            "request": { "subtype": "interrupt" },
        })
        .to_string()
    }

    /// One stream-json line in, Swarn actions out. Anything not listed (hooks, rate limits, status
    /// pings, summaries) is dropped.
    pub fn translate(&mut self, msg: &Value) -> Vec<Action> {
        let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        match (msg["type"].as_str(), msg["subtype"].as_str()) {
            (Some("system"), Some("init")) if !std::mem::replace(&mut self.started, true) => {
                vec![Action::AgentSession {
                    id: s(&msg["session_id"]),
                    payload: json!({
                        "agent": "claude-code", "model": msg["model"], "version": msg["claude_code_version"],
                        "permission_mode": msg["permissionMode"],
                    }),
                }]
            }
            (Some("stream_event"), _) if msg["event"]["type"] == "content_block_delta" => {
                let delta = &msg["event"]["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => vec![Action::Emit(
                        EventKind::AgentMessageDelta,
                        json!({ "text": delta["text"], "index": msg["event"]["index"] }),
                    )],
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
                    Action::Emit(
                        EventKind::ToolCallUpdate,
                        json!({ "call_id": b["tool_use_id"], "status": status, "content": b["content"] }),
                    )
                })
                .collect(),
            (Some("control_request"), _) if msg["request"]["subtype"] == "can_use_tool" => {
                let r = &msg["request"];
                let tool = s(&r["tool_name"]);
                let display = r["display_name"].as_str().unwrap_or(&tool).to_owned();
                let description = s(&r["description"]);
                let summary = if description.is_empty() { display } else { format!("{display}: {description}") };
                vec![Action::Permission(Box::new(Ask {
                    reply: Reply::Claude {
                        request_id: s(&msg["request_id"]),
                        input: r["input"].clone(),
                        suggestions: r.get("permission_suggestions").cloned().unwrap_or(json!([])),
                    },
                    risk: risk(&tool),
                    tool,
                    summary,
                    detail: r["input"].clone(),
                }))]
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
                let end = if subtype == Some("success") && msg["is_error"] != true {
                    Action::Emit(EventKind::AgentMessageEnd, json!({ "stop_reason": msg["stop_reason"] }))
                } else if std::mem::replace(&mut self.cancelling, false) {
                    Action::Emit(EventKind::AgentMessageEnd, json!({ "stop_reason": "cancelled" }))
                } else {
                    let message = msg["result"].as_str().map(str::to_owned).unwrap_or_else(|| s(&msg["subtype"]));
                    Action::Emit(EventKind::Error, json!({ "message": message }))
                };
                vec![usage, end, Action::Idle]
            }
            _ => vec![],
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed and anonymised from a real Claude Code 2.1.300 run (docs/cli-behaviour.md).
    const TRANSCRIPT: &str = include_str!("../tests/fixtures/claude-2.1.300.jsonl");

    #[test]
    fn model_and_effort_become_flags_only_when_picked() {
        let claude = Claude::default();
        let args = claude.args(Some("opus"), Some("xhigh")).join(" ");
        assert!(
            args.contains("--model opus") && args.contains("--effort xhigh"),
            "{args}"
        );
        let args = claude.args(None, None).join(" ");
        assert!(
            !args.contains("--model") && !args.contains("--effort"),
            "{args}"
        );
    }

    #[test]
    fn recorded_transcript_maps_to_the_expected_actions() {
        let mut claude = Claude::default();
        let actions: Vec<Action> = TRANSCRIPT
            .lines()
            .flat_map(|l| claude.translate(&serde_json::from_str(l).unwrap()))
            .collect();
        let kinds: Vec<String> = actions.iter().map(Action::describe).collect();
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
                "idle",
            ]
        );
        let Action::Permission(ask) = &actions[2] else {
            unreachable!()
        };
        let Ask {
            reply: Reply::Claude {
                request_id, input, ..
            },
            risk,
            ..
        } = ask.as_ref()
        else {
            unreachable!()
        };
        assert_eq!(request_id, "3545d1d7-5533-4ce9-bf73-496bf503ed5c");
        assert_eq!(input["content"], "hi");
        assert_eq!(*risk, "write");
    }

    #[test]
    fn an_interrupted_turn_ends_as_cancelled_not_failed() {
        let mut claude = Claude::default();
        let _ = claude.interrupt();
        let actions = claude.translate(
            &json!({"type": "result", "subtype": "error_during_execution", "is_error": true}),
        );
        assert_eq!(
            actions[1],
            Action::Emit(
                EventKind::AgentMessageEnd,
                json!({"stop_reason": "cancelled"})
            )
        );
        let later = claude
            .translate(&json!({"type": "result", "subtype": "error_max_turns", "is_error": true}));
        assert!(
            matches!(&later[1], Action::Emit(EventKind::Error, _)),
            "only the next error counts as the cancel"
        );
    }
}
