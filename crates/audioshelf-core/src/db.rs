use crate::error::AppError;
use parking_lot::Mutex;
use rusqlite::Connection;
use std::{path::Path, sync::Arc};

/// One shared connection, serialised by a mutex. SQLite in WAL mode is fast
/// enough for a desktop library; all access goes through `with_db`.
pub type Db = Arc<Mutex<Connection>>;

const MIGRATION_1: &str = include_str!("../../../migrations/0001_init.sql");

pub fn open(path: &Path) -> Result<Db, AppError> {
    let conn = Connection::open(path)?;
    open_connection(conn)
}

pub fn open_in_memory() -> Result<Db, AppError> {
    open_connection(Connection::open_in_memory()?)
}

fn open_connection(conn: Connection) -> Result<Db, AppError> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
         PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;",
    )?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(MIGRATION_1)?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    // Jobs that were queued or running when the app quit cannot resume.
    conn.execute(
        "UPDATE jobs SET status='failed', error='interrupted by shutdown'
         WHERE status IN ('queued','running')",
        [],
    )?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// Runs blocking SQLite work on the blocking pool, against the shared connection.
pub async fn with_db<T, F>(db: &Db, f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
{
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = db.lock();
        Ok(f(&mut conn)?)
    })
    .await?
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
