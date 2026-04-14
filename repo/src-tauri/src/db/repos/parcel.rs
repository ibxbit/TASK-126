//! SQLite-backed parcel + transition repositories.

use std::sync::{Arc, MutexGuard};

use rusqlite::Connection;
use uuid::Uuid;

use crate::db::connection::Database;
use crate::parcel::state::ParcelState;
use crate::parcel::transition::{ParcelRepository, TransitionRecord, TransitionRepository};

pub struct SqliteParcelRepo {
    db: Arc<Database>,
}

impl SqliteParcelRepo {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
    fn conn(&self) -> MutexGuard<'_, Connection> { self.db.conn() }
}

impl ParcelRepository for SqliteParcelRepo {
    fn current_state(&self, parcel_id: &Uuid) -> Result<Option<ParcelState>, String> {
        let c = self.conn();
        let mut stmt = c
            .prepare("SELECT status FROM parcels WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        let result = stmt
            .query_row([parcel_id.to_string()], |row| {
                let s: String = row.get(0)?;
                Ok(s)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(result.and_then(|s| ParcelState::from_str(&s)))
    }

    fn parcel_tenant(&self, parcel_id: &Uuid) -> Result<Option<Uuid>, String> {
        let c = self.conn();
        let mut stmt = c
            .prepare("SELECT tenant_id FROM parcels WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        let result = stmt
            .query_row([parcel_id.to_string()], |row| {
                let s: String = row.get(0)?;
                Ok(s)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        match result {
            Some(s) => Uuid::parse_str(&s).map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    }

    fn has_check_in_record(&self, parcel_id: &Uuid) -> Result<bool, String> {
        let c = self.conn();
        c.query_row(
            "SELECT COUNT(*) > 0 FROM parcel_transitions WHERE parcel_id = ?1 AND to_state = 'checked_in'",
            [parcel_id.to_string()],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    }

    fn update_state(&self, parcel_id: &Uuid, new_state: ParcelState) -> Result<(), String> {
        let c = self.conn();
        let now = now_unix();
        c.execute(
            "UPDATE parcels SET status = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![new_state.as_str(), now, parcel_id.to_string()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub struct SqliteTransitionRepo {
    db: Arc<Database>,
}

impl SqliteTransitionRepo {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
    fn conn(&self) -> MutexGuard<'_, Connection> { self.db.conn() }
}

impl TransitionRepository for SqliteTransitionRepo {
    fn last_chain_hash(&self, parcel_id: &Uuid) -> Result<Option<String>, String> {
        let c = self.conn();
        c.query_row(
            "SELECT chain_hash FROM parcel_transitions WHERE parcel_id = ?1 ORDER BY occurred_at DESC LIMIT 1",
            [parcel_id.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    }

    fn append(&self, r: &TransitionRecord) -> Result<(), String> {
        let c = self.conn();
        c.execute(
            "INSERT INTO parcel_transitions (id, tenant_id, parcel_id, from_state, to_state,
             operator_user_id, occurred_at, location, notes_enc, prev_chain_hash, chain_hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            rusqlite::params![
                r.id.to_string(),
                r.tenant_id.to_string(),
                r.parcel_id.to_string(),
                r.from_state.map(|s| s.as_str().to_string()),
                r.to_state.as_str(),
                r.operator_user_id.to_string(),
                r.occurred_at_unix as i64,
                r.location,
                r.notes_enc,
                r.prev_chain_hash,
                r.chain_hash,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn history(&self, parcel_id: &Uuid) -> Result<Vec<TransitionRecord>, String> {
        let c = self.conn();
        let mut stmt = c
            .prepare(
                "SELECT id, tenant_id, parcel_id, from_state, to_state,
                 operator_user_id, occurred_at, location, notes_enc,
                 prev_chain_hash, chain_hash
                 FROM parcel_transitions WHERE parcel_id = ?1 ORDER BY occurred_at ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([parcel_id.to_string()], |row| {
                let from_str: Option<String> = row.get(3)?;
                Ok(TransitionRecord {
                    id: parse_uuid(row.get::<_, String>(0)?),
                    tenant_id: parse_uuid(row.get::<_, String>(1)?),
                    parcel_id: parse_uuid(row.get::<_, String>(2)?),
                    from_state: from_str.and_then(|s| ParcelState::from_str(&s)),
                    to_state: ParcelState::from_str(&row.get::<_, String>(4)?).unwrap_or(ParcelState::CheckedIn),
                    operator_user_id: parse_uuid(row.get::<_, String>(5)?),
                    occurred_at_unix: row.get::<_, i64>(6)? as u64,
                    location: row.get(7)?,
                    notes_enc: row.get(8)?,
                    prev_chain_hash: row.get(9)?,
                    chain_hash: row.get(10)?,
                })
            })
            .map_err(|e| e.to_string())?;
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

use rusqlite::OptionalExtension;
