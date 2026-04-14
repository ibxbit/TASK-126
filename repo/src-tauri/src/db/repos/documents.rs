//! SQLite repos for document management: uploads, tags, search, preview.

use std::sync::{Arc, MutexGuard};
use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

use crate::db::connection::Database;
use crate::docs::chunks::{ChunkRepository, UploadSession};

pub struct SqliteChunkRepo { db: Arc<Database> }
impl SqliteChunkRepo {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
    fn c(&self) -> MutexGuard<'_, Connection> { self.db.conn() }
}

impl ChunkRepository for SqliteChunkRepo {
    fn create_session(&self, s: &UploadSession, dn_enc: &[u8], staging: &str) -> Result<(), String> {
        let c = self.c();
        c.execute(
            "INSERT INTO upload_sessions (id, tenant_id, entity_kind, entity_id, display_name_enc,
             mime_type, total_bytes, chunk_size, chunk_count, expected_sha256_hex, staging_rel_path,
             target_attachment_id, status, created_at, updated_at, created_by)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'in_progress',?13,?13,NULL)",
            rusqlite::params![
                s.id.to_string(), s.tenant_id.to_string(), s.entity_kind, s.entity_id.to_string(),
                dn_enc, s.mime_type, s.total_bytes as i64, s.chunk_size as i64,
                s.chunk_count as i64, s.expected_sha256_hex, staging,
                s.target_attachment_id.map(|u| u.to_string()), now()
            ],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn load_session(&self, id: &Uuid) -> Result<Option<UploadSession>, String> {
        let c = self.c();
        c.query_row(
            "SELECT id,tenant_id,entity_kind,entity_id,mime_type,total_bytes,chunk_size,
             chunk_count,expected_sha256_hex,target_attachment_id,status FROM upload_sessions WHERE id=?1",
            [id.to_string()],
            |r| Ok(UploadSession {
                id: pu(r.get::<_,String>(0)?), tenant_id: pu(r.get::<_,String>(1)?),
                entity_kind: r.get(2)?, entity_id: pu(r.get::<_,String>(3)?),
                mime_type: r.get(4)?, total_bytes: r.get::<_,i64>(5)? as u64,
                chunk_size: r.get::<_,i64>(6)? as u64, chunk_count: r.get::<_,i64>(7)? as u32,
                expected_sha256_hex: r.get(8)?,
                target_attachment_id: r.get::<_,Option<String>>(9)?.map(pu),
                status: r.get(10)?,
            }),
        ).optional().map_err(|e| e.to_string())
    }
    fn record_chunk(&self, sid: &Uuid, idx: u32, size: u64, sha: &str, at: i64) -> Result<(), String> {
        let c = self.c();
        c.execute(
            "INSERT OR REPLACE INTO upload_chunks (session_id,chunk_index,byte_size,sha256_hex,received_at) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![sid.to_string(), idx, size as i64, sha, at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn list_received(&self, sid: &Uuid) -> Result<Vec<u32>, String> {
        let c = self.c();
        let mut s = c.prepare("SELECT chunk_index FROM upload_chunks WHERE session_id=?1 ORDER BY chunk_index")
            .map_err(|e| e.to_string())?;
        let r = s.query_map([sid.to_string()], |r| r.get::<_,i64>(0).map(|v| v as u32))
            .map_err(|e| e.to_string())?;
        r.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())
    }
    fn mark_status(&self, sid: &Uuid, status: &str) -> Result<(), String> {
        let c = self.c();
        c.execute("UPDATE upload_sessions SET status=?1, updated_at=?2 WHERE id=?3",
            rusqlite::params![status, now(), sid.to_string()]).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn register_version(&self, session: &UploadSession, _dn: &[u8], rp_enc: &[u8],
        sha: &str, size: u64, ts: i64, by: Option<&Uuid>) -> Result<(Uuid, u32), String> {
        let c = self.c();
        let att_id = session.target_attachment_id.unwrap_or_else(Uuid::new_v4);
        // Upsert attachment row.
        c.execute(
            "INSERT OR IGNORE INTO attachments (id,tenant_id,entity_kind,entity_id,display_name_enc,
             relative_path_enc,mime_type,byte_size,sha256_hex,created_at,created_by)
             VALUES (?1,?2,?3,?4,(SELECT display_name_enc FROM upload_sessions WHERE id=?5),
             ?6,?7,?8,?9,?10,?11)",
            rusqlite::params![att_id.to_string(), session.tenant_id.to_string(),
                session.entity_kind, session.entity_id.to_string(), session.id.to_string(),
                rp_enc, session.mime_type, size as i64, sha, ts,
                by.map(|u| u.to_string())],
        ).map_err(|e| e.to_string())?;
        let ver: i64 = c.query_row(
            "SELECT COALESCE(MAX(version_no),0)+1 FROM attachment_versions WHERE attachment_id=?1",
            [att_id.to_string()], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        c.execute(
            "INSERT INTO attachment_versions (id,attachment_id,version_no,relative_path_enc,byte_size,sha256_hex,created_at,created_by)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![Uuid::new_v4().to_string(), att_id.to_string(), ver, rp_enc,
                size as i64, sha, ts, by.map(|u| u.to_string())],
        ).map_err(|e| e.to_string())?;
        Ok((att_id, ver as u32))
    }
}

pub struct SqliteTagRepo { db: Arc<Database> }
impl SqliteTagRepo {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
}
impl SqliteTagRepo {
    pub fn add(&self, att_id: &Uuid, tag: &str, by: Option<&Uuid>) -> Result<(), String> {
        let c = self.db.conn();
        c.execute("INSERT OR IGNORE INTO attachment_tags (attachment_id,tag,created_at,created_by) VALUES (?1,?2,?3,?4)",
            rusqlite::params![att_id.to_string(), tag, now(), by.map(|u| u.to_string())])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn remove(&self, att_id: &Uuid, tag: &str) -> Result<(), String> {
        let c = self.db.conn();
        c.execute("DELETE FROM attachment_tags WHERE attachment_id=?1 AND tag=?2",
            rusqlite::params![att_id.to_string(), tag]).map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub struct SqliteAttachmentSearch { db: Arc<Database> }
impl SqliteAttachmentSearch {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
    pub fn search(&self, tenant_id: &Uuid, mime: Option<&str>, tag: Option<&str>,
        entity_kind: Option<&str>, entity_id: Option<&Uuid>, limit: u32
    ) -> Result<Vec<serde_json::Value>, String> {
        let c = self.db.conn();
        let mut sql = "SELECT a.id, a.entity_kind, a.entity_id, a.mime_type, a.byte_size, a.sha256_hex, a.created_at
            FROM attachments a WHERE a.tenant_id = ?1 AND a.deleted_at IS NULL".to_string();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(tenant_id.to_string())];
        let mut idx = 2;
        if let Some(m) = mime {
            sql.push_str(&format!(" AND a.mime_type = ?{idx}")); params.push(Box::new(m.to_string())); idx += 1;
        }
        if let Some(ek) = entity_kind {
            sql.push_str(&format!(" AND a.entity_kind = ?{idx}")); params.push(Box::new(ek.to_string())); idx += 1;
        }
        if let Some(eid) = entity_id {
            sql.push_str(&format!(" AND a.entity_id = ?{idx}")); params.push(Box::new(eid.to_string())); idx += 1;
        }
        if let Some(t) = tag {
            sql.push_str(&format!(" AND EXISTS (SELECT 1 FROM attachment_tags t WHERE t.attachment_id = a.id AND t.tag = ?{idx})"));
            params.push(Box::new(t.to_string())); idx += 1;
        }
        sql.push_str(&format!(" ORDER BY a.created_at DESC LIMIT ?{idx}"));
        params.push(Box::new(limit as i64));
        let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = c.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(refs.as_slice(), |r| {
            Ok(serde_json::json!({
                "attachment_id": r.get::<_,String>(0)?,
                "entity_kind": r.get::<_,String>(1)?,
                "entity_id": r.get::<_,String>(2)?,
                "mime_type": r.get::<_,String>(3)?,
                "byte_size": r.get::<_,i64>(4)?,
                "sha256_hex": r.get::<_,String>(5)?,
                "created_at": r.get::<_,i64>(6)?,
            }))
        }).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())
    }
}

fn pu(s: String) -> Uuid { Uuid::parse_str(&s).unwrap_or(Uuid::nil()) }
fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }
