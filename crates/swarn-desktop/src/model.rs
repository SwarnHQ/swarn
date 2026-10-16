//! What the window shows, as plain data. Daemon events go in, transcript blocks come out; no GPUI
//! here, so the mapping is testable without a window (§5.1).

use std::collections::HashMap;

use serde_json::Value;
use swarn_core::local_api::DaemonInfo;
use swarn_core::schema::{Event, EventKind, Repo, Session, Status, Workspace};

#[derive(Clone, Debug, PartialEq)]
pub enum ToolState {
    Pending,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    User(String),
    /// `open` while the agent is still streaming into it.
    Agent {
        text: String,
        open: bool,
    },
    Tool {
        call_id: String,
        name: String,
        target: String,
        state: ToolState,
    },
    Approval(Approval),
    /// One-line notices: errors, cancels, the end of a session.
    Note {
        text: String,
        error: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Approval {
    pub request_id: String,
    pub summary: String,
    pub risk: String,
    pub detail: String,
    /// `allow_once`, `allow_session` or `deny` once answered.
    pub decision: Option<String>,
    pub reason: Option<String>,
    /// Who answered: "local" for this Mac, else a paired device's id.
    pub by: Option<String>,
}

#[derive(Default)]
pub struct Transcript {
    pub blocks: Vec<Block>,
}

impl Transcript {
    /// Applies one event and returns the lowest block index it touched, if any, so the list view
    /// only re-measures from there down.
    pub fn apply(&mut self, event: &Event) -> Option<usize> {
        let p = &event.payload;
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        match event.kind {
            EventKind::UserMessage => {
                let closed = self.close_agent();
                earliest(closed, self.push(Block::User(text(&p["text"]))))
            }
            EventKind::AgentMessageDelta => match self.blocks.last_mut() {
                Some(Block::Agent {
                    text: t,
                    open: true,
                }) => {
                    t.push_str(p["text"].as_str().unwrap_or_default());
                    Some(self.blocks.len() - 1)
                }
                _ => self.push(Block::Agent {
                    text: text(&p["text"]),
                    open: true,
                }),
            },
            EventKind::ToolCall => {
                let closed = self.close_agent();
                let pushed = self.push(Block::Tool {
                    call_id: text(&p["call_id"]),
                    name: text(&p["name"]),
                    target: tool_target(&p["input"]),
                    state: ToolState::Pending,
                });
                earliest(closed, pushed)
            }
            EventKind::ToolCallUpdate => {
                let call_id = text(&p["call_id"]);
                let ix = self
                    .blocks
                    .iter()
                    .rposition(|b| matches!(b, Block::Tool { call_id: c, .. } if *c == call_id))?;
                if let Block::Tool { state, .. } = &mut self.blocks[ix] {
                    *state = if p["status"] == "failed" {
                        ToolState::Failed
                    } else {
                        ToolState::Completed
                    };
                }
                Some(ix)
            }
            EventKind::PermissionRequest => self.push(Block::Approval(Approval {
                request_id: text(&p["request_id"]),
                summary: text(&p["summary"]),
                risk: text(&p["risk"]),
                detail: detail(&p["detail"]),
                decision: None,
                reason: None,
                by: None,
            })),
            // A refused attempt (bad signature, expired…) leaves the request open.
            EventKind::PermissionResolved if p["rejected"] == true => None,
            EventKind::PermissionResolved => {
                let request_id = text(&p["request_id"]);
                let ix = self
                    .blocks
                    .iter()
                    .rposition(|b| matches!(b, Block::Approval(a) if a.request_id == request_id))?;
                if let Block::Approval(a) = &mut self.blocks[ix] {
                    a.decision = Some(text(&p["decision"]));
                    a.reason = p["reason"].as_str().map(str::to_owned);
                    a.by = p["device_id"].as_str().map(str::to_owned);
                }
                Some(ix)
            }
            EventKind::AgentMessageEnd => {
                let closed = self.close_agent();
                if p["stop_reason"] == "cancelled" {
                    return earliest(
                        closed,
                        self.push(Block::Note {
                            text: "Cancelled".into(),
                            error: false,
                        }),
                    );
                }
                closed
            }
            EventKind::Error => {
                let closed = self.close_agent();
                earliest(
                    closed,
                    self.push(Block::Note {
                        text: text(&p["message"]),
                        error: true,
                    }),
                )
            }
            EventKind::SessionEnded => {
                let closed = self.close_agent();
                earliest(
                    closed,
                    self.push(Block::Note {
                        text: "Session ended".into(),
                        error: false,
                    }),
                )
            }
            EventKind::SessionStarted
            | EventKind::StatusChanged
            | EventKind::ThoughtDelta
            | EventKind::Plan
            | EventKind::Usage
            | EventKind::DiffStat
            | EventKind::Unknown => None,
        }
    }

    fn push(&mut self, block: Block) -> Option<usize> {
        self.blocks.push(block);
        Some(self.blocks.len() - 1)
    }

    fn close_agent(&mut self) -> Option<usize> {
        match self.blocks.last_mut() {
            Some(Block::Agent { open, .. }) if *open => {
                *open = false;
                Some(self.blocks.len() - 1)
            }
            _ => None,
        }
    }
}

/// Closing the streaming block changes how it renders, so it counts as touched too.
fn earliest(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    a.into_iter().chain(b).min()
}

/// The one thing a tool call is about: a path, a command, a URL, or the input itself.
fn tool_target(input: &Value) -> String {
    ["file_path", "command", "url", "pattern", "path"]
        .iter()
        .find_map(|k| input[k].as_str())
        .map(|s| {
            // Absolute paths are long and the worktree prefix is noise; keep the last two parts.
            let parts: Vec<&str> = s.rsplit('/').take(2).collect();
            if s.starts_with('/') && parts.len() == 2 {
                format!("{}/{}", parts[1], parts[0])
            } else {
                s.to_owned()
            }
        })
        .unwrap_or_else(|| input.to_string())
}

fn detail(input: &Value) -> String {
    let full = serde_json::to_string_pretty(input).unwrap_or_default();
    // ponytail: a fixed cut; the full input is in the event log, and a scrolling viewer can come later.
    if full.chars().count() > 1200 {
        full.chars().take(1200).collect::<String>() + "\n…"
    } else {
        full
    }
}

/// Everything the window knows. Lists come from polling; transcripts and statuses from the
/// event stream.
#[derive(Default)]
pub struct Board {
    pub info: Option<DaemonInfo>,
    pub repos: Vec<Repo>,
    pub workspaces: Vec<Workspace>,
    pub sessions: Vec<Session>,
    pub transcripts: HashMap<String, Transcript>,
}

impl Board {
    /// Returns the session the event belongs to and the lowest block it touched.
    pub fn apply(&mut self, event: &Event) -> Option<(String, usize)> {
        if event.kind == EventKind::StatusChanged
            && let Ok(status) = serde_json::from_value::<Status>(event.payload["status"].clone())
            && let Some(s) = self.sessions.iter_mut().find(|s| s.id == event.session_id)
        {
            s.status = status;
        }
        let touched = self
            .transcripts
            .entry(event.session_id.clone())
            .or_default()
            .apply(event)?;
        Some((event.session_id.clone(), touched))
    }

    /// The session a workspace shows: its most recent one.
    pub fn latest_session(&self, workspace_id: &str) -> Option<&Session> {
        self.sessions
            .iter()
            .filter(|s| s.workspace_id == workspace_id)
            .max_by(|a, b| a.started_at.cmp(&b.started_at))
    }

    pub fn workspace_status(&self, workspace_id: &str) -> Option<Status> {
        self.latest_session(workspace_id).map(|s| s.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: u64, kind: EventKind, payload: Value) -> Event {
        Event {
            seq,
            session_id: "s1".into(),
            kind,
            payload,
            ts: String::new(),
        }
    }

    #[test]
    fn a_turn_with_an_approval_becomes_these_blocks() {
        let mut t = Transcript::default();
        let events = [
            ev(1, EventKind::UserMessage, json!({"text": "write it"})),
            ev(2, EventKind::AgentMessageDelta, json!({"text": "On "})),
            ev(3, EventKind::AgentMessageDelta, json!({"text": "it."})),
            ev(
                4,
                EventKind::ToolCall,
                json!({"call_id": "t1", "name": "Write", "input": {"file_path": "/w/repo/src/a.rs"}}),
            ),
            ev(
                5,
                EventKind::PermissionRequest,
                json!({"request_id": "r1", "summary": "Write: a.rs", "risk": "write", "detail": {"file_path": "a.rs"}}),
            ),
            ev(
                6,
                EventKind::PermissionResolved,
                json!({"request_id": "r1", "decision": "allow_once"}),
            ),
            ev(
                7,
                EventKind::ToolCallUpdate,
                json!({"call_id": "t1", "status": "completed"}),
            ),
            ev(8, EventKind::AgentMessageDelta, json!({"text": "Done."})),
            ev(
                9,
                EventKind::AgentMessageEnd,
                json!({"stop_reason": "end_turn"}),
            ),
        ];
        let touched: Vec<Option<usize>> = events.iter().map(|e| t.apply(e)).collect();
        assert_eq!(
            touched,
            [
                Some(0),
                Some(1),
                Some(1),
                Some(1),
                Some(3),
                Some(3),
                Some(2),
                Some(4),
                Some(4)
            ]
        );
        assert_eq!(t.blocks[0], Block::User("write it".into()));
        assert_eq!(
            t.blocks[1],
            Block::Agent {
                text: "On it.".into(),
                open: false
            }
        );
        assert!(
            matches!(&t.blocks[2], Block::Tool { target, state: ToolState::Completed, .. } if target == "src/a.rs")
        );
        assert!(
            matches!(&t.blocks[3], Block::Approval(a) if a.decision.as_deref() == Some("allow_once"))
        );
        assert_eq!(
            t.blocks[4],
            Block::Agent {
                text: "Done.".into(),
                open: false
            }
        );
    }

    #[test]
    fn a_cancelled_turn_says_so_and_status_follows_the_stream() {
        let mut board = Board::default();
        board.sessions.push(Session {
            id: "s1".into(),
            workspace_id: "w1".into(),
            agent: "claude-code".into(),
            tier: "native".into(),
            status: Status::Working,
            started_at: String::new(),
            ended_at: None,
            last_change_at: String::new(),
        });
        board.apply(&ev(
            1,
            EventKind::AgentMessageDelta,
            json!({"text": "1, 2"}),
        ));
        board.apply(&ev(
            2,
            EventKind::AgentMessageEnd,
            json!({"stop_reason": "cancelled"}),
        ));
        board.apply(&ev(3, EventKind::StatusChanged, json!({"status": "done"})));
        let blocks = &board.transcripts["s1"].blocks;
        assert_eq!(
            blocks.last(),
            Some(&Block::Note {
                text: "Cancelled".into(),
                error: false
            })
        );
        assert_eq!(board.workspace_status("w1"), Some(Status::Done));
    }
}
