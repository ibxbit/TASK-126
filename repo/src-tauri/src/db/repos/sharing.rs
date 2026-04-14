//! SQLite repo for share packages.

use std::path::PathBuf;
use std::sync::Arc;
use rusqlite::OptionalExtension;
use uuid::Uuid;

use crate::db::connection::Database;
use crate::sharing::expiry::{PackageRecord, PackageRepository};

pub struct SqlitePackageRepo { db: Arc<Database> }
impl SqlitePackageRepo { pub fn new(db: Arc<Database>) -> Self { Self { db } } }

impl PackageRepository for SqlitePackageRepo {
    fn load(&self, id: &Uuid) -> Result<Option<PackageRecord>, String> {
        let c = self.db.conn();
        c.query_row(
            "SELECT id,tenant_id,created_by,artifact_path_enc,password_hash,
             created_at_unix,expires_at_unix,revoked_at_unix
             FROM share_packages WHERE id=?1",
            [id.to_string()],
            |r| Ok(PackageRecord {
                id: pu(r.get::<_,String>(0)?),
                tenant_id: pu(r.get::<_,String>(1)?),
                created_by: pu(r.get::<_,String>(2)?),
                // artifact_path_enc is BLOB — for now store path as UTF-8 bytes.
                artifact_path: PathBuf::from(String::from_utf8_lossy(&r.get::<_,Vec<u8>>(3)?).to_string()),
                password_hash: r.get(4)?,
                created_at_unix: r.get(5)?,
                expires_at_unix: r.get(6)?,
                revoked_at_unix: r.get(7)?,
            }),
        ).optional().map_err(|e| e.to_string())
    }
    fn list_expired(&self, now: i64) -> Result<Vec<PackageRecord>, String> {
        let c = self.db.conn();
        let mut s = c.prepare(
            "SELECT id,tenant_id,created_by,artifact_path_enc,password_hash,
             created_at_unix,expires_at_unix,revoked_at_unix
             FROM share_packages WHERE expires_at_unix <= ?1 AND password_hash != ''"
        ).map_err(|e| e.to_string())?;
        let rows = s.query_map([now], |r| Ok(PackageRecord {
            id: pu(r.get::<_,String>(0)?), tenant_id: pu(r.get::<_,String>(1)?),
            created_by: pu(r.get::<_,String>(2)?),
            artifact_path: PathBuf::from(String::from_utf8_lossy(&r.get::<_,Vec<u8>>(3)?).to_string()),
            password_hash: r.get(4)?, created_at_unix: r.get(5)?,
            expires_at_unix: r.get(6)?, revoked_at_unix: r.get(7)?,
        })).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())
    }
    fn mark_revoked(&self, id: &Uuid, now: i64) -> Result<(), String> {
        let c = self.db.conn();
        c.execute("UPDATE share_packages SET revoked_at_unix=?1, password_hash='' WHERE id=?2",
            rusqlite::params![now, id.to_string()]).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn mark_scrubbed(&self, id: &Uuid) -> Result<(), String> {
        let c = self.db.conn();
        c.execute("UPDATE share_packages SET password_hash='' WHERE id=?1",
            [id.to_string()]).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn record_access(&self, id: &Uuid, now: i64) -> Result<(), String> {
        let c = self.db.conn();
        c.execute("UPDATE share_packages SET last_accessed_at_unix=?1, access_count=access_count+1 WHERE id=?2",
            rusqlite::params![now, id.to_string()]).map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn pu(s: String) -> Uuid { Uuid::parse_str(&s).unwrap_or(Uuid::nil()) }
