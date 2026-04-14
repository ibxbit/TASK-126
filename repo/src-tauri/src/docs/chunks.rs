//! Chunked, resumable uploads.
//!
//! Lifecycle:
//!   1. `start_session` — client declares total size, chunk size,
//!      mime, expected sha256, target entity. Server creates a row in
//!      `upload_sessions` and a staging directory.
//!   2. `put_chunk`      — client streams chunk N. Server writes it to
//!      `staging/<session>/chunk_000N`, records in `upload_chunks`.
//!      Idempotent: re-uploading the same chunk overwrites.
//!   3. `session_status` — UI resumes by calling this to learn which
//!      indices are missing (gaps or a tail past `received.max`).
//!   4. `finalize`       — server concatenates chunks in order,
//!      verifies sha256 + byte length, atomically moves the file to
//!      its permanent location, mints / updates the `attachments` and
//!      `attachment_versions` rows, deletes the staging dir.
//!   5. `abort`          — server marks the session aborted and
//!      deletes the staging dir.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::docs::storage::{StorageError, StorageLayout};

/// Default chunk size (25 MiB). Clients may override per session.
pub const DEFAULT_CHUNK_SIZE: u64 = 25 * 1024 * 1024;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChunkError {
    #[error("unknown session: {0}")]
    UnknownSession(String),

    #[error("session is not in progress (status={0})")]
    WrongStatus(String),

    #[error("chunk index out of range: {index} >= {count}")]
    IndexOutOfRange { index: u32, count: u32 },

    #[error("missing chunks — cannot finalize")]
    MissingChunks,

    #[error("sha256 mismatch on finalize")]
    DigestMismatch,

    #[error("byte size mismatch: expected {expected}, observed {actual}")]
    SizeMismatch { expected: u64, actual: u64 },

    #[error("storage error: {0}")]
    Storage(String),

    #[error("persistence error: {0}")]
    Persistence(String),
}

impl From<StorageError> for ChunkError {
    fn from(e: StorageError) -> Self {
        ChunkError::Storage(e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadSession {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub mime_type: String,
    pub total_bytes: u64,
    pub chunk_size: u64,
    pub chunk_count: u32,
    pub expected_sha256_hex: String,
    pub target_attachment_id: Option<Uuid>,
    pub status: String, // "in_progress" | "finalized" | "aborted"
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionInit {
    pub tenant_id: Uuid,
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub display_name: String,
    pub mime_type: String,
    pub total_bytes: u64,
    pub chunk_size: Option<u64>,
    pub expected_sha256_hex: String,
    /// Some(attachment_id) ⇒ a new version of an existing attachment.
    /// None ⇒ mint a new attachment on finalize.
    pub target_attachment_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkStatus {
    pub session_id: Uuid,
    pub chunk_count: u32,
    pub received_indices: Vec<u32>,
    pub missing_indices: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinalizeOutcome {
    pub attachment_id: Uuid,
    pub version_no: u32,
    pub byte_size: u64,
    pub sha256_hex: String,
}

/// Repository facet for chunked uploads. The concrete SQLite impl
/// lives in the future `repositories/upload_repo.rs`.
pub trait ChunkRepository {
    fn create_session(
        &self,
        session: &UploadSession,
        display_name_enc: &[u8],
        staging_rel_path: &str,
    ) -> Result<(), String>;

    fn load_session(&self, session_id: &Uuid) -> Result<Option<UploadSession>, String>;

    fn record_chunk(
        &self,
        session_id: &Uuid,
        chunk_index: u32,
        byte_size: u64,
        sha256_hex: &str,
        received_at: i64,
    ) -> Result<(), String>;

    fn list_received(&self, session_id: &Uuid) -> Result<Vec<u32>, String>;

    fn mark_status(&self, session_id: &Uuid, status: &str) -> Result<(), String>;

    /// Insert/update the parent attachments row (only on first
    /// finalize if `target_attachment_id` was None) and the
    /// attachment_versions row. Returns the new (attachment_id,
    /// version_no) pair.
    fn register_version(
        &self,
        session: &UploadSession,
        display_name_enc: &[u8],
        relative_path_enc: &[u8],
        sha256_hex: &str,
        byte_size: u64,
        created_at: i64,
        created_by: Option<&Uuid>,
    ) -> Result<(Uuid, u32), String>;
}

fn compute_count(total: u64, chunk: u64) -> u32 {
    if chunk == 0 {
        return 0;
    }
    ((total + chunk - 1) / chunk) as u32
}

/// Create a new upload session, prepare the staging directory, and
/// persist `display_name_enc` via the repository. `display_name_enc`
/// is computed by the caller using the field cipher.
pub fn start_session<R: ChunkRepository>(
    repo: &R,
    layout: &StorageLayout,
    init: SessionInit,
    display_name_enc: Vec<u8>,
) -> Result<UploadSession, ChunkError> {
    let id = Uuid::new_v4();
    let chunk_size = init.chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE);
    let chunk_count = compute_count(init.total_bytes, chunk_size);

    let staging = layout.staging_dir(&id);
    StorageLayout::ensure_dir(&staging)?;

    let session = UploadSession {
        id,
        tenant_id: init.tenant_id,
        entity_kind: init.entity_kind,
        entity_id: init.entity_id,
        mime_type: init.mime_type,
        total_bytes: init.total_bytes,
        chunk_size,
        chunk_count,
        expected_sha256_hex: init.expected_sha256_hex.to_ascii_lowercase(),
        target_attachment_id: init.target_attachment_id,
        status: "in_progress".into(),
    };

    repo.create_session(&session, &display_name_enc, &id.to_string())
        .map_err(ChunkError::Persistence)?;

    Ok(session)
}

/// Write one chunk to staging. Idempotent: if the same index is
/// uploaded again, it overwrites.
pub fn put_chunk<R: ChunkRepository>(
    repo: &R,
    layout: &StorageLayout,
    session_id: &Uuid,
    chunk_index: u32,
    data: &[u8],
    received_at: i64,
) -> Result<(), ChunkError> {
    let session = repo
        .load_session(session_id)
        .map_err(ChunkError::Persistence)?
        .ok_or_else(|| ChunkError::UnknownSession(session_id.to_string()))?;

    if session.status != "in_progress" {
        return Err(ChunkError::WrongStatus(session.status));
    }
    if chunk_index >= session.chunk_count {
        return Err(ChunkError::IndexOutOfRange {
            index: chunk_index,
            count: session.chunk_count,
        });
    }

    let path = layout.chunk_path(session_id, chunk_index);
    StorageLayout::ensure_dir(&layout.staging_dir(session_id))?;
    let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(&path)?;
    f.write_all(data)?;
    f.sync_all()?;

    let mut h = Sha256::new();
    h.update(data);
    let digest = hex::encode(h.finalize());

    repo.record_chunk(session_id, chunk_index, data.len() as u64, &digest, received_at)
        .map_err(ChunkError::Persistence)?;

    Ok(())
}

/// Query which chunks the server already has — the basis for resume.
pub fn session_status<R: ChunkRepository>(
    repo: &R,
    session_id: &Uuid,
) -> Result<ChunkStatus, ChunkError> {
    let session = repo
        .load_session(session_id)
        .map_err(ChunkError::Persistence)?
        .ok_or_else(|| ChunkError::UnknownSession(session_id.to_string()))?;

    let received = repo
        .list_received(session_id)
        .map_err(ChunkError::Persistence)?;
    let recv_set: std::collections::HashSet<u32> = received.iter().copied().collect();
    let missing: Vec<u32> = (0..session.chunk_count)
        .filter(|i| !recv_set.contains(i))
        .collect();
    Ok(ChunkStatus {
        session_id: *session_id,
        chunk_count: session.chunk_count,
        received_indices: received,
        missing_indices: missing,
    })
}

/// Concatenate, verify, move into place, register the version.
/// `relative_path_enc` is computed by the caller via `FieldCipher`
/// with AAD `attachment_versions.relative_path_enc:<attachment_id>`.
pub fn finalize<R: ChunkRepository, F>(
    repo: &R,
    layout: &StorageLayout,
    session_id: &Uuid,
    now_unix: i64,
    created_by: Option<&Uuid>,
    encrypt_relative_path: F,
) -> Result<FinalizeOutcome, ChunkError>
where
    F: FnOnce(&Uuid, &str) -> Result<Vec<u8>, String>,
{
    let session = repo
        .load_session(session_id)
        .map_err(ChunkError::Persistence)?
        .ok_or_else(|| ChunkError::UnknownSession(session_id.to_string()))?;

    if session.status != "in_progress" {
        return Err(ChunkError::WrongStatus(session.status));
    }

    // All chunks present?
    let received = repo
        .list_received(session_id)
        .map_err(ChunkError::Persistence)?;
    if received.len() as u32 != session.chunk_count {
        return Err(ChunkError::MissingChunks);
    }

    // Where the final file will live.
    let attachment_id = session
        .target_attachment_id
        .unwrap_or_else(Uuid::new_v4);
    let att_dir = layout.attachment_dir(&session.tenant_id, &attachment_id, now_unix);
    StorageLayout::ensure_dir(&att_dir)?;

    // Session-unique tmp name so two concurrent finalizes targeting
    // the same attachment dir cannot clobber each other's staging file.
    let tmp_path: PathBuf = att_dir.join(format!("v_new.{session_id}.tmp"));

    // Guard #1: delete the assembly tmp on any error path.
    let mut tmp_guard = TmpGuard::new(tmp_path.clone());

    // Concatenate chunks IN ORDER into the tmp file while hashing.
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    {
        let mut out = File::create(&tmp_path)?;
        for i in 0..session.chunk_count {
            let p = layout.chunk_path(session_id, i);
            let mut f = File::open(&p)?;
            let mut buf = vec![0u8; 1 << 20]; // 1 MiB read buffer
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 { break; }
                out.write_all(&buf[..n])?;
                hasher.update(&buf[..n]);
                total += n as u64;
            }
        }
        out.sync_all()?;
    }

    if total != session.total_bytes {
        // tmp_guard drops → file removed.
        return Err(ChunkError::SizeMismatch {
            expected: session.total_bytes,
            actual: total,
        });
    }
    let digest = hex::encode(hasher.finalize());
    if digest != session.expected_sha256_hex {
        return Err(ChunkError::DigestMismatch);
    }

    // Destination must not already exist — defense in depth against a
    // race with another finalize that somehow chose the same number.
    let version_no = next_version_in_dir(&att_dir);
    let final_path = layout.version_path(&att_dir, version_no);
    if final_path.exists() {
        return Err(ChunkError::Storage(format!(
            "target {} already exists",
            final_path.display()
        )));
    }

    // Atomic publish: rename tmp → vN.bin. On Unix this is atomic
    // within the volume; on Windows rename is durable once it returns.
    std::fs::rename(&tmp_path, &final_path)?;
    tmp_guard.commit(); // the file has moved; nothing left to clean at the old path
    fsync_dir(&att_dir);

    // From here on, any failure must ALSO remove the .bin + its
    // sidecar so the caller never sees an on-disk artifact without a
    // matching `attachment_versions` row.
    let mut bin_guard = TmpGuard::new(final_path.clone());

    // Side-car digest for at-rest integrity checks.
    let digest_path = layout.version_digest_path(&att_dir, version_no);
    StorageLayout::atomic_write(&digest_path, digest.as_bytes())?;
    let mut digest_guard = TmpGuard::new(digest_path.clone());

    // Encrypt relative path (AAD bound to the attachment id).
    let rel = StorageLayout::relative_path_for_version(
        &session.tenant_id,
        &attachment_id,
        now_unix,
        version_no,
    );
    let rel_enc = encrypt_relative_path(&attachment_id, &rel)
        .map_err(ChunkError::Persistence)?;

    // Persist attachment + version rows. If this fails the bin +
    // sidecar guards kick in on drop and the directory is clean again.
    let (att_id_out, version_no_out) = repo
        .register_version(
            &UploadSession {
                target_attachment_id: Some(attachment_id),
                ..session.clone()
            },
            &[], // display_name_enc was stored at start_session time
            &rel_enc,
            &digest,
            total,
            now_unix,
            created_by,
        )
        .map_err(ChunkError::Persistence)?;

    // All committed. Release both guards so the files persist.
    bin_guard.commit();
    digest_guard.commit();

    // Mark session finalized + remove staging (best-effort; a stray
    // staging dir is harmless and gets mopped up by the next
    // recovery sweep).
    repo.mark_status(session_id, "finalized")
        .map_err(ChunkError::Persistence)?;
    let _ = StorageLayout::remove_dir_all_if_exists(&layout.staging_dir(session_id));

    Ok(FinalizeOutcome {
        attachment_id: att_id_out,
        version_no: version_no_out,
        byte_size: total,
        sha256_hex: digest,
    })
}

/// Abort a session: mark aborted + remove staging. Idempotent.
pub fn abort<R: ChunkRepository>(
    repo: &R,
    layout: &StorageLayout,
    session_id: &Uuid,
) -> Result<(), ChunkError> {
    repo.mark_status(session_id, "aborted")
        .map_err(ChunkError::Persistence)?;
    let _ = StorageLayout::remove_dir_all_if_exists(&layout.staging_dir(session_id));
    Ok(())
}

/// RAII guard that removes `path` on drop unless `commit()` was called.
/// Guarantees "no partial files exposed": every file we create during
/// finalize is wrapped in one of these, so ANY error path — including
/// panics — cleans up the on-disk artifact before returning.
struct TmpGuard {
    path: PathBuf,
    committed: bool,
}

impl TmpGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, committed: false }
    }
    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TmpGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Best-effort directory fsync (POSIX). On Windows, `rename` is
/// already durable once it returns.
fn fsync_dir(dir: &std::path::Path) {
    #[cfg(unix)]
    {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

fn next_version_in_dir(dir: &std::path::Path) -> u32 {
    let mut max = 0u32;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Some(num) = name
                    .strip_prefix('v')
                    .and_then(|s| s.strip_suffix(".bin"))
                    .and_then(|s| s.parse::<u32>().ok())
                {
                    if num > max {
                        max = num;
                    }
                }
            }
        }
    }
    max + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[derive(Default)]
    struct MockRepo {
        sessions: RefCell<HashMap<Uuid, UploadSession>>,
        chunks: RefCell<HashMap<Uuid, Vec<u32>>>,
        fail_register: RefCell<bool>,
    }
    impl ChunkRepository for MockRepo {
        fn create_session(&self, s: &UploadSession, _dn: &[u8], _st: &str) -> Result<(), String> {
            self.sessions.borrow_mut().insert(s.id, s.clone());
            self.chunks.borrow_mut().insert(s.id, vec![]);
            Ok(())
        }
        fn load_session(&self, id: &Uuid) -> Result<Option<UploadSession>, String> {
            Ok(self.sessions.borrow().get(id).cloned())
        }
        fn record_chunk(&self, sid: &Uuid, idx: u32, _b: u64, _d: &str, _t: i64) -> Result<(), String> {
            let mut c = self.chunks.borrow_mut();
            let v = c.entry(*sid).or_default();
            if !v.contains(&idx) { v.push(idx); v.sort(); }
            Ok(())
        }
        fn list_received(&self, sid: &Uuid) -> Result<Vec<u32>, String> {
            Ok(self.chunks.borrow().get(sid).cloned().unwrap_or_default())
        }
        fn mark_status(&self, sid: &Uuid, status: &str) -> Result<(), String> {
            if let Some(s) = self.sessions.borrow_mut().get_mut(sid) {
                s.status = status.into();
            }
            Ok(())
        }
        fn register_version(
            &self,
            session: &UploadSession,
            _dn: &[u8],
            _rp: &[u8],
            _sha: &str,
            _size: u64,
            _ts: i64,
            _by: Option<&Uuid>,
        ) -> Result<(Uuid, u32), String> {
            if *self.fail_register.borrow() {
                return Err("simulated DB failure after rename".into());
            }
            Ok((session.target_attachment_id.unwrap(), 1))
        }
    }

    fn seed_session(total: u64, chunk: u64, sha: &str) -> (MockRepo, StorageLayout, Uuid) {
        let tmp = tempdir().unwrap();
        let layout = StorageLayout::new(tmp.path().to_path_buf());
        let repo = MockRepo::default();
        let init = SessionInit {
            tenant_id: Uuid::new_v4(),
            entity_kind: "case".into(),
            entity_id: Uuid::new_v4(),
            display_name: "doc.pdf".into(),
            mime_type: "application/pdf".into(),
            total_bytes: total,
            chunk_size: Some(chunk),
            expected_sha256_hex: sha.into(),
            target_attachment_id: None,
        };
        let s = start_session(&repo, &layout, init, b"enc-name".to_vec()).unwrap();
        (repo, layout, s.id)
    }

    #[test]
    fn chunk_count_rounds_up() {
        assert_eq!(compute_count(0, 10), 0);
        assert_eq!(compute_count(1, 10), 1);
        assert_eq!(compute_count(10, 10), 1);
        assert_eq!(compute_count(11, 10), 2);
    }

    #[test]
    fn put_chunk_rejects_out_of_range() {
        let data = b"hello world";
        let mut h = Sha256::new(); h.update(data); let sha = hex::encode(h.finalize());
        let (repo, layout, sid) = seed_session(data.len() as u64, 100, &sha);
        let err = put_chunk(&repo, &layout, &sid, 5, data, 0).unwrap_err();
        assert!(matches!(err, ChunkError::IndexOutOfRange { .. }));
    }

    #[test]
    fn resume_reports_missing_chunks() {
        // 3 chunks total
        let payload = vec![0xAAu8; 25];
        let mut h = Sha256::new(); h.update(&payload); let sha = hex::encode(h.finalize());
        let (repo, layout, sid) = seed_session(payload.len() as u64, 10, &sha);
        put_chunk(&repo, &layout, &sid, 0, &payload[0..10], 1).unwrap();
        put_chunk(&repo, &layout, &sid, 2, &payload[20..25], 1).unwrap();
        let st = session_status(&repo, &sid).unwrap();
        assert_eq!(st.missing_indices, vec![1]);
        assert_eq!(st.received_indices, vec![0, 2]);
    }

    #[test]
    fn finalize_succeeds_with_correct_digest() {
        let payload = b"the quick brown fox jumps over the lazy dog";
        let mut h = Sha256::new(); h.update(payload); let sha = hex::encode(h.finalize());
        let (repo, layout, sid) = seed_session(payload.len() as u64, 10, &sha);
        // Upload in chunks of 10
        for (i, c) in payload.chunks(10).enumerate() {
            put_chunk(&repo, &layout, &sid, i as u32, c, 1).unwrap();
        }
        let out = finalize(&repo, &layout, &sid, 1700000000, None, |_id, _rp| {
            Ok(b"enc-rp".to_vec())
        })
        .unwrap();
        assert_eq!(out.byte_size as usize, payload.len());
        assert_eq!(out.sha256_hex, sha);
    }

    #[test]
    fn finalize_detects_digest_tampering() {
        let payload = b"real content";
        let mut h = Sha256::new(); h.update(b"fake"); let sha_wrong = hex::encode(h.finalize());
        let (repo, layout, sid) = seed_session(payload.len() as u64, 100, &sha_wrong);
        put_chunk(&repo, &layout, &sid, 0, payload, 1).unwrap();
        let err = finalize(&repo, &layout, &sid, 0, None, |_, _| Ok(vec![])).unwrap_err();
        assert!(matches!(err, ChunkError::DigestMismatch));
    }

    #[test]
    fn finalize_detects_missing_chunks() {
        let payload = vec![0u8; 30];
        let mut h = Sha256::new(); h.update(&payload); let sha = hex::encode(h.finalize());
        let (repo, layout, sid) = seed_session(payload.len() as u64, 10, &sha);
        // Upload only chunks 0 and 2
        put_chunk(&repo, &layout, &sid, 0, &payload[0..10], 1).unwrap();
        put_chunk(&repo, &layout, &sid, 2, &payload[20..30], 1).unwrap();
        let err = finalize(&repo, &layout, &sid, 0, None, |_, _| Ok(vec![])).unwrap_err();
        assert!(matches!(err, ChunkError::MissingChunks));
    }

    /// On digest mismatch the assembly tmp must not be left behind.
    #[test]
    fn finalize_cleans_up_tmp_on_digest_mismatch() {
        let payload = b"real content";
        let mut h = Sha256::new(); h.update(b"fake"); let sha_wrong = hex::encode(h.finalize());
        let (repo, layout, sid) = seed_session(payload.len() as u64, 100, &sha_wrong);
        put_chunk(&repo, &layout, &sid, 0, payload, 1).unwrap();

        let session = repo.load_session(&sid).unwrap().unwrap();
        let att_id = session.target_attachment_id.unwrap_or(Uuid::new_v4());
        // (We can't predict att_id for target_attachment_id==None but
        // the TmpGuard's `path` is derived from it via layout; this
        // test targets the digest branch specifically.)

        let _ = finalize(&repo, &layout, &sid, 0, None, |_, _| Ok(vec![]))
            .unwrap_err();

        // Walk the attachments root looking for any residual `.tmp`
        // file. None should be present.
        let tmps = crate::recovery::checkpoint::find_tmp_files_recursive(
            &[layout.attachments_root()],
        )
        .unwrap_or_default();
        assert!(
            tmps.iter().all(|p| !p.to_string_lossy().contains("v_new")),
            "stray v_new.*.tmp left behind: {:?}", tmps
        );
        let _ = att_id;
    }

    /// If the DB's register_version fails AFTER the rename, the .bin
    /// and its .sha256 sidecar must both be removed so the directory
    /// is not left with an orphan file.
    #[test]
    fn finalize_rolls_back_rename_when_register_fails() {
        let payload = b"the quick brown fox jumps over the lazy dog";
        let mut h = Sha256::new(); h.update(payload); let sha = hex::encode(h.finalize());
        let (repo, layout, sid) = seed_session(payload.len() as u64, 10, &sha);
        *repo.fail_register.borrow_mut() = true;

        for (i, c) in payload.chunks(10).enumerate() {
            put_chunk(&repo, &layout, &sid, i as u32, c, 1).unwrap();
        }

        let err = finalize(&repo, &layout, &sid, 1700000000, None, |_, _| {
            Ok(b"enc".to_vec())
        })
        .unwrap_err();
        assert!(matches!(err, ChunkError::Persistence(_)));

        // Scan the attachments root: there must be NO v1.bin and NO
        // v1.sha256 left behind.
        fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                        out.push(n.to_string());
                    }
                }
            }
        }
        let mut names = Vec::new();
        walk(&layout.attachments_root(), &mut names);
        assert!(!names.iter().any(|n| n == "v1.bin"),
                "v1.bin was not rolled back: {:?}", names);
        assert!(!names.iter().any(|n| n == "v1.sha256"),
                "v1.sha256 was not rolled back: {:?}", names);
    }
}
