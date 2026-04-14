import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { useParcelMachine } from "./useParcelMachine";

vi.mock("../ipc/parcel", () => ({
  availableTransitions: vi.fn(),
  parcelHistory: vi.fn(),
  transitionParcel: vi.fn(),
}));

import { availableTransitions, parcelHistory, transitionParcel } from "../ipc/parcel";

const mockAvail = vi.mocked(availableTransitions);
const mockHistory = vi.mocked(parcelHistory);
const mockTransition = vi.mocked(transitionParcel);

describe("useParcelMachine", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockAvail.mockResolvedValue(["checked_out", "delivered"]);
    mockHistory.mockResolvedValue([]);
    mockTransition.mockResolvedValue({
      id: "r1",
      parcel_id: "p1",
      tenant_id: "t1",
      from_state: "checked_in",
      to_state: "checked_out",
      location: "lobby",
      operator_user_id: "u1",
      occurred_at_unix: 1700000000,
      prev_chain_hash: null,
      chain_hash: "abc",
    });
  });

  it("loads available transitions and history on mount", async () => {
    const { result } = renderHook(() =>
      useParcelMachine("t1", "p1", "checked_in"),
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(mockAvail).toHaveBeenCalledWith("t1", "checked_in");
    expect(mockHistory).toHaveBeenCalledWith("p1");
    expect(result.current.available).toEqual(["checked_out", "delivered"]);
    expect(result.current.error).toBeNull();
  });

  it("sets error on failed refresh", async () => {
    mockAvail.mockRejectedValueOnce("network error");
    const { result } = renderHook(() =>
      useParcelMachine("t1", "p1", "checked_in"),
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.error).toBe("network error");
  });

  it("apply transitions the parcel and refreshes", async () => {
    const { result } = renderHook(() =>
      useParcelMachine("t1", "p1", "checked_in"),
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    await act(async () => {
      const rec = await result.current.apply("checked_out", "lobby", "test");
      expect(rec).toBeTruthy();
      expect(rec?.to_state).toBe("checked_out");
    });

    // transitionParcel called with correct input.
    expect(mockTransition).toHaveBeenCalledWith({
      parcel_id: "p1",
      tenant_id: "t1",
      to_state: "checked_out",
      location: "lobby",
      notes: "test",
    });

    // After transition, current state is updated.
    expect(result.current.current).toBe("checked_out");
  });

  it("apply returns null and sets error on failure", async () => {
    mockTransition.mockRejectedValueOnce({ message: "Guard failed" });

    const { result } = renderHook(() =>
      useParcelMachine("t1", "p1", "checked_in"),
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    await act(async () => {
      const rec = await result.current.apply("delivered", "dock");
      expect(rec).toBeNull();
    });

    expect(result.current.error).toBe("Guard failed");
  });

  it("stringifies unknown error shapes", async () => {
    mockAvail.mockRejectedValueOnce(42);

    const { result } = renderHook(() =>
      useParcelMachine("t1", "p1", "checked_in"),
    );

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    // Numbers get JSON.stringified via the catch-all.
    expect(result.current.error).toBeTruthy();
  });
});
