//! Offline preview handlers.
//!
//! Returns a `PreviewPayload` that the React side renders:
//!   - `Image` / `Pdf` → raw bytes + mime; UI uses <img src=blob:…>
//!     or an <embed>/pdf.js viewer.
//!   - `Text` → decoded UTF-8 string; UI renders in a <pre>.
//!
//! Previewing never spawns external processes and never reaches the
//! network — fully offline by construction.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{self, AuthError, Permission, Principal};
use crate::db::encryption::{aad_for, FieldCipher};
use crate::docs::index::{AttachmentQuery, VersionRow};
use crate::docs::storage::StorageLayout;

/// Upper bound for bytes returned in a preview payload (10 MiB).
/// Larger documents surface a `TooLarge` error and the UI falls back
/// to opening the file outside the preview pane.
pub const PREVIEW_BYTE_LIMIT: u64 = 10 * 1024 * 1024;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PreviewError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("mime type '{0}' is not previewable offline")]
    Unsupported(String),

    #[error("preview file not found at resolved path")]
    NotFound,

    #[error("file exceeds preview size limit ({limit} bytes)")]
    TooLarge { limit: u64 },

    #[error("persistence error: {0}")]
    Persistence(String),

    #[error("decryption failed")]
    Decrypt,

    #[error("I/O error: {0}")]
    Io(String),

    #[error("text content is not valid UTF-8")]
    InvalidUtf8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewPayload {
    /// PDF — returned as raw bytes with mime 'application/pdf'.
    Pdf { bytes: Vec<u8> },
    /// Bitmap image — returned as raw bytes; caller passes `mime`
    /// unchanged ('image/png' or 'image/jpeg') to the UI.
    Image { mime: String, bytes: Vec<u8> },
    /// Plain text — decoded UTF-8.
    Text { content: String },
}

pub struct Previewer<'a, Q: AttachmentQuery> {
    pub query: &'a Q,
    pub layout: &'a StorageLayout,
    pub cipher: &'a FieldCipher,
}

impl<'a, Q: AttachmentQuery> Previewer<'a, Q> {
    pub fn preview(
        &self,
        principal: &Principal,
        tenant_id: Uuid,
        attachment_id: Uuid,
        version_no: Option<u32>,
    ) -> Result<PreviewPayload, PreviewError> {
        auth::require(principal, Permission::ReadAny, &tenant_id)?;

        // Parent row holds mime + tenant scope.
        let parent = self
            .query
            .load(&attachment_id)
            .map_err(PreviewError::Persistence)?
            .ok_or(PreviewError::NotFound)?;
        if parent.tenant_id != tenant_id {
            return Err(PreviewError::Auth(AuthError::TenantScopeViolation {
                tenant_id: tenant_id.to_string(),
            }));
        }

        if !is_previewable(&parent.mime_type) {
            return Err(PreviewError::Unsupported(parent.mime_type));
        }

        // Resolve the target version.
        let versions = self
            .query
            .versions_for(&attachment_id)
            .map_err(PreviewError::Persistence)?;
        let v = pick_version(versions, version_no).ok_or(PreviewError::NotFound)?;

        if (v.byte_size as u64) > PREVIEW_BYTE_LIMIT {
            return Err(PreviewError::TooLarge { limit: PREVIEW_BYTE_LIMIT });
        }

        // Decrypt the relative path to reconstruct the absolute path.
        let aad = aad_for(
            "attachment_versions",
            "relative_path_enc",
            &v.attachment_id.to_string(),
        );
        let rel = self
            .cipher
            .decrypt_str(&v.relative_path_enc, &aad)
            .map_err(|_| PreviewError::Decrypt)?;
        let abs = self.layout.attachments_root().join(PathBuf::from(rel));
        let bytes = read_all(&abs)?;

        match parent.mime_type.as_str() {
            "application/pdf" => Ok(PreviewPayload::Pdf { bytes }),
            "image/png" | "image/jpeg" | "image/jpg" => Ok(PreviewPayload::Image {
                mime: parent.mime_type,
                bytes,
            }),
            "text/plain" => {
                let content = String::from_utf8(bytes).map_err(|_| PreviewError::InvalidUtf8)?;
                Ok(PreviewPayload::Text { content })
            }
            other => Err(PreviewError::Unsupported(other.to_string())),
        }
    }
}

fn is_previewable(mime: &str) -> bool {
    matches!(
        mime,
        "application/pdf" | "image/png" | "image/jpeg" | "image/jpg" | "text/plain"
    )
}

fn pick_version(mut rows: Vec<VersionRow>, requested: Option<u32>) -> Option<VersionRow> {
    match requested {
        Some(n) => rows.into_iter().find(|v| v.version_no == n),
        None => {
            rows.sort_by_key(|v| v.version_no);
            rows.pop()
        }
    }
}

fn read_all(path: &Path) -> Result<Vec<u8>, PreviewError> {
    if !path.exists() {
        return Err(PreviewError::NotFound);
    }
    let mut f = File::open(path).map_err(|e| PreviewError::Io(e.to_string()))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| PreviewError::Io(e.to_string()))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_whitelist_is_closed() {
        assert!(is_previewable("application/pdf"));
        assert!(is_previewable("image/png"));
        assert!(is_previewable("image/jpeg"));
        assert!(is_previewable("text/plain"));
        assert!(!is_previewable("application/zip"));
        assert!(!is_previewable("image/gif"));
        assert!(!is_previewable("video/mp4"));
    }

    #[test]
    fn pick_version_defaults_to_latest() {
        let rows = vec![
            VersionRow {
                id: Uuid::new_v4(), attachment_id: Uuid::new_v4(),
                version_no: 1, relative_path_enc: vec![], byte_size: 1,
                sha256_hex: "".into(), created_at: 0,
            },
            VersionRow {
                id: Uuid::new_v4(), attachment_id: Uuid::new_v4(),
                version_no: 3, relative_path_enc: vec![], byte_size: 1,
                sha256_hex: "".into(), created_at: 0,
            },
            VersionRow {
                id: Uuid::new_v4(), attachment_id: Uuid::new_v4(),
                version_no: 2, relative_path_enc: vec![], byte_size: 1,
                sha256_hex: "".into(), created_at: 0,
            },
        ];
        assert_eq!(pick_version(rows.clone(), None).unwrap().version_no, 3);
        assert_eq!(pick_version(rows, Some(2)).unwrap().version_no, 2);
    }
}
