//! Payout: printable check request + double-entry ledger posting.
//!
//! Online payments are explicitly disallowed. The "payout" produces:
//!   - one row in `check_requests` (status=drafted), pointing at a
//!     printable HTML artifact stored under the attachments root;
//!   - a balanced set of `ledger_entries` rows under one `journal_id`.
//!
//! The settlement state machine then transitions to Paid once the
//! check is marked printed (a separate user action).

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::auth::{self, AuthError, Permission, Principal};
use crate::settlement::statement::SettlementStatement;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedgerError {
    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("settlement has zero refund — no check required")]
    ZeroRefund,

    #[error("ledger entries do not balance: sum={sum}")]
    Unbalanced { sum: i64 },

    #[error("a check request already exists for this settlement")]
    CheckExists,

    #[error("persistence error: {0}")]
    Persistence(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckRequest {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub settlement_id: Uuid,
    pub payee_name: String,
    pub amount_cents: i64,
    pub currency: String,
    pub memo: Option<String>,
    /// Encrypted relative path to the printable HTML artifact (under
    /// the attachments root). The caller computes encryption.
    pub artifact_path_enc: Option<Vec<u8>>,
    pub status: &'static str,   // "drafted"
    pub drafted_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LedgerEntry {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub journal_id: Uuid,
    pub settlement_id: Option<Uuid>,
    pub account: &'static str,  // 'deposit_liability' | 'refund_payable' | 'forfeited_revenue' | 'clearing'
    /// Positive = debit, negative = credit. Sum of all entries in a
    /// journal MUST equal zero.
    pub amount_cents: i64,
    pub memo: Option<String>,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PayoutOutcome {
    pub check_request: CheckRequest,
    pub ledger: Vec<LedgerEntry>,
    /// Self-contained printable HTML for the check request, ready to
    /// be persisted alongside the artifact path and printed in-app.
    pub artifact_html: String,
}

pub trait CheckRequestRepository {
    fn check_exists(&self, settlement_id: &Uuid) -> Result<bool, String>;
    fn insert(&self, req: &CheckRequest) -> Result<(), String>;
}

pub trait LedgerRepository {
    fn append_journal(&self, entries: &[LedgerEntry]) -> Result<(), String>;
}

/// Build the check request artifact + balanced ledger entries for a
/// settlement and persist them. Returns the rendered HTML so the UI
/// can write it under the attachments root via the document module.
pub fn generate_check_request<C: CheckRequestRepository, L: LedgerRepository>(
    check_repo: &C,
    ledger_repo: &L,
    principal: &Principal,
    statement: &SettlementStatement,
    payee_name: String,
    memo: Option<String>,
    artifact_path_enc: Option<Vec<u8>>,
    now_unix: i64,
) -> Result<PayoutOutcome, LedgerError> {
    auth::require(principal, Permission::ApproveSettlement, &statement.tenant_id)?;

    if statement.refund_cents <= 0 {
        return Err(LedgerError::ZeroRefund);
    }
    if check_repo
        .check_exists(&statement.settlement_id)
        .map_err(LedgerError::Persistence)?
    {
        return Err(LedgerError::CheckExists);
    }

    let check = CheckRequest {
        id: Uuid::new_v4(),
        tenant_id: statement.tenant_id,
        settlement_id: statement.settlement_id,
        payee_name: payee_name.clone(),
        amount_cents: statement.refund_cents,
        currency: statement.currency.clone(),
        memo: memo.clone(),
        artifact_path_enc,
        status: "drafted",
        drafted_at: now_unix,
    };

    let ledger = post_ledger_for_settlement(statement, now_unix);
    let sum: i64 = ledger.iter().map(|e| e.amount_cents).sum();
    if sum != 0 {
        return Err(LedgerError::Unbalanced { sum });
    }

    check_repo.insert(&check).map_err(LedgerError::Persistence)?;
    ledger_repo
        .append_journal(&ledger)
        .map_err(LedgerError::Persistence)?;

    let artifact_html = render_check_request_html(&check, &payee_name, statement);
    Ok(PayoutOutcome { check_request: check, ledger, artifact_html })
}

/// Build the balanced journal for a settlement.
///
/// The deposit was previously held as a liability owed to the
/// resident. On settlement we close that liability and split it into:
///   - refund_payable    (= refund_cents)        owed to resident
///   - forfeited_revenue (= deductions_cents)    earned by property
pub fn post_ledger_for_settlement(
    statement: &SettlementStatement,
    now_unix: i64,
) -> Vec<LedgerEntry> {
    let journal = Uuid::new_v4();
    let mut entries = Vec::with_capacity(3);

    // Debit: clear the deposit liability.
    entries.push(LedgerEntry {
        id: Uuid::new_v4(),
        tenant_id: statement.tenant_id,
        journal_id: journal,
        settlement_id: Some(statement.settlement_id),
        account: "deposit_liability",
        amount_cents: statement.deposit_total_cents,
        memo: Some(format!("Settle deposit for case {}", statement.case_number)),
        occurred_at: now_unix,
    });

    // Credit: refund payable (negative), if any.
    if statement.refund_cents > 0 {
        entries.push(LedgerEntry {
            id: Uuid::new_v4(),
            tenant_id: statement.tenant_id,
            journal_id: journal,
            settlement_id: Some(statement.settlement_id),
            account: "refund_payable",
            amount_cents: -statement.refund_cents,
            memo: Some("Refund due to resident".into()),
            occurred_at: now_unix,
        });
    }

    // Credit: forfeited revenue (negative), if any.
    if statement.deductions_total_cents > 0 {
        entries.push(LedgerEntry {
            id: Uuid::new_v4(),
            tenant_id: statement.tenant_id,
            journal_id: journal,
            settlement_id: Some(statement.settlement_id),
            account: "forfeited_revenue",
            amount_cents: -statement.deductions_total_cents,
            memo: Some("Deductions retained".into()),
            occurred_at: now_unix,
        });
    }

    entries
}

fn render_check_request_html(
    check: &CheckRequest,
    payee_name: &str,
    statement: &SettlementStatement,
) -> String {
    let memo = check.memo.as_deref().unwrap_or("");
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>Check Request — {case}</title>
<style>
  body {{ font-family: 'Segoe UI', Arial, sans-serif; margin: 32px; color: #111; }}
  h1 {{ font-size: 18pt; }}
  .field {{ margin: 6px 0; }}
  .label {{ display: inline-block; width: 160px; color: #555; }}
  .amount {{ font-size: 14pt; font-weight: 600; }}
  .signoff {{ margin-top: 60px; border-top: 1px solid #ccc; padding-top: 8px; }}
</style></head>
<body>
  <h1>Check Request</h1>
  <div class="field"><span class="label">Case:</span> {case}</div>
  <div class="field"><span class="label">Payee:</span> {payee}</div>
  <div class="field"><span class="label">Amount:</span> <span class="amount">{amt}</span></div>
  <div class="field"><span class="label">Memo:</span> {memo}</div>
  <div class="signoff">Authorized signature: ____________________________</div>
  <div class="signoff">Date: ____________________________</div>
</body></html>"#,
        case = html_escape(&statement.case_number),
        payee = html_escape(payee_name),
        amt = html_escape(&statement.display.refund),
        memo = html_escape(memo),
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settlement::statement::{generate_statement, DeductionLine, DepositInput, StatementInputs};

    fn statement(refund: i64, ded: i64) -> SettlementStatement {
        // refund + ded must equal deposit
        let deposit = refund + ded;
        let inputs = StatementInputs {
            settlement_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            case_number: "MO-2026-0007".into(),
            resident_display_name: "Sam Tester".into(),
            unit_label: Some("12C".into()),
            move_out_date_unix: 0,
            generated_at_unix: 0,
            deposits: vec![DepositInput {
                id: Uuid::new_v4(),
                amount_cents: deposit,
                currency: "USD".into(),
                received_at_unix: 0,
            }],
            deductions: if ded > 0 {
                vec![DeductionLine {
                    id: Uuid::new_v4(),
                    category: "cleaning".into(),
                    description: "deep clean".into(),
                    amount_cents: ded,
                    evidence_count: 0,
                }]
            } else {
                vec![]
            },
        };
        generate_statement(inputs).unwrap()
    }

    #[test]
    fn ledger_is_balanced_full_refund() {
        let s = statement(100_000, 0);
        let entries = post_ledger_for_settlement(&s, 1);
        assert_eq!(entries.iter().map(|e| e.amount_cents).sum::<i64>(), 0);
    }

    #[test]
    fn ledger_is_balanced_partial() {
        let s = statement(60_000, 40_000);
        let entries = post_ledger_for_settlement(&s, 1);
        assert_eq!(entries.iter().map(|e| e.amount_cents).sum::<i64>(), 0);
        // Three entries: liability debit + refund credit + forfeited credit.
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn ledger_is_balanced_full_forfeit() {
        let s = statement(0, 100_000);
        let entries = post_ledger_for_settlement(&s, 1);
        assert_eq!(entries.iter().map(|e| e.amount_cents).sum::<i64>(), 0);
    }

    #[test]
    fn check_artifact_renders_self_contained() {
        let s = statement(75_000, 25_000);
        let check = CheckRequest {
            id: Uuid::new_v4(),
            tenant_id: s.tenant_id,
            settlement_id: s.settlement_id,
            payee_name: "Sam Tester".into(),
            amount_cents: 75_000,
            currency: "USD".into(),
            memo: Some("Deposit refund".into()),
            artifact_path_enc: None,
            status: "drafted",
            drafted_at: 1,
        };
        let html = render_check_request_html(&check, "Sam Tester", &s);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("$750.00"));
        assert!(!html.contains("http://"));
        assert!(!html.contains("https://"));
    }
}
