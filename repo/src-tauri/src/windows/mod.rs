//! Multi-window workspace manager.
//!
//! Each workspace (Move-Out Case, Parcel Queue, Claims Inbox) may have
//! an arbitrary number of windows open in parallel. Windows are keyed
//! by a stable `label` = "<workspace>:<uuid>" so Tauri can address them
//! and the registry can enforce parallel-instance rules.

pub mod registry;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use thiserror::Error;
use uuid::Uuid;

use crate::ipc::{guard, IpcError, SessionState};
use crate::windows::registry::WindowRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Workspace {
    MoveOutCase,
    ParcelQueue,
    ClaimsInbox,
}

impl Workspace {
    pub fn as_str(&self) -> &'static str {
        match self {
            Workspace::MoveOutCase => "move_out_case",
            Workspace::ParcelQueue => "parcel_queue",
            Workspace::ClaimsInbox => "claims_inbox",
        }
    }

    /// Route inside the React app that renders this workspace.
    pub fn route(&self) -> &'static str {
        match self {
            Workspace::MoveOutCase => "/workspace/move-out",
            Workspace::ParcelQueue => "/workspace/parcel-queue",
            Workspace::ClaimsInbox => "/workspace/claims-inbox",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Workspace::MoveOutCase => "Move-Out Case",
            Workspace::ParcelQueue => "Parcel Queue",
            Workspace::ClaimsInbox => "Claims Inbox",
        }
    }

    /// Initial size (logical pixels) — scaled automatically for high-DPI.
    pub fn default_size(&self) -> (f64, f64) {
        match self {
            Workspace::MoveOutCase => (1280.0, 860.0),
            Workspace::ParcelQueue => (1100.0, 720.0),
            Workspace::ClaimsInbox => (1280.0, 820.0),
        }
    }
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WindowError {
    #[error("failed to build window: {0}")]
    Build(String),
    #[error("window not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenedWindow {
    pub label: String,
    pub workspace: Workspace,
    pub instance_id: Uuid,
}

/// Open a fresh instance of a workspace. Multiple instances per
/// workspace are permitted by design — parallel workflows.
pub fn open_workspace(
    app: &AppHandle,
    registry: &WindowRegistry,
    workspace: Workspace,
    focus_payload: Option<String>,
) -> Result<OpenedWindow, WindowError> {
    let instance_id = Uuid::new_v4();
    let label = format!("{}:{}", workspace.as_str(), instance_id);
    let (w, h) = workspace.default_size();

    let mut url = workspace.route().to_string();
    if let Some(payload) = focus_payload.as_ref() {
        url.push_str("?payload=");
        url.push_str(&urlencoding::encode(payload));
    }

    WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title(workspace.title())
        .inner_size(w, h)
        .min_inner_size(1280.0, 720.0)
        .resizable(true)
        .decorations(true)
        .visible(true)
        .build()
        .map_err(|e| WindowError::Build(e.to_string()))?;

    registry.register(&label, workspace, instance_id);

    Ok(OpenedWindow {
        label,
        workspace,
        instance_id,
    })
}

pub fn focus_window(app: &AppHandle, label: &str) -> Result<(), WindowError> {
    let w = app
        .get_webview_window(label)
        .ok_or_else(|| WindowError::NotFound(label.to_string()))?;
    w.set_focus().map_err(|e| WindowError::Build(e.to_string()))
}

pub fn close_window(
    app: &AppHandle,
    registry: &WindowRegistry,
    label: &str,
) -> Result<(), WindowError> {
    let w = app
        .get_webview_window(label)
        .ok_or_else(|| WindowError::NotFound(label.to_string()))?;
    w.close().map_err(|e| WindowError::Build(e.to_string()))?;
    registry.unregister(label);
    Ok(())
}

// ─── Tauri command surface ──────────────────────────────────────────────

#[tauri::command]
pub fn cmd_open_workspace(
    session: tauri::State<'_, SessionState>,
    app: AppHandle,
    registry: tauri::State<'_, WindowRegistry>,
    workspace: Workspace,
    focus_payload: Option<String>,
) -> Result<OpenedWindow, IpcError> {
    guard::require_authenticated(session.inner())?;
    open_workspace(&app, registry.inner(), workspace, focus_payload)
        .map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_focus_window(
    session: tauri::State<'_, SessionState>,
    app: AppHandle,
    label: String,
) -> Result<(), IpcError> {
    guard::require_authenticated(session.inner())?;
    focus_window(&app, &label).map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_close_window(
    session: tauri::State<'_, SessionState>,
    app: AppHandle,
    registry: tauri::State<'_, WindowRegistry>,
    label: String,
) -> Result<(), IpcError> {
    guard::require_authenticated(session.inner())?;
    close_window(&app, registry.inner(), &label).map_err(|e| IpcError::Internal(e.to_string()))
}

#[tauri::command]
pub fn cmd_list_windows(
    session: tauri::State<'_, SessionState>,
    registry: tauri::State<'_, WindowRegistry>,
) -> Result<Vec<OpenedWindow>, IpcError> {
    guard::require_authenticated(session.inner())?;
    Ok(registry.snapshot())
}
