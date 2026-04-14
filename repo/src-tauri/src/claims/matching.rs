//! Similarity matching for related/duplicate claim detection.
//!
//! Score = weighted sum over four signals:
//!   - category_match    — exact category equality            [0.0 or 1.0]
//!   - address_match     — normalized unit address equality   [0.0 or 1.0]
//!   - time_proximity    — linear decay over configured window [0.0..1.0]
//!   - keyword_overlap   — Jaccard index of keyword tokens    [0.0..1.0]
//!
//! The caller typically pre-filters candidates at the SQL layer using
//! `idx_claims_matching` (tenant_id, kind, category, unit_address) and
//! passes the narrowed list into `find_matches`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::claims::state::ClaimKind;

/// A simplified projection of a claim used for matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimFeatures {
    pub claim_id: Uuid,
    pub tenant_id: Uuid,
    pub kind: ClaimKind,
    pub category: String,
    pub unit_address: Option<String>,
    pub keywords: Vec<String>,
    /// Opened-at timestamp, Unix seconds.
    pub opened_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchWeights {
    pub category: f32,
    pub address: f32,
    pub time: f32,
    pub keywords: f32,
    /// Time proximity is scored 1.0 at delta=0 and decays linearly to
    /// 0.0 at `time_window_seconds`.
    pub time_window_seconds: i64,
    /// Minimum score (0.0..=1.0) for a candidate to be surfaced.
    pub threshold: f32,
}

impl Default for MatchWeights {
    fn default() -> Self {
        // Weights sum to 1.0 so `score` stays in [0.0, 1.0].
        Self {
            category: 0.20,
            address: 0.35,
            time: 0.20,
            keywords: 0.25,
            time_window_seconds: 72 * 3600, // matches the 72h response window
            threshold: 0.50,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchCandidate {
    pub claim_id: Uuid,
    pub score: f32,
    pub breakdown: MatchBreakdown,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchBreakdown {
    pub category: f32,
    pub address: f32,
    pub time: f32,
    pub keywords: f32,
}

/// Normalize a unit address for equality comparison: lowercase, trim,
/// collapse whitespace, strip common punctuation, and remove common
/// unit-type prefixes ("unit", "apt", "suite", "ste"). Deliberately
/// conservative — we prefer false negatives over false positives.
pub fn normalize_address(input: &str) -> String {
    let lowered = input.to_ascii_lowercase();
    let cleaned: String = lowered
        .chars()
        .map(|c| match c {
            ',' | '.' | '#' | '-' => ' ',
            other => other,
        })
        .collect();
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    // Strip a leading unit-type prefix so "Unit 4B" and "4B" compare equal.
    const PREFIXES: &[&str] = &["unit", "apt", "suite", "ste"];
    let tokens = if tokens.len() > 1 && PREFIXES.contains(&tokens[0]) {
        &tokens[1..]
    } else {
        &tokens[..]
    };
    tokens.join(" ")
}

/// Tokenize free-text keywords: lowercase, strip non-alphanumerics,
/// drop tokens shorter than 2 chars.
pub fn tokenize_keywords(input: &str) -> Vec<String> {
    input
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect()
}

fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let set_a: HashSet<&String> = a.iter().collect();
    let set_b: HashSet<&String> = b.iter().collect();
    let inter = set_a.intersection(&set_b).count() as f32;
    let union = set_a.union(&set_b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn score_time(base: i64, other: i64, window: i64) -> f32 {
    if window <= 0 {
        return 0.0;
    }
    let delta = (base - other).abs();
    if delta >= window {
        0.0
    } else {
        1.0 - (delta as f32 / window as f32)
    }
}

fn score_pair(
    base: &ClaimFeatures,
    cand: &ClaimFeatures,
    weights: &MatchWeights,
) -> MatchBreakdown {
    let category = if base.category == cand.category { 1.0 } else { 0.0 };
    let address = match (&base.unit_address, &cand.unit_address) {
        (Some(a), Some(b)) if normalize_address(a) == normalize_address(b) => 1.0,
        _ => 0.0,
    };
    let time = score_time(base.opened_at_unix, cand.opened_at_unix, weights.time_window_seconds);
    let keywords = jaccard(&base.keywords, &cand.keywords);

    MatchBreakdown { category, address, time, keywords }
}

fn combined(b: &MatchBreakdown, w: &MatchWeights) -> f32 {
    b.category * w.category + b.address * w.address + b.time * w.time + b.keywords * w.keywords
}

/// Score every candidate against `base` and return those meeting the
/// threshold, sorted highest score first. `base` itself is excluded.
///
/// Candidates are also filtered to the same `tenant_id` and `kind` —
/// matching never leaks across tenants, and parcel-ownership disputes
/// never match deposit-deduction claims.
pub fn find_matches(
    base: &ClaimFeatures,
    candidates: &[ClaimFeatures],
    weights: &MatchWeights,
) -> Vec<MatchCandidate> {
    let mut out: Vec<MatchCandidate> = candidates
        .iter()
        .filter(|c| c.claim_id != base.claim_id)
        .filter(|c| c.tenant_id == base.tenant_id && c.kind == base.kind)
        .map(|c| {
            let breakdown = score_pair(base, c, weights);
            let score = combined(&breakdown, weights);
            MatchCandidate {
                claim_id: c.claim_id,
                score,
                breakdown,
            }
        })
        .filter(|m| m.score >= weights.threshold)
        .collect();

    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feat(
        tenant: Uuid,
        kind: ClaimKind,
        category: &str,
        addr: Option<&str>,
        keywords: &[&str],
        t: i64,
    ) -> ClaimFeatures {
        ClaimFeatures {
            claim_id: Uuid::new_v4(),
            tenant_id: tenant,
            kind,
            category: category.into(),
            unit_address: addr.map(|s| s.into()),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            opened_at_unix: t,
        }
    }

    #[test]
    fn normalization_merges_punctuation() {
        assert_eq!(normalize_address("Unit #4-B, Bldg.3"), "4 b bldg 3");
        assert_eq!(normalize_address("UNIT 4B"), normalize_address("unit  4b"));
    }

    #[test]
    fn tokenizer_drops_short_and_non_alnum() {
        let t = tokenize_keywords("A missing package! at Bldg-3");
        assert!(t.contains(&"missing".to_string()));
        assert!(t.contains(&"package".to_string()));
        assert!(t.contains(&"at".to_string()));
        assert!(!t.contains(&"a".to_string()));
    }

    #[test]
    fn cross_tenant_never_matches() {
        let t1 = Uuid::new_v4();
        let t2 = Uuid::new_v4();
        let base = feat(t1, ClaimKind::ParcelOwnership, "missing_item", Some("4B"),
                        &["box","fedex"], 1000);
        let cand = feat(t2, ClaimKind::ParcelOwnership, "missing_item", Some("4B"),
                        &["box","fedex"], 1000);
        assert!(find_matches(&base, &[cand], &MatchWeights::default()).is_empty());
    }

    #[test]
    fn cross_kind_never_matches() {
        let t = Uuid::new_v4();
        let base = feat(t, ClaimKind::ParcelOwnership, "missing_item", Some("4B"),
                        &["box"], 1000);
        let cand = feat(t, ClaimKind::DepositDeduction, "missing_item", Some("4B"),
                        &["box"], 1000);
        assert!(find_matches(&base, &[cand], &MatchWeights::default()).is_empty());
    }

    #[test]
    fn identical_claim_scores_one() {
        let t = Uuid::new_v4();
        let base = feat(t, ClaimKind::ParcelOwnership, "missing_item", Some("4B"),
                        &["fedex","box"], 1000);
        let cand = feat(t, ClaimKind::ParcelOwnership, "missing_item", Some("UNIT 4B"),
                        &["fedex","box"], 1000);
        let r = find_matches(&base, &[cand], &MatchWeights::default());
        assert_eq!(r.len(), 1);
        assert!((r[0].score - 1.0).abs() < 0.01);
    }

    #[test]
    fn unrelated_falls_below_threshold() {
        let t = Uuid::new_v4();
        let w = MatchWeights::default();
        let base = feat(t, ClaimKind::ParcelOwnership, "missing_item", Some("4B"),
                        &["fedex"], 1000);
        // Different category, different address, far in time, no keyword overlap.
        let cand = feat(t, ClaimKind::ParcelOwnership, "other", Some("22C"),
                        &["cleaning"], 1000 + 10 * w.time_window_seconds);
        assert!(find_matches(&base, &[cand], &w).is_empty());
    }

    #[test]
    fn time_decay_is_linear() {
        let w = MatchWeights::default();
        let full = score_time(0, 0, w.time_window_seconds);
        let half = score_time(0, w.time_window_seconds / 2, w.time_window_seconds);
        let none = score_time(0, w.time_window_seconds, w.time_window_seconds);
        assert!((full - 1.0).abs() < 1e-6);
        assert!((half - 0.5).abs() < 1e-2);
        assert_eq!(none, 0.0);
    }

    #[test]
    fn results_sorted_highest_first() {
        let t = Uuid::new_v4();
        let w = MatchWeights::default();
        let base = feat(t, ClaimKind::ParcelOwnership, "missing_item", Some("4B"),
                        &["fedex","box"], 1000);
        let strong = feat(t, ClaimKind::ParcelOwnership, "missing_item", Some("4B"),
                          &["fedex","box","amazon"], 1100);
        let weak = feat(t, ClaimKind::ParcelOwnership, "missing_item", Some("4B"),
                        &["amazon"], 1000 + w.time_window_seconds - 10);
        let r = find_matches(&base, &[weak.clone(), strong.clone()], &w);
        assert_eq!(r[0].claim_id, strong.claim_id);
    }
}
