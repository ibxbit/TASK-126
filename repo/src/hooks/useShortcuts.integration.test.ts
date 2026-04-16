// Integration test for useShortcuts hook using the fake backend's
// event bus instead of per-call mocks.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import {
  resetFakeBackend,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
  emit,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import { useShortcuts } from "./useShortcuts";

beforeEach(() => resetFakeBackend());

describe("useShortcuts integration (fake event bus)", () => {
  it("subscribes and dispatches shortcut events from the fake bus", async () => {
    const quickSearch = vi.fn();
    const newCase = vi.fn();

    renderHook(() => useShortcuts({ quick_search: quickSearch, new_case: newCase }));
    await act(async () => {});

    // Emit shortcut events through the fake bus
    emit("shortcut://fired", { action: "quick_search" });
    expect(quickSearch).toHaveBeenCalledOnce();
    expect(newCase).not.toHaveBeenCalled();

    emit("shortcut://fired", { action: "new_case" });
    expect(newCase).toHaveBeenCalledOnce();
  });

  it("ignores unregistered shortcut actions without throwing", async () => {
    const quickSearch = vi.fn();
    renderHook(() => useShortcuts({ quick_search: quickSearch }));
    await act(async () => {});

    expect(() => emit("shortcut://fired", { action: "rename_tag" })).not.toThrow();
    expect(quickSearch).not.toHaveBeenCalled();
  });

  it("cleans up listener on unmount", async () => {
    const handler = vi.fn();
    const { unmount } = renderHook(() => useShortcuts({ quick_search: handler }));
    await act(async () => {});

    unmount();

    // Events after unmount should not reach the handler
    emit("shortcut://fired", { action: "quick_search" });
    expect(handler).not.toHaveBeenCalled();
  });

  it("handles multiple rapid shortcut events", async () => {
    const handler = vi.fn();
    renderHook(() => useShortcuts({ quick_search: handler }));
    await act(async () => {});

    emit("shortcut://fired", { action: "quick_search" });
    emit("shortcut://fired", { action: "quick_search" });
    emit("shortcut://fired", { action: "quick_search" });
    expect(handler).toHaveBeenCalledTimes(3);
  });
});
