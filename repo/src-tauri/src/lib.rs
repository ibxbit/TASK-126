//! Shoreline Property Operations Console — library root.
//!
//! Declares every domain and support module. The binary entry point
//! (`main.rs`) calls `run()` below, which composes the Tauri builder.

// ── Domain / support modules ────────────────────────────────────────────
pub mod analytics;
pub mod audit;
pub mod auth;
pub mod claims;
pub mod commands;
pub mod db;
pub mod docs;
pub mod ipc;
pub mod keys;
pub mod menu;
pub mod parcel;
pub mod recovery;
pub mod scheduling;
pub mod settlement;
pub mod sharing;
pub mod shortcuts;
pub mod tray;
pub mod update;
pub mod windows;

use std::sync::Arc;

use tauri::Manager;

use crate::db::connection::Database;
use crate::docs::storage::StorageLayout;
use crate::ipc::SessionState;
use crate::recovery::HandleTracker;
use crate::tray::ReminderScheduler;
use crate::windows::registry::WindowRegistry;

/// Boot the Tauri application.
///
/// Wiring:
///   - SQLite database (WAL mode, all 11 migrations applied on startup).
///   - Desktop shell: multi-window workspaces, global shortcuts,
///     system tray, local reminder scheduler.
///   - Session state + IPC permission guard on every command.
///   - Shared handle tracker for safe file-lifecycle.
///   - All 51 IPC commands registered and backed by concrete
///     SQLite repository implementations. Zero stubs remain.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(SessionState::new())
        .manage(WindowRegistry::new())
        .manage(ReminderScheduler::new())
        .manage(HandleTracker::new())
        .setup(|app| {
            let handle = app.handle().clone();

            // ── SQLite ─────────────────────────────────────────────
            let app_data = tauri::Manager::path(&handle)
                .app_data_dir()
                .expect("no app-data dir");
            let db_path = app_data.join("shoreline.db");
            let db = Arc::new(
                Database::open(&db_path).expect("failed to open DB"),
            );
            db.run_migrations().expect("migration failed");
            app.manage(Arc::clone(&db));
            app.manage(StorageLayout::new(&app_data));

            // ── Desktop shell ──────────────────────────────────────
            tray::install(&handle)?;
            shortcuts::register_all(&handle)?;

            let scheduler: tauri::State<'_, ReminderScheduler> = handle.state();
            let _ticker = scheduler.start(handle.clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Window management
            windows::cmd_open_workspace,
            windows::cmd_focus_window,
            windows::cmd_close_window,
            windows::cmd_list_windows,
            // Context menus
            menu::cmd_show_context_menu,
            // Reminders
            tray::reminders::cmd_schedule_reminder,
            tray::reminders::cmd_cancel_reminder,
            tray::reminders::cmd_pending_reminder_count,
            // Auth session
            commands::auth_cmds::cmd_login,
            commands::auth_cmds::cmd_logout,
            commands::auth_cmds::cmd_current_user,
            // Parcel lifecycle (SQLite-backed)
            commands::parcel_cmds::cmd_parcel_available_transitions,
            commands::parcel_cmds::cmd_transition_parcel,
            commands::parcel_cmds::cmd_parcel_history,
            // Claims (SQLite-backed)
            commands::claim_cmds::cmd_claim_transition,
            commands::claim_cmds::cmd_find_claim_matches,
            // Settlement (SQLite-backed)
            commands::settlement_cmds::cmd_settlement_transition,
            commands::settlement_cmds::cmd_settlement_prepare,
            commands::settlement_cmds::cmd_settlement_approve,
            commands::settlement_cmds::cmd_settlement_statement,
            commands::settlement_cmds::cmd_settlement_statement_html,
            commands::settlement_cmds::cmd_settlement_check_request,
            // Documents
            commands::doc_cmds::cmd_upload_start,
            commands::doc_cmds::cmd_upload_put_chunk,
            commands::doc_cmds::cmd_upload_status,
            commands::doc_cmds::cmd_upload_finalize,
            commands::doc_cmds::cmd_upload_abort,
            commands::doc_cmds::cmd_attachment_search,
            commands::doc_cmds::cmd_attachment_add_tag,
            commands::doc_cmds::cmd_attachment_remove_tag,
            commands::doc_cmds::cmd_attachment_preview,
            // Analytics
            commands::analytics_cmds::cmd_analytics_track,
            commands::analytics_cmds::cmd_analytics_funnel,
            commands::analytics_cmds::cmd_analytics_retention,
            commands::analytics_cmds::cmd_analytics_quality,
            commands::analytics_cmds::cmd_analytics_export,
            commands::analytics_cmds::cmd_experiment_assign,
            // Scheduling
            commands::scheduling_cmds::cmd_schedule_activate_rule_set,
            commands::scheduling_cmds::cmd_schedule_validate,
            commands::scheduling_cmds::cmd_schedule_propose,
            // Sharing
            commands::sharing_cmds::cmd_wrap_with_watermark,
            commands::sharing_cmds::cmd_share_build_package,
            commands::sharing_cmds::cmd_share_verify_access,
            commands::sharing_cmds::cmd_share_revoke,
            commands::sharing_cmds::cmd_share_sweep_expired,
            // System / recovery / update
            commands::system_cmds::cmd_last_recovery_outcome,
            commands::system_cmds::cmd_open_handles,
            commands::system_cmds::cmd_update_verify,
            commands::system_cmds::cmd_update_install,
            commands::system_cmds::cmd_update_rollback,
            commands::system_cmds::cmd_list_installed_versions,
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch Shoreline");
}
