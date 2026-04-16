//! Permission guard — the single enforcement point for the backend.
//!
//! Every command handler performing a scoped action MUST call
//! `require(&principal, permission, &tenant_id)` (or `require_any`)
//! BEFORE touching a repository. Repositories additionally re-check
//! `tenant_id` at query-time as defense in depth.

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::auth::context::Principal;
use crate::auth::permissions::Permission;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum AuthError {
    #[error("permission denied: role '{role}' lacks '{permission}'")]
    PermissionDenied { role: String, permission: String },

    #[error("tenant scope violation: principal cannot access tenant '{tenant_id}'")]
    TenantScopeViolation { tenant_id: String },
}

/// Require a single permission within a tenant scope.
pub fn require(
    principal: &Principal,
    permission: Permission,
    tenant_id: &Uuid,
) -> Result<(), AuthError> {
    if !principal.role.has(permission) {
        return Err(AuthError::PermissionDenied {
            role: principal.role.as_str().to_string(),
            permission: permission.as_str().to_string(),
        });
    }
    if !principal.scope.allows(tenant_id) {
        return Err(AuthError::TenantScopeViolation {
            tenant_id: tenant_id.to_string(),
        });
    }
    Ok(())
}

/// Require at least one of the listed permissions (OR semantics) within
/// a tenant scope. Useful for endpoints serving multiple roles.
pub fn require_any(
    principal: &Principal,
    permissions: &[Permission],
    tenant_id: &Uuid,
) -> Result<(), AuthError> {
    if !principal.scope.allows(tenant_id) {
        return Err(AuthError::TenantScopeViolation {
            tenant_id: tenant_id.to_string(),
        });
    }
    if permissions.iter().any(|p| principal.role.has(*p)) {
        Ok(())
    } else {
        Err(AuthError::PermissionDenied {
            role: principal.role.as_str().to_string(),
            permission: permissions
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join("|"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::context::{Principal, TenantScope};
    use crate::auth::roles::Role;

    fn p(role: Role, scope: TenantScope) -> Principal {
        Principal::new(Uuid::new_v4(), "u".into(), role, scope)
    }

    #[test]
    fn admin_allowed_globally() {
        let pr = p(Role::Administrator, TenantScope::Global);
        assert!(require(&pr, Permission::ManageUsers, &Uuid::new_v4()).is_ok());
    }

    #[test]
    fn liaison_cannot_approve_settlement() {
        let t = Uuid::new_v4();
        let pr = p(Role::Liaison, TenantScope::single(t));
        assert!(matches!(
            require(&pr, Permission::ApproveSettlement, &t),
            Err(AuthError::PermissionDenied { .. })
        ));
    }

    #[test]
    fn reviewer_cannot_mutate_but_can_export() {
        let t = Uuid::new_v4();
        let pr = p(Role::Reviewer, TenantScope::single(t));
        assert!(require(&pr, Permission::ExportReport, &t).is_ok());
        assert!(require(&pr, Permission::ParcelOperate, &t).is_err());
    }

    #[test]
    fn tenant_scope_is_enforced() {
        let allowed = Uuid::new_v4();
        let foreign = Uuid::new_v4();
        let pr = p(Role::PropertyManager, TenantScope::single(allowed));
        assert!(require(&pr, Permission::ApproveSettlement, &allowed).is_ok());
        assert!(matches!(
            require(&pr, Permission::ApproveSettlement, &foreign),
            Err(AuthError::TenantScopeViolation { .. })
        ));
    }

    #[test]
    fn staff_parcel_ops_scoped() {
        let t = Uuid::new_v4();
        let pr = p(Role::Staff, TenantScope::single(t));
        assert!(require(&pr, Permission::ParcelOperate, &t).is_ok());
        assert!(require(&pr, Permission::ConfigureRules, &t).is_err());
    }

    #[test]
    fn require_any_matches_when_one_granted() {
        let t = Uuid::new_v4();
        let pr = p(Role::Liaison, TenantScope::single(t));
        assert!(require_any(
            &pr,
            &[Permission::ApproveSettlement, Permission::InputResidentData],
            &t
        )
        .is_ok());
    }
}
