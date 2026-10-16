//! The daemon's only SQLite connection. One thread owns it and runs jobs in order, so writes never
//! race and `event.seq` stays monotonic without a transaction dance (§4.1).

use std::path::Path;
use std::sync::mpsc;

use rusqlite::Connection;
use tokio::sync::oneshot;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Clone)]
pub struct Db(mpsc::Sender<Job>);

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut conn)?;

        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("db".into())
            .spawn(move || {
                for job in rx {
                    job(&mut conn);
                }
            })
            .expect("spawn db thread");
        Ok(Self(tx))
    }

    pub async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> rusqlite::Result<T> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(Box::new(move |conn| {
                let _ = tx.send(f(conn));
            }))
            .expect("db thread outlives the daemon");
        rx.await.expect("db thread outlives the daemon")
    }
}

fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 1 {
        let tx = conn.transaction()?;
        tx.execute_batch(include_str!("schema.sql"))?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
    }
    if version < 2 {
        // v2: the last frame counter accepted from each device, for replay protection.
        let tx = conn.transaction()?;
        tx.execute_batch("ALTER TABLE device ADD COLUMN last_ctr INTEGER NOT NULL DEFAULT 0;")?;
        tx.pragma_update(None, "user_version", 2)?;
        tx.commit()?;
    }
    Ok(())
}
