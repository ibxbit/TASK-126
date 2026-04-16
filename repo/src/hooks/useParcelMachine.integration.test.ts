// Integration test for useParcelMachine hook using the fake backend
// instead of per-call mocks. This proves the hook works against a
// coherent IPC layer — same command names, same state transitions.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import {
  resetFakeBackend,
  seedUser,
  seedParcel,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import { login } from "../ipc/auth";
import { useParcelMachine } from "./useParcelMachine";

beforeEach(() => resetFakeBackend());

async function seedAndLogin() {
  seedUser({ username: "ops", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
  await login("ops", "pw");
}

describe("useParcelMachine integration (fake backend)", () => {
  it("loads available transitions from the real dispatcher on mount", async () => {
    await seedAndLogin();
    seedParcel({ parcel_id: "p1", tenant_id: "t1", current_state: "checked_in" });

    const { result } = renderHook(() => useParcelMachine("t1", "p1", "checked_in"));

    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.available).toContain("checked_out");
    expect(result.current.available).toContain("returned_exception");
    expect(result.current.error).toBeNull();
  });

  it("apply transitions the parcel through the dispatcher and updates state", async () => {
    await seedAndLogin();
    seedParcel({ parcel_id: "p2", tenant_id: "t1", current_state: "checked_in" });

    const { result } = renderHook(() => useParcelMachine("t1", "p2", "checked_in"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    let rec: unknown = null;
    await act(async () => {
      rec = await result.current.apply("checked_out", "lobby");
    });

    expect(rec).toBeTruthy();
    expect(result.current.current).toBe("checked_out");
    // History should now contain the transition
    expect(result.current.history.length).toBeGreaterThanOrEqual(1);
  });

  it("apply sets error when transition is invalid", async () => {
    await seedAndLogin();
    seedParcel({ parcel_id: "p3", tenant_id: "t1", current_state: "checked_in" });

    const { result } = renderHook(() => useParcelMachine("t1", "p3", "checked_in"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    await act(async () => {
      const rec = await result.current.apply("receipt_confirmed", "dock");
      expect(rec).toBeNull();
    });

    expect(result.current.error).toBeTruthy();
    expect(result.current.error).toMatch(/not allowed/);
  });

  it("refresh reloads transitions after state change", async () => {
    await seedAndLogin();
    seedParcel({ parcel_id: "p4", tenant_id: "t1", current_state: "checked_in" });

    const { result } = renderHook(() => useParcelMachine("t1", "p4", "checked_in"));
    await waitFor(() => expect(result.current.loading).toBe(false));

    // Apply first transition
    await act(async () => {
      await result.current.apply("checked_out", "lobby");
    });

    // After transition, available should reflect checked_out's transitions
    await waitFor(() => {
      expect(result.current.available).toContain("delivered");
    });
  });

  it("sets error on mount when unauthenticated", async () => {
    // No login — all IPC calls will fail with unauthenticated
    const { result } = renderHook(() => useParcelMachine("t1", "p5", "checked_in"));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toBeTruthy();
  });
});
