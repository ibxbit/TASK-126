//! Crash-safe recovery: lockfile-based unclean-shutdown detection,
//! WAL checkpoints, integrity checks, orphan cleanup, and a tiny
//! file-handle tracker for leak detection.

pub mod checkpoint;
pub mod handles;

pub use checkpoint::{
    run_at_startup, write_lockfile, RecoveryError, RecoveryOutcome, RecoveryRepository,
    StartupOps,
};
pub use handles::{
    retry_io, HandleKind, HandleQuiescer, HandleTracker, QuiesceError, TrackedFile,
    DEFAULT_IO_RETRY_ATTEMPTS, DEFAULT_IO_RETRY_INITIAL_MS,
};
