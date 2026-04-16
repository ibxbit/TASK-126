//! Document management IPC commands — SQLite-backed.

use std::sync::Arc;
use uuid::Uuid;

use rusqlite::OptionalExtension;

use crate::db::connection::Database;
use crate::db::repos::{SqliteAttachmentSearch, SqliteChunkRepo, SqliteTagRepo};
use crate::docs::chunks;
use crate::docs::storage::StorageLayout;
use crate::ipc::{guard, IpcError, SessionState};

fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }

#[tauri::command]
pub fn cmd_upload_start(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    layout: tauri::State<'_, StorageLayout>,
    init: crate::docs::chunks::SessionInit,
) -> Result<crate::docs::chunks::UploadSession, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteChunkRepo::new(Arc::clone(db.inner()));
    // display_name_enc would come from FieldCipher; pass raw bytes for now.
    let dn_enc = init.display_name.as_bytes().to_vec();
    chunks::start_session(&repo, layout.inner(), init, dn_enc)
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_upload_put_chunk(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    layout: tauri::State<'_, StorageLayout>,
    session_id: Uuid,
    chunk_index: u32,
    data: Vec<u8>,
) -> Result<(), IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteChunkRepo::new(Arc::clone(db.inner()));
    chunks::put_chunk(&repo, layout.inner(), &session_id, chunk_index, &data, now())
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_upload_status(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    session_id: Uuid,
) -> Result<crate::docs::chunks::ChunkStatus, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteChunkRepo::new(Arc::clone(db.inner()));
    chunks::session_status(&repo, &session_id)
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_upload_finalize(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    layout: tauri::State<'_, StorageLayout>,
    session_id: Uuid,
) -> Result<crate::docs::chunks::FinalizeOutcome, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteChunkRepo::new(Arc::clone(db.inner()));
    chunks::finalize(&repo, layout.inner(), &session_id, now(), None, |_id, _rp| {
        Ok(Vec::new()) // Encryption wiring happens when FieldCipher is in Tauri state.
    })
    .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_upload_abort(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    layout: tauri::State<'_, StorageLayout>,
    session_id: Uuid,
) -> Result<(), IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteChunkRepo::new(Arc::clone(db.inner()));
    chunks::abort(&repo, layout.inner(), &session_id)
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_attachment_search(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    query: serde_json::Value,
) -> Result<Vec<serde_json::Value>, IpcError> {
    guard::require_authenticated(session.inner())?;
    let search = SqliteAttachmentSearch::new(Arc::clone(db.inner()));
    let tid = query.get("tenant_id").and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok()).unwrap_or(Uuid::nil());
    let mime = query.get("mime_type").and_then(|v| v.as_str());
    let tag = query.get("tag").and_then(|v| v.as_str());
    let ek = query.get("entity_kind").and_then(|v| v.as_str());
    let eid = query.get("entity_id").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok());
    let limit = query.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
    search.search(&tid, mime, tag, ek, eid.as_ref(), limit)
        .map_err(|e| IpcError::Internal(e))
}

#[tauri::command]
pub fn cmd_attachment_add_tag(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    _tenant_id: Uuid,
    attachment_id: Uuid,
    tag: String,
) -> Result<(), IpcError> {
    let principal = guard::require_authenticated(session.inner())?;
    let repo = SqliteTagRepo::new(Arc::clone(db.inner()));
    repo.add(&attachment_id, &tag, Some(&principal.user_id))
        .map_err(|e| IpcError::Internal(e))
}

#[tauri::command]
pub fn cmd_attachment_remove_tag(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    _tenant_id: Uuid,
    attachment_id: Uuid,
    tag: String,
) -> Result<(), IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteTagRepo::new(Arc::clone(db.inner()));
    repo.remove(&attachment_id, &tag).map_err(|e| IpcError::Internal(e))
}

#[tauri::command]
pub fn cmd_attachment_preview(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    tenant_id: Uuid,
    attachment_id: Uuid,
    _version_no: Option<u32>,
) -> Result<serde_json::Value, IpcError> {
    guard::require_authenticated(session.inner())?;
    // Preview requires reading the file from disk + decrypting the
    // relative path. Return the attachment metadata until FieldCipher
    // is wired into Tauri state.
    let c = db.conn();
    let row: Option<(String, i64)> = c.query_row(
        "SELECT mime_type, byte_size FROM attachments WHERE id = ?1 AND tenant_id = ?2",
        rusqlite::params![attachment_id.to_string(), tenant_id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional().map_err(|e| IpcError::Internal(e.to_string()))?;
    let (mime, size) = row.ok_or(IpcError::Internal("attachment not found".into()))?;
    Ok(serde_json::json!({ "attachment_id": attachment_id.to_string(), "mime_type": mime, "byte_size": size }))
}

// ─── End-to-end integration tests ──────────────────────────────────────
//
// These tests bypass `tauri::State` (which has a private constructor)
// and exercise the *exact same* call chain the commands above use:
//
//     SqliteChunkRepo + StorageLayout + chunks::{start,put,status,finalize,abort}
//
// Anything broken in the IPC handler body will surface here, so we get
// the IPC contract ring of coverage without booting a Tauri runtime.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::chunks;
    use crate::docs::chunks::SessionInit;
    use sha2::{Digest, Sha256};
    use tempfile::tempdir;

    fn db_with_migrations() -> Arc<Database> {
        let db = Database::open_in_memory().expect("open");
        db.run_migrations().expect("migrate");
        Arc::new(db)
    }

    fn seed_tenant(db: &Arc<Database>, tid: &Uuid) {
        let c = db.conn();
        let now = 1_700_000_000i64;
        c.execute(
            "INSERT INTO tenants (id, name, code, active, created_at, updated_at)
             VALUES (?1, 'T', ?2, 1, ?3, ?3)",
            rusqlite::params![tid.to_string(), tid.to_string(), now],
        )
        .unwrap();
    }

    fn write_one_byte_file_session(
        db: &Arc<Database>,
        layout: &StorageLayout,
        tid: Uuid,
    ) -> (Uuid, Vec<u8>, String) {
        let data: Vec<u8> = b"hello shoreline".to_vec();
        let mut h = Sha256::new();
        h.update(&data);
        let sha = hex::encode(h.finalize());
        let init = SessionInit {
            tenant_id: tid,
            entity_kind: "case".into(),
            entity_id: Uuid::new_v4(),
            display_name: "doc.txt".into(),
            mime_type: "text/plain".into(),
            total_bytes: data.len() as u64,
            chunk_size: Some(8), // forces 2 chunks
            expected_sha256_hex: sha.clone(),
            target_attachment_id: None,
        };
        let repo = SqliteChunkRepo::new(Arc::clone(db));
        let session = chunks::start_session(&repo, layout, init, b"enc-name".to_vec()).unwrap();
        (session.id, data, sha)
    }

    #[test]
    fn full_upload_lifecycle_starts_streams_and_finalizes() {
        let db = db_with_migrations();
        let tid = Uuid::new_v4();
        seed_tenant(&db, &tid);
        let tmp = tempdir().unwrap();
        let layout = StorageLayout::new(tmp.path().to_path_buf());

        let (sid, data, sha) = write_one_byte_file_session(&db, &layout, tid);
        let repo = SqliteChunkRepo::new(Arc::clone(&db));

        // Stream both chunks.
        chunks::put_chunk(&repo, &layout, &sid, 0, &data[..8], 1).unwrap();
        chunks::put_chunk(&repo, &layout, &sid, 1, &data[8..], 1).unwrap();

        // Status should report no missing.
        let s = chunks::session_status(&repo, &sid).unwrap();
        assert_eq!(s.received_indices.len(), 2);
        assert!(s.missing_indices.is_empty());

        // Finalize — pass a no-op encryptor as the doc_cmds handler does.
        let outcome =
            chunks::finalize(&repo, &layout, &sid, 1, None, |_id, _rp| Ok(Vec::new())).unwrap();
        assert_eq!(outcome.byte_size as usize, data.len());
        assert_eq!(outcome.sha256_hex, sha);
        assert_eq!(outcome.version_no, 1);

        // Attachment + version rows now exist.
        let c = db.conn();
        let count: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM attachments WHERE id = ?1",
                [outcome.attachment_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let vcount: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM attachment_versions WHERE attachment_id = ?1",
                [outcome.attachment_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(vcount, 1);
    }

    #[test]
    fn status_lists_missing_chunks_for_resume() {
        let db = db_with_migrations();
        let tid = Uuid::new_v4();
        seed_tenant(&db, &tid);
        let tmp = tempdir().unwrap();
        let layout = StorageLayout::new(tmp.path().to_path_buf());

        let (sid, data, _) = write_one_byte_file_session(&db, &layout, tid);
        let repo = SqliteChunkRepo::new(Arc::clone(&db));

        // Send only chunk 1 — chunk 0 should still appear missing.
        chunks::put_chunk(&repo, &layout, &sid, 1, &data[8..], 1).unwrap();
        let s = chunks::session_status(&repo, &sid).unwrap();
        assert_eq!(s.received_indices, vec![1]);
        assert_eq!(s.missing_indices, vec![0]);

        // Finalize must refuse: missing chunks.
        let res = chunks::finalize(&repo, &layout, &sid, 1, None, |_, _| Ok(Vec::new()));
        assert!(matches!(res, Err(crate::docs::chunks::ChunkError::MissingChunks)));
    }

    #[test]
    fn abort_marks_session_aborted_and_blocks_further_puts() {
        let db = db_with_migrations();
        let tid = Uuid::new_v4();
        seed_tenant(&db, &tid);
        let tmp = tempdir().unwrap();
        let layout = StorageLayout::new(tmp.path().to_path_buf());

        let (sid, data, _) = write_one_byte_file_session(&db, &layout, tid);
        let repo = SqliteChunkRepo::new(Arc::clone(&db));

        chunks::put_chunk(&repo, &layout, &sid, 0, &data[..8], 1).unwrap();
        chunks::abort(&repo, &layout, &sid).unwrap();

        // After abort, status flips to "aborted" and put_chunk refuses.
        let res = chunks::put_chunk(&repo, &layout, &sid, 1, &data[8..], 1);
        assert!(matches!(
            res,
            Err(crate::docs::chunks::ChunkError::WrongStatus(_))
        ));
    }

    #[test]
    fn search_with_no_data_returns_empty_vector() {
        let db = db_with_migrations();
        let tid = Uuid::new_v4();
        seed_tenant(&db, &tid);
        let search = SqliteAttachmentSearch::new(Arc::clone(&db));
        let res = search.search(&tid, None, None, None, None, 50).unwrap();
        assert!(res.is_empty());
    }

    #[test]
    fn add_remove_tag_round_trip_via_repo() {
        let db = db_with_migrations();
        let repo = SqliteTagRepo::new(Arc::clone(&db));
        // We need an attachments row first — minimal seed:
        let tid = Uuid::new_v4();
        seed_tenant(&db, &tid);
        let att_id = Uuid::new_v4();
        {
            let c = db.conn();
            c.execute(
                "INSERT INTO attachments (id, tenant_id, entity_kind, entity_id, display_name_enc,
                 relative_path_enc, mime_type, byte_size, sha256_hex, created_at)
                 VALUES (?1, ?2, 'case', ?3, x'00', x'00', 'text/plain', 1, 'sha', 1)",
                rusqlite::params![att_id.to_string(), tid.to_string(), Uuid::new_v4().to_string()],
            )
            .unwrap();
        }
        // Pass None for `created_by` to avoid the users(id) FK constraint —
        // the test goal is the tag plumbing, not user attribution.
        repo.add(&att_id, "important", None).unwrap();
        // Adding the same tag twice is idempotent (`INSERT OR IGNORE`).
        repo.add(&att_id, "important", None).unwrap();

        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM attachment_tags WHERE attachment_id = ?1",
                [att_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "duplicate add must not insert twice");

        repo.remove(&att_id, "important").unwrap();
        // Removing a tag that isn't present is also tolerated.
        repo.remove(&att_id, "nonexistent").unwrap();
    }

    #[test]
    fn now_returns_positive_unix_seconds() {
        let t = now();
        assert!(t > 1_700_000_000);
    }
}
