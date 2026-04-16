// Cross-boundary journey test for the parcel lifecycle.
// Exercises the real IPC wrappers (src/ipc/parcel.ts) against the
// fake backend — same command names, argument shapes, error envelopes.

import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  resetFakeBackend,
  seedUser,
  seedParcel,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
  getCallLog,
  IpcError,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import { login } from "../ipc/auth";
import {
  availableTransitions,
  transitionParcel,
  parcelHistory,
  formatParcelTimestamp,
  parcelStateLabel,
} from "../ipc/parcel";

beforeEach(() => resetFakeBackend());

function seedAuthenticatedUser() {
  seedUser({ username: "ops", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
  return login("ops", "pw");
}

// ── Happy path: full lifecycle ───────────────────────────────────────

describe("parcel lifecycle journey", () => {
  it("walks checked_in → checked_out → delivered → receipt_confirmed with chained history", async () => {
    await seedAuthenticatedUser();
    seedParcel({ parcel_id: "p1", tenant_id: "t1" });

    // Step 1: initial transitions from checked_in
    const avail1 = await availableTransitions("t1", "checked_in");
    expect(avail1).toContain("checked_out");
    expect(avail1).toContain("returned_exception");

    // Step 2: transition to checked_out
    const r1 = await transitionParcel({
      parcel_id: "p1", tenant_id: "t1", to_state: "checked_out", location: "lobby",
    });
    expect(r1.from_state).toBe("checked_in");
    expect(r1.to_state).toBe("checked_out");
    expect(r1.chain_hash).toBeTruthy();
    expect(r1.prev_chain_hash).toBeNull(); // first transition

    // Step 3: transition to delivered
    const r2 = await transitionParcel({
      parcel_id: "p1", tenant_id: "t1", to_state: "delivered", location: "unit-3B",
    });
    expect(r2.from_state).toBe("checked_out");
    expect(r2.to_state).toBe("delivered");
    expect(r2.prev_chain_hash).toBe(r1.chain_hash); // chain linked

    // Step 4: transition to receipt_confirmed
    const r3 = await transitionParcel({
      parcel_id: "p1", tenant_id: "t1", to_state: "receipt_confirmed", location: "unit-3B",
    });
    expect(r3.from_state).toBe("delivered");
    expect(r3.to_state).toBe("receipt_confirmed");
    expect(r3.prev_chain_hash).toBe(r2.chain_hash);

    // Step 5: verify full history
    const hist = await parcelHistory("p1");
    expect(hist).toHaveLength(3);
    expect(hist.map((h) => h.to_state)).toEqual(["checked_out", "delivered", "receipt_confirmed"]);

    // Verify all invoke calls hit the right command names
    const cmds = getCallLog().map((c) => c.cmd);
    expect(cmds).toContain("cmd_parcel_available_transitions");
    expect(cmds).toContain("cmd_transition_parcel");
    expect(cmds).toContain("cmd_parcel_history");
  });

  it("receipt_confirmed is a terminal state with no further transitions", async () => {
    await seedAuthenticatedUser();
    const avail = await availableTransitions("t1", "receipt_confirmed");
    expect(avail).toEqual([]);
  });

  it("returned_exception is a terminal state", async () => {
    await seedAuthenticatedUser();
    const avail = await availableTransitions("t1", "returned_exception");
    expect(avail).toEqual([]);
  });
});

// ── Error paths ──────────────────────────────────────────────────────

describe("parcel lifecycle error paths", () => {
  it("rejects an invalid transition (checked_in → receipt_confirmed)", async () => {
    await seedAuthenticatedUser();
    seedParcel({ parcel_id: "p2", tenant_id: "t1", current_state: "checked_in" });

    const err = await transitionParcel({
      parcel_id: "p2", tenant_id: "t1", to_state: "receipt_confirmed", location: "x",
    }).catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).message).toMatch(/not allowed/);
  });

  it("rejects transitions for unauthenticated callers", async () => {
    const err = await availableTransitions("t1", "checked_in").catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).type).toBe("unauthenticated");
  });

  it("returns empty history for unknown parcel", async () => {
    await seedAuthenticatedUser();
    const hist = await parcelHistory("nonexistent");
    expect(hist).toEqual([]);
  });
});

// ── Display helpers (pure, no IPC) ───────────────────────────────────

describe("parcel display helpers", () => {
  it("formatParcelTimestamp produces MM/DD/YYYY hh:mm AM/PM", () => {
    const s = formatParcelTimestamp(1_700_000_000);
    expect(s).toMatch(/^\d{2}\/\d{2}\/\d{4} \d{2}:\d{2} (AM|PM)$/);
  });

  it("parcelStateLabel maps every state", () => {
    expect(parcelStateLabel("checked_in")).toBe("Checked-in");
    expect(parcelStateLabel("checked_out")).toBe("Checked-out");
    expect(parcelStateLabel("delivered")).toBe("Delivered");
    expect(parcelStateLabel("receipt_confirmed")).toBe("Receipt Confirmed");
    expect(parcelStateLabel("returned_exception")).toBe("Returned / Exception");
  });
});
