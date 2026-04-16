//! Reusable IPC permission guard.
//!
//! Every Tauri `#[tauri::command]` that performs a scoped read or
//! mutation MUST call one of:
//!
//!   - `require(&session, permission, &tenant_id)`       — full gate
//!   - `require_any(&session, &[perm, perm], &tenant_id)` — OR semantics
//!   - `require_authenticated(&session)`                  — login-only
//!
//! …BEFORE doing any work. The helper:
//!   1. Extracts the current `Principal` from `SessionState`
//!      (user_id, role, tenant scope, session id).
//!   2. Delegates to the existing `auth::guard::require` for the
//!      permission + tenant-scope check.
//!   3. Returns a single structured `IpcError` whose JSON shape is
//!      stable — the React side switches on `error.type` rather than
//!      parsing `error.message`.
//!
//! Keeping extraction AND validation in one call makes the guard the
//! minimum-visible-code path and removes the "forgot to check" class
//! of bugs: if a handler receives a `Principal`, the permission check
//! already ran.

use std::sync::RwLock;

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{self, AuthError, Permission, Principal};

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum IpcError {
    /// No principal in the session — not logged in / session expired.
    #[error("unauthenticated: session has no principal")]
    Unauthenticated,

    #[error("permission denied: role '{role}' lacks '{permission}'")]
    PermissionDenied { role: String, permission: String },

    #[error("tenant scope violation: cannot access tenant '{tenant_id}'")]
    TenantScopeViolation { tenant_id: String },

    /// Covers lock poisoning and other infrastructural failures.
    /// Surfaced verbatim only to admins; other roles see a generic
    /// message in the UI layer.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<AuthError> for IpcError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::PermissionDenied { role, permission } => {
                IpcError::PermissionDenied { role, permission }
            }
            AuthError::TenantScopeViolation { tenant_id } => {
                IpcError::TenantScopeViolation { tenant_id }
            }
        }
    }
}

/// Holds the current logged-in `Principal`. Bound into Tauri state
/// at application setup via `.manage(SessionState::new())`. The login
/// flow calls `set`; logout calls `clear`.
#[derive(Default)]
pub struct SessionState {
    inner: RwLock<Option<Principal>>,
}

impl SessionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, principal: Principal) {
        *self.inner.write().expect("session state poisoned") = Some(principal);
    }

    pub fn clear(&self) {
        *self.inner.write().expect("session state poisoned") = None;
    }

    /// Snapshot the current principal. `Err(Unauthenticated)` if nobody
    /// is logged in.
    pub fn current(&self) -> Result<Principal, IpcError> {
        let g = self
            .inner
            .read()
            .map_err(|e| IpcError::Internal(e.to_string()))?;
        g.clone().ok_or(IpcError::Unauthenticated)
    }
}

/// Authenticated-only gate. Use when the command is open to any
/// logged-in role and finer-grained checks happen deeper in the call
/// stack.
pub fn require_authenticated(session: &SessionState) -> Result<Principal, IpcError> {
    session.current()
}

/// Primary gate. Extract the current principal AND verify
/// `permission` within `tenant_id`. On success the principal is
/// returned for downstream use (audit metadata, filtering, etc.).
pub fn require(
    session: &SessionState,
    permission: Permission,
    tenant_id: &Uuid,
) -> Result<Principal, IpcError> {
    let principal = session.current()?;
    auth::require(&principal, permission, tenant_id)?;
    Ok(principal)
}

/// OR-gate: any of `permissions` grants access.
pub fn require_any(
    session: &SessionState,
    permissions: &[Permission],
    tenant_id: &Uuid,
) -> Result<Principal, IpcError> {
    let principal = session.current()?;
    auth::require_any(&principal, permissions, tenant_id)?;
    Ok(principal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::context::TenantScope;
    use crate::auth::Role;

    fn session_with(p: Principal) -> SessionState {
        let s = SessionState::new();
        s.set(p);
        s
    }

    fn principal(role: Role, scope: TenantScope) -> Principal {
        Principal::new(Uuid::new_v4(), "u".into(), role, scope)
    }

    #[test]
    fn unauthenticated_when_no_principal() {
        let s = SessionState::new();
        assert!(matches!(
            require(&s, Permission::ViewClaim, &Uuid::new_v4()),
            Err(IpcError::Unauthenticated)
        ));
    }

    #[test]
    fn permission_denied_maps_to_structured_error() {
        let t = Uuid::new_v4();
        let s = session_with(principal(Role::Liaison, TenantScope::single(t)));
        let err = require(&s, Permission::ApproveSettlement, &t).unwrap_err();
        match err {
            IpcError::PermissionDenied { role, permission } => {
                assert_eq!(role, "liaison");
                assert_eq!(permission, "approve_settlement");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn tenant_scope_violation_maps_to_structured_error() {
        let allowed = Uuid::new_v4();
        let foreign = Uuid::new_v4();
        let s = session_with(principal(Role::PropertyManager, TenantScope::single(allowed)));
        let err = require(&s, Permission::ApproveSettlement, &foreign).unwrap_err();
        assert!(matches!(err, IpcError::TenantScopeViolation { .. }));
    }

    #[test]
    fn happy_path_returns_principal() {
        let t = Uuid::new_v4();
        let p = principal(Role::PropertyManager, TenantScope::single(t));
        let uid = p.user_id;
        let s = session_with(p);
        let got = require(&s, Permission::ApproveSettlement, &t).unwrap();
        assert_eq!(got.user_id, uid);
    }

    #[test]
    fn require_any_grants_on_first_match() {
        let t = Uuid::new_v4();
        let s = session_with(principal(Role::Liaison, TenantScope::single(t)));
        // Liaison has InputResidentData but not ApproveSettlement.
        assert!(require_any(
            &s,
            &[Permission::ApproveSettlement, Permission::InputResidentData],
            &t
        )
        .is_ok());
    }

    #[test]
    fn require_authenticated_ignores_permissions() {
        let t = Uuid::new_v4();
        let s = session_with(principal(Role::Reviewer, TenantScope::single(t)));
        assert!(require_authenticated(&s).is_ok());
    }

    #[test]
    fn structured_error_serializes_with_type_tag() {
        let err = IpcError::PermissionDenied {
            role: "liaison".into(),
            permission: "approve_settlement".into(),
        };
        let json = serde_json::to_string(&err).unwrap();
        // UI switches on the `type` tag — stability matters.
        assert!(json.contains(r#""type":"permission_denied""#));
        assert!(json.contains(r#""role":"liaison""#));
        assert!(json.contains(r#""permission":"approve_settlement""#));
    }

    #[test]
    fn clear_invalidates_session() {
        let t = Uuid::new_v4();
        let p = principal(Role::Administrator, TenantScope::Global);
        let s = session_with(p);
        assert!(require(&s, Permission::ManageUsers, &t).is_ok());
        s.clear();
        assert!(matches!(
            require(&s, Permission::ManageUsers, &t),
            Err(IpcError::Unauthenticated)
        ));
    }
}
