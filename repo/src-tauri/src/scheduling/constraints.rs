//! Constraint validator.
//!
//! `validate(rule_set, candidate, existing)` returns a
//! `ConstraintReport` summarizing every hard violation, every soft
//! violation, and a numeric `soft_score` (0 = perfect; higher = worse).
//! Callers use the report to either reject (`hard_violations` non-empty)
//! or rank a set of candidates.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use crate::scheduling::rules::TimeWindow;
use crate::scheduling::rules::{DistributionMode, Rule, RuleSet, RuleSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Hard,
    Soft,
}

/// A schedulable assignment under evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assignment {
    pub resource_id: Uuid,
    /// Identifies "the same booking subject" for distribution rules
    /// (e.g., a resident being inspected, a staff member's shift
    /// chain). None ⇒ subject-based rules don't apply.
    pub subject_id: Option<Uuid>,
    pub window: TimeWindow,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViolationDetail {
    pub rule_id: Uuid,
    pub rule_kind: &'static str,
    pub severity: Severity,
    pub message: String,
    /// Soft rules only: weight applied when accumulating `soft_score`.
    pub weight: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ConstraintReport {
    pub hard_violations: Vec<ViolationDetail>,
    pub soft_violations: Vec<ViolationDetail>,
    /// Sum of weights of soft violations. 0 means a perfectly clean fit.
    pub soft_score: u32,
}

impl ConstraintReport {
    pub fn passed_hard(&self) -> bool {
        self.hard_violations.is_empty()
    }
}

/// Evaluate `candidate` against every enabled rule in `rule_set`,
/// considering `existing` assignments for capacity / distribution
/// checks. Returns a complete report.
pub fn validate(
    rule_set: &RuleSet,
    candidate: &Assignment,
    existing: &[Assignment],
) -> ConstraintReport {
    let mut report = ConstraintReport::default();

    for rule in rule_set.rules.iter().filter(|r| r.enabled) {
        if let Some(detail) = check_rule(rule, candidate, existing) {
            match detail.severity {
                Severity::Hard => report.hard_violations.push(detail),
                Severity::Soft => {
                    report.soft_score = report.soft_score.saturating_add(detail.weight);
                    report.soft_violations.push(detail);
                }
            }
        }
    }
    report
}

fn check_rule(
    rule: &Rule,
    cand: &Assignment,
    existing: &[Assignment],
) -> Option<ViolationDetail> {
    let violated = match &rule.spec {
        RuleSpec::UnavailableWindow { resource_id, windows } => {
            let scoped = resource_id.map_or(true, |r| r == cand.resource_id);
            if !scoped {
                None
            } else {
                let hit = windows.iter().any(|w| w.overlaps(&cand.window));
                if hit { Some("candidate overlaps an unavailable window".to_string()) } else { None }
            }
        }

        RuleSpec::CapacityLimit { resource_id, max_concurrent } => {
            if *resource_id != cand.resource_id {
                None
            } else {
                // Concurrency is a peak count over the candidate's window
                // among existing assignments on the same resource.
                let peak = peak_concurrency(*resource_id, &cand.window, existing) + 1;
                if peak > *max_concurrent {
                    Some(format!(
                        "capacity exceeded: {} concurrent (max {})",
                        peak, max_concurrent
                    ))
                } else {
                    None
                }
            }
        }

        RuleSpec::RequiredDuration { min_seconds, max_seconds } => {
            let dur = cand.window.duration_seconds();
            if dur < *min_seconds {
                Some(format!("duration {dur}s below minimum {min_seconds}s"))
            } else if dur > *max_seconds {
                Some(format!("duration {dur}s above maximum {max_seconds}s"))
            } else {
                None
            }
        }

        RuleSpec::Distribution { mode, gap_seconds } => {
            let Some(subject) = cand.subject_id else { return None };
            let mut others: Vec<&Assignment> = existing
                .iter()
                .filter(|a| a.subject_id == Some(subject))
                .collect();
            // Include a "what-if" by also considering the candidate.
            // We compare candidate to each existing booking.
            others.sort_by_key(|a| a.window.start_unix);

            let nearest_gap = others
                .iter()
                .map(|o| gap_between(&o.window, &cand.window))
                .min();
            match (mode, nearest_gap) {
                (DistributionMode::Consecutive, Some(g)) if g > *gap_seconds => Some(format!(
                    "consecutive sessions: gap {g}s exceeds max {gap_seconds}s"
                )),
                (DistributionMode::Distributed, Some(g)) if g < *gap_seconds => Some(format!(
                    "distributed sessions: gap {g}s below min {gap_seconds}s"
                )),
                _ => None,
            }
        }
    };

    violated.map(|message| ViolationDetail {
        rule_id: rule.id,
        rule_kind: rule.kind.as_str(),
        severity: rule.severity,
        message,
        weight: rule.weight,
    })
}

fn gap_between(a: &TimeWindow, b: &TimeWindow) -> i64 {
    if a.overlaps(b) {
        0
    } else if a.end_unix <= b.start_unix {
        b.start_unix - a.end_unix
    } else {
        a.start_unix - b.end_unix
    }
}

fn peak_concurrency(
    resource_id: Uuid,
    window: &TimeWindow,
    existing: &[Assignment],
) -> u32 {
    // Sweep events at the boundaries of overlapping intervals.
    let mut events: Vec<(i64, i32)> = Vec::new();
    for a in existing.iter().filter(|a| a.resource_id == resource_id) {
        if a.window.overlaps(window) {
            events.push((a.window.start_unix.max(window.start_unix), 1));
            events.push((a.window.end_unix.min(window.end_unix), -1));
        }
    }
    events.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.cmp(&y.1)));

    let mut current: i32 = 0;
    let mut peak: i32 = 0;
    for (_, delta) in events {
        current += delta;
        if current > peak {
            peak = current;
        }
    }
    peak.max(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduling::rules::{DistributionMode, RuleKind};

    fn rs(rules: Vec<Rule>) -> RuleSet {
        RuleSet {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            name: "test".into(),
            version: 1,
            parent_rule_set_id: None,
            enabled: true,
            rules,
        }
    }

    fn rule(kind: RuleKind, severity: Severity, spec: RuleSpec, weight: u32) -> Rule {
        Rule {
            id: Uuid::new_v4(),
            rule_set_id: Uuid::new_v4(),
            kind,
            severity,
            spec,
            weight,
            enabled: true,
        }
    }

    fn assign(resource: Uuid, subject: Option<Uuid>, s: i64, e: i64) -> Assignment {
        Assignment {
            resource_id: resource,
            subject_id: subject,
            window: TimeWindow { start_unix: s, end_unix: e },
        }
    }

    #[test]
    fn unavailable_window_blocks_overlapping_candidate() {
        let r = Uuid::new_v4();
        let set = rs(vec![rule(
            RuleKind::UnavailableWindow,
            Severity::Hard,
            RuleSpec::UnavailableWindow {
                resource_id: Some(r),
                windows: vec![TimeWindow { start_unix: 100, end_unix: 200 }],
            },
            1,
        )]);
        let cand = assign(r, None, 150, 250);
        let report = validate(&set, &cand, &[]);
        assert!(!report.passed_hard());
    }

    #[test]
    fn capacity_caps_concurrent_bookings() {
        let r = Uuid::new_v4();
        let set = rs(vec![rule(
            RuleKind::CapacityLimit,
            Severity::Hard,
            RuleSpec::CapacityLimit {
                resource_id: r,
                max_concurrent: 1,
            },
            1,
        )]);
        let existing = vec![assign(r, None, 0, 100)];
        // Overlapping candidate violates max_concurrent=1.
        let cand_bad = assign(r, None, 50, 150);
        assert!(!validate(&set, &cand_bad, &existing).passed_hard());
        // Adjacent (non-overlapping) candidate passes — half-open.
        let cand_ok = assign(r, None, 100, 200);
        assert!(validate(&set, &cand_ok, &existing).passed_hard());
    }

    #[test]
    fn duration_bounds_inclusive() {
        let r = Uuid::new_v4();
        let set = rs(vec![rule(
            RuleKind::RequiredDuration,
            Severity::Hard,
            RuleSpec::RequiredDuration { min_seconds: 1800, max_seconds: 3600 },
            1,
        )]);
        assert!(validate(&set, &assign(r, None, 0, 1800), &[]).passed_hard());
        assert!(validate(&set, &assign(r, None, 0, 3600), &[]).passed_hard());
        assert!(!validate(&set, &assign(r, None, 0, 1799), &[]).passed_hard());
        assert!(!validate(&set, &assign(r, None, 0, 3601), &[]).passed_hard());
    }

    #[test]
    fn consecutive_distribution_caps_gap() {
        let r = Uuid::new_v4();
        let s = Uuid::new_v4();
        let set = rs(vec![rule(
            RuleKind::Distribution,
            Severity::Hard,
            RuleSpec::Distribution { mode: DistributionMode::Consecutive, gap_seconds: 600 },
            1,
        )]);
        let existing = vec![assign(r, Some(s), 0, 1000)];
        // Gap of 500s — under cap, OK.
        assert!(validate(&set, &assign(r, Some(s), 1500, 2500), &existing).passed_hard());
        // Gap of 1000s — over 600s cap, hard fail.
        assert!(!validate(&set, &assign(r, Some(s), 2000, 3000), &existing).passed_hard());
    }

    #[test]
    fn distributed_distribution_requires_min_gap() {
        let r = Uuid::new_v4();
        let s = Uuid::new_v4();
        let set = rs(vec![rule(
            RuleKind::Distribution,
            Severity::Hard,
            RuleSpec::Distribution { mode: DistributionMode::Distributed, gap_seconds: 3600 },
            1,
        )]);
        let existing = vec![assign(r, Some(s), 0, 1800)];
        // 30 minutes after — too close.
        assert!(!validate(&set, &assign(r, Some(s), 3600, 5400), &existing).passed_hard());
        // 2 hours after — satisfies min gap.
        assert!(validate(&set, &assign(r, Some(s), 7200, 9000), &existing).passed_hard());
    }

    #[test]
    fn soft_violations_accumulate_score() {
        let r = Uuid::new_v4();
        let set = rs(vec![
            rule(
                RuleKind::Distribution,
                Severity::Soft,
                RuleSpec::Distribution { mode: DistributionMode::Distributed, gap_seconds: 3600 },
                3,
            ),
        ]);
        let s = Uuid::new_v4();
        let existing = vec![assign(r, Some(s), 0, 1800)];
        let report = validate(&set, &assign(r, Some(s), 1800, 3600), &existing);
        assert!(report.passed_hard());
        assert_eq!(report.soft_score, 3);
        assert_eq!(report.soft_violations.len(), 1);
    }

    #[test]
    fn unavailable_global_scope_applies_to_all_resources() {
        let set = rs(vec![rule(
            RuleKind::UnavailableWindow,
            Severity::Hard,
            RuleSpec::UnavailableWindow {
                resource_id: None,
                windows: vec![TimeWindow { start_unix: 100, end_unix: 200 }],
            },
            1,
        )]);
        let cand = assign(Uuid::new_v4(), None, 150, 175);
        assert!(!validate(&set, &cand, &[]).passed_hard());
    }
}
