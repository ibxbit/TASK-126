//! System tray + local (offline) reminder scheduler.
//!
//! - Tray menu: Open / Quick Search / Quit, plus a live counter of
//!   pending reminders.
//! - Closing the last window hides to tray rather than quitting.
//! - Reminders are persisted by the caller and scheduled here via
//!   `ReminderScheduler`. A single background thread ticks every
//!   second and emits `reminder://fired` events; no OS-level push
//!   notifications are used.

pub mod reminders;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use thiserror::Error;

pub use reminders::{Reminder, ReminderScheduler};

#[derive(Debug, Error)]
pub enum TrayError {
    #[error("failed to build tray: {0}")]
    Build(String),
}

pub fn install(app: &AppHandle) -> Result<(), TrayError> {
    let open_i = MenuItem::with_id(app, "tray_open", "Open Console", true, None::<&str>)
        .map_err(|e| TrayError::Build(e.to_string()))?;
    let search_i = MenuItem::with_id(app, "tray_search", "Quick Search", true, None::<&str>)
        .map_err(|e| TrayError::Build(e.to_string()))?;
    let quit_i = MenuItem::with_id(app, "tray_quit", "Quit", true, None::<&str>)
        .map_err(|e| TrayError::Build(e.to_string()))?;

    let menu = Menu::with_items(app, &[&open_i, &search_i, &quit_i])
        .map_err(|e| TrayError::Build(e.to_string()))?;

    TrayIconBuilder::with_id("shoreline_tray")
        .tooltip("Shoreline Property Ops")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray_open" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "tray_search" => {
                let _ = app.emit(crate::shortcuts::EVENT_SHORTCUT,
                    serde_json::json!({ "action": "quick_search" }));
            }
            "tray_quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)
        .map_err(|e| TrayError::Build(e.to_string()))?;

    Ok(())
}
