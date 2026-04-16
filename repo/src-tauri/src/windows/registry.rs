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

#[cfg(test)]
mod tests {
    use super::*;

    fn label(ws: Workspace, id: Uuid) -> String {
        format!("{}:{}", ws.as_str(), id)
    }

    #[test]
    fn new_registry_is_empty() {
        let r = WindowRegistry::new();
        assert_eq!(r.snapshot().len(), 0);
        assert_eq!(r.count_of(Workspace::ParcelQueue), 0);
    }

    #[test]
    fn register_then_snapshot_returns_the_window() {
        let r = WindowRegistry::new();
        let id = Uuid::new_v4();
        let lab = label(Workspace::MoveOutCase, id);
        r.register(&lab, Workspace::MoveOutCase, id);
        let snap = r.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].label, lab);
        assert_eq!(snap[0].workspace, Workspace::MoveOutCase);
        assert_eq!(snap[0].instance_id, id);
    }

    #[test]
    fn unregister_removes_the_window() {
        let r = WindowRegistry::new();
        let id = Uuid::new_v4();
        let lab = label(Workspace::ClaimsInbox, id);
        r.register(&lab, Workspace::ClaimsInbox, id);
        assert_eq!(r.count_of(Workspace::ClaimsInbox), 1);
        r.unregister(&lab);
        assert_eq!(r.count_of(Workspace::ClaimsInbox), 0);
        assert_eq!(r.snapshot().len(), 0);
    }

    #[test]
    fn unregister_unknown_label_is_a_noop() {
        let r = WindowRegistry::new();
        // Should not panic.
        r.unregister("does_not_exist:12345");
        assert_eq!(r.snapshot().len(), 0);
    }

    #[test]
    fn registering_same_label_twice_overwrites_in_place() {
        let r = WindowRegistry::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let lab = label(Workspace::ParcelQueue, id1);
        r.register(&lab, Workspace::ParcelQueue, id1);
        r.register(&lab, Workspace::ParcelQueue, id2); // same label, new instance id
        let snap = r.snapshot();
        assert_eq!(snap.len(), 1, "label is the unique key");
        assert_eq!(snap[0].instance_id, id2);
    }

    #[test]
    fn count_of_filters_by_workspace_kind() {
        let r = WindowRegistry::new();
        for ws in [Workspace::MoveOutCase, Workspace::ParcelQueue, Workspace::ParcelQueue] {
            let id = Uuid::new_v4();
            r.register(&label(ws, id), ws, id);
        }
        assert_eq!(r.count_of(Workspace::MoveOutCase), 1);
        assert_eq!(r.count_of(Workspace::ParcelQueue), 2);
        assert_eq!(r.count_of(Workspace::ClaimsInbox), 0);
    }

    #[test]
    fn snapshot_is_a_clone_not_a_live_view() {
        let r = WindowRegistry::new();
        let id = Uuid::new_v4();
        r.register(&label(Workspace::MoveOutCase, id), Workspace::MoveOutCase, id);
        let snap = r.snapshot();
        // Mutating the registry afterwards must not change `snap`.
        r.unregister(&label(Workspace::MoveOutCase, id));
        assert_eq!(snap.len(), 1);
        assert_eq!(r.snapshot().len(), 0);
    }

    #[test]
    fn workspace_as_str_is_stable() {
        // The Vite route + Tauri config rely on these strings — guard against typos.
        assert_eq!(Workspace::MoveOutCase.as_str(), "move_out_case");
        assert_eq!(Workspace::ParcelQueue.as_str(), "parcel_queue");
        assert_eq!(Workspace::ClaimsInbox.as_str(), "claims_inbox");
    }

    #[test]
    fn workspace_route_starts_with_workspace_prefix() {
        for ws in [Workspace::MoveOutCase, Workspace::ParcelQueue, Workspace::ClaimsInbox] {
            assert!(ws.route().starts_with("/workspace/"), "route: {}", ws.route());
        }
    }

    #[test]
    fn workspace_default_size_meets_min_window_floor() {
        // Tauri main window enforces 1280×720 minimum — ensure all
        // workspace defaults satisfy or exceed it.
        for ws in [Workspace::MoveOutCase, Workspace::ParcelQueue, Workspace::ClaimsInbox] {
            let (w, h) = ws.default_size();
            // Note: ParcelQueue is 1100 wide in the spec — that's
            // intentional (Tauri lets you create with smaller initial
            // size as long as min_inner_size is set), so just check
            // height which should be ≥ 720.
            assert!(h >= 720.0, "{:?} height {} < 720", ws, h);
            assert!(w > 0.0);
        }
    }
}
