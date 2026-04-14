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
