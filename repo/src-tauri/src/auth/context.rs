//! The authenticated caller and their tenant scope.
//!
//! A `Principal` is constructed by the session layer AFTER local login
//! succeeds, and is threaded into every command via Tauri state. It is
//! never built from untrusted UI input.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::roles::Role;

/// Which tenant(s) a principal is allowed to act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TenantScope {
    /// Unrestricted — reserved for global administrators.
    Global,
    /// Restricted to one or more tenant IDs.
    Tenants(Vec<Uuid>),
}

impl TenantScope {
    pub fn allows(&self, tenant_id: &Uuid) -> bool {
        match self {
            TenantScope::Global => true,
            TenantScope::Tenants(ids) => ids.iter().any(|id| id == tenant_id),
        }
    }

    pub fn single(tenant_id: Uuid) -> Self {
        TenantScope::Tenants(vec![tenant_id])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub user_id: Uuid,
    pub username: String,
    pub role: Role,
    pub scope: TenantScope,
    /// Monotonic session id, used for audit correlation.
    pub session_id: Uuid,
}

impl Principal {
    pub fn new(user_id: Uuid, username: String, role: Role, scope: TenantScope) -> Self {
        Self {
            user_id,
            username,
            role,
            scope,
            session_id: Uuid::new_v4(),
        }
    }
}
