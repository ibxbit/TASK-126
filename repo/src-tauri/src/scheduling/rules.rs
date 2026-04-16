//! Rule definitions, typed specs, and rule-set versioning.
//!
//! Each rule is one of a small closed set of `RuleKind`s; its
//! parameters live in the typed `RuleSpec` enum which serializes to
//! the `spec_json` column. Rule sets carry a monotonically-increasing
//! `version`; activating a new version atomically deactivates the
//! previously-active version for the same `(tenant, name)` pair —
//! enforced by a partial unique index in the schema.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{self, AuthError, Permission, Principal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    UnavailableWindow,
    CapacityLimit,
    RequiredDuration,
    Distribution,
}

impl RuleKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuleKind::UnavailableWindow => "unavailable_window",
            RuleKind::CapacityLimit => "capacity_limit",
            RuleKind::RequiredDuration => "required_duration",
            RuleKind::Distribution => "distribution",
        }
    }
}

/// Half-open interval [start, end) in Unix seconds (UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeWindow {
    pub start_unix: i64,
    pub end_unix: i64,
}

impl TimeWindow {
    pub fn duration_seconds(&self) -> i64 {
        (self.end_unix - self.start_unix).max(0)
    }
    pub fn overlaps(&self, other: &TimeWindow) -> bool {
        self.start_unix < other.end_unix && other.start_unix < self.end_unix
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionMode {
    /// Sessions for the same subject must be back-to-back: no gap
    /// larger than `max_gap_seconds` between consecutive bookings.
    Consecutive,
    /// Sessions must be spread out: at least `min_gap_seconds`
    /// between any two bookings for the same subject.
    Distributed,
}

/// Typed parameters per rule kind. Persisted as JSON in `spec_json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleSpec {
    UnavailableWindow {
        /// Optional resource scope; None ⇒ applies to all resources.
        resource_id: Option<Uuid>,
        windows: Vec<TimeWindow>,
    },
    CapacityLimit {
        resource_id: Uuid,
        /// Max concurrent assignments allowed at any instant.
        max_concurrent: u32,
    },
    RequiredDuration {
        /// Inclusive bounds. `min_seconds = max_seconds` enforces an
        /// exact duration.
        min_seconds: i64,
        max_seconds: i64,
    },
    Distribution {
        mode: DistributionMode,
        /// Threshold in seconds. For Consecutive: max permissible gap.
        /// For Distributed: min permissible gap.
        gap_seconds: i64,
    },
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: Uuid,
    pub rule_set_id: Uuid,
    pub kind: RuleKind,
    pub severity: super::constraints::Severity,
    pub spec: RuleSpec,
    pub weight: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct RuleSet {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub version: u32,
    pub parent_rule_set_id: Option<Uuid>,
    pub enabled: bool,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum RuleSetError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("rule set not found: {0}")]
    NotFound(String),

    #[error("rule set is already active")]
    AlreadyActive,

    #[error("persistence error: {0}")]
    Persistence(String),
}

pub trait RuleRepository {
    /// Load the currently active (enabled = 1) rule set for
    /// (tenant, name), with all its enabled rules attached.
    fn load_active(
        &self,
        tenant_id: &Uuid,
        name: &str,
    ) -> Result<Option<RuleSet>, String>;

    fn load_by_id(&self, rule_set_id: &Uuid) -> Result<Option<RuleSet>, String>;

    /// Atomically deactivate the previously enabled version (if any)
    /// for (tenant, name) and enable `new_rule_set_id`. The schema's
    /// partial unique index makes this a single transactional swap.
    fn activate(&self, new_rule_set_id: &Uuid) -> Result<(), String>;

    fn deactivate_all(&self, tenant_id: &Uuid, name: &str) -> Result<(), String>;
}

/// Activate a rule-set version. Caller must hold `ConfigureRules` in
/// the rule set's tenant scope.
pub fn activate_version<R: RuleRepository>(
    repo: &R,
    principal: &Principal,
    rule_set_id: Uuid,
) -> Result<(), RuleSetError> {
    let rs = repo
        .load_by_id(&rule_set_id)
        .map_err(RuleSetError::Persistence)?
        .ok_or_else(|| RuleSetError::NotFound(rule_set_id.to_string()))?;

    auth::require(principal, Permission::ConfigureRules, &rs.tenant_id)?;

    if rs.enabled {
        return Err(RuleSetError::AlreadyActive);
    }
    repo.activate(&rule_set_id).map_err(RuleSetError::Persistence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_window_overlap_is_half_open() {
        let a = TimeWindow { start_unix: 0, end_unix: 100 };
        let b = TimeWindow { start_unix: 100, end_unix: 200 };
        // [0,100) and [100,200) touch but do not overlap.
        assert!(!a.overlaps(&b));
        let c = TimeWindow { start_unix: 50, end_unix: 150 };
        assert!(a.overlaps(&c));
        assert!(c.overlaps(&a));
    }

    #[test]
    fn rule_spec_round_trips_json() {
        let s = RuleSpec::Distribution {
            mode: DistributionMode::Consecutive,
            gap_seconds: 600,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: RuleSpec = serde_json::from_str(&json).unwrap();
        match back {
            RuleSpec::Distribution { mode, gap_seconds } => {
                assert_eq!(mode, DistributionMode::Consecutive);
                assert_eq!(gap_seconds, 600);
            }
            _ => panic!("wrong variant"),
        }
    }
}
