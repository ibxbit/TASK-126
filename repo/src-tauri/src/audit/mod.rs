//! Centralized append-only audit log — core model.
//!
//! This module defines the `AuditLog` record and a small constructor
//! used by any service that needs to emit a structured audit entry.
//! The `writer` submodule defines the transactional write contract
//! services call into.

pub mod writer;

pub use writer::{audit_role_for, role_to_audit, AuditWriter, NoopAuditWriter};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

/// Role of the actor at the time the action occurred. Mirrors
/// `auth::Role` plus a `System` variant for scheduler / recovery
/// initiated events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditRole {
    Administrator,
    PropertyManager,
    Staff,
    Reviewer,
    Liaison,
    System,
}

impl AuditRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditRole::Administrator => "administrator",
            AuditRole::PropertyManager => "property_manager",
            AuditRole::Staff => "staff",
            AuditRole::Reviewer => "reviewer",
            AuditRole::Liaison => "liaison",
            AuditRole::System => "system",
        }
    }
}

/// One row in `audit_logs`.
///
/// - `before_state` is `None` for creation events.
/// - `after_state`  is `None` for deletion events.
/// - `metadata` is always present (defaults to an empty JSON object)
///   and carries non-load-bearing context: session id, ip host,
///   correlation id, etc. — never PII that is not already in
///   `before_state` / `after_state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: Uuid,
    pub timestamp_unix: i64,
    pub user_id: Uuid,
    pub role: AuditRole,
    pub tenant_id: Option<Uuid>,
    pub action_type: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub before_state: Option<JsonValue>,
    pub after_state: Option<JsonValue>,
    pub metadata: JsonValue,
}

/// Constructor inputs for a new audit entry. The fields the caller
/// always knows — id and timestamp are filled in by `AuditLog::new`.
#[derive(Debug, Clone)]
pub struct NewAuditLog {
    pub user_id: Uuid,
    pub role: AuditRole,
    pub tenant_id: Option<Uuid>,
    pub action_type: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub before_state: Option<JsonValue>,
    pub after_state: Option<JsonValue>,
    pub metadata: Option<JsonValue>,
}

impl AuditLog {
    /// Build an `AuditLog` with a fresh UUID and the supplied
    /// timestamp. Callers pass `now_unix` explicitly so the function
    /// stays pure and deterministic in tests.
    pub fn new(input: NewAuditLog, now_unix: i64) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp_unix: now_unix,
            user_id: input.user_id,
            role: input.role,
            tenant_id: input.tenant_id,
            action_type: input.action_type,
            entity_type: input.entity_type,
            entity_id: input.entity_id,
            before_state: input.before_state,
            after_state: input.after_state,
            metadata: input
                .metadata
                .unwrap_or_else(|| JsonValue::Object(serde_json::Map::new())),
        }
    }
}
