//! Metadata indexing and search.
//!
//! Display names and relative paths are encrypted at rest, so
//! substring search requires in-process decryption. The search funnel:
//!
//!   1. SQL pre-filter: tenant + entity scope (+ optional tag /
//!      mime-type equality) — cheap, index-backed.
//!   2. Decrypt `display_name_enc` for each survivor (bounded list).
//!   3. Substring match the query against the decrypted name.
//!
//! This keeps the sensitive index keyed AES-GCM while still serving
//! typical "find a document by name" flows quickly in practice.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{self, AuthError, Permission, Principal};
use crate::db::encryption::{aad_for, CipherError, FieldCipher};

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum IndexError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("persistence error: {0}")]
    Persistence(String),

    #[error("decryption failed for attachment {0}")]
    Decrypt(String),
}

impl From<CipherError> for IndexError {
    fn from(_: CipherError) -> Self {
        IndexError::Decrypt("cipher".into())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    pub tenant_id: Uuid,
    /// Free-text substring (case-insensitive). None ⇒ no name filter.
    pub text: Option<String>,
    /// Exact tag match. None ⇒ no tag filter.
    pub tag: Option<String>,
    /// Exact mime match. None ⇒ no mime filter.
    pub mime_type: Option<String>,
    /// Restrict to a specific entity (case/parcel/claim/…).
    pub entity_kind: Option<String>,
    pub entity_id: Option<Uuid>,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub attachment_id: Uuid,
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub display_name: String,
    pub mime_type: String,
    pub byte_size: i64,
    pub sha256_hex: String,
    pub tags: Vec<String>,
    pub latest_version_no: u32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentVersion {
    pub id: Uuid,
    pub attachment_id: Uuid,
    pub version_no: u32,
    /// Decrypted relative path (under the attachments root).
    pub relative_path: String,
    pub byte_size: i64,
    pub sha256_hex: String,
    pub created_at: i64,
}

/// Row as fetched from SQL before decryption.
#[derive(Debug, Clone)]
pub struct AttachmentRow {
    pub attachment_id: Uuid,
    pub tenant_id: Uuid,
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub display_name_enc: Vec<u8>,
    pub mime_type: String,
    pub byte_size: i64,
    pub sha256_hex: String,
    pub latest_version_no: u32,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct VersionRow {
    pub id: Uuid,
    pub attachment_id: Uuid,
    pub version_no: u32,
    pub relative_path_enc: Vec<u8>,
    pub byte_size: i64,
    pub sha256_hex: String,
    pub created_at: i64,
}

pub trait AttachmentQuery {
    fn search(&self, query: &SearchQuery) -> Result<Vec<AttachmentRow>, String>;
    fn tags_for(&self, attachment_id: &Uuid) -> Result<Vec<String>, String>;
    fn versions_for(&self, attachment_id: &Uuid) -> Result<Vec<VersionRow>, String>;
    fn load(&self, attachment_id: &Uuid) -> Result<Option<AttachmentRow>, String>;
}

pub trait TagRepository {
    fn add_tag(&self, attachment_id: &Uuid, tag: &str, now: i64, by: Option<&Uuid>) -> Result<(), String>;
    fn remove_tag(&self, attachment_id: &Uuid, tag: &str) -> Result<(), String>;
}

pub struct DocumentIndex<'a, Q: AttachmentQuery, T: TagRepository> {
    pub query: &'a Q,
    pub tags: &'a T,
    pub cipher: &'a FieldCipher,
}

impl<'a, Q: AttachmentQuery, T: TagRepository> DocumentIndex<'a, Q, T> {
    /// Permission-gated search. Any viewer of claims/residents has
    /// `ReadAny`; reviewers + admins additionally have `ExportReport`
    /// for the underlying content. Here we only gate on the read
    /// permission, and rely on the caller to gate per-entity access.
    pub fn search(
        &self,
        principal: &Principal,
        query: SearchQuery,
    ) -> Result<Vec<SearchHit>, IndexError> {
        auth::require(principal, Permission::ReadAny, &query.tenant_id)?;

        let rows = self.query.search(&query).map_err(IndexError::Persistence)?;

        let needle = query.text.as_ref().map(|s| s.to_ascii_lowercase());
        let mut hits: Vec<SearchHit> = Vec::with_capacity(rows.len());
        for r in rows {
            let aad = aad_for("attachments", "display_name_enc", &r.attachment_id.to_string());
            let name = self
                .cipher
                .decrypt_str(&r.display_name_enc, &aad)
                .map_err(|_| IndexError::Decrypt(r.attachment_id.to_string()))?;

            if let Some(n) = &needle {
                if !name.to_ascii_lowercase().contains(n) {
                    continue;
                }
            }

            let tags = self
                .tags
                .tags_for_wrapper(self.query, &r.attachment_id)
                .map_err(IndexError::Persistence)?;

            hits.push(SearchHit {
                attachment_id: r.attachment_id,
                entity_kind: r.entity_kind,
                entity_id: r.entity_id,
                display_name: name,
                mime_type: r.mime_type,
                byte_size: r.byte_size,
                sha256_hex: r.sha256_hex,
                tags,
                latest_version_no: r.latest_version_no,
                created_at: r.created_at,
            });

            if hits.len() as u32 >= query.limit {
                break;
            }
        }
        Ok(hits)
    }

    pub fn versions(
        &self,
        principal: &Principal,
        tenant_id: Uuid,
        attachment_id: Uuid,
    ) -> Result<Vec<AttachmentVersion>, IndexError> {
        auth::require(principal, Permission::ReadAny, &tenant_id)?;

        let rows = self
            .query
            .versions_for(&attachment_id)
            .map_err(IndexError::Persistence)?;

        rows.into_iter()
            .map(|v| {
                let aad = aad_for(
                    "attachment_versions",
                    "relative_path_enc",
                    &v.attachment_id.to_string(),
                );
                let rel = self
                    .cipher
                    .decrypt_str(&v.relative_path_enc, &aad)
                    .map_err(|_| IndexError::Decrypt(v.attachment_id.to_string()))?;
                Ok(AttachmentVersion {
                    id: v.id,
                    attachment_id: v.attachment_id,
                    version_no: v.version_no,
                    relative_path: rel,
                    byte_size: v.byte_size,
                    sha256_hex: v.sha256_hex,
                    created_at: v.created_at,
                })
            })
            .collect()
    }

    pub fn add_tag(
        &self,
        principal: &Principal,
        tenant_id: Uuid,
        attachment_id: Uuid,
        tag: String,
        now: i64,
    ) -> Result<(), IndexError> {
        auth::require(principal, Permission::InputResidentData, &tenant_id)?;
        let t = normalize_tag(&tag);
        self.tags
            .add_tag(&attachment_id, &t, now, Some(&principal.user_id))
            .map_err(IndexError::Persistence)
    }

    pub fn remove_tag(
        &self,
        principal: &Principal,
        tenant_id: Uuid,
        attachment_id: Uuid,
        tag: String,
    ) -> Result<(), IndexError> {
        auth::require(principal, Permission::InputResidentData, &tenant_id)?;
        let t = normalize_tag(&tag);
        self.tags
            .remove_tag(&attachment_id, &t)
            .map_err(IndexError::Persistence)
    }
}

// Small dispatch helper so the TagRepository trait stays minimal —
// tags_for is logically a query operation, placed on AttachmentQuery.
trait TagsForHelper {
    fn tags_for_wrapper<Q: AttachmentQuery>(
        &self,
        q: &Q,
        id: &Uuid,
    ) -> Result<Vec<String>, String>;
}
impl<T: TagRepository> TagsForHelper for T {
    fn tags_for_wrapper<Q: AttachmentQuery>(
        &self,
        q: &Q,
        id: &Uuid,
    ) -> Result<Vec<String>, String> {
        q.tags_for(id)
    }
}

/// Normalize a tag: lowercase, trim, collapse internal whitespace to
/// single dashes. Empty tags are rejected at the caller.
pub fn normalize_tag(raw: &str) -> String {
    let lowered = raw.to_ascii_lowercase();
    let parts: Vec<&str> = lowered.split_whitespace().collect();
    parts.join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_normalization() {
        assert_eq!(normalize_tag("  Move Out  "), "move-out");
        assert_eq!(normalize_tag("Urgent"), "urgent");
        assert_eq!(normalize_tag("a  b   c"), "a-b-c");
    }
}
