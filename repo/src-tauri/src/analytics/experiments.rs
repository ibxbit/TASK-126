//! A/B experiment engine.
//!
//! - Experiments + variants are configured locally; weights are
//!   integer basis points so percentages are exact (sum must be 10_000).
//! - Assignment is deterministic: `bucket = SHA-256(experiment_id ||
//!   subject_id) mod 10_000`. The same subject in the same experiment
//!   always lands in the same variant for the experiment's lifetime.
//! - First assignment is persisted (sticky). Subsequent calls return
//!   the recorded variant.
//! - Outside the active window OR with `enabled = 0`, callers receive
//!   `Inactive` and should fall back to the control / default UX.

use sha2::{Digest, Sha256};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experiment {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: String,
    pub start_at_unix: i64,
    pub end_at_unix: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    pub id: Uuid,
    pub experiment_id: Uuid,
    pub name: String,
    /// 0..=10_000. Sum across an experiment's variants MUST equal 10_000.
    pub weight_bp: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct VariantAssignment {
    pub experiment_id: Uuid,
    pub subject_id: Uuid,
    pub variant_id: Uuid,
    pub variant_name: String,
    pub assigned_at_unix: i64,
    pub sticky: bool,
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum AssignmentError {
    #[error("experiment not found: {0}")]
    NotFound(String),
    #[error("experiment is disabled or outside its active window")]
    Inactive,
    #[error("experiment has no variants")]
    NoVariants,
    #[error("variant weights must sum to 10000 bp (got {got})")]
    BadWeights { got: u32 },
    #[error("persistence error: {0}")]
    Persistence(String),
}

pub trait ExperimentRepository {
    fn load_experiment(&self, id: &Uuid) -> Result<Option<Experiment>, String>;
    fn load_variants(&self, experiment_id: &Uuid) -> Result<Vec<Variant>, String>;
    fn load_assignment(
        &self,
        experiment_id: &Uuid,
        subject_id: &Uuid,
    ) -> Result<Option<Uuid>, String>;
    fn record_assignment(
        &self,
        experiment_id: &Uuid,
        subject_id: &Uuid,
        variant_id: &Uuid,
        now_unix: i64,
    ) -> Result<(), String>;
}

/// Pure decision: given an active experiment + its variants + a
/// subject, deterministically pick a variant. Does NOT persist.
pub fn decide_variant<'a>(
    experiment: &Experiment,
    variants: &'a [Variant],
    subject_id: &Uuid,
    now_unix: i64,
) -> Result<&'a Variant, AssignmentError> {
    if !experiment.enabled
        || now_unix < experiment.start_at_unix
        || now_unix >= experiment.end_at_unix
    {
        return Err(AssignmentError::Inactive);
    }
    if variants.is_empty() {
        return Err(AssignmentError::NoVariants);
    }
    let sum: u32 = variants.iter().map(|v| v.weight_bp).sum();
    if sum != 10_000 {
        return Err(AssignmentError::BadWeights { got: sum });
    }

    let bucket = bucket_for(&experiment.id, subject_id);

    // Sort by id so the cumulative scan is deterministic regardless of
    // SQL row order.
    let mut sorted: Vec<&Variant> = variants.iter().collect();
    sorted.sort_by_key(|v| v.id);

    let mut acc: u32 = 0;
    for v in sorted {
        let next = acc + v.weight_bp;
        if bucket < next {
            return Ok(unsafe_lookup(variants, v.id));
        }
        acc = next;
    }
    // bucket is always < 10_000 once weights validate, so this is unreachable.
    Ok(unsafe_lookup(variants, variants.last().unwrap().id))
}

fn unsafe_lookup(variants: &[Variant], id: Uuid) -> &Variant {
    variants.iter().find(|v| v.id == id).unwrap()
}

/// Compute the assignment bucket for (experiment, subject).
/// Stable, depends only on the inputs — no clock, no salt.
fn bucket_for(experiment_id: &Uuid, subject_id: &Uuid) -> u32 {
    let mut h = Sha256::new();
    h.update(experiment_id.as_bytes());
    h.update(subject_id.as_bytes());
    let digest = h.finalize();
    // Take the first 4 bytes as a big-endian u32, then mod 10_000.
    let n = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    n % 10_000
}

/// Sticky assignment: returns the existing variant if any, otherwise
/// decides + persists. Idempotent.
pub fn assign_variant<R: ExperimentRepository>(
    repo: &R,
    experiment_id: Uuid,
    subject_id: Uuid,
    now_unix: i64,
) -> Result<VariantAssignment, AssignmentError> {
    let exp = repo
        .load_experiment(&experiment_id)
        .map_err(AssignmentError::Persistence)?
        .ok_or_else(|| AssignmentError::NotFound(experiment_id.to_string()))?;
    let variants = repo
        .load_variants(&experiment_id)
        .map_err(AssignmentError::Persistence)?;

    if let Some(existing_id) = repo
        .load_assignment(&experiment_id, &subject_id)
        .map_err(AssignmentError::Persistence)?
    {
        let v = variants
            .iter()
            .find(|v| v.id == existing_id)
            .cloned()
            .ok_or_else(|| AssignmentError::Persistence("dangling variant id".into()))?;
        return Ok(VariantAssignment {
            experiment_id,
            subject_id,
            variant_id: v.id,
            variant_name: v.name,
            assigned_at_unix: now_unix,
            sticky: true,
        });
    }

    let v = decide_variant(&exp, &variants, &subject_id, now_unix)?;
    repo.record_assignment(&experiment_id, &subject_id, &v.id, now_unix)
        .map_err(AssignmentError::Persistence)?;
    Ok(VariantAssignment {
        experiment_id,
        subject_id,
        variant_id: v.id,
        variant_name: v.name.clone(),
        assigned_at_unix: now_unix,
        sticky: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct MockRepo {
        exp: Experiment,
        variants: Vec<Variant>,
        assigned: RefCell<HashMap<(Uuid, Uuid), Uuid>>,
    }
    impl ExperimentRepository for MockRepo {
        fn load_experiment(&self, _: &Uuid) -> Result<Option<Experiment>, String> {
            Ok(Some(self.exp.clone()))
        }
        fn load_variants(&self, _: &Uuid) -> Result<Vec<Variant>, String> {
            Ok(self.variants.clone())
        }
        fn load_assignment(&self, e: &Uuid, s: &Uuid) -> Result<Option<Uuid>, String> {
            Ok(self.assigned.borrow().get(&(*e, *s)).copied())
        }
        fn record_assignment(&self, e: &Uuid, s: &Uuid, v: &Uuid, _: i64) -> Result<(), String> {
            self.assigned.borrow_mut().insert((*e, *s), *v);
            Ok(())
        }
    }

    fn build(weights: &[(u32, &str)]) -> MockRepo {
        let exp_id = Uuid::new_v4();
        // Use deterministic v5 ids per name so tests are stable.
        let variants: Vec<Variant> = weights
            .iter()
            .map(|(w, name)| Variant {
                id: Uuid::new_v5(&exp_id, name.as_bytes()),
                experiment_id: exp_id,
                name: (*name).into(),
                weight_bp: *w,
            })
            .collect();
        MockRepo {
            exp: Experiment {
                id: exp_id,
                tenant_id: Uuid::new_v4(),
                name: "test".into(),
                start_at_unix: 0,
                end_at_unix: 1_000_000,
                enabled: true,
            },
            variants,
            assigned: RefCell::new(HashMap::new()),
        }
    }

    #[test]
    fn deterministic_for_same_subject() {
        let repo = build(&[(5000, "control"), (5000, "treatment")]);
        let s = Uuid::new_v4();
        let a = assign_variant(&repo, repo.exp.id, s, 100).unwrap();
        let b = assign_variant(&repo, repo.exp.id, s, 200).unwrap();
        assert_eq!(a.variant_id, b.variant_id);
        assert!(b.sticky);
        assert!(!a.sticky);
    }

    #[test]
    fn weights_must_sum_to_10000() {
        let repo = build(&[(5000, "control"), (4000, "treatment")]);
        let s = Uuid::new_v4();
        let err = assign_variant(&repo, repo.exp.id, s, 100).unwrap_err();
        assert!(matches!(err, AssignmentError::BadWeights { got: 9000 }));
    }

    #[test]
    fn outside_window_inactive() {
        let mut repo = build(&[(5000, "control"), (5000, "treatment")]);
        repo.exp.end_at_unix = 50;
        let s = Uuid::new_v4();
        assert!(matches!(
            assign_variant(&repo, repo.exp.id, s, 100).unwrap_err(),
            AssignmentError::Inactive
        ));
    }

    #[test]
    fn approximate_distribution_at_scale() {
        let repo = build(&[(2000, "a"), (3000, "b"), (5000, "c")]);
        let mut counts = HashMap::new();
        for _ in 0..2000 {
            let s = Uuid::new_v4();
            let a = assign_variant(&repo, repo.exp.id, s, 100).unwrap();
            *counts.entry(a.variant_name).or_insert(0u32) += 1;
        }
        // Each variant should land near its declared share (±10%).
        let total = 2000.0;
        let a_share = *counts.get("a").unwrap_or(&0) as f64 / total;
        let b_share = *counts.get("b").unwrap_or(&0) as f64 / total;
        let c_share = *counts.get("c").unwrap_or(&0) as f64 / total;
        assert!((a_share - 0.20).abs() < 0.05, "a={}", a_share);
        assert!((b_share - 0.30).abs() < 0.05, "b={}", b_share);
        assert!((c_share - 0.50).abs() < 0.05, "c={}", c_share);
    }

    #[test]
    fn bucket_function_is_pure() {
        let e = Uuid::new_v4();
        let s = Uuid::new_v4();
        assert_eq!(bucket_for(&e, &s), bucket_for(&e, &s));
    }
}
