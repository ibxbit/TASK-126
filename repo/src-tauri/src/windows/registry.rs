//! In-memory registry of open workspace windows. Lives behind a
//! `Mutex` and is managed by Tauri state so commands can query and
//! mutate it from any thread.

use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::windows::{OpenedWindow, Workspace};

#[derive(Default)]
pub struct WindowRegistry {
    inner: Mutex<HashMap<String, OpenedWindow>>,
}

impl WindowRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, label: &str, workspace: Workspace, instance_id: Uuid) {
        let mut g = self.inner.lock().expect("window registry poisoned");
        g.insert(
            label.to_string(),
            OpenedWindow {
                label: label.to_string(),
                workspace,
                instance_id,
            },
        );
    }

    pub fn unregister(&self, label: &str) {
        let mut g = self.inner.lock().expect("window registry poisoned");
        g.remove(label);
    }

    pub fn snapshot(&self) -> Vec<OpenedWindow> {
        let g = self.inner.lock().expect("window registry poisoned");
        g.values().cloned().collect()
    }

    pub fn count_of(&self, workspace: Workspace) -> usize {
        let g = self.inner.lock().expect("window registry poisoned");
        g.values().filter(|w| w.workspace == workspace).count()
    }
}
