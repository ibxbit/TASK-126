//! Local document management: storage, chunked uploads, metadata
//! index, and offline previews.

pub mod chunks;
pub mod index;
pub mod preview;
pub mod recovery;
pub mod storage;

pub use recovery::{
    cleanup_orphaned_uploads, CleanupError, CleanupReport, UploadRecoveryRepository,
};

pub use chunks::{
    ChunkError, ChunkRepository, ChunkStatus, FinalizeOutcome, SessionInit, UploadSession,
    DEFAULT_CHUNK_SIZE,
};
pub use index::{AttachmentVersion, DocumentIndex, IndexError, SearchHit, SearchQuery};
pub use preview::{PreviewError, PreviewPayload, Previewer};
pub use storage::{StorageError, StorageLayout};
