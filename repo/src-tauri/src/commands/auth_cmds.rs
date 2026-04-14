//! Auth IPC commands — login + logout that make SessionState operational.

use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::context::{Principal, TenantScope};
use crate::auth::roles::Role;
use crate::db::connection::Database;
use crate::ipc::{guard, IpcError, SessionState};

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user_id: String,
    pub username: String,
    pub role: String,
    pub tenant_ids: Vec<String>,
}

#[tauri::command]
pub fn cmd_login(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    username: String,
    password: String,
) -> Result<LoginResponse, IpcError> {
    let conn = db.conn();

    // 1. Look up user.
    let row: Option<(String, String, String, i64)> = conn
        .query_row(
            "SELECT id, username, password_hash, active FROM users WHERE username = ?1",
            [&username],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|e| IpcError::Internal(e.to_string()))?;

    let (user_id_str, uname, hash, active) =
        row.ok_or(IpcError::Internal("invalid credentials".into()))?;
    if active == 0 {
        return Err(IpcError::Internal("account disabled".into()));
    }

    // 2. Verify password (argon2id).
    let parsed = argon2::password_hash::PasswordHash::new(&hash)
        .map_err(|e| IpcError::Internal(e.to_string()))?;
    argon2::Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| IpcError::Internal("invalid credentials".into()))?;

    let user_id =
        Uuid::parse_str(&user_id_str).map_err(|e| IpcError::Internal(e.to_string()))?;

    // 3. Load role.
    let role_row: Option<(String, String)> = conn
        .query_row(
            "SELECT role_code, scope_kind FROM user_roles WHERE user_id = ?1",
            [&user_id_str],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| IpcError::Internal(e.to_string()))?;

    let (role_code, scope_kind) =
        role_row.ok_or(IpcError::Internal("user has no role assignment".into()))?;
    let role = Role::from_str(&role_code)
        .ok_or(IpcError::Internal(format!("unknown role: {role_code}")))?;

    // 4. Load tenant scope.
    let scope = if scope_kind == "global" {
        TenantScope::Global
    } else {
        let mut stmt = conn
            .prepare("SELECT tenant_id FROM user_role_tenants WHERE user_id = ?1")
            .map_err(|e| IpcError::Internal(e.to_string()))?;
        let ids: Vec<Uuid> = stmt
            .query_map([&user_id_str], |r| {
                let s: String = r.get(0)?;
                Ok(Uuid::parse_str(&s).unwrap_or(Uuid::nil()))
            })
            .map_err(|e| IpcError::Internal(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        TenantScope::Tenants(ids.clone())
    };

    let tenant_ids: Vec<String> = match &scope {
        TenantScope::Global => vec!["*".into()],
        TenantScope::Tenants(ids) => ids.iter().map(|t| t.to_string()).collect(),
    };

    // 5. Set session.
    let principal = Principal::new(user_id, uname.clone(), role, scope);
    session.set(principal);

    // 6. Update last_login.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let _ = conn.execute(
        "UPDATE users SET last_login_at = ?1 WHERE id = ?2",
        rusqlite::params![now, user_id_str],
    );

    Ok(LoginResponse {
        user_id: user_id_str,
        username: uname,
        role: role_code,
        tenant_ids,
    })
}

#[tauri::command]
pub fn cmd_logout(session: tauri::State<'_, SessionState>) -> Result<(), IpcError> {
    session.clear();
    Ok(())
}

#[tauri::command]
pub fn cmd_current_user(
    session: tauri::State<'_, SessionState>,
) -> Result<Option<LoginResponse>, IpcError> {
    match session.current() {
        Ok(p) => {
            let tenant_ids = match &p.scope {
                TenantScope::Global => vec!["*".into()],
                TenantScope::Tenants(ids) => ids.iter().map(|t| t.to_string()).collect(),
            };
            Ok(Some(LoginResponse {
                user_id: p.user_id.to_string(),
                username: p.username.clone(),
                role: p.role.as_str().to_string(),
                tenant_ids,
            }))
        }
        Err(_) => Ok(None),
    }
}

use argon2::PasswordVerifier;
