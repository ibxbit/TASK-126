//! Claim IPC commands — SQLite-backed.

use std::sync::Arc;
use uuid::Uuid;

use crate::claims::machine::{apply_transition, ClaimEvent, TransitionOutcome};
use crate::claims::matching::{find_matches, ClaimFeatures, MatchCandidate, MatchWeights};
use crate::claims::timeout::enforce_timeout_lazy;
use crate::db::connection::Database;
use crate::db::repos::{SqliteAuditWriter, SqliteClaimRepo};
use crate::ipc::{guard, IpcError, SessionState};
use crate::auth::Permission;

fn system_uid() -> Uuid {
    // In production, bootstrap stores a dedicated system user id.
    // Placeholder: nil UUID for the timeout-enforcer identity.
    Uuid::nil()
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn cmd_claim_transition(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    claim_id: Uuid,
    event: ClaimEvent,
) -> Result<TransitionOutcome, IpcError> {
    let principal = guard::require_authenticated(session.inner())?;
    let repo = SqliteClaimRepo::new(Arc::clone(db.inner()));
    let audit = SqliteAuditWriter::new(Arc::clone(db.inner()));

    // Lazy timeout enforcement before the user's event.
    enforce_timeout_lazy(&repo, &audit, system_uid(), &claim_id, now_unix())
        .map_err(|e| IpcError::Internal(e.to_string()))?;

    apply_transition(&repo, &audit, &principal, &claim_id, event, now_unix())
        .map_err(|e| IpcError::Internal(format!("{e:?}")))
}

#[tauri::command]
pub fn cmd_find_claim_matches(
    session: tauri::State<'_, SessionState>,
    claim_id: Uuid,
) -> Result<Vec<MatchCandidate>, IpcError> {
    guard::require_authenticated(session.inner())?;
    // Matching is a pure function over features. In a full
    // implementation the command would load features from the DB for
    // both the base claim and its candidates. Here we return an empty
    // set — the algorithm is exercised via unit tests, and the DB
    // query to hydrate features lands with the search index work.
    Ok(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::state::ClaimKind;

    #[test]
    fn system_uid_is_nil() {
        assert_eq!(system_uid(), Uuid::nil());
    }

    #[test]
    fn now_unix_returns_positive() {
        assert!(now_unix() > 0);
    }

    #[test]
    fn find_matches_returns_empty_for_no_candidates() {
        let tid = Uuid::new_v4();
        let base = ClaimFeatures {
            claim_id: Uuid::new_v4(),
            tenant_id: tid,
            kind: ClaimKind::DepositDeduction,
            category: "damage".into(),
            unit_address: Some("Unit 4B".into()),
            keywords: vec!["scratch".into()],
            opened_at_unix: 1_700_000_000,
        };
        let candidates: Vec<ClaimFeatures> = vec![];
        let weights = MatchWeights::default();
        let results = find_matches(&base, &candidates, &weights);
        assert!(results.is_empty());
    }

    #[test]
    fn find_matches_scores_identical_as_high() {
        let tid = Uuid::new_v4();
        let base = ClaimFeatures {
            claim_id: Uuid::new_v4(),
            tenant_id: tid,
            kind: ClaimKind::ParcelOwnership,
            category: "missing_item".into(),
            unit_address: Some("4B".into()),
            keywords: vec!["fedex".into(), "box".into()],
            opened_at_unix: 1_700_000_000,
        };
        let cand = ClaimFeatures {
            claim_id: Uuid::new_v4(),
            tenant_id: tid,
            kind: ClaimKind::ParcelOwnership,
            category: "missing_item".into(),
            unit_address: Some("Unit 4B".into()),
            keywords: vec!["fedex".into(), "box".into()],
            opened_at_unix: 1_700_000_000,
        };
        let weights = MatchWeights::default();
        let results = find_matches(&base, &[cand], &weights);
        assert_eq!(results.len(), 1);
        assert!((results[0].score - 1.0).abs() < 0.01, "identical should score ~1.0");
    }
}
