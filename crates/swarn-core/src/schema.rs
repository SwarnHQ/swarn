//! Session schema v1 (technical PRD §2.4): what travels between the daemon and a device, and the
//! rows the daemon keeps. Plain serde types; the envelope that wraps them lives elsewhere.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_MIN: u32 = 1;
pub const SCHEMA_MAX: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Starting,
    Working,
    NeedsInput,
    Done,
    Failed,
    Exited,
}

/// Also the `kind` column of the event log.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStarted,
    /// A prompt or follow-up the user sent. Not in the PRD's first list; added so a transcript
    /// shows both sides.
    UserMessage,
    StatusChanged,
    AgentMessageDelta,
    AgentMessageEnd,
    ThoughtDelta,
    ToolCall,
    ToolCallUpdate,
    Plan,
    PermissionRequest,
    PermissionResolved,
    Usage,
    DiffStat,
    SessionEnded,
    Error,
    /// A kind from a newer daemon. Clients skip it instead of failing, which is what lets a new
    /// kind ship without bumping the schema.
    #[serde(other)]
    Unknown,
}

/// The one place status is derived (§4.2). `pending` is how many permission requests are still
/// open after this event, because with parallel tool calls one resolution can leave others open.
pub fn next_status(current: Status, kind: EventKind, pending: usize) -> Status {
    use EventKind::*;
    let busy = if pending > 0 {
        Status::NeedsInput
    } else {
        Status::Working
    };
    match kind {
        SessionStarted | UserMessage => busy,
        AgentMessageDelta | ThoughtDelta | ToolCall | ToolCallUpdate | Plan => busy,
        PermissionRequest => Status::NeedsInput,
        PermissionResolved => busy,
        AgentMessageEnd => Status::Done,
        Error => Status::Failed,
        SessionEnded if current == Status::Failed => Status::Failed,
        SessionEnded => Status::Exited,
        StatusChanged | Usage | DiffStat | Unknown => current,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub seq: u64,
    pub session_id: String,
    pub kind: EventKind,
    pub payload: Value,
    pub ts: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub id: String,
    pub path: String,
    pub default_branch: String,
    pub remote_url: Option<String>,
    pub setup_script: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceState {
    Creating,
    Ready,
    Archived,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub branch: String,
    pub base_ref: String,
    pub worktree_path: String,
    pub state: WorkspaceState,
    pub created_at: String,
    pub archived_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub workspace_id: String,
    pub agent: String,
    pub tier: String,
    pub status: Status,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub last_change_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    All,
    Sessions(Vec<String>),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    AllowSession,
    Deny,
}

/// A signed answer to a `permission_request` (§2.6). One message type covers approve and deny;
/// `decision` says which.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub request_id: String,
    pub nonce: String,
    pub decision: Decision,
    pub device_id: String,
    pub signed_at: String,
    pub sig: String,
}

/// What a pairing phone sends first (§2.5 step 3), sealed to the daemon's static key. Keys and
/// the proof are base64url without padding.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PairRequest {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub x25519_pub: String,
    pub p256_pub: String,
    #[serde(default)]
    pub push_token: Option<String>,
    /// The pairing window token from the QR code.
    pub pw: String,
    /// P-256 signature over `crypto::pairing_signing_input`.
    pub proof: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PairAck {
    pub machine_id: String,
    pub machine_name: String,
    pub schema_min: u32,
    pub schema_max: u32,
}

/// The plaintext inside every sealed live frame. `ctr` rises strictly per sender, and a receiver
/// drops anything at or below the last one it accepted, so a frame captured on the network can't
/// be replayed (PRD review item 6).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Wire<M> {
    pub ctr: u64,
    pub msg: M,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeviceMessage {
    PairRequest(PairRequest),
    Hello {
        schema_min: u32,
        schema_max: u32,
        device_id: String,
        app_version: String,
    },
    Subscribe {
        scope: Scope,
        /// 0 asks for a snapshot first.
        after_seq: u64,
    },
    Prompt {
        session_id: String,
        text: String,
        #[serde(default)]
        attachments: Vec<Value>,
    },
    Approve(Approval),
    Cancel {
        session_id: String,
    },
    CreateWorkspace {
        repo_id: String,
        base_ref: String,
        prompt: String,
        agent: String,
    },
    Ack {
        through_seq: u64,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub kind: String,
    pub host: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonMessage {
    PairAck(PairAck),
    HelloAck {
        schema: u32,
        machine_id: String,
        machine_name: String,
        caps: Vec<String>,
    },
    Snapshot {
        workspaces: Vec<Workspace>,
        sessions: Vec<Session>,
        seq: u64,
    },
    Event(Event),
    Gap {
        from: u64,
        to: u64,
    },
    Addresses {
        candidates: Vec<Candidate>,
        sig: String,
    },
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: T) {
        let text = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<T>(&text).unwrap(), v, "{text}");
    }

    #[test]
    fn every_message_round_trips() {
        let approval = Approval {
            request_id: "01JA7".into(),
            nonce: "bm9uY2U".into(),
            decision: Decision::AllowOnce,
            device_id: "d1".into(),
            signed_at: "2026-10-15T14:00:00Z".into(),
            sig: "c2ln".into(),
        };
        for m in [
            DeviceMessage::Hello {
                schema_min: 1,
                schema_max: 1,
                device_id: "d1".into(),
                app_version: "0.1.0".into(),
            },
            DeviceMessage::Subscribe {
                scope: Scope::All,
                after_seq: 0,
            },
            DeviceMessage::Subscribe {
                scope: Scope::Sessions(vec!["s1".into()]),
                after_seq: 9,
            },
            DeviceMessage::Prompt {
                session_id: "s1".into(),
                text: "go".into(),
                attachments: vec![],
            },
            DeviceMessage::Approve(approval),
            DeviceMessage::Cancel {
                session_id: "s1".into(),
            },
            DeviceMessage::CreateWorkspace {
                repo_id: "r1".into(),
                base_ref: "main".into(),
                prompt: "fix".into(),
                agent: "claude-code".into(),
            },
            DeviceMessage::Ack { through_seq: 12 },
        ] {
            round_trip(m);
        }

        let event = Event {
            seq: 3,
            session_id: "s1".into(),
            kind: EventKind::AgentMessageDelta,
            payload: json!({"text": "hi", "index": 17}),
            ts: "2026-10-15T14:00:00Z".into(),
        };
        for m in [
            DaemonMessage::HelloAck {
                schema: 1,
                machine_id: "m1".into(),
                machine_name: "Mac".into(),
                caps: vec![],
            },
            DaemonMessage::Snapshot {
                workspaces: vec![],
                sessions: vec![],
                seq: 0,
            },
            DaemonMessage::Event(event),
            DaemonMessage::Gap { from: 1, to: 2 },
            DaemonMessage::Addresses {
                candidates: vec![Candidate {
                    kind: "lan".into(),
                    host: "192.168.1.42".into(),
                    port: 8787,
                }],
                sig: "c2ln".into(),
            },
            DaemonMessage::Error {
                code: "schema_too_old".into(),
                message: "update".into(),
                retryable: false,
            },
        ] {
            round_trip(m);
        }
    }

    #[test]
    fn wire_names_match_the_prd() {
        let v = serde_json::to_value(DeviceMessage::Ack { through_seq: 1 }).unwrap();
        assert_eq!(v, json!({"type": "ack", "through_seq": 1}));
        assert_eq!(
            serde_json::to_value(Status::NeedsInput).unwrap(),
            json!("needs_input")
        );
    }

    #[test]
    fn status_follows_every_event_kind() {
        use EventKind::*;
        use Status::*;
        let cases = [
            (Starting, SessionStarted, 0, Working),
            (Done, UserMessage, 0, Working),
            (Working, AgentMessageDelta, 0, Working),
            (Working, ThoughtDelta, 0, Working),
            (Working, ToolCall, 0, Working),
            (Working, ToolCallUpdate, 0, Working),
            (Working, Plan, 0, Working),
            (Working, PermissionRequest, 1, NeedsInput),
            (NeedsInput, PermissionResolved, 0, Working),
            // Two parallel requests: resolving one leaves the session waiting on the other.
            (NeedsInput, PermissionResolved, 1, NeedsInput),
            (NeedsInput, ToolCallUpdate, 1, NeedsInput),
            (Working, AgentMessageEnd, 0, Done),
            (Working, Error, 0, Failed),
            (Failed, SessionEnded, 0, Failed),
            (Done, SessionEnded, 0, Exited),
            (Working, Usage, 0, Working),
            (NeedsInput, DiffStat, 1, NeedsInput),
            (Done, StatusChanged, 0, Done),
            (Working, Unknown, 0, Working),
        ];
        for (current, kind, pending, expected) in cases {
            assert_eq!(
                next_status(current, kind, pending),
                expected,
                "{current:?} + {kind:?} ({pending} pending)"
            );
        }
    }

    #[test]
    fn unknown_event_kind_is_ignored_not_an_error() {
        let ev: Event = serde_json::from_value(json!({
            "seq": 1, "session_id": "s1", "kind": "from_the_future", "payload": {}, "ts": "t"
        }))
        .unwrap();
        assert_eq!(ev.kind, EventKind::Unknown);
    }
}
