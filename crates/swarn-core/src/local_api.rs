//! JSON-RPC 2.0 over the daemon's Unix socket (§4.4). One request per line, one response per line.
//! The desktop and `swarnd call` use this; the phone never does.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Request {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub const PARSE: i64 = -32700;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    /// Anything the daemon itself refused or failed at.
    pub const DAEMON: i64 = -32000;
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DaemonInfo {
    pub machine_id: String,
    pub machine_name: String,
    pub version: String,
    pub schema_min: u32,
    pub schema_max: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RepoAdd {
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorkspaceCreate {
    pub repo_id: String,
    /// Lowercase letters, digits and dashes. Becomes the branch `swarn/<name>` and the folder name.
    pub name: String,
    /// Defaults to the repo's default branch.
    #[serde(default)]
    pub base_ref: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorkspaceArchive {
    pub workspace_id: String,
    /// Remove the worktree even if it has uncommitted changes.
    #[serde(default)]
    pub force: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionStart {
    pub workspace_id: String,
    pub prompt: String,
    /// Only `claude-code` until the Codex driver lands in week 4.
    #[serde(default = "default_agent")]
    pub agent: String,
    /// Passed to the agent as-is, e.g. `haiku`. The agent's own default when absent.
    #[serde(default)]
    pub model: Option<String>,
}

fn default_agent() -> String {
    "claude-code".into()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionPrompt {
    pub session_id: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionId {
    pub session_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ApprovalResolve {
    pub request_id: String,
    pub decision: crate::schema::Decision,
}

/// `session.subscribe` answers once, then the connection carries one `event` notification per
/// line: `{"jsonrpc":"2.0","method":"event","params":<Event>}`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Subscribe {
    #[serde(default)]
    pub after_seq: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

#[cfg(feature = "local-client")]
pub use client::*;

#[cfg(feature = "local-client")]
mod client {
    use super::{Request, Response};
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    /// `$SWARN_HOME`, else `~/Library/Application Support/swarn`. Holds the socket and the database.
    pub fn swarn_home() -> PathBuf {
        std::env::var_os("SWARN_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::home_dir()
                    .expect("HOME is set")
                    .join("Library/Application Support/swarn")
            })
    }

    pub fn socket_path() -> PathBuf {
        swarn_home().join("daemon.sock")
    }

    #[derive(Debug)]
    pub enum CallError {
        /// No daemon listening, or the connection broke.
        Io(std::io::Error),
        /// The daemon answered with a JSON-RPC error.
        Rpc(super::RpcError),
        /// The daemon answered with something we could not read.
        Decode(serde_json::Error),
    }

    impl std::fmt::Display for CallError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Io(e) => write!(f, "daemon unreachable: {e}"),
                Self::Rpc(e) => write!(f, "{} ({})", e.message, e.code),
                Self::Decode(e) => write!(f, "bad response from daemon: {e}"),
            }
        }
    }

    impl std::error::Error for CallError {}

    /// One blocking request on a fresh connection. Callers on a UI thread run it in the background.
    pub fn call<T: DeserializeOwned>(method: &str, params: Value) -> Result<T, CallError> {
        let (_, value) = request(method, params)?;
        serde_json::from_value(value).map_err(CallError::Decode)
    }

    /// Streams every event after `after_seq`, old ones first, until the daemon goes away or
    /// `on_event` returns false. Blocks the calling thread.
    pub fn subscribe(
        after_seq: u64,
        mut on_event: impl FnMut(crate::schema::Event) -> bool,
    ) -> Result<(), CallError> {
        let params = serde_json::json!({ "after_seq": after_seq });
        let (mut reader, _) = request("session.subscribe", params)?;
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).map_err(CallError::Io)? == 0 {
                return Ok(());
            }
            let note: super::Notification =
                serde_json::from_str(&line).map_err(CallError::Decode)?;
            let event = serde_json::from_value(note.params).map_err(CallError::Decode)?;
            if !on_event(event) {
                return Ok(());
            }
        }
    }

    fn request(method: &str, params: Value) -> Result<(BufReader<UnixStream>, Value), CallError> {
        let mut stream = UnixStream::connect(socket_path()).map_err(CallError::Io)?;
        let request = Request {
            jsonrpc: "2.0".into(),
            id: 1.into(),
            method: method.into(),
            params,
        };
        let mut line = serde_json::to_string(&request).map_err(CallError::Decode)?;
        line.push('\n');
        stream.write_all(line.as_bytes()).map_err(CallError::Io)?;

        let mut reader = BufReader::new(stream);
        let mut reply = String::new();
        reader.read_line(&mut reply).map_err(CallError::Io)?;
        let response: Response = serde_json::from_str(&reply).map_err(CallError::Decode)?;
        match (response.result, response.error) {
            (_, Some(error)) => Err(CallError::Rpc(error)),
            (result, None) => Ok((reader, result.unwrap_or(Value::Null))),
        }
    }
}
