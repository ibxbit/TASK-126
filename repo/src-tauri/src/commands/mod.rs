//! Tauri command surface. Each submodule groups the `#[tauri::command]`
//! handlers for one domain. All commands are re-exported here for
//! registration in `lib.rs::generate_handler![]`.

pub mod analytics_cmds;
pub mod auth_cmds;
pub mod claim_cmds;
pub mod doc_cmds;
pub mod parcel_cmds;
pub mod scheduling_cmds;
pub mod settlement_cmds;
pub mod sharing_cmds;
pub mod system_cmds;

#[cfg(test)]
mod lifecycle_tests;
