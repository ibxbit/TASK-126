// Tests for the desktop IPC wrapper: window management, shortcuts,
// context menus, and reminders. Tauri runtime is mocked at the module
// boundary; these tests verify the wrapper's command-name + payload
// contract that the Rust backend signs.

import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({ label: "main" })),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import {
  openWorkspace,
  focusWindow,
  closeWindow,
  listWindows,
  onShortcut,
  showContextMenu,
  scheduleReminder,
  cancelReminder,
  pendingReminderCount,
  onReminderFired,
  type Reminder,
  type ContextMenuSpec,
  type ShortcutAction,
} from "./desktop";

const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);
const mockGetCurrentWindow = vi.mocked(getCurrentWindow);

beforeEach(() => {
  mockInvoke.mockReset();
  mockListen.mockReset();
  mockGetCurrentWindow.mockReturnValue({ label: "main" } as never);
});

// ── Window management ──────────────────────────────────────────────────

describe("openWorkspace()", () => {
  it("invokes cmd_open_workspace with workspace + null payload by default", async () => {
    mockInvoke.mockResolvedValueOnce({
      label: "move_out_case:abc",
      workspace: "move_out_case",
      instance_id: "abc",
    });
    const out = await openWorkspace("move_out_case");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_open_workspace", {
      workspace: "move_out_case",
      focusPayload: null,
    });
    expect(out.workspace).toBe("move_out_case");
  });

  it("forwards a focus payload when provided", async () => {
    mockInvoke.mockResolvedValueOnce({
      label: "parcel_queue:xyz",
      workspace: "parcel_queue",
      instance_id: "xyz",
    });
    await openWorkspace("parcel_queue", "case-7");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_open_workspace", {
      workspace: "parcel_queue",
      focusPayload: "case-7",
    });
  });

  it("propagates IPC failure to the caller", async () => {
    mockInvoke.mockRejectedValueOnce({ type: "internal", message: "failed to build window" });
    await expect(openWorkspace("claims_inbox")).rejects.toMatchObject({
      type: "internal",
    });
  });
});

describe("focusWindow()", () => {
  it("invokes cmd_focus_window with the label", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await focusWindow("main");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_focus_window", { label: "main" });
  });
});

describe("closeWindow()", () => {
  it("invokes cmd_close_window with the label", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await closeWindow("parcel_queue:abc");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_close_window", {
      label: "parcel_queue:abc",
    });
  });

  it("rejects when backend reports window not found", async () => {
    mockInvoke.mockRejectedValueOnce({ type: "internal" });
    await expect(closeWindow("nope:1")).rejects.toMatchObject({ type: "internal" });
  });
});

describe("listWindows()", () => {
  it("invokes cmd_list_windows and returns the array verbatim", async () => {
    const fake = [
      { label: "main", workspace: "move_out_case", instance_id: "1" },
      { label: "p:2", workspace: "parcel_queue", instance_id: "2" },
    ];
    mockInvoke.mockResolvedValueOnce(fake);
    const out = await listWindows();
    expect(mockInvoke).toHaveBeenCalledWith("cmd_list_windows");
    expect(out).toEqual(fake);
    expect(out).toHaveLength(2);
  });

  it("returns empty array when no windows are open", async () => {
    mockInvoke.mockResolvedValueOnce([]);
    const out = await listWindows();
    expect(out).toEqual([]);
  });
});

// ── Shortcuts ──────────────────────────────────────────────────────────

describe("onShortcut()", () => {
  it("subscribes to shortcut://fired and unwraps payload.action", async () => {
    let captured: ((event: { payload: { action: ShortcutAction } }) => void) | null = null;
    mockListen.mockImplementation(async (_event, cb) => {
      captured = cb as never;
      return () => {};
    });
    const handler = vi.fn();
    await onShortcut(handler);

    expect(mockListen).toHaveBeenCalledWith("shortcut://fired", expect.any(Function));
    expect(captured).not.toBeNull();
    captured!({ payload: { action: "quick_search" } });
    expect(handler).toHaveBeenCalledWith("quick_search");
  });

  it("returns an unlisten function", async () => {
    const unlisten = vi.fn();
    mockListen.mockResolvedValueOnce(unlisten);
    const out = await onShortcut(() => {});
    expect(out).toBe(unlisten);
  });

  it("dispatches each of the three shortcut actions", async () => {
    let cb: ((e: { payload: { action: ShortcutAction } }) => void) | null = null;
    mockListen.mockImplementation(async (_e, fn) => {
      cb = fn as never;
      return () => {};
    });
    const actions: ShortcutAction[] = [];
    await onShortcut((a) => actions.push(a));
    cb!({ payload: { action: "quick_search" } });
    cb!({ payload: { action: "new_case" } });
    cb!({ payload: { action: "rename_tag" } });
    expect(actions).toEqual(["quick_search", "new_case", "rename_tag"]);
  });
});

// ── Context menu ───────────────────────────────────────────────────────

describe("showContextMenu()", () => {
  it("invokes cmd_show_context_menu with the current window label", async () => {
    mockGetCurrentWindow.mockReturnValueOnce({ label: "parcel_queue:42" } as never);
    mockInvoke.mockResolvedValueOnce({ target: "case:7", chosen_id: "open" });
    const spec: ContextMenuSpec = {
      target: "case:7",
      items: [
        { kind: "action", id: "open", label: "Open", enabled: true },
        { kind: "separator" },
        { kind: "action", id: "close", label: "Close", enabled: true },
      ],
    };
    const out = await showContextMenu(spec);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_show_context_menu", {
      windowLabel: "parcel_queue:42",
      spec,
    });
    expect(out.chosen_id).toBe("open");
  });

  it("returns chosen_id null when the user dismisses the menu", async () => {
    mockInvoke.mockResolvedValueOnce({ target: "x", chosen_id: null });
    const out = await showContextMenu({ target: "x", items: [] });
    expect(out.chosen_id).toBeNull();
  });

  it("supports nested submenus in the spec", async () => {
    mockInvoke.mockResolvedValueOnce({ target: "x", chosen_id: "status.review" });
    await showContextMenu({
      target: "x",
      items: [
        {
          kind: "submenu",
          label: "Status",
          items: [{ kind: "action", id: "status.review", label: "Review", enabled: true }],
        },
      ],
    });
    expect(mockInvoke).toHaveBeenCalledWith(
      "cmd_show_context_menu",
      expect.objectContaining({ windowLabel: "main" }),
    );
  });
});

// ── Reminders ──────────────────────────────────────────────────────────

describe("scheduleReminder()", () => {
  it("invokes cmd_schedule_reminder wrapping the reminder under `reminder`", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    const r: Reminder = {
      id: "r1",
      title: "Inspection",
      fire_at_unix: 1_700_000_000,
    };
    await scheduleReminder(r);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_schedule_reminder", { reminder: r });
  });

  it("propagates an authentication error from the backend", async () => {
    mockInvoke.mockRejectedValueOnce({ type: "unauthenticated" });
    await expect(
      scheduleReminder({ id: "r1", title: "x", fire_at_unix: 1 }),
    ).rejects.toMatchObject({ type: "unauthenticated" });
  });
});

describe("cancelReminder()", () => {
  it("invokes cmd_cancel_reminder with the id", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await cancelReminder("r1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_cancel_reminder", { id: "r1" });
  });
});

describe("pendingReminderCount()", () => {
  it("invokes cmd_pending_reminder_count and returns the count", async () => {
    mockInvoke.mockResolvedValueOnce(3);
    const n = await pendingReminderCount();
    expect(mockInvoke).toHaveBeenCalledWith("cmd_pending_reminder_count");
    expect(n).toBe(3);
  });

  it("returns 0 for an empty heap", async () => {
    mockInvoke.mockResolvedValueOnce(0);
    expect(await pendingReminderCount()).toBe(0);
  });
});

describe("onReminderFired()", () => {
  it("listens on reminder://fired and unwraps the payload", async () => {
    let cb: ((e: { payload: Reminder }) => void) | null = null;
    mockListen.mockImplementation(async (_e, fn) => {
      cb = fn as never;
      return () => {};
    });
    const handler = vi.fn();
    await onReminderFired(handler);
    expect(mockListen).toHaveBeenCalledWith("reminder://fired", expect.any(Function));
    cb!({
      payload: {
        id: "r1",
        title: "Pickup",
        fire_at_unix: 1_700_000_000,
      },
    });
    expect(handler).toHaveBeenCalledWith({
      id: "r1",
      title: "Pickup",
      fire_at_unix: 1_700_000_000,
    });
  });

  it("returns the underlying unlisten function", async () => {
    const unlisten = vi.fn();
    mockListen.mockResolvedValueOnce(unlisten);
    const out = await onReminderFired(() => {});
    expect(out).toBe(unlisten);
  });
});
