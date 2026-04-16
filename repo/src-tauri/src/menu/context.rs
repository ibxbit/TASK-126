//! Declarative context-menu framework.
//!
//! The React side sends a `ContextMenuSpec` (items + target payload)
//! via `cmd_show_context_menu`. This command builds a native Tauri
//! menu, pops it up at the current cursor position, and returns the
//! chosen item id (or `None` if dismissed). The caller then applies
//! the action — status transition, attachment operation, etc. Keeping
//! the action catalog on the UI side means we never need a backend
//! redeploy to add a menu entry, while the rendering stays native.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager};
use thiserror::Error;

use crate::ipc::{guard, IpcError, SessionState};

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum ContextMenuError {
    #[error("window not found: {0}")]
    WindowNotFound(String),
    #[error("menu build failed: {0}")]
    Build(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextMenuItem {
    /// A selectable action. `id` is echoed back to the caller.
    Action {
        id: String,
        label: String,
        #[serde(default)]
        enabled: bool,
        #[serde(default)]
        accelerator: Option<String>,
    },
    Separator,
    Submenu {
        label: String,
        items: Vec<ContextMenuItem>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMenuSpec {
    /// Opaque identifier echoed to the caller — e.g. "case:<uuid>".
    pub target: String,
    pub items: Vec<ContextMenuItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextMenuResult {
    pub target: String,
    pub chosen_id: Option<String>,
}

/// Build and show the menu at the cursor within `window_label`.
#[tauri::command]
pub fn cmd_show_context_menu(
    session: tauri::State<'_, SessionState>,
    app: AppHandle,
    window_label: String,
    spec: ContextMenuSpec,
) -> Result<ContextMenuResult, IpcError> {
    guard::require_authenticated(session.inner())?;

    let window = app
        .get_webview_window(&window_label)
        .ok_or_else(|| IpcError::Internal(format!("window not found: {}", window_label)))?;

    let chosen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let menu = build_menu(&app, &spec.items, Arc::clone(&chosen))
        .map_err(|e| IpcError::Internal(e.to_string()))?;

    // Attach handler that records the chosen id.
    let chosen_for_handler = Arc::clone(&chosen);
    app.on_menu_event(move |_app, event| {
        let id = event.id().0.clone();
        if let Ok(mut g) = chosen_for_handler.lock() {
            if g.is_none() {
                *g = Some(id);
            }
        }
    });

    window
        .popup_menu(&menu)
        .map_err(|e| IpcError::Internal(e.to_string()))?;

    let chosen_id = chosen.lock().ok().and_then(|g| g.clone());
    Ok(ContextMenuResult {
        target: spec.target,
        chosen_id,
    })
}

fn build_menu(
    app: &AppHandle,
    items: &[ContextMenuItem],
    _chosen: Arc<Mutex<Option<String>>>,
) -> Result<Menu<tauri::Wry>, ContextMenuError> {
    let menu = Menu::new(app).map_err(|e| ContextMenuError::Build(e.to_string()))?;
    append_items(app, &menu, items)?;
    Ok(menu)
}

fn append_items(
    app: &AppHandle,
    menu: &Menu<tauri::Wry>,
    items: &[ContextMenuItem],
) -> Result<(), ContextMenuError> {
    for item in items {
        match item {
            ContextMenuItem::Action {
                id,
                label,
                enabled,
                accelerator,
            } => {
                let mi = MenuItem::with_id(app, id, label, *enabled, accelerator.as_deref())
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
                menu.append(&mi)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
            }
            ContextMenuItem::Separator => {
                let sep = PredefinedMenuItem::separator(app)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
                menu.append(&sep)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
            }
            ContextMenuItem::Submenu { label, items } => {
                let sub = Submenu::new(app, label, true)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
                append_submenu_items(app, &sub, items)?;
                menu.append(&sub)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_item_serializes_with_kind_tag() {
        let item = ContextMenuItem::Action {
            id: "transition.deliver".into(),
            label: "Mark as Delivered".into(),
            enabled: true,
            accelerator: Some("Ctrl+D".into()),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains(r#""kind":"action""#), "got: {json}");
        assert!(json.contains(r#""id":"transition.deliver""#));
        assert!(json.contains(r#""accelerator":"Ctrl+D""#));
    }

    #[test]
    fn separator_serializes_with_kind_tag_only() {
        let item = ContextMenuItem::Separator;
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains(r#""kind":"separator""#));
    }

    #[test]
    fn submenu_serializes_with_nested_items() {
        let item = ContextMenuItem::Submenu {
            label: "More".into(),
            items: vec![
                ContextMenuItem::Action {
                    id: "x".into(),
                    label: "X".into(),
                    enabled: false,
                    accelerator: None,
                },
                ContextMenuItem::Separator,
            ],
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains(r#""kind":"submenu""#));
        assert!(json.contains(r#""label":"More""#));
        assert!(json.contains(r#""kind":"separator""#));
    }

    #[test]
    fn spec_round_trips_through_serde() {
        let spec = ContextMenuSpec {
            target: "case:abc-123".into(),
            items: vec![
                ContextMenuItem::Action {
                    id: "open".into(),
                    label: "Open".into(),
                    enabled: true,
                    accelerator: None,
                },
                ContextMenuItem::Separator,
                ContextMenuItem::Submenu {
                    label: "Status".into(),
                    items: vec![ContextMenuItem::Action {
                        id: "status.review".into(),
                        label: "Mark for Review".into(),
                        enabled: true,
                        accelerator: Some("F8".into()),
                    }],
                },
            ],
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: ContextMenuSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.target, spec.target);
        assert_eq!(back.items.len(), 3);
    }

    #[test]
    fn deserialize_action_from_typescript_shape() {
        // Matches what `src/ipc/desktop.ts::ContextMenuItem` emits.
        let json = r#"{"kind":"action","id":"a","label":"A","enabled":true}"#;
        let item: ContextMenuItem = serde_json::from_str(json).unwrap();
        match item {
            ContextMenuItem::Action { id, label, enabled, accelerator } => {
                assert_eq!(id, "a");
                assert_eq!(label, "A");
                assert!(enabled);
                assert!(accelerator.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn result_serializes_with_chosen_id_field() {
        let r = ContextMenuResult {
            target: "x".into(),
            chosen_id: Some("open".into()),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""target":"x""#));
        assert!(json.contains(r#""chosen_id":"open""#));
    }

    #[test]
    fn error_serializes_with_type_tag() {
        let e = ContextMenuError::WindowNotFound("workspace:1".into());
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""type":"window_not_found""#));
    }
}

fn append_submenu_items(
    app: &AppHandle,
    submenu: &Submenu<tauri::Wry>,
    items: &[ContextMenuItem],
) -> Result<(), ContextMenuError> {
    for item in items {
        match item {
            ContextMenuItem::Action {
                id,
                label,
                enabled,
                accelerator,
            } => {
                let mi = MenuItem::with_id(app, id, label, *enabled, accelerator.as_deref())
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
                submenu
                    .append(&mi)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
            }
            ContextMenuItem::Separator => {
                let sep = PredefinedMenuItem::separator(app)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
                submenu
                    .append(&sep)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
            }
            ContextMenuItem::Submenu { label, items } => {
                let sub = Submenu::new(app, label, true)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
                append_submenu_items(app, &sub, items)?;
                submenu
                    .append(&sub)
                    .map_err(|e| ContextMenuError::Build(e.to_string()))?;
            }
        }
    }
    Ok(())
}
