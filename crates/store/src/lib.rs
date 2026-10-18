//! SQLite storage (WAL mode). One connection guarded by a mutex: this is a
//! single-user desktop app, and every write is a short transaction.
//! Use [`Db::call`] from async code (runs on the blocking pool) and
//! [`Db::with`] from sync code.

pub mod accounts;
pub mod activity;
pub mod bench;
pub mod ddragon;
pub mod goals;
pub mod jobs;
pub mod live;
pub mod matches;
pub mod metrics;
pub mod raw;
pub mod reports;
pub mod settings;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

pub use rusqlite;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("background task failed: {0}")]
    Join(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("../migrations/001_init.sql")),
        M::up(include_str!("../migrations/002_metrics_ai_activity.sql")),
        M::up(include_str!("../migrations/003_signals_goals_sessions.sql")),
        M::up(include_str!("../migrations/004_live.sql")),
        M::up(include_str!("../migrations/005_benchmarks_ranks_briefs.sql")),
    ])
}

#[derive(Clone, Debug)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(&mut conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        Self::init(&mut conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    fn init(conn: &mut Connection) -> Result<()> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrations().to_latest(conn)?;
        Ok(())
    }

    /// Run a short operation on the calling thread.
    pub fn with<R>(&self, f: impl FnOnce(&mut Connection) -> Result<R>) -> Result<R> {
        let mut conn = self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut conn)
    }

    /// Run an operation on the blocking thread pool (for async callers).
    pub async fn call<R, F>(&self, f: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.with(f)).await.map_err(|e| StoreError::Join(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_valid() {
        migrations().validate().expect("migrations validate");
    }

    #[test]
    fn opens_file_db_in_wal_mode() {
        let dir = std::env::temp_dir().join(format!("rc-store-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let db = Db::open(&dir.join("t.db")).expect("open");
        let mode: String = db.with(|c| Ok(c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?)).expect("pragma");
        assert_eq!(mode.to_lowercase(), "wal");
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
