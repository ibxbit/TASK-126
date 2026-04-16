//! Configurable state-machine engine.
//!
//! The rule set is loaded from `parcel_transition_rules`. Each rule is
//! a `(from, to, guard_code?, required_permission?)` tuple. Guards are
//! referenced by string and resolved at registration time into
//! callable `GuardFn`s, so new guards are added by registering them —
//! NOT by editing the engine.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::parcel::state::{ParcelState, GENESIS};

/// A guard runs with read-only access to the parcel in question and
/// returns `Ok(())` if the transition is admissible, or a human
/// readable reason string otherwise.
pub type GuardFn = Arc<dyn Fn(&GuardContext) -> Result<(), String> + Send + Sync>;

/// Closed set of guard identifiers. Rules in the DB store a string
/// (`schedule_rules.guard_code` / `parcel_transition_rules.guard_code`);
/// loading code MUST parse into this enum so typos surface at boundary
/// parse time rather than at transition evaluation. Adding a variant
/// is a compile-time signal to register its function in
/// `guard_for` + `default_guards()` (exhaustive matches enforce it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardCode {
    /// A parcel may only advance to `Delivered` if a `CheckedIn`
    /// event has been recorded in its history.
    RequiresCheckInExists,
}

impl GuardCode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            GuardCode::RequiresCheckInExists => "requires_check_in_exists",
        }
    }

    /// Parse from the persisted string form. Returns `None` for any
    /// unknown code — callers SHOULD reject at load time rather than
    /// silently dropping the guard.
    pub fn parse(s: &str) -> Option<GuardCode> {
        match s {
            "requires_check_in_exists" => Some(GuardCode::RequiresCheckInExists),
            _ => None,
        }
    }
}

pub struct GuardContext {
    pub parcel_id: Uuid,
    pub tenant_id: Uuid,
    pub from: Option<ParcelState>,
    pub to: ParcelState,
    /// Whether this parcel has ever had a CheckedIn event recorded in
    /// `parcel_transitions`. Populated by the caller before invoking
    /// `StateMachine::apply`.
    pub has_check_in_record: bool,
}

#[derive(Debug, Clone)]
pub struct TransitionRule {
    pub rule_id: Uuid,
    pub from_state: String,
    pub to_state: ParcelState,
    pub guard_code: Option<GuardCode>,
    pub required_permission: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum StateMachineError {
    #[error("no rule permits transition {from} → {to}")]
    NoRule { from: String, to: String },

    #[error("transition {from} → {to} is disabled")]
    Disabled { from: String, to: String },

    #[error("guard '{guard}' rejected the transition: {reason}")]
    GuardRejected { guard: &'static str, reason: String },

    #[error("guard '{}' is not registered for this state machine", .0.as_str())]
    UnknownGuard(GuardCode),

    #[error("target state '{0}' is not a recognized ParcelState")]
    InvalidTargetState(String),
}

pub struct StateMachine {
    /// Index: (from_state_str) → Vec<rule>. Multiple rules per `from`
    /// when several transitions share a source.
    rules_by_from: HashMap<String, Vec<TransitionRule>>,
    guards: HashMap<GuardCode, GuardFn>,
}

impl StateMachine {
    /// Build the engine from persisted rules. Guards passed in
    /// `guards` must cover every `guard_code` referenced by an enabled
    /// rule; otherwise `apply` will fail with `UnknownGuard` when that
    /// rule is exercised.
    pub fn new(rules: Vec<TransitionRule>, guards: HashMap<GuardCode, GuardFn>) -> Self {
        let mut rules_by_from: HashMap<String, Vec<TransitionRule>> = HashMap::new();
        for r in rules {
            rules_by_from.entry(r.from_state.clone()).or_default().push(r);
        }
        Self { rules_by_from, guards }
    }

    /// Return the set of states reachable from `from` given the
    /// currently enabled rules (used by the UI to render only the
    /// buttons the user can actually click).
    pub fn available_from(&self, from: Option<ParcelState>) -> Vec<ParcelState> {
        let key = from.map(|s| s.as_str().to_string()).unwrap_or_else(|| GENESIS.to_string());
        self.rules_by_from
            .get(&key)
            .map(|rs| rs.iter().filter(|r| r.enabled).map(|r| r.to_state).collect())
            .unwrap_or_default()
    }

    /// Look up the required permission for a given edge. The calling
    /// command layer pairs this with `auth::guard::require` before
    /// invoking `apply`.
    pub fn required_permission(
        &self,
        from: Option<ParcelState>,
        to: ParcelState,
    ) -> Option<String> {
        self.find_rule(from, to)
            .and_then(|r| r.required_permission.clone())
    }

    /// Validate that `from → to` is admissible in this rule set,
    /// running any attached guard. Returns `Ok(())` if the transition
    /// may proceed; callers are then responsible for persisting the
    /// new state + appending a `TransitionRecord`.
    pub fn apply(
        &self,
        ctx: &GuardContext,
    ) -> Result<&TransitionRule, StateMachineError> {
        let rule = self.find_rule(ctx.from, ctx.to).ok_or_else(|| {
            StateMachineError::NoRule {
                from: ctx
                    .from
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| GENESIS.to_string()),
                to: ctx.to.as_str().to_string(),
            }
        })?;

        if !rule.enabled {
            return Err(StateMachineError::Disabled {
                from: rule.from_state.clone(),
                to: rule.to_state.as_str().to_string(),
            });
        }

        if let Some(code) = rule.guard_code {
            let guard = self
                .guards
                .get(&code)
                .ok_or(StateMachineError::UnknownGuard(code))?;
            guard(ctx).map_err(|reason| StateMachineError::GuardRejected {
                guard: code.as_str(),
                reason,
            })?;
        }
        Ok(rule)
    }

    fn find_rule(&self, from: Option<ParcelState>, to: ParcelState) -> Option<&TransitionRule> {
        let key = from.map(|s| s.as_str().to_string()).unwrap_or_else(|| GENESIS.to_string());
        self.rules_by_from
            .get(&key)?
            .iter()
            .find(|r| r.to_state == to)
    }
}

// ── Built-in guards ─────────────────────────────────────────────────────

/// Canonical guard: a parcel may only be marked Delivered if a
/// CheckedIn event exists in its history.
pub fn guard_requires_check_in_exists() -> GuardFn {
    Arc::new(|ctx: &GuardContext| {
        if ctx.has_check_in_record {
            Ok(())
        } else {
            Err(format!(
                "parcel {} cannot advance to {} without a prior check-in record",
                ctx.parcel_id,
                ctx.to.as_str()
            ))
        }
    })
}

/// Map each `GuardCode` variant to its implementation. The match is
/// intentionally exhaustive: if a new variant is added to `GuardCode`,
/// this function will fail to compile until the corresponding
/// function exists.
pub fn guard_for(code: GuardCode) -> GuardFn {
    match code {
        GuardCode::RequiresCheckInExists => guard_requires_check_in_exists(),
    }
}

/// The default transition rules shipped with the app. These encode
/// the standard parcel lifecycle: genesis → checked_in → checked_out
/// → delivered → receipt_confirmed, plus checked_in → returned_exception.
pub fn default_rules() -> Vec<TransitionRule> {
    vec![
        TransitionRule {
            rule_id: Uuid::new_v4(),
            from_state: GENESIS.to_string(),
            to_state: ParcelState::CheckedIn,
            guard_code: None,
            required_permission: Some("parcel_operate".into()),
            enabled: true,
        },
        TransitionRule {
            rule_id: Uuid::new_v4(),
            from_state: "checked_in".to_string(),
            to_state: ParcelState::CheckedOut,
            guard_code: None,
            required_permission: Some("parcel_operate".into()),
            enabled: true,
        },
        TransitionRule {
            rule_id: Uuid::new_v4(),
            from_state: "checked_out".to_string(),
            to_state: ParcelState::Delivered,
            guard_code: Some(GuardCode::RequiresCheckInExists),
            required_permission: Some("parcel_operate".into()),
            enabled: true,
        },
        TransitionRule {
            rule_id: Uuid::new_v4(),
            from_state: "delivered".to_string(),
            to_state: ParcelState::ReceiptConfirmed,
            guard_code: None,
            required_permission: Some("parcel_operate".into()),
            enabled: true,
        },
        TransitionRule {
            rule_id: Uuid::new_v4(),
            from_state: "checked_in".to_string(),
            to_state: ParcelState::ReturnedException,
            guard_code: None,
            required_permission: Some("parcel_operate".into()),
            enabled: true,
        },
    ]
}

/// Convenience: the default guard registry shipped with the app.
/// Built from the full list of `GuardCode` variants so any new
/// variant is registered automatically.
pub fn default_guards() -> HashMap<GuardCode, GuardFn> {
    const ALL: &[GuardCode] = &[GuardCode::RequiresCheckInExists];
    let mut m: HashMap<GuardCode, GuardFn> = HashMap::new();
    for code in ALL {
        m.insert(*code, guard_for(*code));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        from: &str,
        to: ParcelState,
        guard: Option<GuardCode>,
        enabled: bool,
    ) -> TransitionRule {
        TransitionRule {
            rule_id: Uuid::new_v4(),
            from_state: from.into(),
            to_state: to,
            guard_code: guard,
            required_permission: Some("parcel_operate".into()),
            enabled,
        }
    }

    fn ctx(from: Option<ParcelState>, to: ParcelState, has_check_in: bool) -> GuardContext {
        GuardContext {
            parcel_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            from,
            to,
            has_check_in_record: has_check_in,
        }
    }

    fn full_rules() -> Vec<TransitionRule> {
        vec![
            rule(GENESIS, ParcelState::CheckedIn, None, true),
            rule("checked_in", ParcelState::CheckedOut, None, true),
            rule(
                "checked_out",
                ParcelState::Delivered,
                Some(GuardCode::RequiresCheckInExists),
                true,
            ),
            rule("delivered", ParcelState::ReceiptConfirmed, None, true),
            rule("checked_in", ParcelState::ReturnedException, None, true),
        ]
    }

    #[test]
    fn genesis_to_checked_in_ok() {
        let sm = StateMachine::new(full_rules(), default_guards());
        assert!(sm.apply(&ctx(None, ParcelState::CheckedIn, false)).is_ok());
    }

    #[test]
    fn delivered_requires_check_in_history() {
        let sm = StateMachine::new(full_rules(), default_guards());
        let err = sm
            .apply(&ctx(Some(ParcelState::CheckedOut), ParcelState::Delivered, false))
            .unwrap_err();
        assert!(matches!(err, StateMachineError::GuardRejected { .. }));
    }

    #[test]
    fn delivered_with_check_in_ok() {
        let sm = StateMachine::new(full_rules(), default_guards());
        assert!(sm
            .apply(&ctx(Some(ParcelState::CheckedOut), ParcelState::Delivered, true))
            .is_ok());
    }

    #[test]
    fn illegal_edge_rejected() {
        let sm = StateMachine::new(full_rules(), default_guards());
        let err = sm
            .apply(&ctx(Some(ParcelState::CheckedIn), ParcelState::ReceiptConfirmed, true))
            .unwrap_err();
        assert!(matches!(err, StateMachineError::NoRule { .. }));
    }

    #[test]
    fn disabled_rule_rejected() {
        let mut rules = full_rules();
        rules[1].enabled = false; // checked_in → checked_out
        let sm = StateMachine::new(rules, default_guards());
        let err = sm
            .apply(&ctx(Some(ParcelState::CheckedIn), ParcelState::CheckedOut, true))
            .unwrap_err();
        assert!(matches!(err, StateMachineError::Disabled { .. }));
    }

    #[test]
    fn available_from_lists_only_enabled() {
        let mut rules = full_rules();
        rules[4].enabled = false; // checked_in → returned_exception
        let sm = StateMachine::new(rules, default_guards());
        let avail = sm.available_from(Some(ParcelState::CheckedIn));
        assert_eq!(avail, vec![ParcelState::CheckedOut]);
    }

    #[test]
    fn unknown_guard_surfaces_clearly() {
        // A rule references a valid GuardCode, but the registry is
        // empty — the only remaining way to reach UnknownGuard once
        // guard identifiers are typed. Typos in the rule's identifier
        // are impossible: they don't compile.
        let rules = vec![rule(
            "checked_in",
            ParcelState::Delivered,
            Some(GuardCode::RequiresCheckInExists),
            true,
        )];
        let sm = StateMachine::new(rules, HashMap::new());
        let err = sm
            .apply(&ctx(Some(ParcelState::CheckedIn), ParcelState::Delivered, true))
            .unwrap_err();
        assert!(matches!(
            err,
            StateMachineError::UnknownGuard(GuardCode::RequiresCheckInExists)
        ));
    }

    #[test]
    fn guard_code_parses_known_and_rejects_unknown() {
        assert_eq!(
            GuardCode::parse("requires_check_in_exists"),
            Some(GuardCode::RequiresCheckInExists)
        );
        assert_eq!(GuardCode::parse("bogus"), None);
        assert_eq!(
            GuardCode::RequiresCheckInExists.as_str(),
            "requires_check_in_exists"
        );
    }
}
