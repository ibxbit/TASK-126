//! Claim lifecycle enums. String values match the DB CHECK constraint
//! in `0003_claims_dispute.sql`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    /// Being edited by the claimant; not yet filed.
    Draft,
    /// Filed; countdown started, awaiting respondent engagement.
    Submitted,
    /// Respondent acknowledged; both sides may still act.
    UnderReview,
    /// Both parties accepted — closed-loop success.
    Confirmed,
    /// Post-confirmation resolution event (payment / hand-off complete).
    Resolved,
    /// Parties disagree; escalated for manager decision.
    Contested,
    /// 72-hour window elapsed with no response — terminal.
    AutoCancelled,
    /// Claimant pulled the claim — terminal.
    Withdrawn,
    /// Manager ruled against the claim — terminal.
    RejectedFinal,
    /// Re-opened exactly once by manager approval.
    Reopened,
}

impl ClaimStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ClaimStatus::Draft => "draft",
            ClaimStatus::Submitted => "submitted",
            ClaimStatus::UnderReview => "under_review",
            ClaimStatus::Confirmed => "confirmed",
            ClaimStatus::Resolved => "resolved",
            ClaimStatus::Contested => "contested",
            ClaimStatus::AutoCancelled => "auto_cancelled",
            ClaimStatus::Withdrawn => "withdrawn",
            ClaimStatus::RejectedFinal => "rejected_final",
            ClaimStatus::Reopened => "reopened",
        }
    }

    pub fn from_str(s: &str) -> Option<ClaimStatus> {
        use ClaimStatus::*;
        Some(match s {
            "draft" => Draft,
            "submitted" => Submitted,
            "under_review" => UnderReview,
            "confirmed" => Confirmed,
            "resolved" => Resolved,
            "contested" => Contested,
            "auto_cancelled" => AutoCancelled,
            "withdrawn" => Withdrawn,
            "rejected_final" => RejectedFinal,
            "reopened" => Reopened,
            _ => return None,
        })
    }

    /// Terminal (absorbing) states — no further transitions permitted
    /// except a manager-approved single reopen.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ClaimStatus::Resolved
                | ClaimStatus::AutoCancelled
                | ClaimStatus::Withdrawn
                | ClaimStatus::RejectedFinal
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    ParcelOwnership,
    DepositDeduction,
}

impl ClaimKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ClaimKind::ParcelOwnership => "parcel_ownership",
            ClaimKind::DepositDeduction => "deposit_deduction",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartyRole {
    Claimant,
    Respondent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartyResponse {
    Accept,
    Reject,
}
