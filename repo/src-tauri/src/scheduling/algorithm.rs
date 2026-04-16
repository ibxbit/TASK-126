//! Greedy slot allocator.
//!
//! For each `Demand`, we generate a deterministic sweep of candidate
//! `(resource, window)` pairs across the planning horizon, evaluate
//! each via `constraints::validate`, drop hard violators, and pick the
//! lowest-soft-score candidate (ties broken by earliest start time
//! then resource id). The selected assignment is added to the working
//! set so subsequent demands see it for capacity / distribution
//! purposes.
//!
//! Properties:
//!   - Deterministic: same inputs produce the same proposal.
//!   - Auditable: every choice is logged with the score that won.
//!   - Bounded: stride * horizon * resources iterations max.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::scheduling::constraints::{validate, Assignment, ConstraintReport};
use crate::scheduling::rules::{RuleSet, TimeWindow};

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum ScheduleError {
    #[error("demand duration {got}s must be > 0")]
    InvalidDuration { got: i64 },
    #[error("planning horizon must be non-empty")]
    EmptyHorizon,
    #[error("stride must be > 0 (got {0})")]
    InvalidStride(i64),
    #[error("at least one candidate resource is required")]
    NoResources,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Demand {
    pub demand_id: Uuid,
    /// Optional subject (e.g. resident, case) used by Distribution rules.
    pub subject_id: Option<Uuid>,
    /// Required duration in seconds.
    pub duration_seconds: i64,
    /// Earliest acceptable start.
    pub earliest_unix: i64,
    /// Latest acceptable end.
    pub latest_unix: i64,
    /// Resources eligible for this demand, in priority order
    /// (preferred first). Ties in score break to earlier index.
    pub eligible_resources: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProposedAssignment {
    pub demand_id: Uuid,
    pub resource_id: Uuid,
    pub window: TimeWindow,
    pub soft_score: u32,
    /// Soft-rule notes — useful for surfacing "scheduled but warned".
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnfulfilledDemand {
    pub demand_id: Uuid,
    /// Best report observed across attempted candidates — empty
    /// when no candidate could even be generated.
    pub best_attempt: Option<ConstraintReport>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Proposal {
    pub assigned: Vec<ProposedAssignment>,
    pub unfulfilled: Vec<UnfulfilledDemand>,
}

/// Run the allocator.
///
/// `existing` is the calendar state (current bookings); it is read but
/// not mutated. `stride_seconds` controls how finely the planner steps
/// through the horizon (e.g. 900 = 15-minute boundaries).
pub fn propose_schedule(
    rule_set: &RuleSet,
    demands: &[Demand],
    existing: &[Assignment],
    stride_seconds: i64,
) -> Result<Proposal, ScheduleError> {
    if stride_seconds <= 0 {
        return Err(ScheduleError::InvalidStride(stride_seconds));
    }

    // Process demands in a deterministic order: earliest deadline,
    // then longest duration, then by demand id.
    let mut order: Vec<usize> = (0..demands.len()).collect();
    order.sort_by(|&a, &b| {
        demands[a]
            .latest_unix
            .cmp(&demands[b].latest_unix)
            .then(demands[b].duration_seconds.cmp(&demands[a].duration_seconds))
            .then(demands[a].demand_id.cmp(&demands[b].demand_id))
    });

    let mut working: Vec<Assignment> = existing.to_vec();
    let mut proposal = Proposal::default();

    for idx in order {
        let demand = &demands[idx];

        if demand.duration_seconds <= 0 {
            return Err(ScheduleError::InvalidDuration {
                got: demand.duration_seconds,
            });
        }
        if demand.latest_unix <= demand.earliest_unix {
            return Err(ScheduleError::EmptyHorizon);
        }
        if demand.eligible_resources.is_empty() {
            return Err(ScheduleError::NoResources);
        }

        let chosen = pick_best(rule_set, demand, &working, stride_seconds);
        match chosen {
            Some((assignment, report)) => {
                let notes: Vec<String> = report
                    .soft_violations
                    .iter()
                    .map(|v| v.message.clone())
                    .collect();
                working.push(assignment.clone());
                proposal.assigned.push(ProposedAssignment {
                    demand_id: demand.demand_id,
                    resource_id: assignment.resource_id,
                    window: assignment.window,
                    soft_score: report.soft_score,
                    notes,
                });
            }
            None => {
                proposal.unfulfilled.push(UnfulfilledDemand {
                    demand_id: demand.demand_id,
                    best_attempt: best_failed_attempt(rule_set, demand, &working, stride_seconds),
                });
            }
        }
    }

    Ok(proposal)
}

/// Sweep candidates and return the best (lowest soft_score) that
/// passes hard constraints. Resource preference order serves as
/// secondary tie-break (earlier index wins).
fn pick_best(
    rule_set: &RuleSet,
    demand: &Demand,
    existing: &[Assignment],
    stride: i64,
) -> Option<(Assignment, ConstraintReport)> {
    let mut best: Option<(Assignment, ConstraintReport, usize, i64)> = None;

    for (res_idx, resource_id) in demand.eligible_resources.iter().enumerate() {
        let mut start = align_up(demand.earliest_unix, stride);
        while start + demand.duration_seconds <= demand.latest_unix {
            let cand = Assignment {
                resource_id: *resource_id,
                subject_id: demand.subject_id,
                window: TimeWindow {
                    start_unix: start,
                    end_unix: start + demand.duration_seconds,
                },
            };
            let report = validate(rule_set, &cand, existing);
            if report.passed_hard() {
                let key = (report.soft_score, res_idx, start);
                let take = match &best {
                    None => true,
                    Some((_, br, bi, bs)) => {
                        let cur = (br.soft_score, *bi, *bs);
                        key < cur
                    }
                };
                if take {
                    best = Some((cand, report, res_idx, start));
                    if key.0 == 0 {
                        // Perfect fit on the preferred resource, earliest
                        // start — short-circuit further search.
                        if res_idx == 0 {
                            return Some((best.as_ref().unwrap().0.clone(),
                                         best.as_ref().unwrap().1.clone()));
                        }
                    }
                }
            }
            start += stride;
        }
    }

    best.map(|(a, r, _, _)| (a, r))
}

/// When no candidate passes hard constraints, return the report with
/// the fewest hard violations (then lowest soft_score) for diagnostics.
fn best_failed_attempt(
    rule_set: &RuleSet,
    demand: &Demand,
    existing: &[Assignment],
    stride: i64,
) -> Option<ConstraintReport> {
    let mut best: Option<ConstraintReport> = None;
    for resource_id in &demand.eligible_resources {
        let mut start = align_up(demand.earliest_unix, stride);
        while start + demand.duration_seconds <= demand.latest_unix {
            let cand = Assignment {
                resource_id: *resource_id,
                subject_id: demand.subject_id,
                window: TimeWindow {
                    start_unix: start,
                    end_unix: start + demand.duration_seconds,
                },
            };
            let report = validate(rule_set, &cand, existing);
            let take = match &best {
                None => true,
                Some(b) => {
                    (report.hard_violations.len(), report.soft_score)
                        < (b.hard_violations.len(), b.soft_score)
                }
            };
            if take {
                best = Some(report);
            }
            start += stride;
        }
    }
    best
}

fn align_up(value: i64, stride: i64) -> i64 {
    if stride <= 1 {
        return value;
    }
    let r = value.rem_euclid(stride);
    if r == 0 { value } else { value + (stride - r) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduling::constraints::Severity;
    use crate::scheduling::rules::{DistributionMode, Rule, RuleKind, RuleSet, RuleSpec};

    fn empty_rs() -> RuleSet {
        RuleSet {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            name: "empty".into(),
            version: 1,
            parent_rule_set_id: None,
            enabled: true,
            rules: vec![],
        }
    }

    fn with_rule(spec: RuleSpec, severity: Severity, weight: u32) -> RuleSet {
        let mut rs = empty_rs();
        rs.rules.push(Rule {
            id: Uuid::new_v4(),
            rule_set_id: rs.id,
            kind: match &spec {
                RuleSpec::UnavailableWindow { .. } => RuleKind::UnavailableWindow,
                RuleSpec::CapacityLimit { .. } => RuleKind::CapacityLimit,
                RuleSpec::RequiredDuration { .. } => RuleKind::RequiredDuration,
                RuleSpec::Distribution { .. } => RuleKind::Distribution,
            },
            severity,
            spec,
            weight,
            enabled: true,
        });
        rs
    }

    fn demand(duration: i64, earliest: i64, latest: i64, resources: Vec<Uuid>) -> Demand {
        Demand {
            demand_id: Uuid::new_v4(),
            subject_id: None,
            duration_seconds: duration,
            earliest_unix: earliest,
            latest_unix: latest,
            eligible_resources: resources,
        }
    }

    #[test]
    fn align_up_snaps_to_stride() {
        assert_eq!(align_up(0, 900), 0);
        assert_eq!(align_up(1, 900), 900);
        assert_eq!(align_up(900, 900), 900);
        assert_eq!(align_up(901, 900), 1800);
    }

    #[test]
    fn assigns_demand_to_first_acceptable_slot() {
        let r = Uuid::new_v4();
        let p = propose_schedule(
            &empty_rs(),
            &[demand(1800, 0, 7200, vec![r])],
            &[],
            900,
        )
        .unwrap();
        assert_eq!(p.assigned.len(), 1);
        assert_eq!(p.unfulfilled.len(), 0);
        let a = &p.assigned[0];
        assert_eq!(a.resource_id, r);
        assert_eq!(a.window.start_unix, 0);
        assert_eq!(a.window.end_unix, 1800);
    }

    #[test]
    fn skips_unavailable_window() {
        let r = Uuid::new_v4();
        let rs = with_rule(
            RuleSpec::UnavailableWindow {
                resource_id: Some(r),
                windows: vec![TimeWindow { start_unix: 0, end_unix: 1800 }],
            },
            Severity::Hard,
            1,
        );
        let p = propose_schedule(&rs, &[demand(1800, 0, 7200, vec![r])], &[], 900).unwrap();
        assert_eq!(p.assigned.len(), 1);
        assert_eq!(p.assigned[0].window.start_unix, 1800);
    }

    #[test]
    fn capacity_blocks_concurrent_demands() {
        let r = Uuid::new_v4();
        let rs = with_rule(
            RuleSpec::CapacityLimit { resource_id: r, max_concurrent: 1 },
            Severity::Hard,
            1,
        );
        // Two demands of 1800s each, horizon 0..3600, single resource cap=1
        let d1 = demand(1800, 0, 3600, vec![r]);
        let d2 = demand(1800, 0, 3600, vec![r]);
        let p = propose_schedule(&rs, &[d1, d2], &[], 900).unwrap();
        assert_eq!(p.assigned.len(), 2);
        // Should be back-to-back, not overlapping.
        let a = &p.assigned[0];
        let b = &p.assigned[1];
        assert!(a.window.end_unix <= b.window.start_unix
                || b.window.end_unix <= a.window.start_unix);
    }

    #[test]
    fn unfulfilled_demand_reports_diagnostics() {
        let r = Uuid::new_v4();
        // Required duration 3600..3600, but demand asks for only 1800.
        let rs = with_rule(
            RuleSpec::RequiredDuration { min_seconds: 3600, max_seconds: 3600 },
            Severity::Hard,
            1,
        );
        let p = propose_schedule(&rs, &[demand(1800, 0, 7200, vec![r])], &[], 900).unwrap();
        assert_eq!(p.assigned.len(), 0);
        assert_eq!(p.unfulfilled.len(), 1);
        assert!(p.unfulfilled[0].best_attempt.is_some());
    }

    #[test]
    fn prefers_lower_index_resource_on_tie() {
        let pref = Uuid::new_v4();
        let alt = Uuid::new_v4();
        let p = propose_schedule(
            &empty_rs(),
            &[demand(1800, 0, 7200, vec![pref, alt])],
            &[],
            900,
        )
        .unwrap();
        assert_eq!(p.assigned[0].resource_id, pref);
    }

    #[test]
    fn soft_rule_does_not_block_but_is_reported() {
        let r = Uuid::new_v4();
        let s = Uuid::new_v4();
        let rs = with_rule(
            RuleSpec::Distribution { mode: DistributionMode::Distributed, gap_seconds: 7200 },
            Severity::Soft,
            5,
        );
        let mut d1 = demand(1800, 0, 5400, vec![r]);
        let mut d2 = demand(1800, 0, 5400, vec![r]);
        d1.subject_id = Some(s);
        d2.subject_id = Some(s);
        let p = propose_schedule(&rs, &[d1, d2], &[], 900).unwrap();
        assert_eq!(p.assigned.len(), 2);
        // The second one is too close (gap < 7200) so it carries soft_score > 0.
        let with_warn = p.assigned.iter().find(|a| a.soft_score > 0);
        assert!(with_warn.is_some());
    }

    #[test]
    fn rejects_invalid_inputs() {
        let r = Uuid::new_v4();
        assert!(propose_schedule(&empty_rs(), &[demand(0, 0, 100, vec![r])], &[], 900).is_err());
        assert!(propose_schedule(&empty_rs(), &[demand(100, 100, 100, vec![r])], &[], 900).is_err());
        assert!(propose_schedule(&empty_rs(), &[demand(100, 0, 200, vec![])], &[], 900).is_err());
        assert!(propose_schedule(&empty_rs(), &[demand(100, 0, 200, vec![r])], &[], 0).is_err());
    }

    #[test]
    fn deterministic_across_runs() {
        let r1 = Uuid::new_v4();
        let r2 = Uuid::new_v4();
        let demands = vec![
            demand(1800, 0, 7200, vec![r1, r2]),
            demand(1800, 0, 7200, vec![r2, r1]),
        ];
        let a = propose_schedule(&empty_rs(), &demands, &[], 900).unwrap();
        let b = propose_schedule(&empty_rs(), &demands, &[], 900).unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }
}
