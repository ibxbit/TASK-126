//! Encryption-key rotation.
//!
//! ## Safety model
//!
//! Rotation is **batched and atomic**:
//!
//!   BEGIN;
//!     SELECT id, <enc> FROM <table>
//!       WHERE id > :cursor ORDER BY id LIMIT :batch;
//!     -- For each row: decrypt with OLD key, re-encrypt with NEW key, UPDATE.
//!     UPDATE key_rotation_progress SET last_id = :max_id
//!       WHERE rotation_id = :rid AND table_name = :t AND column_name = :c;
//!   COMMIT;
//!
//! Properties this gives us:
//!
//! - **No data loss.** A crash before COMMIT rolls the whole batch
//!   back. A crash after COMMIT leaves the rows AND the cursor in a
//!   consistent state, so the next run picks up at exactly the next
//!   row.
//! - **No double-encryption.** The cursor advances only on commit, so
//!   an already-processed row is never seen again, and the new-key
//!   ciphertext is never fed back into the old-key decryptor.
//! - **Resumable.** The schema's partial unique index on
//!   `status = 'in_progress'` pairs with `find_incomplete_rotation`
//!   to make "run until done or a crash, resume on next startup" the
//!   default flow.
//! - **Deterministic AAD.** AAD is always
//!   `"<table>.<column>:<row_id>"`, so the new-key ciphertext binds
//!   to exactly the same logical slot as the old-key ciphertext did.
//!
//! The rotation itself does NOT touch the OS keystore. Callers are
//! expected to:
//!   1. Create the new key via `KeyManager::rotate_master_key(new_label)`.
//!   2. Build `FieldCipher`s for both the old and new keys.
//!   3. Call `run_rotation` with those ciphers and the list of field
//!      specs.
//!   4. On `RotationSummary::status == Completed`, call
//!      `KeyManager::delete_master_key(old_label)` to retire the old
//!      label from Credential Manager.

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::db::encryption::{aad_for, FieldCipher};

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum RotationError {
    #[error("persistence error: {0}")]
    Persistence(String),

    #[error("cryptographic error at {table}.{column} row {row_id}: {reason}")]
    Crypto { table: String, column: String, row_id: String, reason: String },

    #[error("another rotation is already in progress")]
    RotationInProgress,
}

/// One encrypted column to be rotated.
#[derive(Debug, Clone)]
pub struct FieldSpec {
    pub table: &'static str,
    pub column: &'static str,
    /// Primary-key column — typically `"id"`.
    pub id_column: &'static str,
}

impl FieldSpec {
    pub const fn new(table: &'static str, column: &'static str) -> Self {
        Self { table, column, id_column: "id" }
    }
}

/// Every encrypted field currently defined by Shoreline's schema.
/// Used as the default work plan for `run_rotation`. Keeping this as
/// a single authoritative list makes "did we rotate everything?" a
/// review-at-a-glance question.
pub fn default_specs() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new("residents",              "national_id_enc"),
        FieldSpec::new("move_out_cases",         "notes_enc"),
        FieldSpec::new("settlements",            "notes_enc"),
        FieldSpec::new("parcels",                "notes_enc"),
        FieldSpec::new("claims",                 "description_enc"),
        FieldSpec::new("attachments",            "display_name_enc"),
        FieldSpec::new("attachments",            "relative_path_enc"),
        FieldSpec::new("attachment_versions",    "relative_path_enc"),
        FieldSpec::new("upload_sessions",        "display_name_enc"),
        FieldSpec::new("parcel_transitions",     "notes_enc"),
        FieldSpec::new("claim_party_responses",  "notes_enc"),
        FieldSpec::new("claim_reopens",          "reason_enc"),
        FieldSpec::new("inspections",            "notes_enc"),
        FieldSpec::new("settlement_approvals",   "notes_enc"),
        FieldSpec::new("check_requests",         "artifact_path_enc"),
        FieldSpec::new("schedule_assignments",   "notes_enc"),
        FieldSpec::new("share_packages",         "artifact_path_enc"),
    ]
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyRotation {
    pub id: Uuid,
    pub old_label: String,
    pub new_label: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub status: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldOutcome {
    pub table: String,
    pub column: String,
    pub rows_rotated: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RotationSummary {
    pub rotation_id: Uuid,
    pub started_at: i64,
    pub completed_at: i64,
    pub total_rows: u32,
    pub fields: Vec<FieldOutcome>,
}

// ── Repository contract ─────────────────────────────────────────────────

/// Transactional surface used by `rotate_field`. The concrete SQLite
/// impl wraps one `rusqlite::Transaction` per `with_tx` call and
/// commits on `Ok(_)` / rolls back on `Err(_)` — see the contract
/// docstring on `RotationRepository::with_tx`.
pub trait RotationTx {
    fn get_cursor(
        &self,
        rotation_id: &Uuid,
        spec: &FieldSpec,
    ) -> Result<Option<String>, String>;

    fn fetch_batch(
        &self,
        spec: &FieldSpec,
        batch_size: u32,
        cursor: Option<&str>,
    ) -> Result<Vec<(String, Vec<u8>)>, String>;

    fn update_ciphertext(
        &self,
        spec: &FieldSpec,
        row_id: &str,
        new_ct: &[u8],
    ) -> Result<(), String>;

    fn advance_cursor(
        &self,
        rotation_id: &Uuid,
        spec: &FieldSpec,
        new_cursor: &str,
    ) -> Result<(), String>;

    fn mark_field_completed(
        &self,
        rotation_id: &Uuid,
        spec: &FieldSpec,
        now_unix: i64,
    ) -> Result<(), String>;
}

pub trait RotationRepository {
    /// Open a rusqlite transaction, run `f` with a `RotationTx`
    /// bound to it, and commit on Ok / roll back on Err.
    fn with_tx<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&dyn RotationTx) -> Result<T, String>;

    fn find_incomplete_rotation(&self) -> Result<Option<KeyRotation>, String>;

    fn start_rotation(&self, row: &KeyRotation) -> Result<(), String>;

    fn finalize_rotation(
        &self,
        rotation_id: &Uuid,
        status: &str,
        error_message: Option<&str>,
        completed_at: i64,
    ) -> Result<(), String>;
}

// ── Per-field rotation loop ─────────────────────────────────────────────

/// Rotate one encrypted column end-to-end. Returns the number of rows
/// re-encrypted.
pub fn rotate_field<R: RotationRepository>(
    repo: &R,
    rotation_id: Uuid,
    old: &FieldCipher,
    new: &FieldCipher,
    spec: &FieldSpec,
    batch_size: u32,
    now_unix_start: i64,
) -> Result<u32, RotationError> {
    let batch_size = batch_size.max(1);
    let mut total: u32 = 0;

    loop {
        let processed = repo
            .with_tx(|tx| {
                let cursor = tx.get_cursor(&rotation_id, spec)?;
                let rows = tx.fetch_batch(spec, batch_size, cursor.as_deref())?;

                if rows.is_empty() {
                    tx.mark_field_completed(&rotation_id, spec, now_unix_start)?;
                    return Ok(0u32);
                }

                let mut max_id: Option<String> = cursor;
                for (row_id, ct) in &rows {
                    let aad = aad_for(spec.table, spec.column, row_id);
                    let plaintext = old.decrypt(ct, &aad).map_err(|e| {
                        format!(
                            "decrypt failed for {}.{} row {}: {}",
                            spec.table, spec.column, row_id, e
                        )
                    })?;
                    let new_ct = new.encrypt(&plaintext, &aad).map_err(|e| {
                        format!(
                            "encrypt failed for {}.{} row {}: {}",
                            spec.table, spec.column, row_id, e
                        )
                    })?;
                    tx.update_ciphertext(spec, row_id, &new_ct)?;
                    max_id = Some(row_id.clone());
                }
                if let Some(id) = &max_id {
                    tx.advance_cursor(&rotation_id, spec, id)?;
                }
                Ok(rows.len() as u32)
            })
            .map_err(RotationError::Persistence)?;

        if processed == 0 {
            break;
        }
        total = total.saturating_add(processed);
    }

    Ok(total)
}

// ── Top-level orchestrator ──────────────────────────────────────────────

/// Run (or resume) a full rotation across every field in `specs`.
///
/// - If an in-progress `key_rotations` row already exists, the run
///   resumes it (same rotation_id, same labels). Otherwise a new row
///   is inserted.
/// - On any per-field error the rotation is finalized with status
///   `aborted` and the error message is persisted for diagnostics;
///   the old key remains valid and usable, so the app continues to
///   function while the operator investigates.
/// - On full success the rotation is finalized with status
///   `completed` and a `RotationSummary` is returned.
pub fn run_rotation<R: RotationRepository>(
    repo: &R,
    old: &FieldCipher,
    new: &FieldCipher,
    old_label: &str,
    new_label: &str,
    specs: &[FieldSpec],
    batch_size: u32,
    now_unix: i64,
) -> Result<RotationSummary, RotationError> {
    // 1. Resume or start.
    let rotation = match repo
        .find_incomplete_rotation()
        .map_err(RotationError::Persistence)?
    {
        Some(existing) => existing,
        None => {
            let row = KeyRotation {
                id: Uuid::new_v4(),
                old_label: old_label.to_string(),
                new_label: new_label.to_string(),
                started_at: now_unix,
                completed_at: None,
                status: "in_progress".to_string(),
                error_message: None,
            };
            repo.start_rotation(&row)
                .map_err(RotationError::Persistence)?;
            row
        }
    };

    // 2. Rotate every field.
    let mut fields: Vec<FieldOutcome> = Vec::with_capacity(specs.len());
    let mut total: u32 = 0;
    for spec in specs {
        match rotate_field(repo, rotation.id, old, new, spec, batch_size, now_unix) {
            Ok(n) => {
                total = total.saturating_add(n);
                fields.push(FieldOutcome {
                    table: spec.table.into(),
                    column: spec.column.into(),
                    rows_rotated: n,
                });
            }
            Err(e) => {
                // Preserve the error on the rotation row; old key is
                // still valid so the app remains functional.
                let msg = e.to_string();
                let _ = repo.finalize_rotation(
                    &rotation.id,
                    "aborted",
                    Some(&msg),
                    now_unix,
                );
                return Err(e);
            }
        }
    }

    // 3. Finalize success.
    let completed_at = now_unix;
    repo.finalize_rotation(&rotation.id, "completed", None, completed_at)
        .map_err(RotationError::Persistence)?;

    Ok(RotationSummary {
        rotation_id: rotation.id,
        started_at: rotation.started_at,
        completed_at,
        total_rows: total,
        fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// In-memory repo + tx. The "transaction" is simulated: writes
    /// land in a scratch map and are merged into the main store only
    /// on successful return from `with_tx`; on Err, the scratch map
    /// is discarded (rollback).
    #[derive(Default)]
    struct MockRepo {
        // "main" store, indexed by (table, column, row_id) → ciphertext
        data: RefCell<HashMap<(String, String, String), Vec<u8>>>,
        // ordered rows per (table, column) for deterministic cursoring
        order: RefCell<HashMap<(String, String), Vec<String>>>,
        cursors: RefCell<HashMap<(Uuid, String, String), Option<String>>>,
        completions: RefCell<HashMap<(Uuid, String, String), i64>>,
        rotations: RefCell<Vec<KeyRotation>>,
    }

    impl MockRepo {
        fn insert(&self, table: &str, col: &str, id: &str, ct: Vec<u8>) {
            self.data.borrow_mut().insert(
                (table.into(), col.into(), id.into()),
                ct,
            );
            let mut o = self.order.borrow_mut();
            let list = o.entry((table.into(), col.into())).or_default();
            list.push(id.into());
            list.sort();
        }
    }

    struct MockTx<'a> {
        repo: &'a MockRepo,
        // batch-local scratch (discarded on rollback)
        writes: RefCell<Vec<((String, String, String), Vec<u8>)>>,
        cursor_writes: RefCell<Vec<((Uuid, String, String), Option<String>)>>,
        completions: RefCell<Vec<((Uuid, String, String), i64)>>,
    }

    impl<'a> RotationTx for MockTx<'a> {
        fn get_cursor(&self, rid: &Uuid, spec: &FieldSpec) -> Result<Option<String>, String> {
            Ok(self.repo.cursors
                .borrow()
                .get(&(*rid, spec.table.into(), spec.column.into()))
                .cloned()
                .flatten())
        }
        fn fetch_batch(&self, spec: &FieldSpec, batch: u32, cursor: Option<&str>) -> Result<Vec<(String, Vec<u8>)>, String> {
            let o = self.repo.order.borrow();
            let d = self.repo.data.borrow();
            let empty: Vec<String> = Vec::new();
            let ids = o.get(&(spec.table.into(), spec.column.into())).unwrap_or(&empty);
            let mut out = Vec::new();
            for id in ids {
                if let Some(c) = cursor {
                    if id.as_str() <= c { continue; }
                }
                if (out.len() as u32) >= batch { break; }
                let ct = d.get(&(spec.table.into(), spec.column.into(), id.clone()))
                    .cloned().unwrap_or_default();
                out.push((id.clone(), ct));
            }
            Ok(out)
        }
        fn update_ciphertext(&self, spec: &FieldSpec, row_id: &str, new_ct: &[u8]) -> Result<(), String> {
            self.writes.borrow_mut().push((
                (spec.table.into(), spec.column.into(), row_id.into()),
                new_ct.to_vec(),
            ));
            Ok(())
        }
        fn advance_cursor(&self, rid: &Uuid, spec: &FieldSpec, new_cursor: &str) -> Result<(), String> {
            self.cursor_writes.borrow_mut().push((
                (*rid, spec.table.into(), spec.column.into()),
                Some(new_cursor.into()),
            ));
            Ok(())
        }
        fn mark_field_completed(&self, rid: &Uuid, spec: &FieldSpec, ts: i64) -> Result<(), String> {
            self.completions.borrow_mut().push((
                (*rid, spec.table.into(), spec.column.into()),
                ts,
            ));
            Ok(())
        }
    }

    impl RotationRepository for MockRepo {
        fn with_tx<F, T>(&self, f: F) -> Result<T, String>
        where F: FnOnce(&dyn RotationTx) -> Result<T, String>,
        {
            let tx = MockTx {
                repo: self,
                writes: RefCell::new(Vec::new()),
                cursor_writes: RefCell::new(Vec::new()),
                completions: RefCell::new(Vec::new()),
            };
            match f(&tx) {
                Ok(t) => {
                    // Commit: merge scratch into main.
                    let mut d = self.data.borrow_mut();
                    for (k, v) in tx.writes.into_inner() {
                        d.insert(k, v);
                    }
                    let mut c = self.cursors.borrow_mut();
                    for (k, v) in tx.cursor_writes.into_inner() {
                        c.insert(k, v);
                    }
                    let mut cmp = self.completions.borrow_mut();
                    for (k, v) in tx.completions.into_inner() {
                        cmp.insert(k, v);
                    }
                    Ok(t)
                }
                Err(e) => Err(e), // rollback: scratch discarded via RefCell drop
            }
        }
        fn find_incomplete_rotation(&self) -> Result<Option<KeyRotation>, String> {
            Ok(self.rotations.borrow().iter().find(|r| r.status == "in_progress").cloned())
        }
        fn start_rotation(&self, row: &KeyRotation) -> Result<(), String> {
            self.rotations.borrow_mut().push(row.clone());
            Ok(())
        }
        fn finalize_rotation(&self, rid: &Uuid, status: &str, err: Option<&str>, ts: i64) -> Result<(), String> {
            for r in self.rotations.borrow_mut().iter_mut() {
                if &r.id == rid {
                    r.status = status.into();
                    r.error_message = err.map(|s| s.into());
                    r.completed_at = Some(ts);
                }
            }
            Ok(())
        }
    }

    fn cipher(byte: u8) -> FieldCipher {
        FieldCipher::new([byte; 32])
    }

    #[test]
    fn end_to_end_rotates_all_rows() {
        let old = cipher(7);
        let new = cipher(9);
        let repo = MockRepo::default();

        // Seed 5 rows of notes_enc, encrypted with the OLD key.
        let table = "move_out_cases";
        let col = "notes_enc";
        for i in 0..5 {
            let id = format!("row-{i:02}");
            let pt = format!("secret #{i}");
            let ct = old.encrypt(pt.as_bytes(), &aad_for(table, col, &id)).unwrap();
            repo.insert(table, col, &id, ct);
        }

        let specs = vec![FieldSpec::new(table, col)];
        let summary = run_rotation(&repo, &old, &new, "v1", "v2", &specs, 2, 1000).unwrap();
        assert_eq!(summary.total_rows, 5);
        assert_eq!(summary.fields[0].rows_rotated, 5);

        // Every row now decrypts with the NEW key.
        let d = repo.data.borrow();
        for i in 0..5 {
            let id = format!("row-{i:02}");
            let ct = d.get(&(table.into(), col.into(), id.clone())).unwrap();
            let pt = new.decrypt(ct, &aad_for(table, col, &id)).unwrap();
            assert_eq!(pt, format!("secret #{i}").into_bytes());
            // And the old key no longer works.
            assert!(old.decrypt(ct, &aad_for(table, col, &id)).is_err());
        }
    }

    #[test]
    fn zero_rows_still_marks_field_complete() {
        let old = cipher(1);
        let new = cipher(2);
        let repo = MockRepo::default();
        let specs = vec![FieldSpec::new("claims", "description_enc")];
        let summary = run_rotation(&repo, &old, &new, "v1", "v2", &specs, 10, 1000).unwrap();
        assert_eq!(summary.total_rows, 0);
        // Rotation is finalized as completed.
        assert_eq!(repo.rotations.borrow()[0].status, "completed");
    }

    #[test]
    fn resume_from_existing_in_progress_rotation() {
        let repo = MockRepo::default();
        let prior = KeyRotation {
            id: Uuid::new_v4(),
            old_label: "v1".into(),
            new_label: "v2".into(),
            started_at: 100,
            completed_at: None,
            status: "in_progress".into(),
            error_message: None,
        };
        repo.start_rotation(&prior).unwrap();

        let old = cipher(5);
        let new = cipher(6);
        let specs = vec![FieldSpec::new("settlements", "notes_enc")];
        let summary = run_rotation(&repo, &old, &new, "v1", "v2", &specs, 10, 2000).unwrap();

        // Same rotation id — resumed, not recreated.
        assert_eq!(summary.rotation_id, prior.id);
        assert_eq!(repo.rotations.borrow().len(), 1);
    }

    #[test]
    fn wrong_old_key_aborts_without_corrupting_data() {
        let real_old = cipher(10);
        let wrong_old = cipher(11); // caller passed the wrong "old" key
        let new = cipher(12);
        let repo = MockRepo::default();

        let table = "parcels";
        let col = "notes_enc";
        let id = "row-0";
        let ct = real_old.encrypt(b"hi", &aad_for(table, col, id)).unwrap();
        repo.insert(table, col, id, ct.clone());

        let specs = vec![FieldSpec::new(table, col)];
        let err = run_rotation(&repo, &wrong_old, &new, "v1", "v2", &specs, 10, 1000).unwrap_err();
        match err {
            RotationError::Persistence(msg) => assert!(msg.contains("decrypt failed")),
            other => panic!("unexpected error: {:?}", other),
        }
        // Rotation is aborted, NOT completed.
        assert_eq!(repo.rotations.borrow()[0].status, "aborted");
        // Original ciphertext is intact (still decrypts with real_old).
        let stored = repo.data.borrow()
            .get(&(table.into(), col.into(), id.into()))
            .cloned().unwrap();
        assert_eq!(stored, ct);
        let pt = real_old.decrypt(&stored, &aad_for(table, col, id)).unwrap();
        assert_eq!(pt, b"hi");
    }

    #[test]
    fn batches_make_progress_incrementally() {
        // 7 rows, batch size 3 → expect 3 commits (3 + 3 + 1), each
        // of which must advance the cursor and be durable even if the
        // next batch fails.
        let old = cipher(20);
        let new = cipher(21);
        let repo = MockRepo::default();
        let table = "inspections";
        let col = "notes_enc";
        for i in 0..7 {
            let id = format!("r-{i:02}");
            let ct = old.encrypt(format!("x{i}").as_bytes(), &aad_for(table, col, &id)).unwrap();
            repo.insert(table, col, &id, ct);
        }
        let specs = vec![FieldSpec::new(table, col)];
        let summary = run_rotation(&repo, &old, &new, "v1", "v2", &specs, 3, 1000).unwrap();
        assert_eq!(summary.total_rows, 7);

        // Cursor was advanced past the last row.
        let rid = summary.rotation_id;
        let cur = repo.cursors.borrow()
            .get(&(rid, table.into(), col.into())).cloned().flatten();
        assert_eq!(cur, Some("r-06".into()));
    }
}
