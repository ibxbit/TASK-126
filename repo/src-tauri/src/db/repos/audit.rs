//! SQLite-backed AuditWriter.

use rusqlite::Connection;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::audit::{AuditLog, AuditWriter};
use crate::db::connection::Database;

pub struct SqliteAuditWriter {
    db: Arc<Database>,
}

impl SqliteAuditWriter {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.db.conn()
    }
}

impl AuditWriter for SqliteAuditWriter {
    fn append(&self, log: &AuditLog) -> Result<(), String> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO audit_logs (id, timestamp_unix, user_id, role, tenant_id,
             action_type, entity_type, entity_id, before_state, after_state, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                log.id.to_string(),
                log.timestamp_unix,
                log.user_id.to_string(),
                log.role.as_str(),
                log.tenant_id.map(|t| t.to_string()),
                log.action_type,
                log.entity_type,
                log.entity_id,
                log.before_state.as_ref().map(|v| v.to_string()),
                log.after_state.as_ref().map(|v| v.to_string()),
                log.metadata.to_string(),
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
