//! Local share-package builder.
//!
//! Outputs an AES-encrypted ZIP containing:
//!   - one watermarked HTML wrapper per input asset
//!   - the original asset bytes (so power users with the password can
//!     also recover the source); the wrapper references them via
//!     relative path
//!   - a `manifest.json` with package id, expiry, contents list, and
//!     the issuing user / tenant
//!
//! The user-supplied password drives the ZIP encryption directly
//! (AES-256). An Argon2id hash of the password is returned so the
//! caller can persist a verifier in `share_packages.password_hash` —
//! the password itself never reaches disk in plaintext.

use std::io::Write;

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHasher};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use crate::auth::{self, AuthError, Permission, Principal};
use crate::sharing::watermark::{wrap_with_watermark, WatermarkError, WatermarkSpec};

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackageError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("password must be at least 8 characters")]
    WeakPassword,

    #[error("package must contain at least one item")]
    Empty,

    #[error("expiry must be in the future")]
    BadExpiry,

    #[error(transparent)]
    Watermark(#[from] WatermarkError),

    #[error("zip error: {0}")]
    Zip(String),

    #[error("hash error: {0}")]
    Hash(String),
}

impl From<zip::result::ZipError> for PackageError {
    fn from(e: zip::result::ZipError) -> Self {
        PackageError::Zip(e.to_string())
    }
}
impl From<std::io::Error> for PackageError {
    fn from(e: std::io::Error) -> Self {
        PackageError::Zip(e.to_string())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageItem {
    /// Display filename without path components, e.g. "Statement.pdf".
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageBuildInput {
    pub tenant_id: Uuid,
    pub recipient_label: Option<String>,
    pub items: Vec<PackageItem>,
    /// User-typed password — drives ZIP encryption AND is hashed for
    /// the access verifier. Never persisted.
    pub password: String,
    pub expires_at_unix: i64,
    pub created_at_unix: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageBuildOutcome {
    pub package_id: Uuid,
    pub zip_bytes: Vec<u8>,
    pub sha256_hex: String,
    pub password_hash: String,
    pub password_salt: Vec<u8>,
    pub contents_summary: String,
}

#[derive(Debug, Serialize)]
struct Manifest<'a> {
    package_id: String,
    issued_by: &'a str,
    issued_by_user_id: String,
    tenant_id: String,
    recipient_label: Option<&'a str>,
    created_at_unix: i64,
    expires_at_unix: i64,
    items: Vec<ManifestItem<'a>>,
    notice: &'static str,
}

#[derive(Debug, Serialize)]
struct ManifestItem<'a> {
    filename: &'a str,
    watermarked_html: String,
    mime_type: &'a str,
    sha256_hex: String,
}

const NOTICE: &str =
    "This package is intended for the named recipient only. \
     The contents are watermarked. Re-distribution is not authorized. \
     The package will become inaccessible after its expiry timestamp.";

const MIN_PASSWORD_LEN: usize = 8;

/// Build the encrypted package. Caller must hold `ExportReport` in
/// the package's tenant scope.
pub fn build_share_package(
    principal: &Principal,
    input: PackageBuildInput,
) -> Result<PackageBuildOutcome, PackageError> {
    auth::require(principal, Permission::ExportReport, &input.tenant_id)?;

    if input.items.is_empty() {
        return Err(PackageError::Empty);
    }
    if input.password.chars().count() < MIN_PASSWORD_LEN {
        return Err(PackageError::WeakPassword);
    }
    if input.expires_at_unix <= input.created_at_unix {
        return Err(PackageError::BadExpiry);
    }

    let password = input.password.clone();
    let package_id = Uuid::new_v4();
    let watermark = WatermarkSpec {
        username: principal.username.clone(),
        generated_at_unix: input.created_at_unix,
        label: Some("SHARED PACKAGE — DO NOT REDISTRIBUTE".into()),
    };

    // Pre-hash for the access verifier.
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(input.password.as_bytes(), &salt)
        .map_err(|e| PackageError::Hash(e.to_string()))?
        .to_string();
    let salt_bytes = salt.as_str().as_bytes().to_vec();

    // Build manifest while writing zip entries.
    let mut buf = Vec::<u8>::new();
    let cursor = std::io::Cursor::new(&mut buf);
    let mut zip = zip::ZipWriter::new(cursor);

    // AES-256 encryption is provided by the `aes` feature of the zip crate.
    let opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .with_aes_encryption(zip::AesMode::Aes256, &password);

    let mut manifest_items: Vec<ManifestItem> = Vec::with_capacity(input.items.len());

    for item in &input.items {
        if item.filename.contains('/') || item.filename.contains('\\') {
            return Err(PackageError::Zip(format!(
                "filename contains a path separator: {}",
                item.filename
            )));
        }

        // Watermarked HTML wrapper.
        let wrapped = wrap_with_watermark(&item.bytes, &item.mime_type, &watermark)?;
        let wrapped_name = format!("watermarked/{}.html", strip_extension(&item.filename));
        zip.start_file(&wrapped_name, opts)?;
        zip.write_all(wrapped.as_bytes())?;

        // Original asset, preserved for traceability.
        let original_name = format!("originals/{}", item.filename);
        zip.start_file(&original_name, opts)?;
        zip.write_all(&item.bytes)?;

        let mut h = Sha256::new();
        h.update(&item.bytes);
        manifest_items.push(ManifestItem {
            filename: &item.filename,
            watermarked_html: wrapped_name,
            mime_type: &item.mime_type,
            sha256_hex: hex::encode(h.finalize()),
        });
    }

    let manifest = Manifest {
        package_id: package_id.to_string(),
        issued_by: &principal.username,
        issued_by_user_id: principal.user_id.to_string(),
        tenant_id: input.tenant_id.to_string(),
        recipient_label: input.recipient_label.as_deref(),
        created_at_unix: input.created_at_unix,
        expires_at_unix: input.expires_at_unix,
        items: manifest_items,
        notice: NOTICE,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| PackageError::Zip(e.to_string()))?;

    zip.start_file("manifest.json", opts)?;
    zip.write_all(&manifest_bytes)?;

    zip.finish()?;

    let mut h = Sha256::new();
    h.update(&buf);
    let sha = hex::encode(h.finalize());

    let summary = format!(
        "{} item(s); expires at {}",
        input.items.len(),
        input.expires_at_unix
    );

    Ok(PackageBuildOutcome {
        package_id,
        zip_bytes: buf,
        sha256_hex: sha,
        password_hash: hash,
        password_salt: salt_bytes,
        contents_summary: summary,
    })
}

fn strip_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(idx) => name[..idx].to_string(),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::context::TenantScope;
    use crate::auth::Role;

    fn admin(tenant: Uuid) -> Principal {
        Principal::new(
            Uuid::new_v4(),
            "alice".into(),
            Role::Administrator,
            TenantScope::single(tenant),
        )
    }

    fn liaison(tenant: Uuid) -> Principal {
        Principal::new(
            Uuid::new_v4(),
            "lin".into(),
            Role::Liaison,
            TenantScope::single(tenant),
        )
    }

    fn input(tenant: Uuid, password: &str) -> PackageBuildInput {
        PackageBuildInput {
            tenant_id: tenant,
            recipient_label: Some("Test recipient".into()),
            items: vec![PackageItem {
                filename: "report.txt".into(),
                mime_type: "text/plain".into(),
                bytes: b"hello world".to_vec(),
            }],
            password: password.into(),
            expires_at_unix: 1_000_000,
            created_at_unix: 0,
        }
    }

    #[test]
    fn rejects_weak_password() {
        let t = Uuid::new_v4();
        let err = build_share_package(&admin(t), input(t, "short")).unwrap_err();
        assert!(matches!(err, PackageError::WeakPassword));
    }

    #[test]
    fn rejects_empty_input() {
        let t = Uuid::new_v4();
        let mut i = input(t, "longpassword");
        i.items.clear();
        let err = build_share_package(&admin(t), i).unwrap_err();
        assert!(matches!(err, PackageError::Empty));
    }

    #[test]
    fn rejects_bad_expiry() {
        let t = Uuid::new_v4();
        let mut i = input(t, "longpassword");
        i.expires_at_unix = i.created_at_unix;
        let err = build_share_package(&admin(t), i).unwrap_err();
        assert!(matches!(err, PackageError::BadExpiry));
    }

    #[test]
    fn rejects_path_traversal_filename() {
        let t = Uuid::new_v4();
        let mut i = input(t, "longpassword");
        i.items[0].filename = "../etc/passwd".into();
        let err = build_share_package(&admin(t), i).unwrap_err();
        assert!(matches!(err, PackageError::Zip(_)));
    }

    #[test]
    fn liaison_lacks_export_permission() {
        let t = Uuid::new_v4();
        let err = build_share_package(&liaison(t), input(t, "longpassword")).unwrap_err();
        assert!(matches!(err, PackageError::Auth(_)));
    }

    #[test]
    fn happy_path_emits_zip_and_hash() {
        let t = Uuid::new_v4();
        let out = build_share_package(&admin(t), input(t, "longpassword")).unwrap();
        // ZIP local file header magic.
        assert_eq!(&out.zip_bytes[0..4], b"PK\x03\x04");
        assert!(!out.password_hash.is_empty());
        assert!(!out.password_salt.is_empty());
        assert_eq!(out.sha256_hex.len(), 64);
    }
}
