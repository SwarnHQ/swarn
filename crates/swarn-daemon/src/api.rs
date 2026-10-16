//! Local API: JSON-RPC 2.0 over `daemon.sock`, newline framed (§4.4). Filesystem permissions are
//! the only authentication on this hop, by design.

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::{OptionalExtension, params};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use swarn_core::local_api::{
    ApprovalResolve, DaemonInfo, Notification, PrCreate, RepoAdd, Request, Response, RpcError,
    SessionId, SessionPrompt, SessionStart, Subscribe, WorkspaceArchive, WorkspaceCreate,
    WorkspaceId,
};
use swarn_core::schema::{Event, Repo, WorkspaceState};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::process::Command;
use tokio::sync::broadcast::error::RecvError;

use crate::db::Db;
use crate::env::AgentEnv;
use crate::events::EventLog;
use crate::review;
use crate::sessions::{Driver, Sessions};
use crate::workspace::{self, run_git};

#[derive(Clone)]
pub struct Api {
    pub db: Db,
    pub info: DaemonInfo,
    pub home: PathBuf,
    pub log: EventLog,
    pub sessions: Arc<Sessions>,
    pub env: AgentEnv,
}

impl Api {
    pub async fn serve(self, stream: UnixStream) -> std::io::Result<()> {
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        while let Some(line) = lines.next_line().await? {
            let request = match serde_json::from_str::<Request>(&line) {
                Ok(request) => request,
                Err(e) => {
                    let response = Response::err(Value::Null, RpcError::PARSE, e.to_string());
                    send(&mut write, &response).await?;
                    continue;
                }
            };
            if request.method == "session.subscribe" {
                // The connection becomes a one-way event stream from here on (§4.4).
                let after = match params::<Subscribe>(request.params) {
                    Ok(p) => p.after_seq,
                    Err(e) => {
                        let response = Response::err(request.id, RpcError::INVALID_PARAMS, e);
                        return send(&mut write, &response).await;
                    }
                };
                send(
                    &mut write,
                    &Response::ok(request.id, json!({ "subscribed": true })),
                )
                .await?;
                return self.stream(write, after).await;
            }
            let response = self.dispatch(request).await;
            send(&mut write, &response).await?;
        }
        Ok(())
    }

    /// Backlog from the database, then live events. Subscribing before reading the backlog and
    /// skipping anything at or below the last sent `seq` means nothing falls in the gap between
    /// them, and nothing is sent twice.
    async fn stream(&self, mut write: OwnedWriteHalf, mut last: u64) -> std::io::Result<()> {
        let mut live = self.log.subscribe();
        loop {
            let backlog = self.log.since(last).await.map_err(std::io::Error::other)?;
            for event in backlog {
                last = event.seq;
                send(&mut write, &notification(&event)).await?;
            }
            loop {
                match live.recv().await {
                    Ok(event) if event.seq <= last => {}
                    Ok(event) => {
                        last = event.seq;
                        send(&mut write, &notification(&event)).await?;
                    }
                    // Fell behind the broadcast buffer: catch up from the database instead.
                    Err(RecvError::Lagged(_)) => break,
                    Err(RecvError::Closed) => return Ok(()),
                }
            }
        }
    }

    async fn dispatch(&self, request: Request) -> Response {
        let id = request.id;
        let p = request.params;
        let result = match request.method.as_str() {
            "daemon.info" => Ok(json!(self.info)),
            "repo.add" => match params::<RepoAdd>(p) {
                Ok(p) => self.repo_add(p.path).await.map(|repo| json!(repo)),
                Err(e) => Err(Invalid(e)),
            },
            "repo.list" => self.repo_list().await.map(|repos| json!(repos)),
            "workspace.create" => match params::<WorkspaceCreate>(p) {
                Ok(p) => self.workspace_create(p).await,
                Err(e) => Err(Invalid(e)),
            },
            "workspace.list" => workspace::list(&self.db)
                .await
                .map(|w| json!(w))
                .map_err(Failed),
            "setup.check" => Ok(crate::setup::check(&self.env).await),
            "policy.get" => Ok(json!(crate::policy::load())),
            "policy.set" => match params::<swarn_core::local_api::Policy>(p) {
                Ok(policy) => crate::policy::save(&policy)
                    .map(|()| json!(policy))
                    .map_err(Failed),
                Err(e) => Err(Invalid(e)),
            },
            "diff.get" => match params::<WorkspaceId>(p) {
                Ok(p) => match self.workspace(&p.workspace_id).await {
                    Ok(ws) => review::diff(&ws)
                        .await
                        .map(|diff| json!({ "diff": diff }))
                        .map_err(Failed),
                    Err(e) => Err(e),
                },
                Err(e) => Err(Invalid(e)),
            },
            "pr.create" => match params::<PrCreate>(p) {
                Ok(p) => match self.workspace(&p.workspace_id).await {
                    Ok(ws) => review::create_pr(&self.env, &ws, &p.title, &p.body)
                        .await
                        .map(|url| json!({ "url": url }))
                        .map_err(Failed),
                    Err(e) => Err(e),
                },
                Err(e) => Err(Invalid(e)),
            },
            "workspace.archive" => match params::<WorkspaceArchive>(p) {
                Ok(p) => workspace::archive(&self.db, p.workspace_id, p.force)
                    .await
                    .map(|w| json!(w))
                    .map_err(Failed),
                Err(e) => Err(Invalid(e)),
            },
            "session.start" => match params::<SessionStart>(p) {
                Ok(p) => self.session_start(p).await,
                Err(e) => Err(Invalid(e)),
            },
            "session.list" => self.sessions.list().await.map(|s| json!(s)).map_err(Failed),
            "session.prompt" => match params::<SessionPrompt>(p) {
                Ok(p) => self
                    .sessions
                    .prompt(&p.session_id, p.text)
                    .await
                    .map(|delivery| json!({ "delivery": delivery }))
                    .map_err(Failed),
                Err(e) => Err(Invalid(e)),
            },
            "session.cancel" => match params::<SessionId>(p) {
                Ok(p) => self
                    .sessions
                    .cancel(&p.session_id)
                    .await
                    .map(|()| json!({}))
                    .map_err(Failed),
                Err(e) => Err(Invalid(e)),
            },
            // Local and unsigned: the socket's file permissions are the authentication (§1.2).
            // Signed approvals from a phone arrive in week 5 with the §2.6 checks.
            "approval.resolve" => match params::<ApprovalResolve>(p) {
                Ok(p) => self
                    .sessions
                    .resolve(&p.request_id, p.decision, None)
                    .await
                    .map(|()| json!({}))
                    .map_err(Failed),
                Err(e) => Err(Invalid(e)),
            },
            other => Err(Unknown(format!("no method {other}"))),
        };
        match result {
            Ok(value) => Response::ok(id, value),
            Err(Invalid(message)) => Response::err(id, RpcError::INVALID_PARAMS, message),
            Err(Unknown(message)) => Response::err(id, RpcError::METHOD_NOT_FOUND, message),
            Err(Failed(message)) => Response::err(id, RpcError::DAEMON, message),
        }
    }

    async fn workspace_create(&self, p: WorkspaceCreate) -> Result<Value, Failure> {
        let repo_id = p.repo_id.clone();
        let repo = self
            .db
            .call(move |conn| {
                conn.query_row(&format!("{REPO_SELECT} WHERE id = ?1"), [repo_id], repo_row)
                    .optional()
            })
            .await
            .map_err(|e| Failed(e.to_string()))?
            .ok_or_else(|| Failed(format!("no repo {}", p.repo_id)))?;
        workspace::create(&self.db, &self.home, repo, p.name, p.base_ref)
            .await
            .map(|w| json!(w))
            .map_err(Failed)
    }

    async fn workspace(&self, id: &str) -> Result<swarn_core::schema::Workspace, Failure> {
        workspace::get(&self.db, id)
            .await
            .map_err(Failed)?
            .ok_or_else(|| Failed(format!("no workspace {id}")))
    }

    async fn session_start(&self, p: SessionStart) -> Result<Value, Failure> {
        let driver =
            Driver::for_agent(&p.agent, crate::policy::load().approvals.mode).ok_or_else(|| {
                Failed(format!(
                    "unknown agent {:?}; use \"claude-code\" or \"codex\"",
                    p.agent
                ))
            })?;
        let ws = workspace::get(&self.db, &p.workspace_id)
            .await
            .map_err(Failed)?
            .ok_or_else(|| Failed(format!("no workspace {}", p.workspace_id)))?;
        if ws.state != WorkspaceState::Ready {
            return Err(Failed(format!(
                "workspace {} is {:?}, not ready",
                ws.name, ws.state
            )));
        }
        self.sessions
            .start(driver, ws.id, ws.worktree_path, p.prompt, p.model)
            .await
            .map(|s| json!(s))
            .map_err(Failed)
    }

    /// Idempotent: adding a repo twice returns the existing row.
    async fn repo_add(&self, path: String) -> Result<Repo, Failure> {
        let root = run_git(&path, &["rev-parse", "--show-toplevel"])
            .await
            .map_err(|_| Failed(format!("{path} is not inside a git repository")))?;
        let remote_head = ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"];
        let default_branch = match run_git(&root, &remote_head).await {
            Ok(head) => head.trim_start_matches("origin/").to_owned(),
            // ponytail: no origin/HEAD means "whatever is checked out now"; repo.update can fix it later.
            Err(_) => run_git(&root, &["symbolic-ref", "--short", "HEAD"])
                .await
                .unwrap_or_else(|_| "main".into()),
        };
        let remote_url = run_git(&root, &["remote", "get-url", "origin"]).await.ok();
        let id = ulid::Ulid::generate().to_string();

        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO repo (id, path, default_branch, remote_url, created_at)
                     VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                     ON CONFLICT(path) DO NOTHING",
                    params![id, root, default_branch, remote_url],
                )?;
                conn.query_row(&format!("{REPO_SELECT} WHERE path = ?1"), [&root], repo_row)
            })
            .await
            .map_err(|e| Failed(e.to_string()))
    }

    async fn repo_list(&self) -> Result<Vec<Repo>, Failure> {
        self.db
            .call(|conn| {
                conn.prepare(&format!("{REPO_SELECT} ORDER BY created_at"))?
                    .query_map([], repo_row)?
                    .collect()
            })
            .await
            .map_err(|e| Failed(e.to_string()))
    }
}

/// Which JSON-RPC error code a failure becomes.
enum Failure {
    Invalid(String),
    Unknown(String),
    Failed(String),
}
use Failure::{Failed, Invalid, Unknown};

const REPO_SELECT: &str =
    "SELECT id, path, default_branch, remote_url, setup_script, created_at FROM repo";

fn repo_row(row: &rusqlite::Row) -> rusqlite::Result<Repo> {
    Ok(Repo {
        id: row.get(0)?,
        path: row.get(1)?,
        default_branch: row.get(2)?,
        remote_url: row.get(3)?,
        setup_script: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn params<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn notification(event: &Event) -> Notification {
    Notification {
        jsonrpc: "2.0".into(),
        method: "event".into(),
        params: json!(event),
    }
}

async fn send(write: &mut OwnedWriteHalf, message: &impl serde::Serialize) -> std::io::Result<()> {
    let mut out = serde_json::to_vec(message)?;
    out.push(b'\n');
    write.write_all(&out).await
}

/// The machine row is created on first start and never changes after that.
pub async fn machine_info(db: &Db) -> rusqlite::Result<DaemonInfo> {
    let name = computer_name().await;
    let id = random_id()?;
    db.call(move |conn| {
        let existing: Option<(String, String)> = conn
            .query_row("SELECT id, name FROM machine LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let (machine_id, machine_name) = match existing {
            Some(row) => row,
            None => {
                conn.execute(
                    "INSERT INTO machine (id, name, schema_min, schema_max, created_at)
                     VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                    params![
                        id,
                        name,
                        swarn_core::schema::SCHEMA_MIN,
                        swarn_core::schema::SCHEMA_MAX
                    ],
                )?;
                (id, name)
            }
        };
        Ok(DaemonInfo {
            machine_id,
            machine_name,
            version: env!("CARGO_PKG_VERSION").into(),
            schema_min: swarn_core::schema::SCHEMA_MIN,
            schema_max: swarn_core::schema::SCHEMA_MAX,
        })
    })
    .await
}

async fn computer_name() -> String {
    let out = Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .await;
    match out {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        _ => "Mac".into(),
    }
}

/// 16 random bytes, hex. §2.5 wants base64url in the QR; re-encode there.
fn random_id() -> rusqlite::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
