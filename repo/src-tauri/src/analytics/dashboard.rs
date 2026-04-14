//! Dashboard query functions: funnel conversion, retention cohorts,
//! and quality metrics. Pure, deterministic computations over rows
//! fetched by the repository — no SQL embedded in the algorithms.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const DAY_SECONDS: i64 = 86_400;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DashboardError {
    #[error("funnel must define at least 2 steps")]
    EmptyFunnel,
    #[error("retention windows must be > 0")]
    InvalidRetentionWindow,
    #[error("persistence error: {0}")]
    Persistence(String),
}

// ── Funnel ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct FunnelDefinition {
    pub name: String,
    /// Ordered list of event kinds, step 1 → step N.
    pub steps: Vec<FunnelStepDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FunnelStepDef {
    pub event_kind: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunnelStepResult {
    pub step_no: u32,
    pub label: String,
    pub event_kind: String,
    pub user_count: u64,
    /// 0.0..=1.0; conversion FROM previous step. Step 1 reports 1.0.
    pub conversion_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunnelResult {
    pub funnel_name: String,
    pub steps: Vec<FunnelStepResult>,
    pub overall_conversion_rate: f64,
}

/// Compute funnel conversion. `events` is a flat stream of
/// (subject_id, event_kind, occurred_at) for the analysis window —
/// typically the result of one indexed SQL scan over `events`.
///
/// A subject counts at step N iff they completed step 1, then step 2,
/// …, then step N in chronological order (timestamps strictly
/// non-decreasing). This is the "ordered, all-prior" funnel definition.
pub fn compute_funnel(
    funnel: &FunnelDefinition,
    events: &[(Uuid, String, i64)],
) -> Result<FunnelResult, DashboardError> {
    if funnel.steps.len() < 2 {
        return Err(DashboardError::EmptyFunnel);
    }

    // Group events per subject in chronological order.
    let mut by_subject: BTreeMap<Uuid, Vec<(String, i64)>> = BTreeMap::new();
    for (sid, kind, t) in events {
        by_subject
            .entry(*sid)
            .or_default()
            .push((kind.clone(), *t));
    }
    for v in by_subject.values_mut() {
        v.sort_by_key(|(_, t)| *t);
    }

    let step_kinds: Vec<&str> = funnel.steps.iter().map(|s| s.event_kind.as_str()).collect();
    let mut counts: Vec<u64> = vec![0; step_kinds.len()];

    for events in by_subject.values() {
        let mut next = 0usize;
        for (kind, _t) in events {
            if next < step_kinds.len() && kind == step_kinds[next] {
                counts[next] += 1;
                next += 1;
                if next == step_kinds.len() {
                    break;
                }
            }
        }
    }

    let mut steps = Vec::with_capacity(step_kinds.len());
    for (i, def) in funnel.steps.iter().enumerate() {
        let conversion = if i == 0 {
            1.0
        } else if counts[i - 1] == 0 {
            0.0
        } else {
            counts[i] as f64 / counts[i - 1] as f64
        };
        steps.push(FunnelStepResult {
            step_no: (i + 1) as u32,
            label: def.label.clone(),
            event_kind: def.event_kind.clone(),
            user_count: counts[i],
            conversion_rate: conversion,
        });
    }

    let overall = if counts[0] == 0 {
        0.0
    } else {
        *counts.last().unwrap() as f64 / counts[0] as f64
    };

    Ok(FunnelResult {
        funnel_name: funnel.name.clone(),
        steps,
        overall_conversion_rate: overall,
    })
}

// ── Retention ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct RetentionInput {
    /// Per-subject earliest activity timestamp (cohort assignment).
    pub first_seen: Vec<(Uuid, i64)>,
    /// All activity events in scope: (subject, occurred_at).
    pub activity: Vec<(Uuid, i64)>,
    /// Cohort granularity in seconds. 86_400 = daily.
    pub cohort_window_seconds: i64,
    /// How many subsequent windows to track per cohort.
    pub follow_up_windows: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetentionCohort {
    pub cohort_unix: i64,
    pub cohort_size: u64,
    /// Index 0 = window 0 (cohort window itself, always = cohort_size).
    /// Subsequent indices: how many of the cohort were active in
    /// window 1, 2, ….
    pub retained: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetentionResult {
    pub cohort_window_seconds: i64,
    pub follow_up_windows: u32,
    pub cohorts: Vec<RetentionCohort>,
}

pub fn compute_retention(input: RetentionInput) -> Result<RetentionResult, DashboardError> {
    if input.cohort_window_seconds <= 0 || input.follow_up_windows == 0 {
        return Err(DashboardError::InvalidRetentionWindow);
    }
    let w = input.cohort_window_seconds;

    // Bucket subjects into cohorts.
    let mut cohort_of: BTreeMap<Uuid, i64> = BTreeMap::new();
    for (sid, t) in &input.first_seen {
        cohort_of.insert(*sid, t - t.rem_euclid(w));
    }

    // For each cohort, prepare a presence map: window_idx → set of subjects.
    let mut cohort_subjects: BTreeMap<i64, HashSet<Uuid>> = BTreeMap::new();
    for (sid, c) in &cohort_of {
        cohort_subjects.entry(*c).or_default().insert(*sid);
    }
    let mut cohort_presence: BTreeMap<i64, Vec<HashSet<Uuid>>> = BTreeMap::new();
    for c in cohort_subjects.keys() {
        cohort_presence.insert(
            *c,
            (0..input.follow_up_windows + 1)
                .map(|_| HashSet::new())
                .collect(),
        );
    }

    for (sid, t) in &input.activity {
        let Some(c) = cohort_of.get(sid).copied() else { continue };
        if t < &c {
            continue;
        }
        let idx = ((t - c) / w) as i64;
        if idx < 0 || idx as u32 > input.follow_up_windows {
            continue;
        }
        if let Some(buckets) = cohort_presence.get_mut(&c) {
            buckets[idx as usize].insert(*sid);
        }
    }

    let mut cohorts = Vec::new();
    for (c, members) in cohort_subjects {
        let buckets = cohort_presence.remove(&c).unwrap();
        let retained: Vec<u64> = buckets.iter().map(|s| s.len() as u64).collect();
        cohorts.push(RetentionCohort {
            cohort_unix: c,
            cohort_size: members.len() as u64,
            retained,
        });
    }

    Ok(RetentionResult {
        cohort_window_seconds: input.cohort_window_seconds,
        follow_up_windows: input.follow_up_windows,
        cohorts,
    })
}

// ── Quality metrics ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct QualityMetrics {
    pub total_events: u64,
    pub success_rate: f64,
    pub mean_duration_ms: f64,
    pub p50_duration_ms: i64,
    pub p95_duration_ms: i64,
}

/// Compute quality metrics over a stream of (success, duration_ms?)
/// rows. Events without a duration are counted toward total_events
/// and success_rate but excluded from latency percentiles.
pub fn compute_quality(rows: &[(Option<bool>, Option<i64>)]) -> QualityMetrics {
    let total = rows.len() as u64;
    if total == 0 {
        return QualityMetrics {
            total_events: 0,
            success_rate: 0.0,
            mean_duration_ms: 0.0,
            p50_duration_ms: 0,
            p95_duration_ms: 0,
        };
    }

    let successes = rows.iter().filter(|(s, _)| matches!(s, Some(true))).count() as u64;
    let success_rate = successes as f64 / total as f64;

    let mut durations: Vec<i64> = rows.iter().filter_map(|(_, d)| *d).filter(|d| *d >= 0).collect();
    durations.sort_unstable();

    let mean = if durations.is_empty() {
        0.0
    } else {
        durations.iter().sum::<i64>() as f64 / durations.len() as f64
    };
    let p50 = percentile(&durations, 50.0);
    let p95 = percentile(&durations, 95.0);

    QualityMetrics {
        total_events: total,
        success_rate,
        mean_duration_ms: mean,
        p50_duration_ms: p50,
        p95_duration_ms: p95,
    }
}

fn percentile(sorted: &[i64], pct: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    let p = pct.clamp(0.0, 100.0);
    let rank = (p / 100.0) * (sorted.len() as f64 - 1.0);
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        // Linear interpolation between the two neighbors.
        let frac = rank - lo as f64;
        let lo_v = sorted[lo] as f64;
        let hi_v = sorted[hi] as f64;
        (lo_v + (hi_v - lo_v) * frac).round() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fdef() -> FunnelDefinition {
        FunnelDefinition {
            name: "move_out".into(),
            steps: vec![
                FunnelStepDef { event_kind: "case.opened".into(), label: "Open".into() },
                FunnelStepDef { event_kind: "settlement.prepared".into(), label: "Prepared".into() },
                FunnelStepDef { event_kind: "settlement.approved".into(), label: "Approved".into() },
            ],
        }
    }

    fn ev(s: Uuid, k: &str, t: i64) -> (Uuid, String, i64) {
        (s, k.into(), t)
    }

    #[test]
    fn funnel_counts_ordered_progressions() {
        let s1 = Uuid::new_v4();
        let s2 = Uuid::new_v4();
        let s3 = Uuid::new_v4();
        let evs = vec![
            ev(s1, "case.opened", 100),
            ev(s1, "settlement.prepared", 200),
            ev(s1, "settlement.approved", 300),
            ev(s2, "case.opened", 100),
            ev(s2, "settlement.prepared", 200),
            ev(s3, "case.opened", 100),
        ];
        let r = compute_funnel(&fdef(), &evs).unwrap();
        assert_eq!(r.steps[0].user_count, 3);
        assert_eq!(r.steps[1].user_count, 2);
        assert_eq!(r.steps[2].user_count, 1);
        assert!((r.overall_conversion_rate - 1.0/3.0).abs() < 1e-9);
    }

    #[test]
    fn funnel_requires_chronological_order() {
        let s = Uuid::new_v4();
        let evs = vec![
            ev(s, "settlement.approved", 100),
            ev(s, "case.opened", 200),
        ];
        let r = compute_funnel(&fdef(), &evs).unwrap();
        // case.opened arrived after settlement.approved → no progress past step 1.
        assert_eq!(r.steps[0].user_count, 1);
        assert_eq!(r.steps[1].user_count, 0);
    }

    #[test]
    fn funnel_rejects_too_few_steps() {
        let mut f = fdef();
        f.steps.truncate(1);
        assert!(matches!(
            compute_funnel(&f, &[]).unwrap_err(),
            DashboardError::EmptyFunnel
        ));
    }

    #[test]
    fn retention_groups_by_cohort_window() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let day = 86_400i64;
        let input = RetentionInput {
            first_seen: vec![(a, 0), (b, 0)],
            activity: vec![
                (a, 0),         // window 0
                (a, day + 10),  // window 1
                (a, 2*day + 10),// window 2
                (b, 0),         // window 0
                // b never returns
            ],
            cohort_window_seconds: day,
            follow_up_windows: 3,
        };
        let r = compute_retention(input).unwrap();
        assert_eq!(r.cohorts.len(), 1);
        let c = &r.cohorts[0];
        assert_eq!(c.cohort_size, 2);
        assert_eq!(c.retained[0], 2);
        assert_eq!(c.retained[1], 1);
        assert_eq!(c.retained[2], 1);
        assert_eq!(c.retained[3], 0);
    }

    #[test]
    fn quality_metrics_basic() {
        let rows = vec![
            (Some(true),  Some(10)),
            (Some(true),  Some(20)),
            (Some(false), Some(30)),
            (Some(true),  Some(40)),
            (Some(true),  Some(50)),
            (None,        None),
        ];
        let m = compute_quality(&rows);
        assert_eq!(m.total_events, 6);
        assert!((m.success_rate - (4.0/6.0)).abs() < 1e-9);
        assert_eq!(m.p50_duration_ms, 30);
        assert!(m.p95_duration_ms >= 40);
    }

    #[test]
    fn quality_handles_empty() {
        let m = compute_quality(&[]);
        assert_eq!(m.total_events, 0);
        assert_eq!(m.p50_duration_ms, 0);
    }
}
