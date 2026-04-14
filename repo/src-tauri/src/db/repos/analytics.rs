//! SQLite repos for analytics events + A/B experiments.

use std::sync::Arc;
use rusqlite::OptionalExtension;
use uuid::Uuid;

use crate::analytics::events::{EventCategory, EventRepository, PersistableEvent};
use crate::analytics::experiments::{Experiment, ExperimentRepository, Variant};
use crate::db::connection::Database;

pub struct SqliteEventRepo { db: Arc<Database> }
impl SqliteEventRepo {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
}
impl EventRepository for SqliteEventRepo {
    fn insert(&self, ev: &PersistableEvent) -> Result<(), String> {
        let c = self.db.conn();
        c.execute(
            "INSERT INTO events (id,tenant_id,actor_user_id,session_id,kind,entity_kind,entity_id,
             payload_json,occurred_at,category,funnel,funnel_step,duration_ms,success,experiment_id,variant_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            rusqlite::params![
                ev.id.to_string(),
                ev.tenant_id.map(|u| u.to_string()),
                ev.actor_user_id.map(|u| u.to_string()),
                ev.session_id.map(|u| u.to_string()),
                ev.kind,
                ev.entity_kind,
                ev.entity_id.map(|u| u.to_string()),
                ev.payload_json,
                ev.occurred_at_unix,
                ev.category.as_str(),
                ev.funnel,
                ev.funnel_step.map(|v| v as i64),
                ev.duration_ms,
                ev.success.map(|b| if b { 1i64 } else { 0 }),
                ev.experiment_id.map(|u| u.to_string()),
                ev.variant_id.map(|u| u.to_string()),
            ],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn roll_up(&self, tenant_id: Option<&Uuid>, day: i64, cat: EventCategory, kind: &str, succ: i64, dur: i64) -> Result<(), String> {
        let c = self.db.conn();
        c.execute(
            "INSERT INTO daily_event_aggregates (tenant_id,day_unix,category,kind,count_total,count_success,sum_duration_ms)
             VALUES (?1,?2,?3,?4,1,?5,?6)
             ON CONFLICT(tenant_id,day_unix,category,kind) DO UPDATE SET
             count_total=count_total+1, count_success=count_success+?5, sum_duration_ms=sum_duration_ms+?6",
            rusqlite::params![tenant_id.map(|u| u.to_string()), day, cat.as_str(), kind, succ, dur],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub struct SqliteExperimentRepo { db: Arc<Database> }
impl SqliteExperimentRepo {
    pub fn new(db: Arc<Database>) -> Self { Self { db } }
}
impl ExperimentRepository for SqliteExperimentRepo {
    fn load_experiment(&self, id: &Uuid) -> Result<Option<Experiment>, String> {
        let c = self.db.conn();
        c.query_row("SELECT id,tenant_id,name,start_at_unix,end_at_unix,enabled FROM experiments WHERE id=?1",
            [id.to_string()], |r| Ok(Experiment {
                id: pu(r.get::<_,String>(0)?), tenant_id: pu(r.get::<_,String>(1)?),
                name: r.get(2)?, start_at_unix: r.get(3)?, end_at_unix: r.get(4)?,
                enabled: r.get::<_,i64>(5)? == 1,
            })
        ).optional().map_err(|e| e.to_string())
    }
    fn load_variants(&self, exp_id: &Uuid) -> Result<Vec<Variant>, String> {
        let c = self.db.conn();
        let mut s = c.prepare("SELECT id,experiment_id,name,weight_bp FROM experiment_variants WHERE experiment_id=?1")
            .map_err(|e| e.to_string())?;
        let rows = s.query_map([exp_id.to_string()], |r| Ok(Variant {
            id: pu(r.get::<_,String>(0)?), experiment_id: pu(r.get::<_,String>(1)?),
            name: r.get(2)?, weight_bp: r.get::<_,i64>(3)? as u32,
        })).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())
    }
    fn load_assignment(&self, exp_id: &Uuid, sub_id: &Uuid) -> Result<Option<Uuid>, String> {
        let c = self.db.conn();
        c.query_row("SELECT variant_id FROM experiment_assignments WHERE experiment_id=?1 AND subject_id=?2",
            rusqlite::params![exp_id.to_string(), sub_id.to_string()],
            |r| { let s: String = r.get(0)?; Ok(pu(s)) },
        ).optional().map_err(|e| e.to_string())
    }
    fn record_assignment(&self, exp_id: &Uuid, sub_id: &Uuid, var_id: &Uuid, now: i64) -> Result<(), String> {
        let c = self.db.conn();
        c.execute("INSERT INTO experiment_assignments (experiment_id,subject_id,variant_id,assigned_at) VALUES (?1,?2,?3,?4)",
            rusqlite::params![exp_id.to_string(), sub_id.to_string(), var_id.to_string(), now],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn pu(s: String) -> Uuid { Uuid::parse_str(&s).unwrap_or(Uuid::nil()) }
