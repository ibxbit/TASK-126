//! File-handle leak prevention.
//!
//! `TrackedFile` wraps `std::fs::File` and registers itself with a
//! shared `HandleTracker` on construction. The `Drop` impl decrements
//! the count and the SQLite/connection-pool layers register their
//! own handles via `HandleTracker::register` / `release`.
//!
//! At graceful shutdown, the orchestrator calls `tracker.snapshot()`
//! and logs/asserts that the count is zero. A non-zero count is a
//! programming bug — we want to know about it BEFORE it becomes a
//! production leak.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HandleKind {
    File,
    DbConnection,
    UploadChunk,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandleEntry {
    pub id: Uuid,
    pub kind: HandleKind,
    pub label: String,
    pub opened_at_unix: i64,
}

#[derive(Default)]
pub struct HandleTracker {
    inner: Mutex<HashMap<Uuid, HandleEntry>>,
}

impl HandleTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn register(&self, kind: HandleKind, label: impl Into<String>) -> Uuid {
        let entry = HandleEntry {
            id: Uuid::new_v4(),
            kind,
            label: label.into(),
            opened_at_unix: now_unix(),
        };
        let id = entry.id;
        self.inner.lock().expect("handle tracker poisoned").insert(id, entry);
        id
    }

    pub fn release(&self, id: Uuid) {
        self.inner.lock().expect("handle tracker poisoned").remove(&id);
    }

    pub fn count(&self) -> usize {
        self.inner.lock().expect("handle tracker poisoned").len()
    }

    pub fn snapshot(&self) -> Vec<HandleEntry> {
        self.inner
            .lock()
            .expect("handle tracker poisoned")
            .values()
            .cloned()
            .collect()
    }
}

/// `File` wrapper that auto-deregisters on drop.
pub struct TrackedFile {
    file: File,
    tracker: Arc<HandleTracker>,
    id: Uuid,
    path: PathBuf,
}

impl TrackedFile {
    pub fn open(
        tracker: Arc<HandleTracker>,
        path: impl AsRef<Path>,
        opts: &OpenOptions,
    ) -> std::io::Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let file = opts.open(&path_buf)?;
        let id = tracker.register(HandleKind::File, path_buf.display().to_string());
        Ok(Self {
            file,
            tracker,
            id,
            path: path_buf,
        })
    }

    pub fn create(tracker: Arc<HandleTracker>, path: impl AsRef<Path>) -> std::io::Result<Self> {
        let mut opts = OpenOptions::new();
        opts.create(true).write(true).truncate(true);
        Self::open(tracker, path, &opts)
    }

    pub fn open_read(tracker: Arc<HandleTracker>, path: impl AsRef<Path>) -> std::io::Result<Self> {
        let mut opts = OpenOptions::new();
        opts.read(true);
        Self::open(tracker, path, &opts)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sync_all(&self) -> std::io::Result<()> {
        self.file.sync_all()
    }
}

impl Read for TrackedFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Write for TrackedFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl Seek for TrackedFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl Drop for TrackedFile {
    fn drop(&mut self) {
        // Best-effort flush so a panic doesn't lose the last buffer.
        let _ = self.file.flush();
        self.tracker.release(self.id);
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ── Quiesce enforcement ─────────────────────────────────────────────────

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuiesceError {
    /// The caller's quiesce hook itself failed (e.g. DB close errored).
    #[error("quiesce hook failed: {0}")]
    Hook(String),

    /// One or more handles are still open AFTER the quiesce hook ran.
    /// Update / rollback MUST abort: proceeding would either lock up
    /// mid-operation or leave the previous install in a partially
    /// unwound state.
    #[error("{count} file handle(s) still open after quiesce")]
    HandlesOpen {
        count: usize,
        open: Vec<HandleEntry>,
    },
}

/// Composable verifier: runs the caller's `quiesce` closure, then
/// confirms the tracker reports zero live handles. Any non-zero
/// count surfaces the full entry list for diagnostics.
///
/// Usage — in the concrete `InstallerOps::quiesce` / `RollbackOps::quiesce`
/// implementation registered at bootstrap:
///
/// ```ignore
/// let quiescer = HandleQuiescer::new(&tracker);
/// quiescer.quiesce_and_verify(|| {
///     stop_background_threads()?;
///     close_db_connections()?;
///     Ok(())
/// })
/// .map_err(|e| e.to_string())
/// ```
pub struct HandleQuiescer<'a> {
    tracker: &'a HandleTracker,
}

impl<'a> HandleQuiescer<'a> {
    pub fn new(tracker: &'a HandleTracker) -> Self {
        Self { tracker }
    }

    pub fn quiesce_and_verify<F>(&self, hook: F) -> Result<(), QuiesceError>
    where
        F: FnOnce() -> Result<(), String>,
    {
        hook().map_err(QuiesceError::Hook)?;
        let open = self.tracker.snapshot();
        if !open.is_empty() {
            return Err(QuiesceError::HandlesOpen {
                count: open.len(),
                open,
            });
        }
        Ok(())
    }
}

// ── Bounded retry for transient file-lock races ─────────────────────────
// Windows antivirus scans, Explorer's indexer, and even the
// WebView2 runtime can briefly hold file handles on files we need
// to rename / remove. The errors are transient; retry with backoff
// clears them without forcing the user to restart the app.

pub const DEFAULT_IO_RETRY_ATTEMPTS: u32 = 6;
pub const DEFAULT_IO_RETRY_INITIAL_MS: u64 = 50;

pub fn retry_io<T, F>(mut op: F, attempts: u32, initial_delay_ms: u64) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    let attempts = attempts.max(1);
    let mut delay = initial_delay_ms.max(1);
    for i in 0..attempts {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) if i + 1 == attempts => return Err(e),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(delay));
                delay = (delay.saturating_mul(2)).min(2_000);
            }
        }
    }
    unreachable!("retry_io loop exhausted without return")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_then_drop_decrements_count() {
        let t = HandleTracker::new();
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.bin");
        {
            let mut f = TrackedFile::create(Arc::clone(&t), &path).unwrap();
            f.write_all(b"hi").unwrap();
            assert_eq!(t.count(), 1);
        }
        assert_eq!(t.count(), 0);
    }

    #[test]
    fn snapshot_lists_open_handles() {
        let t = HandleTracker::new();
        let id = t.register(HandleKind::DbConnection, "shoreline.db");
        let s = t.snapshot();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].kind, HandleKind::DbConnection);
        t.release(id);
        assert_eq!(t.count(), 0);
    }

    #[test]
    fn double_release_is_safe() {
        let t = HandleTracker::new();
        let id = t.register(HandleKind::Other, "x");
        t.release(id);
        t.release(id); // no panic
        assert_eq!(t.count(), 0);
    }

    #[test]
    fn shutdown_assertion_fires_on_leak() {
        let t = HandleTracker::new();
        let _id = t.register(HandleKind::Other, "leaked");
        // Caller's shutdown hook would do:
        //     assert_eq!(tracker.count(), 0, "{:?}", tracker.snapshot());
        // We model that pattern here.
        assert!(t.count() > 0);
        assert!(t.snapshot().iter().any(|e| e.label == "leaked"));
    }

    // ── HandleQuiescer ─────────────────────────────────────────────

    #[test]
    fn quiescer_passes_when_hook_closes_all_handles() {
        let t = HandleTracker::new();
        let id = t.register(HandleKind::DbConnection, "shoreline.db");
        let t_for_hook = Arc::clone(&t);
        let q = HandleQuiescer::new(&t);
        let r = q.quiesce_and_verify(|| {
            t_for_hook.release(id);
            Ok(())
        });
        assert!(r.is_ok());
    }

    #[test]
    fn quiescer_fails_when_handles_remain() {
        let t = HandleTracker::new();
        let _id = t.register(HandleKind::File, "attachments/v1.bin");
        let q = HandleQuiescer::new(&t);
        let err = q.quiesce_and_verify(|| Ok(())).unwrap_err();
        match err {
            QuiesceError::HandlesOpen { count, open } => {
                assert_eq!(count, 1);
                assert_eq!(open[0].label, "attachments/v1.bin");
            }
            QuiesceError::Hook(_) => panic!("wrong variant"),
        }
    }

    #[test]
    fn quiescer_propagates_hook_error() {
        let t = HandleTracker::new();
        let q = HandleQuiescer::new(&t);
        let err = q
            .quiesce_and_verify(|| Err("db close failed".into()))
            .unwrap_err();
        assert!(matches!(err, QuiesceError::Hook(m) if m == "db close failed"));
    }

    // ── retry_io ───────────────────────────────────────────────────

    #[test]
    fn retry_io_returns_first_success() {
        let mut tries = 0;
        let v = retry_io(
            || {
                tries += 1;
                Ok::<u32, std::io::Error>(42)
            },
            3,
            1,
        )
        .unwrap();
        assert_eq!(v, 42);
        assert_eq!(tries, 1);
    }

    #[test]
    fn retry_io_eventually_succeeds() {
        use std::cell::Cell;
        let tries = Cell::new(0u32);
        let v = retry_io(
            || {
                let n = tries.get() + 1;
                tries.set(n);
                if n < 3 {
                    Err(std::io::Error::new(std::io::ErrorKind::Other, "transient"))
                } else {
                    Ok::<_, std::io::Error>("ok")
                }
            },
            5,
            1,
        )
        .unwrap();
        assert_eq!(v, "ok");
        assert_eq!(tries.get(), 3);
    }

    #[test]
    fn retry_io_returns_last_error_when_exhausted() {
        let err = retry_io::<(), _>(
            || Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope")),
            3,
            1,
        )
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
