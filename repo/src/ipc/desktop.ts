// Typed wrappers over Tauri commands for window/shortcut/context-menu
// interactions. All desktop UX flows go through this module so the
// rest of the React code never imports `@tauri-apps/api` directly.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export type Workspace = "move_out_case" | "parcel_queue" | "claims_inbox";

export interface OpenedWindow {
  label: string;
  workspace: Workspace;
  instance_id: string;
}

export async function openWorkspace(
  workspace: Workspace,
  focusPayload?: string,
): Promise<OpenedWindow> {
  return invoke<OpenedWindow>("cmd_open_workspace", {
    workspace,
    focusPayload: focusPayload ?? null,
  });
}

export async function focusWindow(label: string): Promise<void> {
  await invoke("cmd_focus_window", { label });
}

export async function closeWindow(label: string): Promise<void> {
  await invoke("cmd_close_window", { label });
}

export async function listWindows(): Promise<OpenedWindow[]> {
  return invoke<OpenedWindow[]>("cmd_list_windows");
}

// ─── Shortcuts ──────────────────────────────────────────────────────────

export type ShortcutAction = "quick_search" | "new_case" | "rename_tag";

export async function onShortcut(
  handler: (action: ShortcutAction) => void,
): Promise<UnlistenFn> {
  return listen<{ action: ShortcutAction }>(
    "shortcut://fired",
    (e) => handler(e.payload.action),
  );
}

// ─── Context menu ───────────────────────────────────────────────────────

export type ContextMenuItem =
  | {
      kind: "action";
      id: string;
      label: string;
      enabled?: boolean;
      accelerator?: string;
    }
  | { kind: "separator" }
  | { kind: "submenu"; label: string; items: ContextMenuItem[] };

export interface ContextMenuSpec {
  target: string;
  items: ContextMenuItem[];
}

export interface ContextMenuResult {
  target: string;
  chosen_id: string | null;
}

export async function showContextMenu(
  spec: ContextMenuSpec,
): Promise<ContextMenuResult> {
  const windowLabel = getCurrentWindow().label;
  return invoke<ContextMenuResult>("cmd_show_context_menu", {
    windowLabel,
    spec,
  });
}

// ─── Reminders ──────────────────────────────────────────────────────────

export interface Reminder {
  id: string;
  title: string;
  body?: string | null;
  fire_at_unix: number;
  workspace?: string | null;
  entity_id?: string | null;
}

export async function scheduleReminder(r: Reminder): Promise<void> {
  await invoke("cmd_schedule_reminder", { reminder: r });
}

export async function cancelReminder(id: string): Promise<void> {
  await invoke("cmd_cancel_reminder", { id });
}

export async function pendingReminderCount(): Promise<number> {
  return invoke<number>("cmd_pending_reminder_count");
}

export async function onReminderFired(
  handler: (r: Reminder) => void,
): Promise<UnlistenFn> {
  return listen<Reminder>("reminder://fired", (e) => handler(e.payload));
}
