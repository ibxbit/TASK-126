//! Signature verification for offline `.spkg` update packages.
//!
//! Package layout (zip):
//!     manifest.json     — canonical UTF-8 JSON, see `PackageManifest`.
//!     signature.bin     — 64-byte Ed25519 signature over the
//!                         CANONICAL serialization of manifest.json
//!                         (NOT the raw file bytes — see below).
//!     payload/...       — files to install.
//!
//! Canonical form: `manifest.json` is parsed into `PackageManifest`
//! and re-serialized with `canonical_manifest_bytes`, which sorts
//! object keys and uses no extra whitespace. This way a publisher
//! tool can pretty-print the file for humans while the verifier still
//! recomputes the bytes that were signed.
//!
//! The Ed25519 public key is embedded in the binary at compile time
//! (or supplied via `verify_package`'s argument for tests). No
//! network keys, no PKI dependency.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

use ed25519_dalek::{Signature, VerifyingKey, SIGNATURE_LENGTH};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum VerifyError {
    #[error("io error reading package: {0}")]
    Io(String),
    #[error("zip error: {0}")]
    Zip(String),
    #[error("manifest.json missing or malformed: {0}")]
    Manifest(String),
    #[error("signature.bin missing or wrong length")]
    Signature,
    #[error("payload sha256 mismatch")]
    PayloadDigest,
    #[error("ed25519 verification failed")]
    BadSignature,
    #[error("unsupported manifest version: {0}")]
    UnsupportedVersion(u32),
    #[error("public key length must be 32 bytes")]
    BadPublicKey,
}

impl From<std::io::Error> for VerifyError {
    fn from(e: std::io::Error) -> Self {
        VerifyError::Io(e.to_string())
    }
}
impl From<zip::result::ZipError> for VerifyError {
    fn from(e: zip::result::ZipError) -> Self {
        VerifyError::Zip(e.to_string())
    }
}

/// Manifest format. `manifest_format` is bumped if the schema changes;
/// verifiers refuse unknown majors.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageManifest {
    pub manifest_format: u32,
    pub package_id: Uuid,
    /// Semver-ish string, e.g. "1.4.2". Compared as a plain string
    /// against `app_versions.version`.
    pub version: String,
    pub created_at_unix: i64,
    /// Required current version, if the publisher wants to gate the
    /// update. `None` ⇒ no gate.
    pub min_required_version: Option<String>,
    /// SHA-256 (lowercase hex) over the concatenation of payload file
    /// bytes IN sorted-by-name order. Computed by the publisher tool.
    pub payload_sha256_hex: String,
    /// Human-readable release notes; not load-bearing.
    pub notes: Option<String>,
}

/// Successfully verified package — ready to be installed.
#[derive(Debug, Clone, Serialize)]
pub struct VerifiedPackage {
    pub manifest: PackageManifest,
    /// Absolute path of the package file on disk.
    pub package_path: PathBuf,
}

const SUPPORTED_FORMAT: u32 = 1;

/// Re-serialize a manifest in canonical form so signers and verifiers
/// agree on the byte sequence that was signed.
///
/// Canonical = sorted object keys, no extra whitespace, UTF-8.
pub fn canonical_manifest_bytes(m: &PackageManifest) -> Result<Vec<u8>, VerifyError> {
    // Parse → BTreeMap (sorted keys) → emit compact JSON.
    let v = serde_json::to_value(m).map_err(|e| VerifyError::Manifest(e.to_string()))?;
    let canon = canonicalize_value(&v);
    serde_json::to_vec(&canon).map_err(|e| VerifyError::Manifest(e.to_string()))
}

fn canonicalize_value(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(m) => {
            let sorted: BTreeMap<String, serde_json::Value> = m
                .iter()
                .map(|(k, vv)| (k.clone(), canonicalize_value(vv)))
                .collect();
            // BTreeMap → Value::Object preserves insertion order in
            // serde_json (which matches BTreeMap iteration order).
            let mut out = serde_json::Map::new();
            for (k, vv) in sorted {
                out.insert(k, vv);
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(canonicalize_value).collect()),
        other => other.clone(),
    }
}

/// Verify a package end-to-end. Returns the manifest if every check
/// passes; consumers then proceed to `installer::install_package`.
pub fn verify_package(
    package_path: PathBuf,
    public_key_bytes: &[u8],
) -> Result<VerifiedPackage, VerifyError> {
    if public_key_bytes.len() != 32 {
        return Err(VerifyError::BadPublicKey);
    }
    let mut pk_arr = [0u8; 32];
    pk_arr.copy_from_slice(public_key_bytes);
    let verifying_key =
        VerifyingKey::from_bytes(&pk_arr).map_err(|_| VerifyError::BadPublicKey)?;

    let file = std::fs::File::open(&package_path)?;
    let mut zip = zip::ZipArchive::new(file)?;

    // Read manifest.
    let manifest_raw = read_entry(&mut zip, "manifest.json")?;
    let manifest: PackageManifest = serde_json::from_slice(&manifest_raw)
        .map_err(|e| VerifyError::Manifest(e.to_string()))?;

    if manifest.manifest_format != SUPPORTED_FORMAT {
        return Err(VerifyError::UnsupportedVersion(manifest.manifest_format));
    }

    // Read signature.
    let sig_raw = read_entry(&mut zip, "signature.bin")?;
    if sig_raw.len() != SIGNATURE_LENGTH {
        return Err(VerifyError::Signature);
    }
    let mut sig_arr = [0u8; SIGNATURE_LENGTH];
    sig_arr.copy_from_slice(&sig_raw);
    let signature = Signature::from_bytes(&sig_arr);

    // Verify signature over canonical manifest bytes.
    let canon = canonical_manifest_bytes(&manifest)?;
    use ed25519_dalek::Verifier;
    verifying_key
        .verify(&canon, &signature)
        .map_err(|_| VerifyError::BadSignature)?;

    // Verify payload digest.
    let computed = compute_payload_digest(&mut zip)?;
    if computed != manifest.payload_sha256_hex.to_ascii_lowercase() {
        return Err(VerifyError::PayloadDigest);
    }

    Ok(VerifiedPackage { manifest, package_path })
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<Vec<u8>, VerifyError> {
    let mut entry = zip.by_name(name)?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

fn compute_payload_digest(
    zip: &mut zip::ZipArchive<std::fs::File>,
) -> Result<String, VerifyError> {
    // Collect payload file names in sorted order — order matters
    // because the digest concatenates files in this order.
    let mut names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|e| e.name().to_string()))
        .filter(|n| n.starts_with("payload/") && !n.ends_with('/'))
        .collect();
    names.sort();

    let mut h = Sha256::new();
    for name in &names {
        let mut entry = zip.by_name(name)?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
    }
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    fn build_package(
        signing_key: &SigningKey,
        manifest: &PackageManifest,
        payload_files: &[(&str, &[u8])],
    ) -> PathBuf {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.spkg");
        // Leak the dir so the file survives this scope for callers.
        std::mem::forget(dir);

        let f = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(f);
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        // payload first so we can compute digest in the manifest below
        for (name, bytes) in payload_files {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(bytes).unwrap();
        }

        // Manifest
        let manifest_bytes = serde_json::to_vec_pretty(manifest).unwrap();
        zip.start_file("manifest.json", opts).unwrap();
        zip.write_all(&manifest_bytes).unwrap();

        // Signature over canonical manifest bytes
        let canon = canonical_manifest_bytes(manifest).unwrap();
        let sig = signing_key.sign(&canon);
        zip.start_file("signature.bin", opts).unwrap();
        zip.write_all(&sig.to_bytes()).unwrap();

        zip.finish().unwrap();
        path
    }

    fn payload_digest(files: &[(&str, &[u8])]) -> String {
        let mut sorted: Vec<_> = files.iter().collect();
        sorted.sort_by_key(|(n, _)| *n);
        let mut h = Sha256::new();
        for (_, b) in sorted {
            h.update(b);
        }
        hex::encode(h.finalize())
    }

    #[test]
    fn happy_path_verifies() {
        let mut csprng = OsRng;
        let signing = SigningKey::generate(&mut csprng);
        let payload = vec![("payload/a.bin", &b"hello"[..]), ("payload/b.bin", &b"world"[..])];
        let manifest = PackageManifest {
            manifest_format: 1,
            package_id: Uuid::new_v4(),
            version: "1.0.0".into(),
            created_at_unix: 1,
            min_required_version: None,
            payload_sha256_hex: payload_digest(&payload),
            notes: None,
        };
        let path = build_package(&signing, &manifest, &payload);
        let v = verify_package(path, signing.verifying_key().as_bytes()).unwrap();
        assert_eq!(v.manifest.version, "1.0.0");
    }

    #[test]
    fn tampered_manifest_fails_signature() {
        let mut csprng = OsRng;
        let signing = SigningKey::generate(&mut csprng);
        let payload = vec![("payload/a.bin", &b"hello"[..])];
        let mut manifest = PackageManifest {
            manifest_format: 1,
            package_id: Uuid::new_v4(),
            version: "1.0.0".into(),
            created_at_unix: 1,
            min_required_version: None,
            payload_sha256_hex: payload_digest(&payload),
            notes: None,
        };
        let path = build_package(&signing, &manifest, &payload);
        // Now mutate the manifest in the zip — simulate an attacker
        // bumping the version. We re-build with a new manifest under
        // the OLD signature.
        // Simulate this by re-signing with a different key:
        let attacker = SigningKey::generate(&mut csprng);
        manifest.version = "9.9.9".into();
        let bad = build_package(&attacker, &manifest, &payload);
        let err = verify_package(bad, signing.verifying_key().as_bytes()).unwrap_err();
        assert!(matches!(err, VerifyError::BadSignature));

        // The original package should still verify.
        let _ = verify_package(path, signing.verifying_key().as_bytes()).unwrap();
    }

    #[test]
    fn payload_digest_mismatch_detected() {
        let mut csprng = OsRng;
        let signing = SigningKey::generate(&mut csprng);
        let payload = vec![("payload/a.bin", &b"hello"[..])];
        // Manifest claims a digest for the payload that doesn't match.
        let manifest = PackageManifest {
            manifest_format: 1,
            package_id: Uuid::new_v4(),
            version: "1.0.0".into(),
            created_at_unix: 1,
            min_required_version: None,
            payload_sha256_hex: "deadbeef".into(),
            notes: None,
        };
        let path = build_package(&signing, &manifest, &payload);
        let err = verify_package(path, signing.verifying_key().as_bytes()).unwrap_err();
        assert!(matches!(err, VerifyError::PayloadDigest));
    }

    #[test]
    fn unsupported_format_rejected() {
        let mut csprng = OsRng;
        let signing = SigningKey::generate(&mut csprng);
        let payload = vec![("payload/a.bin", &b"x"[..])];
        let manifest = PackageManifest {
            manifest_format: 999,
            package_id: Uuid::new_v4(),
            version: "1.0.0".into(),
            created_at_unix: 1,
            min_required_version: None,
            payload_sha256_hex: payload_digest(&payload),
            notes: None,
        };
        let path = build_package(&signing, &manifest, &payload);
        let err = verify_package(path, signing.verifying_key().as_bytes()).unwrap_err();
        assert!(matches!(err, VerifyError::UnsupportedVersion(999)));
    }
}
