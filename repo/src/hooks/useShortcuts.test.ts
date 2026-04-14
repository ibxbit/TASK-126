import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useShortcuts } from "./useShortcuts";

// Capture the listener callback for simulating events.
let capturedHandler: ((action: string) => void) | null = null;
const mockUnlisten = vi.fn();

vi.mock("../ipc/desktop", () => ({
  onShortcut: vi.fn((handler: (action: string) => void) => {
    capturedHandler = handler;
    return Promise.resolve(mockUnlisten);
  }),
}));

import { onShortcut } from "../ipc/desktop";

describe("useShortcuts", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    capturedHandler = null;
  });

  it("subscribes to shortcut events on mount", async () => {
    const handlers = { quick_search: vi.fn() };
    renderHook(() => useShortcuts(handlers));

    // Allow the promise in useEffect to resolve.
    await act(async () => {});

    expect(onShortcut).toHaveBeenCalledOnce();
  });

  it("dispatches to the correct handler", async () => {
    const quickSearch = vi.fn();
    const newCase = vi.fn();
    renderHook(() => useShortcuts({ quick_search: quickSearch, new_case: newCase }));
    await act(async () => {});

    // Simulate shortcut events.
    capturedHandler?.("quick_search");
    expect(quickSearch).toHaveBeenCalledOnce();
    expect(newCase).not.toHaveBeenCalled();

    capturedHandler?.("new_case");
    expect(newCase).toHaveBeenCalledOnce();
  });

  it("ignores actions with no handler registered", async () => {
    const quickSearch = vi.fn();
    renderHook(() => useShortcuts({ quick_search: quickSearch }));
    await act(async () => {});

    // Fire an action that has no registered handler — should not throw.
    expect(() => capturedHandler?.("rename_tag")).not.toThrow();
    expect(quickSearch).not.toHaveBeenCalled();
  });

  it("calls unlisten on unmount", async () => {
    const { unmount } = renderHook(() => useShortcuts({ quick_search: vi.fn() }));
    await act(async () => {});

    unmount();
    expect(mockUnlisten).toHaveBeenCalledOnce();
  });
});
