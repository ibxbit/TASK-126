//! Sharing / data-protection IPC commands — SQLite-backed.

use std::sync::Arc;
use uuid::Uuid;

use crate::auth::Permission;
use crate::db::connection::Database;
use crate::db::repos::SqlitePackageRepo;
use crate::ipc::{guard, IpcError, SessionState};
use crate::sharing::expiry::{self, PackageRepository};
use crate::sharing::package::{build_share_package, PackageBuildInput, PackageBuildOutcome};
use crate::sharing::watermark::{wrap_with_watermark, WatermarkSpec};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[tauri::command]
pub fn cmd_wrap_with_watermark(
    session: tauri::State<'_, SessionState>,
    bytes: Vec<u8>,
    mime: String,
    spec: WatermarkSpec,
) -> Result<String, IpcError> {
    guard::require_authenticated(session.inner())?;
    wrap_with_watermark(&bytes, &mime, &spec)
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_share_build_package(
    session: tauri::State<'_, SessionState>,
    input: PackageBuildInput,
) -> Result<PackageBuildOutcome, IpcError> {
    let principal = guard::require(session.inner(), Permission::ExportReport, &input.tenant_id)?;
    build_share_package(&principal, input)
        .map_err(|e| IpcError::Internal(format!("{e:?}")))
}

#[tauri::command]
pub fn cmd_share_verify_access(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    package_id: Uuid,
    password: String,
) -> Result<serde_json::Value, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqlitePackageRepo::new(Arc::clone(db.inner()));
    let record = repo.load(&package_id)
        .map_err(|e| IpcError::Internal(e))?
        .ok_or_else(|| IpcError::Internal("package not found".into()))?;

    match expiry::verify_access(&record, &password, now()) {
        Ok(()) => {
            let _ = repo.record_access(&package_id, now());
            Ok(serde_json::json!({ "ok": true }))
        }
        Err(e) => {
            let reason = match &e {
                expiry::ExpiryError::Expired => "expired",
                expiry::ExpiryError::Revoked => "revoked",
                expiry::ExpiryError::BadPassword => "bad_password",
                _ => "error",
            };
            Ok(serde_json::json!({ "ok": false, "reason": reason }))
        }
    }
}

#[tauri::command]
pub fn cmd_share_revoke(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    package_id: Uuid,
) -> Result<(), IpcError> {
    let principal = guard::require_authenticated(session.inner())?;
    let repo = SqlitePackageRepo::new(Arc::clone(db.inner()));
    expiry::revoke_package(&repo, &principal, package_id, now())
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_share_sweep_expired(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<u32, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqlitePackageRepo::new(Arc::clone(db.inner()));
    expiry::sweep_expired(&repo, now())
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_returns_nonzero() {
        assert!(now() > 0);
    }

    #[test]
    fn watermark_renders_html_for_text() {
        let spec = WatermarkSpec {
            username: "testuser".into(),
            generated_at_unix: 1_700_000_000,
            label: Some("CONFIDENTIAL".into()),
        };
        let html = wrap_with_watermark(b"Hello, World!", "text/plain", &spec).unwrap();
        assert!(html.contains("testuser"), "watermark should include username");
        assert!(html.contains("CONFIDENTIAL"), "watermark should include label");
    }

    #[test]
    fn watermark_renders_html_for_image() {
        let spec = WatermarkSpec {
            username: "admin".into(),
            generated_at_unix: 1_700_000_000,
            label: None,
        };
        // Minimal PNG header bytes.
        let result = wrap_with_watermark(&[0x89, 0x50, 0x4e, 0x47], "image/png", &spec);
        assert!(result.is_ok(), "should produce watermarked HTML for images");
        let html = result.unwrap();
        assert!(html.contains("admin"));
    }
}
