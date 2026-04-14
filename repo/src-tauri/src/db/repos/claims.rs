//! SQLite-backed claim repositories.

use std::sync::{Arc, MutexGuard};

use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

use crate::auth::Principal;
use crate::claims::machine::{ClaimRepository, ClaimView};
use crate::claims::state::{ClaimStatus, PartyResponse, PartyRole};
use crate::claims::timeout::ExpiredClaimFinder;
use crate::db::connection::Database;

pub struct SqliteClaimRepo {
    db: Arc<Database>,
}

impl SqliteClaimRepo {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
    fn conn(&self) -> MutexGuard<'_, Connection> { self.db.conn() }
}

impl ClaimRepository for SqliteClaimRepo {
    fn load(&self, claim_id: &Uuid) -> Result<Option<ClaimView>, String> {
        let c = self.conn();
        let id_str = claim_id.to_string();
        c.query_row(
            "SELECT id, tenant_id, claimant_user_id, respondent_user_id,
                    status, reopened_count, response_deadline_unix
             FROM claims WHERE id = ?1",
            [&id_str],
            |row| {
                let status_str: String = row.get(4)?;
                Ok(ClaimView {
                    claim_id: parse_uuid(row.get::<_, String>(0)?),
                    tenant_id: parse_uuid(row.get::<_, String>(1)?),
                    claimant_user_id: parse_uuid(row.get::<_, String>(2)?),
                    respondent_user_id: row.get::<_, Option<String>>(3)?.map(parse_uuid),
                    status: ClaimStatus::from_str(&status_str).unwrap_or(ClaimStatus::Draft),
                    reopened_count: row.get::<_, i64>(5)? as u32,
                    claimant_response: None,
                    respondent_response: None,
                    response_deadline_unix: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
        .map(|opt| {
            opt.map(|mut v| {
                // Load party responses.
                if let Ok(resp) = load_party_response(&c, &id_str, "claimant") {
                    v.claimant_response = resp;
                }
                if let Ok(resp) = load_party_response(&c, &id_str, "respondent") {
                    v.respondent_response = resp;
                }
                v
            })
        })
    }

    fn apply_status(
        &self,
        claim_id: &Uuid,
        new_status: ClaimStatus,
        _event: &str,
        _actor: &Principal,
    ) -> Result<(), String> {
        let c = self.conn();
        let now = now_unix();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(new_status.as_str().to_string()),
            Box::new(now),
        ];

        let mut sql = "UPDATE claims SET status = ?1, updated_at = ?2".to_string();

        if new_status == ClaimStatus::Submitted {
            sql.push_str(", submitted_at = ?3, response_deadline_unix = ?4");
            params.push(Box::new(now));
            params.push(Box::new(now + 72 * 3600));
        }
        if new_status.is_terminal() {
            let idx = params.len() + 1;
            sql.push_str(&format!(", closed_at = ?{idx}"));
            params.push(Box::new(now));
        }
        if new_status == ClaimStatus::Reopened {
            let idx = params.len() + 1;
            sql.push_str(&format!(", reopened_count = reopened_count + 1, closed_at = NULL, response_deadline_unix = NULL"));
            // No new param needed for the expression.
            let _ = idx;
        }

        let idx = params.len() + 1;
        sql.push_str(&format!(" WHERE id = ?{idx}"));
        params.push(Box::new(claim_id.to_string()));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        c.execute(&sql, param_refs.as_slice()).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn record_response(
        &self,
        claim_id: &Uuid,
        party: PartyRole,
        response: PartyResponse,
        actor_user_id: &Uuid,
    ) -> Result<(), String> {
        let c = self.conn();
        let now = now_unix();
        let party_str = match party { PartyRole::Claimant => "claimant", PartyRole::Respondent => "respondent" };
        let resp_str = match response { PartyResponse::Accept => "accept", PartyResponse::Reject => "reject" };
        c.execute(
            "INSERT OR REPLACE INTO claim_party_responses (id, claim_id, party_role, user_id, response, responded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                Uuid::new_v4().to_string(),
                claim_id.to_string(),
                party_str,
                actor_user_id.to_string(),
                resp_str,
                now,
            ],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn record_reopen(
        &self,
        claim_id: &Uuid,
        requested_by: &Uuid,
        approved_by: &Uuid,
    ) -> Result<(), String> {
        let c = self.conn();
        let now = now_unix();
        c.execute(
            "INSERT INTO claim_reopens (id, claim_id, requested_by, approved_by, approved_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                Uuid::new_v4().to_string(),
                claim_id.to_string(),
                requested_by.to_string(),
                approved_by.to_string(),
                now,
            ],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn load_party_response(
    conn: &Connection,
    claim_id: &str,
    party_role: &str,
) -> Result<Option<PartyResponse>, String> {
    conn.query_row(
        "SELECT response FROM claim_party_responses WHERE claim_id = ?1 AND party_role = ?2",
        rusqlite::params![claim_id, party_role],
        |row| {
            let s: String = row.get(0)?;
            Ok(match s.as_str() {
                "accept" => Some(PartyResponse::Accept),
                "reject" => Some(PartyResponse::Reject),
                _ => None,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
    .map(|opt| opt.flatten())
}

pub struct SqliteExpiredClaimFinder {
    db: Arc<Database>,
}

impl SqliteExpiredClaimFinder {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
}

impl ExpiredClaimFinder for SqliteExpiredClaimFinder {
    fn find_expired(&self, now_unix: i64) -> Result<Vec<crate::claims::timeout::ExpiredClaim>, String> {
        let c = self.db.conn();
        let mut stmt = c.prepare(
            "SELECT id, tenant_id FROM claims
             WHERE status IN ('submitted','under_review')
             AND response_deadline_unix IS NOT NULL
             AND response_deadline_unix <= ?1"
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map([now_unix], |row| {
            Ok(crate::claims::timeout::ExpiredClaim {
                claim_id: parse_uuid(row.get::<_, String>(0)?),
                tenant_id: parse_uuid(row.get::<_, String>(1)?),
            })
        }).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }
}

fn parse_uuid(s: String) -> Uuid {
    Uuid::parse_str(&s).unwrap_or(Uuid::nil())
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
