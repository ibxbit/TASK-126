//! SQLite connection manager + migration runner.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(String),
    #[error("migration error: {0}")]
    Migration(String),
    #[error("lock poisoned")]
    Poisoned,
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sqlite(e.to_string())
    }
}

pub struct Database {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DbError::Sqlite(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            path: Some(path.to_path_buf()),
        })
    }

    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        Ok(Self { conn: Mutex::new(conn), path: None })
    }

    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().expect("db mutex poisoned")
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Run all embedded migrations in order. Each migration runs in
    /// its own transaction. Already-applied migrations (tracked by
    /// name in `_migrations`) are skipped.
    pub fn run_migrations(&self) -> Result<(), DbError> {
        let conn = self.conn();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _migrations (
                name TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );",
        )?;

        let migrations: Vec<(&str, &str)> = vec![
            ("0001_initial_schema", include_str!("../../migrations/0001_initial_schema.sql")),
            ("0002_parcel_state_machine", include_str!("../../migrations/0002_parcel_state_machine.sql")),
            ("0003_claims_dispute", include_str!("../../migrations/0003_claims_dispute.sql")),
            ("0004_documents", include_str!("../../migrations/0004_documents.sql")),
            ("0005_settlement", include_str!("../../migrations/0005_settlement.sql")),
            ("0006_scheduling", include_str!("../../migrations/0006_scheduling.sql")),
            ("0007_analytics", include_str!("../../migrations/0007_analytics.sql")),
            ("0008_share_packages", include_str!("../../migrations/0008_share_packages.sql")),
            ("0009_recovery_updates", include_str!("../../migrations/0009_recovery_updates.sql")),
            ("0010_audit_logs", include_str!("../../migrations/0010_audit_logs.sql")),
            ("0011_key_rotation", include_str!("../../migrations/0011_key_rotation.sql")),
        ];

        for (name, sql) in migrations {
            let already: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM _migrations WHERE name = ?1",
                    [name],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            if already {
                continue;
            }
            conn.execute_batch(sql)
                .map_err(|e| DbError::Migration(format!("{name}: {e}")))?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            conn.execute(
                "INSERT INTO _migrations (name, applied_at) VALUES (?1, ?2)",
                rusqlite::params![name, now as i64],
            )?;
        }
        Ok(())
    }
}
