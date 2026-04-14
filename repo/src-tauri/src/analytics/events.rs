//! Event schema & tracking hook.
//!
//! Every analytics call funnels through `track_event`. The function:
//!   1. Validates the typed input (category, kind, optional funnel
//!      step, success / duration, optional A/B attribution).
//!   2. Sanity-checks payload size to keep the local DB tidy.
//!   3. Persists the row via `EventRepository::insert` AND increments
//!      the matching `daily_event_aggregates` bucket via
//!      `EventRepository::roll_up`. Both happen in the caller's
//!      transaction so dashboards never see a partial state.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventCategory {
    Impression,
    Click,
    Completion,
    Conversion,
}

impl EventCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventCategory::Impression => "impression",
            EventCategory::Click => "click",
            EventCategory::Completion => "completion",
            EventCategory::Conversion => "conversion",
        }
    }
}

const MAX_PAYLOAD_BYTES: usize = 8 * 1024;
const MAX_KIND_LEN: usize = 80;

#[derive(Debug, Clone, Deserialize)]
pub struct EventInput {
    pub tenant_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub category: EventCategory,
    /// Dotted action namespace, e.g. "case.opened", "parcel.delivered".
    pub kind: String,
    pub entity_kind: Option<String>,
    pub entity_id: Option<Uuid>,
    pub funnel: Option<String>,
    pub funnel_step: Option<u32>,
    pub duration_ms: Option<i64>,
    pub success: Option<bool>,
    /// JSON object string. Must NOT contain sensitive PII —
    /// security-sensitive context belongs in `audit_log`, not here.
    pub payload_json: Option<String>,
    pub experiment_id: Option<Uuid>,
    pub variant_id: Option<Uuid>,
    /// Caller's clock, Unix seconds UTC. None ⇒ server-side `now`.
    pub occurred_at_unix: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackedEvent {
    pub id: Uuid,
    pub category: EventCategory,
    pub kind: String,
    pub occurred_at_unix: i64,
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TrackError {
    #[error("kind must be 1..={limit} chars (was {got})", limit = MAX_KIND_LEN)]
    BadKind { got: usize },

    #[error("payload exceeds {limit} bytes (was {got})", limit = MAX_PAYLOAD_BYTES)]
    PayloadTooLarge { got: usize },

    #[error("payload is not a JSON object")]
    PayloadShape,

    #[error("funnel_step must be >= 1 when provided")]
    BadFunnelStep,

    #[error("duration_ms must be >= 0 when provided")]
    BadDuration,

    #[error("variant_id requires experiment_id (and vice versa)")]
    VariantWithoutExperiment,

    #[error("persistence error: {0}")]
    Persistence(String),
}

pub trait EventRepository {
    fn insert(&self, ev: &PersistableEvent) -> Result<(), String>;

    /// Increment the daily aggregate bucket for (tenant, day, category, kind).
    /// `success_increment` is 0 or 1; `duration_ms` may be 0.
    fn roll_up(
        &self,
        tenant_id: Option<&Uuid>,
        day_unix: i64,
        category: EventCategory,
        kind: &str,
        success_increment: i64,
        duration_ms: i64,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct PersistableEvent {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub category: EventCategory,
    pub kind: String,
    pub entity_kind: Option<String>,
    pub entity_id: Option<Uuid>,
    pub funnel: Option<String>,
    pub funnel_step: Option<u32>,
    pub duration_ms: Option<i64>,
    pub success: Option<bool>,
    pub payload_json: Option<String>,
    pub experiment_id: Option<Uuid>,
    pub variant_id: Option<Uuid>,
    pub occurred_at_unix: i64,
}

const DAY_SECONDS: i64 = 86_400;

fn day_bucket(unix: i64) -> i64 {
    // Floor to UTC midnight.
    unix - unix.rem_euclid(DAY_SECONDS)
}

/// Validate + persist + roll-up.
pub fn track_event<R: EventRepository>(
    repo: &R,
    input: EventInput,
    now_unix: i64,
) -> Result<TrackedEvent, TrackError> {
    if input.kind.is_empty() || input.kind.len() > MAX_KIND_LEN {
        return Err(TrackError::BadKind { got: input.kind.len() });
    }
    if let Some(p) = &input.payload_json {
        if p.len() > MAX_PAYLOAD_BYTES {
            return Err(TrackError::PayloadTooLarge { got: p.len() });
        }
        // Validate JSON shape: must parse and be an object.
        match serde_json::from_str::<serde_json::Value>(p) {
            Ok(serde_json::Value::Object(_)) => {}
            _ => return Err(TrackError::PayloadShape),
        }
    }
    if let Some(s) = input.funnel_step {
        if s == 0 {
            return Err(TrackError::BadFunnelStep);
        }
    }
    if let Some(d) = input.duration_ms {
        if d < 0 {
            return Err(TrackError::BadDuration);
        }
    }
    if input.experiment_id.is_some() ^ input.variant_id.is_some() {
        return Err(TrackError::VariantWithoutExperiment);
    }

    let occurred_at_unix = input.occurred_at_unix.unwrap_or(now_unix);
    let ev = PersistableEvent {
        id: Uuid::new_v4(),
        tenant_id: input.tenant_id,
        actor_user_id: input.actor_user_id,
        session_id: input.session_id,
        category: input.category,
        kind: input.kind.clone(),
        entity_kind: input.entity_kind,
        entity_id: input.entity_id,
        funnel: input.funnel,
        funnel_step: input.funnel_step,
        duration_ms: input.duration_ms,
        success: input.success,
        payload_json: input.payload_json,
        experiment_id: input.experiment_id,
        variant_id: input.variant_id,
        occurred_at_unix,
    };

    repo.insert(&ev).map_err(TrackError::Persistence)?;
    repo.roll_up(
        ev.tenant_id.as_ref(),
        day_bucket(occurred_at_unix),
        ev.category,
        &ev.kind,
        ev.success.map(|b| if b { 1 } else { 0 }).unwrap_or(0),
        ev.duration_ms.unwrap_or(0),
    )
    .map_err(TrackError::Persistence)?;

    Ok(TrackedEvent {
        id: ev.id,
        category: ev.category,
        kind: ev.kind,
        occurred_at_unix,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct MockRepo {
        events: RefCell<Vec<PersistableEvent>>,
        rollups: RefCell<Vec<(i64, EventCategory, String, i64, i64)>>,
    }
    impl EventRepository for MockRepo {
        fn insert(&self, ev: &PersistableEvent) -> Result<(), String> {
            self.events.borrow_mut().push(ev.clone());
            Ok(())
        }
        fn roll_up(
            &self,
            _t: Option<&Uuid>,
            day: i64,
            cat: EventCategory,
            kind: &str,
            succ: i64,
            dur: i64,
        ) -> Result<(), String> {
            self.rollups.borrow_mut().push((day, cat, kind.into(), succ, dur));
            Ok(())
        }
    }

    fn good_input() -> EventInput {
        EventInput {
            tenant_id: Some(Uuid::new_v4()),
            actor_user_id: Some(Uuid::new_v4()),
            session_id: Some(Uuid::new_v4()),
            category: EventCategory::Click,
            kind: "case.opened".into(),
            entity_kind: Some("case".into()),
            entity_id: Some(Uuid::new_v4()),
            funnel: Some("move_out".into()),
            funnel_step: Some(1),
            duration_ms: Some(120),
            success: Some(true),
            payload_json: Some(r#"{"source":"toolbar"}"#.into()),
            experiment_id: None,
            variant_id: None,
            occurred_at_unix: Some(1_700_000_000),
        }
    }

    #[test]
    fn happy_path_persists_and_rolls_up() {
        let repo = MockRepo::default();
        let ev = track_event(&repo, good_input(), 1_700_000_000).unwrap();
        assert_eq!(ev.category, EventCategory::Click);
        assert_eq!(repo.events.borrow().len(), 1);
        let r = repo.rollups.borrow();
        assert_eq!(r.len(), 1);
        // Day bucket is midnight UTC of the event time.
        assert_eq!(r[0].0 % DAY_SECONDS, 0);
        assert_eq!(r[0].3, 1); // success=true → 1
        assert_eq!(r[0].4, 120); // duration_ms
    }

    #[test]
    fn payload_must_be_object() {
        let repo = MockRepo::default();
        let mut i = good_input();
        i.payload_json = Some("[1,2,3]".into());
        assert!(matches!(
            track_event(&repo, i, 0).unwrap_err(),
            TrackError::PayloadShape
        ));
    }

    #[test]
    fn variant_without_experiment_rejected() {
        let repo = MockRepo::default();
        let mut i = good_input();
        i.variant_id = Some(Uuid::new_v4());
        assert!(matches!(
            track_event(&repo, i, 0).unwrap_err(),
            TrackError::VariantWithoutExperiment
        ));
    }

    #[test]
    fn empty_kind_rejected() {
        let repo = MockRepo::default();
        let mut i = good_input();
        i.kind = "".into();
        assert!(matches!(
            track_event(&repo, i, 0).unwrap_err(),
            TrackError::BadKind { .. }
        ));
    }

    #[test]
    fn day_bucket_floors_to_midnight() {
        assert_eq!(day_bucket(0), 0);
        assert_eq!(day_bucket(DAY_SECONDS - 1), 0);
        assert_eq!(day_bucket(DAY_SECONDS), DAY_SECONDS);
        assert_eq!(day_bucket(DAY_SECONDS + 7200), DAY_SECONDS);
    }
}
