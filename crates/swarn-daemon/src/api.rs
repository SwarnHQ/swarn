//! Local API: JSON-RPC 2.0 over `daemon.sock`, newline framed (§4.4). Filesystem permissions are
//! the only authentication on this hop, by design.

use std::path::Path;

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use swarn_core::local_api::{DaemonInfo, RepoAdd, Request, Response, RpcError};
use swarn_core::schema::Repo;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::Command;

use crate::db::Db;

#[derive(Clone)]
pub struct Api {
    pub db: Db,
    pub info: DaemonInfo,
}

impl Api {
    pub async fn serve(self, stream: UnixStream) -> std::io::Result<()> {
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        while let Some(line) = lines.next_line().await? {
            let response = match serde_json::from_str::<Request>(&line) {
                Ok(request) => self.dispatch(request).await,
                Err(e) => Response::err(Value::Null, RpcError::PARSE, e.to_string()),
            };
            let mut out = serde_json::to_vec(&response)?;
            out.push(b'\n');
            write.write_all(&out).await?;
        }
        Ok(())
    }

    async fn dispatch(&self, request: Request) -> Response {
        let id = request.id;
        let result = match request.method.as_str() {
            "daemon.info" => Ok(json!(self.info)),
            "repo.add" => match serde_json::from_value::<RepoAdd>(request.params) {
                Ok(p) => self.repo_add(p.path).await.map(|repo| json!(repo)),
                Err(e) => return Response::err(id, RpcError::INVALID_PARAMS, e.to_string()),
            },
            "repo.list" => self.repo_list().await.map(|repos| json!(repos)),
            other => {
                return Response::err(id, RpcError::METHOD_NOT_FOUND, format!("no method {other}"));
            }
        };
        match result {
            Ok(value) => Response::ok(id, value),
            Err(message) => Response::err(id, RpcError::DAEMON, message),
        }
    }

    /// Idempotent: adding a repo twice returns the existing row.
    async fn repo_add(&self, path: String) -> Result<Repo, String> {
        let root = git(&path, &["rev-parse", "--show-toplevel"])
            .await
            .ok_or_else(|| format!("{path} is not inside a git repository"))?;
        let default_branch = match git(
            &root,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .await
        {
            Some(remote_head) => remote_head.trim_start_matches("origin/").to_owned(),
            // ponytail: no origin/HEAD means "whatever is checked out now"; repo.update can fix it later.
            None => git(&root, &["symbolic-ref", "--short", "HEAD"])
                .await
                .unwrap_or_else(|| "main".into()),
        };
        let remote_url = git(&root, &["remote", "get-url", "origin"]).await;
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
            .map_err(|e| e.to_string())
    }

    async fn repo_list(&self) -> Result<Vec<Repo>, String> {
        self.db
            .call(|conn| {
                conn.prepare(&format!("{REPO_SELECT} ORDER BY created_at"))?
                    .query_map([], repo_row)?
                    .collect()
            })
            .await
            .map_err(|e| e.to_string())
    }
}

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

/// Trimmed stdout of a successful `git -C <dir> ...`, or None.
async fn git(dir: impl AsRef<Path>, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir.as_ref())
        .args(args)
        .output()
        .await
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
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
