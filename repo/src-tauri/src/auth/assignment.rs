//! Role assignment, revocation, and validation.
//!
//! Persistence is delegated to a `UserRoleRepository` trait so this
//! module stays pure and unit-testable. The SQLite implementation of
//! that trait lives in `repositories/user_role_repo.rs` (future step).

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::context::{Principal, TenantScope};
use crate::auth::guard::{require, AuthError};
use crate::auth::permissions::Permission;
use crate::auth::roles::Role;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssignmentError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("target user does not exist: {0}")]
    UserNotFound(String),

    #[error("tenant does not exist: {0}")]
    TenantNotFound(String),

    #[error("assignment would leave system with no administrator")]
    LastAdministrator,

    #[error("a global scope may only be granted to administrators")]
    InvalidGlobalScope,

    #[error("persistence error: {0}")]
    Persistence(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub user_id: Uuid,
    pub role: Role,
    pub scope: TenantScope,
}

/// Abstraction over storage — implemented by the SQLite repo layer.
pub trait UserRoleRepository {
    fn user_exists(&self, user_id: &Uuid) -> Result<bool, String>;
    fn tenant_exists(&self, tenant_id: &Uuid) -> Result<bool, String>;
    fn count_administrators(&self) -> Result<usize, String>;
    fn is_administrator(&self, user_id: &Uuid) -> Result<bool, String>;
    fn upsert_assignment(&self, assignment: &RoleAssignment) -> Result<(), String>;
    fn delete_assignment(&self, user_id: &Uuid) -> Result<(), String>;
}

/// Validate an assignment before persisting. Pure function — no I/O
/// beyond the repo lookups it is handed.
pub fn validate_assignment<R: UserRoleRepository>(
    repo: &R,
    assignment: &RoleAssignment,
) -> Result<(), AssignmentError> {
    if !repo
        .user_exists(&assignment.user_id)
        .map_err(AssignmentError::Persistence)?
    {
        return Err(AssignmentError::UserNotFound(assignment.user_id.to_string()));
    }

    // Global scope is only valid for Administrator.
    if matches!(assignment.scope, TenantScope::Global)
        && assignment.role != Role::Administrator
    {
        return Err(AssignmentError::InvalidGlobalScope);
    }

    // Every listed tenant must exist.
    if let TenantScope::Tenants(ids) = &assignment.scope {
        for t in ids {
            if !repo
                .tenant_exists(t)
                .map_err(AssignmentError::Persistence)?
            {
                return Err(AssignmentError::TenantNotFound(t.to_string()));
            }
        }
    }

    Ok(())
}

/// Assign (or replace) a role for a user. Caller must hold
/// `Permission::ManageUsers` in the target tenant scope.
pub fn assign_role<R: UserRoleRepository>(
    repo: &R,
    actor: &Principal,
    assignment: RoleAssignment,
) -> Result<(), AssignmentError> {
    // Authorization: actor must have ManageUsers in every tenant the
    // assignment touches. Global scope requires an actor with global
    // scope (only Administrators can have that — enforced elsewhere).
    match &assignment.scope {
        TenantScope::Global => {
            if !matches!(actor.scope, TenantScope::Global) {
                return Err(AssignmentError::Auth(AuthError::TenantScopeViolation {
                    tenant_id: "global".to_string(),
                }));
            }
            // Any tenant id works for the perm check under Global actor.
            require(actor, Permission::ManageUsers, &Uuid::nil())?;
        }
        TenantScope::Tenants(ids) => {
            for t in ids {
                require(actor, Permission::ManageUsers, t)?;
            }
        }
    }

    validate_assignment(repo, &assignment)?;

    repo.upsert_assignment(&assignment)
        .map_err(AssignmentError::Persistence)?;
    Ok(())
}

/// Revoke a user's role. Blocks removal of the last administrator.
pub fn revoke_role<R: UserRoleRepository>(
    repo: &R,
    actor: &Principal,
    target_user_id: &Uuid,
) -> Result<(), AssignmentError> {
    // Only global admins may revoke arbitrary users; tenant-scoped
    // managers must go through `assign_role` to downgrade.
    if !matches!(actor.scope, TenantScope::Global) {
        return Err(AssignmentError::Auth(AuthError::TenantScopeViolation {
            tenant_id: "global".to_string(),
        }));
    }
    require(actor, Permission::ManageUsers, &Uuid::nil())?;

    let target_is_admin = repo
        .is_administrator(target_user_id)
        .map_err(AssignmentError::Persistence)?;
    if target_is_admin {
        let admins = repo
            .count_administrators()
            .map_err(AssignmentError::Persistence)?;
        if admins <= 1 {
            return Err(AssignmentError::LastAdministrator);
        }
    }

    repo.delete_assignment(target_user_id)
        .map_err(AssignmentError::Persistence)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashSet;

    struct MockRepo {
        users: HashSet<Uuid>,
        tenants: HashSet<Uuid>,
        admins: RefCell<HashSet<Uuid>>,
        assignments: RefCell<Vec<RoleAssignment>>,
    }

    impl UserRoleRepository for MockRepo {
        fn user_exists(&self, u: &Uuid) -> Result<bool, String> {
            Ok(self.users.contains(u))
        }
        fn tenant_exists(&self, t: &Uuid) -> Result<bool, String> {
            Ok(self.tenants.contains(t))
        }
        fn count_administrators(&self) -> Result<usize, String> {
            Ok(self.admins.borrow().len())
        }
        fn is_administrator(&self, u: &Uuid) -> Result<bool, String> {
            Ok(self.admins.borrow().contains(u))
        }
        fn upsert_assignment(&self, a: &RoleAssignment) -> Result<(), String> {
            if a.role == Role::Administrator {
                self.admins.borrow_mut().insert(a.user_id);
            } else {
                self.admins.borrow_mut().remove(&a.user_id);
            }
            self.assignments.borrow_mut().push(a.clone());
            Ok(())
        }
        fn delete_assignment(&self, u: &Uuid) -> Result<(), String> {
            self.admins.borrow_mut().remove(u);
            self.assignments.borrow_mut().retain(|a| &a.user_id != u);
            Ok(())
        }
    }

    fn admin_principal() -> Principal {
        Principal::new(
            Uuid::new_v4(),
            "root".into(),
            Role::Administrator,
            TenantScope::Global,
        )
    }

    #[test]
    fn global_scope_rejected_for_non_admin() {
        let repo = MockRepo {
            users: [Uuid::new_v4()].into_iter().collect(),
            tenants: HashSet::new(),
            admins: RefCell::new(HashSet::new()),
            assignments: RefCell::new(vec![]),
        };
        let uid = *repo.users.iter().next().unwrap();
        let err = validate_assignment(
            &repo,
            &RoleAssignment {
                user_id: uid,
                role: Role::Staff,
                scope: TenantScope::Global,
            },
        )
        .unwrap_err();
        assert!(matches!(err, AssignmentError::InvalidGlobalScope));
    }

    #[test]
    fn cannot_revoke_last_admin() {
        let actor = admin_principal();
        let target = Uuid::new_v4();
        let repo = MockRepo {
            users: [target].into_iter().collect(),
            tenants: HashSet::new(),
            admins: RefCell::new([target].into_iter().collect()),
            assignments: RefCell::new(vec![]),
        };
        let err = revoke_role(&repo, &actor, &target).unwrap_err();
        assert!(matches!(err, AssignmentError::LastAdministrator));
    }

    #[test]
    fn property_manager_cannot_manage_users() {
        let tenant = Uuid::new_v4();
        let target = Uuid::new_v4();
        let actor = Principal::new(
            Uuid::new_v4(),
            "pm".into(),
            Role::PropertyManager,
            TenantScope::single(tenant),
        );
        let repo = MockRepo {
            users: [target].into_iter().collect(),
            tenants: [tenant].into_iter().collect(),
            admins: RefCell::new(HashSet::new()),
            assignments: RefCell::new(vec![]),
        };
        let err = assign_role(
            &repo,
            &actor,
            RoleAssignment {
                user_id: target,
                role: Role::Staff,
                scope: TenantScope::single(tenant),
            },
        )
        .unwrap_err();
        assert!(matches!(err, AssignmentError::Auth(_)));
    }

    #[test]
    fn admin_assigns_staff_to_tenant() {
        let actor = admin_principal();
        let tenant = Uuid::new_v4();
        let target = Uuid::new_v4();
        let repo = MockRepo {
            users: [target].into_iter().collect(),
            tenants: [tenant].into_iter().collect(),
            admins: RefCell::new([actor.user_id].into_iter().collect()),
            assignments: RefCell::new(vec![]),
        };
        assign_role(
            &repo,
            &actor,
            RoleAssignment {
                user_id: target,
                role: Role::Staff,
                scope: TenantScope::single(tenant),
            },
        )
        .unwrap();
        assert_eq!(repo.assignments.borrow().len(), 1);
    }
}
