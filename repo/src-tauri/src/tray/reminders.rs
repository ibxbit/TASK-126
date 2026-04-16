//! Background timer for local reminders.
//!
//! Fully offline: one ticker thread inspects an in-memory min-heap of
//! scheduled reminders once per second and emits a `reminder://fired`
//! event to the main window when any are due. Reminders are expected
//! to also be persisted by the caller so they survive restarts.

use std::collections::BinaryHeap;
use std::cmp::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::ipc::{guard, IpcError, SessionState};

pub const EVENT_REMINDER_FIRED: &str = "reminder://fired";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id: Uuid,
    pub title: String,
    pub body: Option<String>,
    /// Absolute fire time in Unix seconds (UTC).
    pub fire_at_unix: u64,
    pub workspace: Option<String>,
    pub entity_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
struct Scheduled(Reminder);

impl PartialEq for Scheduled {
    fn eq(&self, o: &Self) -> bool {
        self.0.fire_at_unix == o.0.fire_at_unix
    }
}
impl Eq for Scheduled {}
impl Ord for Scheduled {
    // Min-heap via reversed order.
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.fire_at_unix.cmp(&self.0.fire_at_unix)
    }
}
impl PartialOrd for Scheduled {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

#[derive(Default)]
pub struct ReminderScheduler {
    heap: Arc<Mutex<BinaryHeap<Scheduled>>>,
}

impl ReminderScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn schedule(&self, r: Reminder) {
        let mut h = self.heap.lock().expect("reminder heap poisoned");
        h.push(Scheduled(r));
    }

    pub fn cancel(&self, id: &Uuid) {
        let mut h = self.heap.lock().expect("reminder heap poisoned");
        let kept: Vec<Scheduled> = h.drain().filter(|s| &s.0.id != id).collect();
        for s in kept {
            h.push(s);
        }
    }

    pub fn pending_count(&self) -> usize {
        self.heap.lock().expect("reminder heap poisoned").len()
    }

    /// Spawn the ticker. The returned `JoinHandle` is typically leaked
    /// for the lifetime of the app.
    pub fn start(&self, app: AppHandle) -> thread::JoinHandle<()> {
        let heap = Arc::clone(&self.heap);
        thread::Builder::new()
            .name("shoreline-reminder-ticker".into())
            .spawn(move || loop {
                thread::sleep(Duration::from_secs(1));
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let mut due: Vec<Reminder> = Vec::new();
                {
                    let mut h = heap.lock().expect("reminder heap poisoned");
                    while let Some(top) = h.peek() {
                        if top.0.fire_at_unix <= now {
                            // Safe: we just peeked.
                            due.push(h.pop().unwrap().0);
                        } else {
                            break;
                        }
                    }
                }
                for r in due {
                    let _ = app.emit(EVENT_REMINDER_FIRED, &r);
                }
            })
            .expect("failed to spawn reminder ticker")
    }
}

// ─── Tauri command surface ──────────────────────────────────────────────

#[tauri::command]
pub fn cmd_schedule_reminder(
    session: tauri::State<'_, SessionState>,
    scheduler: tauri::State<'_, ReminderScheduler>,
    reminder: Reminder,
) -> Result<(), IpcError> {
    guard::require_authenticated(session.inner())?;
    scheduler.schedule(reminder);
    Ok(())
}

#[tauri::command]
pub fn cmd_cancel_reminder(
    session: tauri::State<'_, SessionState>,
    scheduler: tauri::State<'_, ReminderScheduler>,
    id: Uuid,
) -> Result<(), IpcError> {
    guard::require_authenticated(session.inner())?;
    scheduler.cancel(&id);
    Ok(())
}

#[tauri::command]
pub fn cmd_pending_reminder_count(
    session: tauri::State<'_, SessionState>,
    scheduler: tauri::State<'_, ReminderScheduler>,
) -> Result<usize, IpcError> {
    guard::require_authenticated(session.inner())?;
    Ok(scheduler.pending_count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(secs: u64, title: &str) -> Reminder {
        Reminder {
            id: Uuid::new_v4(),
            title: title.into(),
            body: None,
            fire_at_unix: secs,
            workspace: None,
            entity_id: None,
        }
    }

    #[test]
    fn new_scheduler_is_empty() {
        let s = ReminderScheduler::new();
        assert_eq!(s.pending_count(), 0);
    }

    #[test]
    fn schedule_increments_pending_count() {
        let s = ReminderScheduler::new();
        s.schedule(r(1_000, "a"));
        s.schedule(r(2_000, "b"));
        assert_eq!(s.pending_count(), 2);
    }

    #[test]
    fn cancel_by_id_removes_only_the_matching_reminder() {
        let s = ReminderScheduler::new();
        let keep = r(2_000, "keep");
        let drop = r(1_500, "drop");
        let drop_id = drop.id;
        s.schedule(keep);
        s.schedule(drop);
        assert_eq!(s.pending_count(), 2);
        s.cancel(&drop_id);
        assert_eq!(s.pending_count(), 1);
    }

    #[test]
    fn cancel_unknown_id_is_a_noop() {
        let s = ReminderScheduler::new();
        s.schedule(r(2_000, "a"));
        s.cancel(&Uuid::new_v4());
        assert_eq!(s.pending_count(), 1);
    }

    #[test]
    fn min_heap_orders_earliest_deadline_first() {
        // Verify the Ord impl on Scheduled — a min-heap surfaces the
        // smallest fire_at_unix at the top.
        use std::collections::BinaryHeap;
        let mut heap: BinaryHeap<Scheduled> = BinaryHeap::new();
        heap.push(Scheduled(r(3_000, "late")));
        heap.push(Scheduled(r(1_000, "soon")));
        heap.push(Scheduled(r(2_000, "mid")));
        assert_eq!(heap.pop().unwrap().0.fire_at_unix, 1_000);
        assert_eq!(heap.pop().unwrap().0.fire_at_unix, 2_000);
        assert_eq!(heap.pop().unwrap().0.fire_at_unix, 3_000);
    }

    #[test]
    fn scheduled_equality_is_based_on_fire_time() {
        let a = Scheduled(r(1_000, "x"));
        let b = Scheduled(r(1_000, "y")); // different id+title, same time
        let c = Scheduled(r(1_001, "x"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn reminder_serializes_with_snake_case_fields() {
        let rem = r(1_700_000_000, "Pickup");
        let json = serde_json::to_string(&rem).unwrap();
        assert!(json.contains("fire_at_unix"));
        assert!(json.contains("Pickup"));
    }

    #[test]
    fn reminder_round_trips_through_serde() {
        let rem = Reminder {
            id: Uuid::new_v4(),
            title: "Inspection".into(),
            body: Some("Suite 2A".into()),
            fire_at_unix: 1_700_000_000,
            workspace: Some("move_out_case".into()),
            entity_id: Some(Uuid::new_v4()),
        };
        let json = serde_json::to_string(&rem).unwrap();
        let back: Reminder = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, rem.id);
        assert_eq!(back.title, rem.title);
        assert_eq!(back.body, rem.body);
        assert_eq!(back.fire_at_unix, rem.fire_at_unix);
        assert_eq!(back.workspace, rem.workspace);
        assert_eq!(back.entity_id, rem.entity_id);
    }

    #[test]
    fn event_constant_is_stable() {
        // The frontend listens on this exact string.
        assert_eq!(EVENT_REMINDER_FIRED, "reminder://fired");
    }

    #[test]
    fn cancel_then_reschedule_is_consistent() {
        let s = ReminderScheduler::new();
        let rem = r(1_000, "first");
        let id = rem.id;
        s.schedule(rem);
        s.cancel(&id);
        assert_eq!(s.pending_count(), 0);
        s.schedule(r(2_000, "second"));
        assert_eq!(s.pending_count(), 1);
    }
}
