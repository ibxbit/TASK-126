//! Settlement IPC commands — SQLite-backed.

use std::sync::Arc;
use uuid::Uuid;

use crate::db::connection::Database;
use crate::db::repos::{SqliteApprovalRepo, SqliteSettlementRepo};
use crate::ipc::{guard, IpcError, SessionState};
use crate::settlement::approval::{approve_settlement, prepare_settlement, ApprovalRecord};
use crate::settlement::statement::{
    generate_statement, render_statement_html, SettlementStatement, StatementInputs,
};
use crate::settlement::workflow::{apply_event, SettlementEvent, SettlementStatus};

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[tauri::command]
pub fn cmd_settlement_transition(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    settlement_id: Uuid,
    event: SettlementEvent,
) -> Result<SettlementStatus, IpcError> {
    let principal = guard::require_authenticated(session.inner())?;
    let repo = SqliteSettlementRepo::new(Arc::clone(db.inner()));
    apply_event(&repo, &principal, &settlement_id, event, now_unix())
        .map_err(|e| IpcError::Internal(format!("{e:?}")))
}

#[tauri::command]
pub fn cmd_settlement_prepare(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    settlement_id: Uuid,
    notes: Option<String>,
) -> Result<ApprovalRecord, IpcError> {
    let principal = guard::require_authenticated(session.inner())?;
    let s_repo = SqliteSettlementRepo::new(Arc::clone(db.inner()));
    let a_repo = SqliteApprovalRepo::new(Arc::clone(db.inner()));
    prepare_settlement(
        &s_repo, &a_repo, &principal, settlement_id,
        notes.map(|s| s.into_bytes()), now_unix(),
    )
    .map_err(|e| IpcError::Internal(format!("{e:?}")))
}

#[tauri::command]
pub fn cmd_settlement_approve(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    settlement_id: Uuid,
    notes: Option<String>,
) -> Result<ApprovalRecord, IpcError> {
    let principal = guard::require_authenticated(session.inner())?;
    let s_repo = SqliteSettlementRepo::new(Arc::clone(db.inner()));
    let a_repo = SqliteApprovalRepo::new(Arc::clone(db.inner()));
    approve_settlement(
        &s_repo, &a_repo, &principal, settlement_id,
        notes.map(|s| s.into_bytes()), now_unix(),
    )
    .map_err(|e| IpcError::Internal(format!("{e:?}")))
}

#[tauri::command]
pub fn cmd_settlement_statement(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    settlement_id: Uuid,
) -> Result<SettlementStatement, IpcError> {
    guard::require_authenticated(session.inner())?;
    let inputs = hydrate_statement_inputs(db.inner(), &settlement_id)?;
    generate_statement(inputs).map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_settlement_statement_html(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    settlement_id: Uuid,
) -> Result<String, IpcError> {
    guard::require_authenticated(session.inner())?;
    let inputs = hydrate_statement_inputs(db.inner(), &settlement_id)?;
    let stmt = generate_statement(inputs).map_err(|e| IpcError::Internal(e.to_string()))?;
    Ok(render_statement_html(&stmt))
}

#[tauri::command]
pub fn cmd_settlement_check_request(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    settlement_id: Uuid,
    payee_name: String,
    _memo: Option<String>,
) -> Result<serde_json::Value, IpcError> {
    guard::require_authenticated(session.inner())?;
    let inputs = hydrate_statement_inputs(db.inner(), &settlement_id)?;
    let stmt = generate_statement(inputs).map_err(|e| IpcError::Internal(e.to_string()))?;
    Ok(serde_json::json!({
        "settlement_id": settlement_id.to_string(),
        "payee_name": payee_name,
        "refund_cents": stmt.refund_cents,
        "html": render_statement_html(&stmt),
    }))
}

fn hydrate_statement_inputs(
    db: &Arc<Database>,
    settlement_id: &Uuid,
) -> Result<StatementInputs, IpcError> {
    use crate::settlement::statement::{DeductionLine, DepositInput};
    let conn = db.conn();
    let sid = settlement_id.to_string();

    let (tenant_id_str, case_id_str): (String, String) = conn
        .query_row(
            "SELECT tenant_id, case_id FROM settlements WHERE id = ?1",
            [&sid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| IpcError::Internal(e.to_string()))?;

    let (case_number, resident_name, unit_label, move_out_date): (String, String, Option<String>, i64) = conn
        .query_row(
            "SELECT c.case_number, r.full_name, r.unit_label, c.move_out_date
             FROM move_out_cases c JOIN residents r ON r.id = c.resident_id
             WHERE c.id = ?1",
            [&case_id_str],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|e| IpcError::Internal(e.to_string()))?;

    let mut dep_stmt = conn
        .prepare("SELECT id, amount_cents, currency, received_at FROM deposits WHERE case_id = ?1")
        .map_err(|e| IpcError::Internal(e.to_string()))?;
    let deposits: Vec<DepositInput> = dep_stmt
        .query_map([&case_id_str], |r| Ok(DepositInput {
            id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or(Uuid::nil()),
            amount_cents: r.get(1)?,
            currency: r.get(2)?,
            received_at_unix: r.get(3)?,
        }))
        .map_err(|e| IpcError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let mut ded_stmt = conn
        .prepare(
            "SELECT di.id, di.category, di.description, di.amount_cents,
                    (SELECT COUNT(*) FROM deduction_evidence de WHERE de.deduction_item_id = di.id)
             FROM deduction_items di WHERE di.settlement_id = ?1",
        )
        .map_err(|e| IpcError::Internal(e.to_string()))?;
    let deductions: Vec<DeductionLine> = ded_stmt
        .query_map([&sid], |r| Ok(DeductionLine {
            id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or(Uuid::nil()),
            category: r.get(1)?,
            description: r.get(2)?,
            amount_cents: r.get(3)?,
            evidence_count: r.get::<_, i64>(4)? as u32,
        }))
        .map_err(|e| IpcError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(StatementInputs {
        settlement_id: *settlement_id,
        tenant_id: Uuid::parse_str(&tenant_id_str).unwrap_or(Uuid::nil()),
        case_number,
        resident_display_name: resident_name,
        unit_label,
        move_out_date_unix: move_out_date,
        deposits,
        deductions,
        generated_at_unix: now_unix(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settlement::statement::DepositInput;

    #[test]
    fn now_unix_returns_positive() {
        assert!(now_unix() > 0);
    }

    #[test]
    fn generate_statement_computes_refund() {
        let inputs = StatementInputs {
            settlement_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            case_number: "MO-001".into(),
            resident_display_name: "Jane Doe".into(),
            unit_label: Some("Unit 4B".into()),
            move_out_date_unix: 1_700_000_000,
            deposits: vec![DepositInput {
                id: Uuid::new_v4(),
                amount_cents: 2000_00,
                currency: "USD".into(),
                received_at_unix: 1_699_900_000,
            }],
            deductions: vec![
                crate::settlement::statement::DeductionLine {
                    id: Uuid::new_v4(),
                    category: "cleaning".into(),
                    description: "Deep clean".into(),
                    amount_cents: 500_00,
                    evidence_count: 2,
                },
            ],
            generated_at_unix: 1_700_000_000,
        };
        let stmt = generate_statement(inputs).unwrap();
        // Refund = deposits - deductions = 2000.00 - 500.00 = 1500.00
        assert_eq!(stmt.refund_cents, 1500_00);
    }

    #[test]
    fn statement_html_escapes_user_text() {
        let inputs = StatementInputs {
            settlement_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            case_number: "MO-XSS".into(),
            resident_display_name: "<script>alert('xss')</script>".into(),
            unit_label: Some("4B".into()),
            move_out_date_unix: 1_700_000_000,
            deposits: vec![DepositInput {
                id: Uuid::new_v4(),
                amount_cents: 100,
                currency: "USD".into(),
                received_at_unix: 1_700_000_000,
            }],
            deductions: vec![],
            generated_at_unix: 1_700_000_000,
        };
        let stmt = generate_statement(inputs).unwrap();
        let html = render_statement_html(&stmt);
        assert!(!html.contains("<script>"), "HTML must escape user text to prevent XSS");
    }
}
