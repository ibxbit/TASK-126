//! SQLite repo for scheduling rule sets.

use std::sync::Arc;
use rusqlite::OptionalExtension;
use uuid::Uuid;

use crate::db::connection::Database;
use crate::scheduling::constraints::Severity;
use crate::scheduling::rules::{Rule, RuleKind, RuleRepository, RuleSet, RuleSpec, RuleSetError};

pub struct SqliteRuleRepo { db: Arc<Database> }
impl SqliteRuleRepo { pub fn new(db: Arc<Database>) -> Self { Self { db } } }

impl RuleRepository for SqliteRuleRepo {
    fn load_active(&self, tenant_id: &Uuid, name: &str) -> Result<Option<RuleSet>, String> {
        let c = self.db.conn();
        let rs_opt: Option<(String, String, i64, Option<String>)> = c.query_row(
            "SELECT id,name,version,parent_rule_set_id FROM schedule_rule_sets WHERE tenant_id=?1 AND name=?2 AND enabled=1",
            rusqlite::params![tenant_id.to_string(), name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).optional().map_err(|e| e.to_string())?;
        let Some((id_str, nm, ver, parent)) = rs_opt else { return Ok(None) };
        let rs_id = pu(&id_str);
        let rules = load_rules(&c, &id_str)?;
        Ok(Some(RuleSet { id: rs_id, tenant_id: *tenant_id, name: nm, version: ver as u32,
            parent_rule_set_id: parent.map(|s| pu(&s)), enabled: true, rules }))
    }
    fn load_by_id(&self, id: &Uuid) -> Result<Option<RuleSet>, String> {
        let c = self.db.conn();
        let rs_opt: Option<(String, String, i64, Option<String>, i64)> = c.query_row(
            "SELECT tenant_id,name,version,parent_rule_set_id,enabled FROM schedule_rule_sets WHERE id=?1",
            [id.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).optional().map_err(|e| e.to_string())?;
        let Some((tid,nm,ver,parent,en)) = rs_opt else { return Ok(None) };
        let rules = load_rules(&c, &id.to_string())?;
        Ok(Some(RuleSet { id: *id, tenant_id: pu(&tid), name: nm, version: ver as u32,
            parent_rule_set_id: parent.map(|s| pu(&s)), enabled: en==1, rules }))
    }
    fn activate(&self, new_id: &Uuid) -> Result<(), String> {
        let c = self.db.conn();
        // Get tenant + name for the target.
        let (tid, nm): (String, String) = c.query_row(
            "SELECT tenant_id, name FROM schedule_rule_sets WHERE id=?1", [new_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|e| e.to_string())?;
        c.execute("UPDATE schedule_rule_sets SET enabled=0 WHERE tenant_id=?1 AND name=?2 AND enabled=1",
            rusqlite::params![tid, nm]).map_err(|e| e.to_string())?;
        c.execute("UPDATE schedule_rule_sets SET enabled=1, updated_at=?1 WHERE id=?2",
            rusqlite::params![now(), new_id.to_string()]).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn deactivate_all(&self, tenant_id: &Uuid, name: &str) -> Result<(), String> {
        let c = self.db.conn();
        c.execute("UPDATE schedule_rule_sets SET enabled=0 WHERE tenant_id=?1 AND name=?2",
            rusqlite::params![tenant_id.to_string(), name]).map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn load_rules(c: &rusqlite::Connection, rs_id: &str) -> Result<Vec<Rule>, String> {
    let mut s = c.prepare(
        "SELECT id,kind,severity,spec_json,weight,enabled FROM schedule_rules WHERE rule_set_id=?1"
    ).map_err(|e| e.to_string())?;
    let rows = s.query_map([rs_id], |r| {
        let kind_s: String = r.get(1)?;
        let sev_s: String = r.get(2)?;
        let spec_json: String = r.get(3)?;
        Ok((r.get::<_,String>(0)?, kind_s, sev_s, spec_json, r.get::<_,i64>(4)?, r.get::<_,i64>(5)?))
    }).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id_s, kind_s, sev_s, spec_json, weight, enabled) = row.map_err(|e| e.to_string())?;
        let kind = match kind_s.as_str() {
            "unavailable_window" => RuleKind::UnavailableWindow,
            "capacity_limit" => RuleKind::CapacityLimit,
            "required_duration" => RuleKind::RequiredDuration,
            "distribution" => RuleKind::Distribution,
            other => return Err(format!("unknown rule kind: {other}")),
        };
        let severity = match sev_s.as_str() {
            "hard" => Severity::Hard, _ => Severity::Soft,
        };
        let spec: RuleSpec = serde_json::from_str(&spec_json).map_err(|e| e.to_string())?;
        out.push(Rule { id: pu(&id_s), rule_set_id: pu(rs_id), kind, severity, spec, weight: weight as u32, enabled: enabled==1 });
    }
    Ok(out)
}

fn pu(s: &str) -> Uuid { Uuid::parse_str(s).unwrap_or(Uuid::nil()) }
fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }
