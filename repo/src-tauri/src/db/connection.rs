//! SQLite connection manager + migration runner.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_in_memory_succeeds_and_has_no_path() {
        let db = Database::open_in_memory().expect("open");
        assert!(db.path().is_none(), "in-memory DB has no path");
    }

    #[test]
    fn open_creates_parent_directory_when_missing() {
        let tmp = tempdir().unwrap();
        let nested = tmp.path().join("nested/dir/that/does/not/exist/yet/shoreline.db");
        let db = Database::open(&nested).expect("open creates parents");
        assert_eq!(db.path(), Some(nested.as_path()));
        assert!(nested.exists(), "DB file must exist on disk");
    }

    #[test]
    fn open_sets_pragmas() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1, "foreign keys must be ON");
        // synchronous=NORMAL (1)
        let sync: i64 = conn
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sync, 1);
    }

    #[test]
    fn run_migrations_creates_users_table() {
        let db = Database::open_in_memory().unwrap();
        db.run_migrations().expect("first migrate ok");
        let conn = db.conn();
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='users'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 1);
    }

    #[test]
    fn run_migrations_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        db.run_migrations().expect("first");
        db.run_migrations().expect("second");
        db.run_migrations().expect("third");

        let conn = db.conn();
        let applied: i64 = conn
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        // 11 migrations declared in connection.rs
        assert_eq!(applied, 11);
    }

    #[test]
    fn run_migrations_records_each_name_in_underscore_migrations() {
        let db = Database::open_in_memory().unwrap();
        db.run_migrations().expect("ok");
        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT name FROM _migrations ORDER BY name")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(names.len(), 11);
        assert_eq!(names[0], "0001_initial_schema");
        assert_eq!(names[10], "0011_key_rotation");
    }

    #[test]
    fn run_migrations_persists_across_reopen_for_file_db() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("shoreline.db");
        {
            let db = Database::open(&path).unwrap();
            db.run_migrations().unwrap();
        }
        // Re-open the same file: nothing to apply, no error.
        let db2 = Database::open(&path).unwrap();
        db2.run_migrations().expect("second open re-migrates cleanly");
        let conn = db2.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 11);
    }

    #[test]
    fn db_error_serde_uses_snake_case_type_tag() {
        let err = DbError::Migration("boom".into());
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains(r#""type":"migration""#));
    }

    #[test]
    fn from_rusqlite_wraps_into_sqlite_variant() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();
        let res = conn.execute("SELECT * FROM nonexistent_table", []);
        let e: DbError = res.unwrap_err().into();
        assert!(matches!(e, DbError::Sqlite(_)));
    }
}
