//! Document management IPC commands — SQLite-backed.

use std::sync::Arc;
use uuid::Uuid;

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
    tenant_id: Uuid,
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
    tenant_id: Uuid,
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
    version_no: Option<u32>,
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
