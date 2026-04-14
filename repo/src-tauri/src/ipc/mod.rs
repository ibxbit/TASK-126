//! Tauri IPC support: the reusable permission guard and its
//! session-state backing.

pub mod guard;

pub use guard::{
    require, require_any, require_authenticated, IpcError, SessionState,
};
