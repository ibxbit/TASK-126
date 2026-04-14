//! SQLite repos for recovery events + installed versions.

use std::path::PathBuf;
use std::sync::Arc;
use rusqlite::OptionalExtension;
use uuid::Uuid;

use crate::db::connection::Database;
use crate::recovery::checkpoint::{RecoveryOutcome, RecoveryRepository};
use crate::update::installer::{InstalledVersion, VersionRepository};
use crate::update::rollback::RollbackRepository;

// ── RecoveryRepository ──────────────────────────────────────────────────

pub struct SqliteRecoveryRepo { db: Arc<Database> }
impl SqliteRecoveryRepo { pub fn new(db: Arc<Database>) -> Self { Self { db } } }

impl RecoveryRepository for SqliteRecoveryRepo {
    fn record_event(&self, outcome: RecoveryOutcome, started: i64, completed: i64, details: &str) -> Result<(), String> {
        let c = self.db.conn();
        c.execute(
            "INSERT INTO recovery_events (id,started_at,completed_at,outcome,details) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![Uuid::new_v4().to_string(), started, completed, outcome.as_str(), details],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl SqliteRecoveryRepo {
    pub fn last_outcome(&self) -> Result<Option<String>, String> {
        let c = self.db.conn();
        c.query_row(
            "SELECT outcome FROM recovery_events ORDER BY started_at DESC LIMIT 1",
            [], |r| r.get(0),
        ).optional().map_err(|e| e.to_string())
    }
}

// ── VersionRepository ───────────────────────────────────────────────────

pub struct SqliteVersionRepo { db: Arc<Database> }
impl SqliteVersionRepo { pub fn new(db: Arc<Database>) -> Self { Self { db } } }

impl VersionRepository for SqliteVersionRepo {
    fn active(&self) -> Result<Option<InstalledVersion>, String> {
        load_version_where(&self.db, "is_active = 1")
    }
    fn previous(&self) -> Result<Option<InstalledVersion>, String> {
        load_version_where(&self.db, "is_active = 0 ORDER BY installed_at DESC LIMIT 1")
    }
    fn exists(&self, version: &str) -> Result<bool, String> {
        let c = self.db.conn();
        c.query_row("SELECT COUNT(*)>0 FROM app_versions WHERE version=?1", [version], |r| r.get(0))
            .map_err(|e| e.to_string())
    }
    fn install_and_activate(&self, new: &InstalledVersion) -> Result<(), String> {
        let c = self.db.conn();
        c.execute("UPDATE app_versions SET is_active=0 WHERE is_active=1", []).map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO app_versions (id,version,package_id,installed_at,is_active,snapshot_path,notes) VALUES (?1,?2,?3,?4,1,?5,?6)",
            rusqlite::params![new.id.to_string(), new.version,
                new.package_id.map(|u| u.to_string()), new.installed_at_unix,
                new.snapshot_path.as_ref().map(|p| p.display().to_string()),
                None::<String>],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn prune_older_than_previous(&self) -> Result<Vec<PathBuf>, String> {
        let c = self.db.conn();
        let mut s = c.prepare(
            "SELECT id, snapshot_path FROM app_versions WHERE is_active=0
             ORDER BY installed_at DESC"
        ).map_err(|e| e.to_string())?;
        let rows: Vec<(String, Option<String>)> = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?.filter_map(|r| r.ok()).collect();
        let mut to_prune = Vec::new();
        // Keep the first inactive (= previous), prune older.
        for (id, snap) in rows.iter().skip(1) {
            if let Some(p) = snap { to_prune.push(PathBuf::from(p)); }
            c.execute("DELETE FROM app_versions WHERE id=?1", [id]).map_err(|e| e.to_string())?;
        }
        Ok(to_prune)
    }
}

impl RollbackRepository for SqliteVersionRepo {
    fn activate_version(&self, target_id: &Uuid) -> Result<(), String> {
        let c = self.db.conn();
        c.execute("UPDATE app_versions SET is_active=0 WHERE is_active=1", []).map_err(|e| e.to_string())?;
        c.execute("UPDATE app_versions SET is_active=1 WHERE id=?1", [target_id.to_string()]).map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn load_version_where(db: &Arc<Database>, clause: &str) -> Result<Option<InstalledVersion>, String> {
    let c = db.conn();
    let sql = format!("SELECT id,version,package_id,installed_at,is_active,snapshot_path FROM app_versions WHERE {clause}");
    c.query_row(&sql, [], |r| Ok(InstalledVersion {
        id: pu(r.get::<_,String>(0)?), version: r.get(1)?,
        package_id: r.get::<_,Option<String>>(2)?.map(pu),
        installed_at_unix: r.get(3)?, is_active: r.get::<_,i64>(4)? == 1,
        snapshot_path: r.get::<_,Option<String>>(5)?.map(PathBuf::from),
    })).optional().map_err(|e| e.to_string())
}

fn pu(s: String) -> Uuid { Uuid::parse_str(&s).unwrap_or(Uuid::nil()) }
