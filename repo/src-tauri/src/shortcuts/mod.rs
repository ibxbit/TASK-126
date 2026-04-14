//! Global shortcut handler.
//!
//! Shortcuts are registered at app startup via the
//! `tauri-plugin-global-shortcut` plugin. When a shortcut fires, we
//! emit a typed `shortcut://<action>` event to the currently focused
//! window (falling back to the main window). The React app listens and
//! routes the event to the appropriate handler (quick search modal,
//! new-case wizard, inline rename, …).

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use thiserror::Error;

pub const EVENT_SHORTCUT: &str = "shortcut://fired";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutAction {
    QuickSearch, // Ctrl+K
    NewCase,     // Ctrl+Shift+N
    RenameTag,   // F2
}

impl ShortcutAction {
    fn as_str(&self) -> &'static str {
        match self {
            ShortcutAction::QuickSearch => "quick_search",
            ShortcutAction::NewCase => "new_case",
            ShortcutAction::RenameTag => "rename_tag",
        }
    }
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShortcutError {
    #[error("failed to register shortcut '{0}': {1}")]
    Register(String, String),
    #[error("plugin unavailable: {0}")]
    Plugin(String),
}

#[derive(Debug, Clone, Serialize)]
struct ShortcutEvent {
    action: &'static str,
}

/// Bind all desktop shortcuts. Called once from `tauri::Builder::setup`.
pub fn register_all(app: &AppHandle) -> Result<(), ShortcutError> {
    let manager = app.global_shortcut();

    let bindings = [
        (
            Shortcut::new(Some(Modifiers::CONTROL), Code::KeyK),
            ShortcutAction::QuickSearch,
        ),
        (
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyN),
            ShortcutAction::NewCase,
        ),
        (Shortcut::new(None, Code::F2), ShortcutAction::RenameTag),
    ];

    for (sc, action) in bindings {
        let app_for_handler = app.clone();
        manager
            .on_shortcut(sc, move |_app, _sc, event| {
                // Fire only on press, not release.
                if event.state != ShortcutState::Pressed {
                    return;
                }
                dispatch(&app_for_handler, action);
            })
            .map_err(|e| ShortcutError::Register(action.as_str().to_string(), e.to_string()))?;
    }

    Ok(())
}

fn dispatch(app: &AppHandle, action: ShortcutAction) {
    let payload = ShortcutEvent {
        action: action.as_str(),
    };

    // Prefer the focused window so the user's current context handles
    // the action. Fall back to main window, then broadcast.
    let target = app
        .webview_windows()
        .into_iter()
        .find(|(_, w)| w.is_focused().unwrap_or(false))
        .map(|(_, w)| w);

    if let Some(w) = target {
        let _ = w.emit(EVENT_SHORTCUT, &payload);
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.emit(EVENT_SHORTCUT, &payload);
        return;
    }
    let _ = app.emit(EVENT_SHORTCUT, &payload);
}
