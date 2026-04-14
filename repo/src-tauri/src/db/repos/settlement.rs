//! SQLite-backed settlement repositories.

use std::sync::{Arc, MutexGuard};

use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

use crate::auth::Principal;
use crate::db::connection::Database;
use crate::settlement::approval::{ApprovalRecord, ApprovalRepository, ApprovalStep};
use crate::settlement::workflow::{SettlementRepository, SettlementStatus, SettlementView};

pub struct SqliteSettlementRepo {
    db: Arc<Database>,
}

impl SqliteSettlementRepo {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
    fn conn(&self) -> MutexGuard<'_, Connection> { self.db.conn() }
}

impl SettlementRepository for SqliteSettlementRepo {
    fn load(&self, settlement_id: &Uuid) -> Result<Option<SettlementView>, String> {
        let c = self.conn();
        c.query_row(
            "SELECT id, tenant_id, status FROM settlements WHERE id = ?1",
            [settlement_id.to_string()],
            |row| {
                let status_str: String = row.get(2)?;
                let status = match status_str.as_str() {
                    "draft" => SettlementStatus::Draft,
                    "pending_approval" => SettlementStatus::PendingApproval,
                    "approved" => SettlementStatus::Approved,
                    "paid" => SettlementStatus::Paid,
                    "reopened" => SettlementStatus::Reopened,
                    "void" => SettlementStatus::Void,
                    _ => SettlementStatus::Draft,
                };
                Ok(SettlementView {
                    settlement_id: parse_uuid(row.get::<_, String>(0)?),
                    tenant_id: parse_uuid(row.get::<_, String>(1)?),
                    status,
                    prepared_by: None,
                    approved_by: None,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|opt| {
            opt.map(|mut v| {
                // Hydrate approval signers.
                if let Ok(Some(p)) = load_signer(&c, settlement_id, "prepared") {
                    v.prepared_by = Some(p);
                }
                if let Ok(Some(a)) = load_signer(&c, settlement_id, "approved") {
                    v.approved_by = Some(a);
                }
                v
            })
        })
    }

    fn set_status(
        &self,
        settlement_id: &Uuid,
        new_status: SettlementStatus,
        _actor: &Principal,
        now_unix: i64,
    ) -> Result<(), String> {
        let c = self.conn();
        c.execute(
            "UPDATE settlements SET status = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![new_status.as_str(), now_unix, settlement_id.to_string()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn load_signer(conn: &Connection, sid: &Uuid, step: &str) -> Result<Option<Uuid>, String> {
    conn.query_row(
        "SELECT user_id FROM settlement_approvals WHERE settlement_id = ?1 AND step = ?2",
        rusqlite::params![sid.to_string(), step],
        |row| {
            let s: String = row.get(0)?;
            Ok(parse_uuid(s))
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub struct SqliteApprovalRepo {
    db: Arc<Database>,
}

impl SqliteApprovalRepo {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
    fn conn(&self) -> MutexGuard<'_, Connection> { self.db.conn() }
}

impl ApprovalRepository for SqliteApprovalRepo {
    fn insert(&self, record: &ApprovalRecord, notes_enc: Option<&[u8]>) -> Result<(), String> {
        let c = self.conn();
        c.execute(
            "INSERT INTO settlement_approvals (id, settlement_id, step, user_id, signed_at, notes_enc)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                record.id.to_string(),
                record.settlement_id.to_string(),
                record.step.as_str(),
                record.user_id.to_string(),
                record.signed_at,
                notes_enc,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn fetch(&self, settlement_id: &Uuid, step: ApprovalStep) -> Result<Option<ApprovalRecord>, String> {
        let c = self.conn();
        c.query_row(
            "SELECT id, settlement_id, step, user_id, signed_at
             FROM settlement_approvals WHERE settlement_id = ?1 AND step = ?2",
            rusqlite::params![settlement_id.to_string(), step.as_str()],
            |row| {
                Ok(ApprovalRecord {
                    id: parse_uuid(row.get::<_, String>(0)?),
                    settlement_id: parse_uuid(row.get::<_, String>(1)?),
                    step,
                    user_id: parse_uuid(row.get::<_, String>(3)?),
                    signed_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    }
}

fn parse_uuid(s: String) -> Uuid {
    Uuid::parse_str(&s).unwrap_or(Uuid::nil())
}
