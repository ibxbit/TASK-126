// Integration test for useContextMenu hook using the fake backend.
// Exercises the full flow: right-click → showContextMenu IPC → handler dispatch.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import {
  resetFakeBackend,
  seedUser,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
  getCallLog,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import { login } from "../ipc/auth";
import { useContextMenu } from "./ContextMenu";

beforeEach(() => resetFakeBackend());

describe("useContextMenu integration (fake backend)", () => {
  it("calls cmd_show_context_menu through the IPC dispatcher", async () => {
    seedUser({ username: "u", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    await login("u", "pw");

    const handler = vi.fn();
    const { result } = renderHook(() =>
      useContextMenu({
        target: "case:123",
        items: [{ kind: "action", id: "approve", label: "Approve", enabled: true }],
        handlers: { approve: handler },
      }),
    );

    const fakeEvent = {
      preventDefault: vi.fn(),
      stopPropagation: vi.fn(),
    } as unknown as React.MouseEvent;

    await act(async () => {
      await result.current(fakeEvent);
    });

    expect(fakeEvent.preventDefault).toHaveBeenCalled();
    expect(fakeEvent.stopPropagation).toHaveBeenCalled();

    const cmds = getCallLog().map((c) => c.cmd);
    expect(cmds).toContain("cmd_show_context_menu");
  });

  it("does not throw when chosen_id has no matching handler", async () => {
    seedUser({ username: "u2", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    await login("u2", "pw");

    // Handler map is empty — the backend returns a chosen_id but we have
    // no matching handler. This exercises the `if (handler)` branch.
    const { result } = renderHook(() =>
      useContextMenu({
        target: "case:789",
        items: [{ kind: "action", id: "nonexistent", label: "X", enabled: true }],
        handlers: {}, // no handler for "nonexistent"
      }),
    );

    const fakeEvent = {
      preventDefault: vi.fn(),
      stopPropagation: vi.fn(),
    } as unknown as React.MouseEvent;

    await act(async () => {
      // Should NOT throw even though chosen_id has no handler
      await result.current(fakeEvent);
    });

    expect(fakeEvent.preventDefault).toHaveBeenCalled();
  });

  it("propagates IpcError when unauthenticated", async () => {
    // No login — showContextMenu will reject with unauthenticated
    const { result } = renderHook(() =>
      useContextMenu({
        target: "case:456",
        items: [],
        handlers: {},
      }),
    );

    const fakeEvent = {
      preventDefault: vi.fn(),
      stopPropagation: vi.fn(),
    } as unknown as React.MouseEvent;

    // The hook's callback is async and doesn't catch errors internally,
    // so the IPC error surfaces as a rejection.
    await act(async () => {
      await expect(result.current(fakeEvent)).rejects.toMatchObject({
        type: "unauthenticated",
      });
    });
  });
});
