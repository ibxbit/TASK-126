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
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_scope_allows_any_tenant() {
        let s = TenantScope::Global;
        assert!(s.allows(&Uuid::new_v4()));
        assert!(s.allows(&Uuid::nil()));
    }

    #[test]
    fn single_tenant_scope_allows_only_that_tenant() {
        let t = Uuid::new_v4();
        let other = Uuid::new_v4();
        let s = TenantScope::single(t);
        assert!(s.allows(&t));
        assert!(!s.allows(&other));
    }

    #[test]
    fn multi_tenant_scope_allows_each_listed_id() {
        let t1 = Uuid::new_v4();
        let t2 = Uuid::new_v4();
        let foreign = Uuid::new_v4();
        let s = TenantScope::Tenants(vec![t1, t2]);
        assert!(s.allows(&t1));
        assert!(s.allows(&t2));
        assert!(!s.allows(&foreign));
    }

    #[test]
    fn empty_tenants_scope_denies_everything() {
        let s = TenantScope::Tenants(vec![]);
        assert!(!s.allows(&Uuid::new_v4()));
    }

    #[test]
    fn tenant_scope_round_trips_through_serde() {
        let t = Uuid::new_v4();
        let s = TenantScope::Tenants(vec![t]);
        let json = serde_json::to_string(&s).unwrap();
        // tag = "kind", snake_case
        assert!(json.contains(r#""kind":"tenants""#), "got: {json}");
        let back: TenantScope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);

        let g = TenantScope::Global;
        let json = serde_json::to_string(&g).unwrap();
        assert!(json.contains(r#""kind":"global""#), "got: {json}");
        let back: TenantScope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn principal_new_assigns_a_fresh_session_id_each_time() {
        let uid = Uuid::new_v4();
        let p1 = Principal::new(uid, "u".into(), Role::Staff, TenantScope::Global);
        let p2 = Principal::new(uid, "u".into(), Role::Staff, TenantScope::Global);
        assert_ne!(p1.session_id, p2.session_id);
        assert_eq!(p1.user_id, p2.user_id);
    }
}
