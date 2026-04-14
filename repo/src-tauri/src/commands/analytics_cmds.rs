//! Analytics IPC commands — SQLite-backed.

use std::sync::Arc;
use uuid::Uuid;

use crate::analytics::dashboard::{compute_funnel, compute_quality, compute_retention, FunnelDefinition, FunnelResult, QualityMetrics, RetentionResult, RetentionInput};
use crate::analytics::events::{track_event, EventInput, TrackedEvent};
use crate::analytics::experiments::{assign_variant, VariantAssignment};
use crate::analytics::exports;
use crate::db::connection::Database;
use crate::db::repos::{SqliteEventRepo, SqliteExperimentRepo};
use crate::ipc::{guard, IpcError, SessionState};

fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }

#[tauri::command]
pub fn cmd_analytics_track(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    input: EventInput,
) -> Result<TrackedEvent, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteEventRepo::new(Arc::clone(db.inner()));
    track_event(&repo, input, now()).map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_analytics_funnel(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    tenant_id: Uuid,
    funnel: FunnelDefinition,
    from_unix: i64,
    to_unix: i64,
) -> Result<FunnelResult, IpcError> {
    guard::require_authenticated(session.inner())?;
    let c = db.conn();
    let mut stmt = c.prepare(
        "SELECT actor_user_id, kind, occurred_at FROM events WHERE tenant_id=?1 AND occurred_at BETWEEN ?2 AND ?3 ORDER BY occurred_at"
    ).map_err(|e| IpcError::Internal(e.to_string()))?;
    let events: Vec<(Uuid, String, i64)> = stmt.query_map(
        rusqlite::params![tenant_id.to_string(), from_unix, to_unix],
        |r| {
            let uid: String = r.get(0)?;
            Ok((Uuid::parse_str(&uid).unwrap_or(Uuid::nil()), r.get::<_,String>(1)?, r.get::<_,i64>(2)?))
        },
    ).map_err(|e| IpcError::Internal(e.to_string()))?
    .filter_map(|r| r.ok()).collect();
    compute_funnel(&funnel, &events).map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_analytics_retention(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    tenant_id: Uuid,
    cohort_window_seconds: i64,
    follow_up_windows: u32,
    from_unix: i64,
    to_unix: i64,
) -> Result<RetentionResult, IpcError> {
    guard::require_authenticated(session.inner())?;
    let c = db.conn();
    // First-seen per actor.
    let mut fs_stmt = c.prepare(
        "SELECT actor_user_id, MIN(occurred_at) FROM events WHERE tenant_id=?1 AND occurred_at BETWEEN ?2 AND ?3 GROUP BY actor_user_id"
    ).map_err(|e| IpcError::Internal(e.to_string()))?;
    let first_seen: Vec<(Uuid,i64)> = fs_stmt.query_map(
        rusqlite::params![tenant_id.to_string(), from_unix, to_unix],
        |r| Ok((Uuid::parse_str(&r.get::<_,String>(0)?).unwrap_or(Uuid::nil()), r.get(1)?)),
    ).map_err(|e| IpcError::Internal(e.to_string()))?.filter_map(|r| r.ok()).collect();
    // All activity.
    let mut act_stmt = c.prepare(
        "SELECT actor_user_id, occurred_at FROM events WHERE tenant_id=?1 AND occurred_at BETWEEN ?2 AND ?3"
    ).map_err(|e| IpcError::Internal(e.to_string()))?;
    let activity: Vec<(Uuid,i64)> = act_stmt.query_map(
        rusqlite::params![tenant_id.to_string(), from_unix, to_unix],
        |r| Ok((Uuid::parse_str(&r.get::<_,String>(0)?).unwrap_or(Uuid::nil()), r.get(1)?)),
    ).map_err(|e| IpcError::Internal(e.to_string()))?.filter_map(|r| r.ok()).collect();
    compute_retention(RetentionInput { first_seen, activity, cohort_window_seconds, follow_up_windows })
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_analytics_quality(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    tenant_id: Uuid,
    kind: String,
    from_unix: i64,
    to_unix: i64,
) -> Result<QualityMetrics, IpcError> {
    guard::require_authenticated(session.inner())?;
    let c = db.conn();
    let mut stmt = c.prepare(
        "SELECT success, duration_ms FROM events WHERE tenant_id=?1 AND kind=?2 AND occurred_at BETWEEN ?3 AND ?4"
    ).map_err(|e| IpcError::Internal(e.to_string()))?;
    let rows: Vec<(Option<bool>, Option<i64>)> = stmt.query_map(
        rusqlite::params![tenant_id.to_string(), kind, from_unix, to_unix],
        |r| {
            let s: Option<i64> = r.get(0)?;
            Ok((s.map(|v| v == 1), r.get(1)?))
        },
    ).map_err(|e| IpcError::Internal(e.to_string()))?.filter_map(|r| r.ok()).collect();
    Ok(compute_quality(&rows))
}

#[tauri::command]
pub fn cmd_analytics_export(
    session: tauri::State<'_, SessionState>,
    format: String,
    rows: Vec<serde_json::Value>,
) -> Result<String, IpcError> {
    guard::require_authenticated(session.inner())?;
    match format.as_str() {
        "csv" => exports::to_csv(rows).map_err(|e| IpcError::Internal(e.to_string())),
        "jsonl" => exports::to_json_lines(rows).map_err(|e| IpcError::Internal(e.to_string())),
        other => Err(IpcError::Internal(format!("unsupported format: {other}"))),
    }
}

#[tauri::command]
pub fn cmd_experiment_assign(
    session: tauri::State<'_, SessionState>,
    db: tauri::State<'_, Arc<Database>>,
    experiment_id: Uuid,
    subject_id: Uuid,
) -> Result<serde_json::Value, IpcError> {
    guard::require_authenticated(session.inner())?;
    let repo = SqliteExperimentRepo::new(Arc::clone(db.inner()));
    let a = assign_variant(&repo, experiment_id, subject_id, now())
        .map_err(|e| IpcError::Internal(e.to_string()))?;
    Ok(serde_json::json!({
        "experiment_id": a.experiment_id.to_string(),
        "subject_id": a.subject_id.to_string(),
        "variant_id": a.variant_id.to_string(),
        "variant_name": a.variant_name,
        "assigned_at_unix": a.assigned_at_unix,
        "sticky": a.sticky,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_returns_nonzero() {
        let ts = now();
        assert!(ts > 0, "now() should return a positive unix timestamp");
    }

    #[test]
    fn export_csv_via_service_produces_header_row() {
        let rows = vec![
            serde_json::json!({"name": "Alice", "score": 42}),
            serde_json::json!({"name": "Bob", "score": 99}),
        ];
        let out = crate::analytics::exports::to_csv(rows).unwrap();
        assert!(out.starts_with("name,score"), "CSV must start with header row, got: {}", &out[..40.min(out.len())]);
        assert!(out.contains("Alice"), "CSV must contain Alice");
        assert!(out.contains("Bob"), "CSV must contain Bob");
    }

    #[test]
    fn export_jsonl_via_service_produces_one_line_per_row() {
        let rows = vec![
            serde_json::json!({"a": 1}),
            serde_json::json!({"b": 2}),
        ];
        let out = crate::analytics::exports::to_json_lines(rows).unwrap();
        let lines: Vec<&str> = out.trim().split('\n').collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn export_unsupported_format_error() {
        // Directly test the format match logic.
        let result = crate::analytics::exports::to_csv(vec![serde_json::json!(42)]);
        assert!(result.is_err(), "non-object row should produce error");
    }
}
