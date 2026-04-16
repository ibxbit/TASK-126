// Cross-boundary journey test: drive the real React App through the
// real IPC wrappers (`src/ipc/auth.ts`, `src/ipc/desktop.ts`) into the
// fake-but-realistic backend defined in `src/test/fake-backend.ts`.
//
// The fake mirrors the contract of the Rust `#[tauri::command]`
// handlers — same command names, same argument shapes, same error
// envelopes. Nothing in this test mocks an individual `invoke` call;
// `invoke` itself is replaced once, then the journey plays out against
// a single coherent backend that holds session state across calls.
//
// What this catches that wrapper-only tests don't:
//   • A typo in a TS command name (`cmd_logn`) throws on the real
//     dispatcher; the wrapper test is none the wiser.
//   • A renamed argument (`focus_payload` ↔ `focusPayload`) propagates
//     through wrapper, dispatcher, and fake handler.
//   • Loading-state regressions in `<App>` while the IPC promise is in
//     flight surface as a stuck spinner.
//   • Permission/auth gates in the fake propagate through React's
//     error boundaries the same way the real backend would.

import { describe, expect, it, vi, beforeEach, beforeAll } from "vitest";
import {
  resetFakeBackend,
  seedUser,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
  emit,
  getCallLog,
  snapshotState,
  IpcError,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import { render, screen, waitFor, fireEvent, act } from "@testing-library/react";
import App from "../App";
// IPC wrappers are exercised through the real `invoke` symbol — no mock
// at the wrapper layer.
import {
  login,
  logout,
  currentUser,
} from "../ipc/auth";
import {
  openWorkspace,
  listWindows,
  closeWindow,
  scheduleReminder,
  cancelReminder,
  pendingReminderCount,
  onReminderFired,
} from "../ipc/desktop";

beforeAll(() => {
  // Make sure jsdom thinks we're at "/" so the auth gate renders the
  // dashboard rather than a workspace view.
  Object.defineProperty(window, "location", {
    value: { pathname: "/" },
    writable: true,
  });
});

beforeEach(() => {
  resetFakeBackend();
});

// ── Direct journey through the IPC wrappers (no React) ─────────────────

describe("cross-boundary auth journey", () => {
  it("login persists the principal so currentUser returns it", async () => {
    seedUser({
      username: "alice",
      password: "shoreline123",
      role: "property_manager",
      tenant_ids: ["tenant-a"],
      active: true,
    });

    expect(await currentUser()).toBeNull();
    const resp = await login("alice", "shoreline123");
    expect(resp.username).toBe("alice");
    expect(resp.role).toBe("property_manager");
    expect(resp.tenant_ids).toEqual(["tenant-a"]);

    // Subsequent currentUser() must reflect the same session — proves
    // the fake (and contract) holds state across calls.
    const after = await currentUser();
    expect(after).not.toBeNull();
    expect(after?.username).toBe("alice");

    await logout();
    expect(await currentUser()).toBeNull();
  });

  it("login rejects an unknown user with a structured IpcError envelope", async () => {
    seedUser({
      username: "bob",
      password: "x",
      role: "staff",
      tenant_ids: ["t"],
      active: true,
    });
    const err = await login("nobody", "x").catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).type).toBe("internal");
    expect((err as IpcError).message).toBe("invalid credentials");
    expect(snapshotState().currentUserName).toBeNull();
  });

  it("login rejects a wrong password without leaking session state", async () => {
    seedUser({
      username: "alice",
      password: "right",
      role: "property_manager",
      tenant_ids: ["t"],
      active: true,
    });
    await expect(login("alice", "wrong")).rejects.toMatchObject({
      type: "internal",
      message: "invalid credentials",
    });
    expect(await currentUser()).toBeNull();
  });

  it("login rejects an inactive user", async () => {
    seedUser({
      username: "frozen",
      password: "x",
      role: "staff",
      tenant_ids: ["t"],
      active: false,
    });
    await expect(login("frozen", "x")).rejects.toMatchObject({
      type: "internal",
      message: "account disabled",
    });
  });
});

// ── Permission denied paths ────────────────────────────────────────────

describe("permission boundary through the dispatcher", () => {
  it("openWorkspace returns Unauthenticated when no session is set", async () => {
    const err = await openWorkspace("move_out_case").catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).type).toBe("unauthenticated");
  });

  it("listWindows rejects unauthenticated callers", async () => {
    const err = await listWindows().catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });

  it("scheduleReminder rejects unauthenticated callers", async () => {
    const err = await scheduleReminder({
      id: "r1",
      title: "x",
      fire_at_unix: 1,
    }).catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });

  it("after login the same calls succeed", async () => {
    seedUser({
      username: "u",
      password: "p",
      role: "staff",
      tenant_ids: ["t"],
      active: true,
    });
    await login("u", "p");
    await expect(listWindows()).resolves.toEqual([]);
    await expect(scheduleReminder({ id: "r1", title: "x", fire_at_unix: 1 })).resolves.toBeUndefined();
    await expect(pendingReminderCount()).resolves.toBe(1);
    await expect(cancelReminder("r1")).resolves.toBeUndefined();
    await expect(pendingReminderCount()).resolves.toBe(0);
  });
});

// ── Workspace open / close round-trip ──────────────────────────────────

describe("workspace open/close round-trip", () => {
  it("openWorkspace registers a window that listWindows returns and closeWindow removes", async () => {
    seedUser({
      username: "u",
      password: "p",
      role: "property_manager",
      tenant_ids: ["t"],
      active: true,
    });
    await login("u", "p");

    const w1 = await openWorkspace("parcel_queue");
    expect(w1.workspace).toBe("parcel_queue");
    expect(w1.label.startsWith("parcel_queue:")).toBe(true);

    const w2 = await openWorkspace("claims_inbox", "case-7");
    expect(w2.workspace).toBe("claims_inbox");

    const all = await listWindows();
    expect(all).toHaveLength(2);
    expect(all.map((w) => w.workspace).sort()).toEqual(["claims_inbox", "parcel_queue"]);

    await closeWindow(w1.label);
    const after = await listWindows();
    expect(after).toHaveLength(1);
    expect(after[0].label).toBe(w2.label);

    // Closing an unknown label surfaces a real "internal" error from
    // the fake — the wrapper does not need to invent one.
    await expect(closeWindow("nope:0")).rejects.toMatchObject({ type: "internal" });
  });

  it("rejects an unknown workspace string with an internal error", async () => {
    seedUser({
      username: "u",
      password: "p",
      role: "property_manager",
      tenant_ids: ["t"],
      active: true,
    });
    await login("u", "p");
    // Force a bad workspace through the wrapper — the dispatcher must
    // still reject it.
    await expect(openWorkspace("not_a_workspace" as never)).rejects.toMatchObject({
      type: "internal",
    });
  });
});

// ── Reminder event bus ────────────────────────────────────────────────

describe("reminder event bus", () => {
  it("onReminderFired delivers payloads emitted onto reminder://fired", async () => {
    const got: Array<{ id: string; title: string }> = [];
    await onReminderFired((r) => got.push(r));
    emit("reminder://fired", { id: "r1", title: "Pickup", fire_at_unix: 1 });
    emit("reminder://fired", { id: "r2", title: "Inspection", fire_at_unix: 2 });
    expect(got).toEqual([
      { id: "r1", title: "Pickup", fire_at_unix: 1 },
      { id: "r2", title: "Inspection", fire_at_unix: 2 },
    ]);
  });
});

// ── Full UI journey: login form → dashboard → openWorkspace ───────────

describe("App.tsx end-to-end journey through the fake backend", () => {
  it("login form → success → dashboard → click workspace card → window registered", async () => {
    seedUser({
      username: "alice",
      password: "shoreline123",
      role: "property_manager",
      tenant_ids: ["tenant-a"],
      active: true,
    });

    render(<App />);

    // 1. Initial render → loading (currentUser still in flight).
    // After it settles, the LoginForm appears.
    await waitFor(() => {
      expect(screen.getByText(/sign in to continue/i)).toBeInTheDocument();
    });

    // 2. Submit valid credentials.
    fireEvent.change(screen.getByLabelText(/username/i), {
      target: { value: "alice" },
    });
    fireEvent.change(screen.getByLabelText(/password/i), {
      target: { value: "shoreline123" },
    });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    // 3. Dashboard appears with role + workspace cards.
    await waitFor(() => {
      expect(
        screen.getByText("Shoreline Property Operations Console"),
      ).toBeInTheDocument();
      expect(screen.getByText(/property_manager/)).toBeInTheDocument();
    });

    // 4. Click a workspace card → openWorkspace fires through the
    //    dispatcher and registers a window.
    expect(snapshotState().windowCount).toBe(0);
    const cardButtons = screen.getAllByRole("button");
    const parcelBtn = cardButtons.find((b) =>
      b.textContent?.toLowerCase().includes("parcel queue"),
    );
    expect(parcelBtn, "Parcel Queue card should render").toBeDefined();
    await act(async () => {
      fireEvent.click(parcelBtn!);
    });
    await waitFor(() => {
      expect(snapshotState().windowCount).toBe(1);
    });

    // 5. The dispatcher saw the exact command name the wrapper emits —
    //    proves the wrapper-to-handler contract is intact.
    const cmdNames = getCallLog().map((c) => c.cmd);
    expect(cmdNames).toContain("cmd_current_user");
    expect(cmdNames).toContain("cmd_login");
    expect(cmdNames).toContain("cmd_open_workspace");
  });

  it("login error from the dispatcher renders the form's error UI", async () => {
    seedUser({
      username: "alice",
      password: "right",
      role: "property_manager",
      tenant_ids: ["t"],
      active: true,
    });
    render(<App />);
    await waitFor(() => screen.getByText(/sign in to continue/i));

    fireEvent.change(screen.getByLabelText(/username/i), {
      target: { value: "alice" },
    });
    fireEvent.change(screen.getByLabelText(/password/i), {
      target: { value: "wrong" },
    });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    // The form must surface the structured error message.
    await waitFor(() => {
      expect(screen.getByText("invalid credentials")).toBeInTheDocument();
    });
    // Sign-in button is re-enabled after the failure (no stuck loading).
    const btn = screen.getByRole("button", { name: /sign in/i });
    expect(btn).not.toBeDisabled();
    // Session should still be empty.
    expect(snapshotState().currentUserName).toBeNull();
  });
});
