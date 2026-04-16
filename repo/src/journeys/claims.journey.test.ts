// Cross-boundary journey test for the claims workflow.
// Exercises real IPC wrappers against the fake backend.

import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  resetFakeBackend,
  seedUser,
  seedClaim,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
  emit,
  IpcError,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import { login } from "../ipc/auth";
import { claimTransition, findMatches, onClaimAutoCancelled } from "../ipc/claims";

beforeEach(() => resetFakeBackend());

function seedAuthenticatedUser() {
  seedUser({ username: "reviewer", password: "pw", role: "property_manager", tenant_ids: ["t1"], active: true });
  return login("reviewer", "pw");
}

// ── Happy path: draft → submitted → under_review → confirmed ────────

describe("claims journey happy path", () => {
  it("submits a claim and progresses through review to confirmed", async () => {
    await seedAuthenticatedUser();
    seedClaim({ claim_id: "c1", tenant_id: "t1", status: "draft" });

    const r1 = await claimTransition("c1", { event: "submit" });
    expect(r1.from).toBe("draft");
    expect(r1.to).toBe("submitted");

    const r2 = await claimTransition("c1", { event: "respondent_engaged" });
    expect(r2.from).toBe("submitted");
    expect(r2.to).toBe("under_review");

    const r3 = await claimTransition("c1", { event: "party_respond", party: "claimant", response: "accept" });
    expect(r3.from).toBe("under_review");
    expect(r3.to).toBe("confirmed");
  });

  it("claim can be resolved after confirmation", async () => {
    await seedAuthenticatedUser();
    seedClaim({ claim_id: "c2", tenant_id: "t1", status: "confirmed" });

    const r = await claimTransition("c2", { event: "mark_resolved" });
    expect(r.from).toBe("confirmed");
    expect(r.to).toBe("resolved");
  });
});

// ── Alternate paths ──────────────────────────────────────────────────

describe("claims alternate paths", () => {
  it("claim can be withdrawn from submitted state", async () => {
    await seedAuthenticatedUser();
    seedClaim({ claim_id: "c3", tenant_id: "t1", status: "submitted" });

    const r = await claimTransition("c3", { event: "withdraw" });
    expect(r.from).toBe("submitted");
    expect(r.to).toBe("withdrawn");
  });

  it("manager can reject from under_review", async () => {
    await seedAuthenticatedUser();
    seedClaim({ claim_id: "c4", tenant_id: "t1", status: "under_review" });

    const r = await claimTransition("c4", { event: "manager_reject" });
    expect(r.from).toBe("under_review");
    expect(r.to).toBe("rejected_final");
  });

  it("auto-cancel from under_review", async () => {
    await seedAuthenticatedUser();
    seedClaim({ claim_id: "c5", tenant_id: "t1", status: "under_review" });

    const r = await claimTransition("c5", { event: "auto_cancel" });
    expect(r.from).toBe("under_review");
    expect(r.to).toBe("auto_cancelled");
  });
});

// ── Error paths ──────────────────────────────────────────────────────

describe("claims error paths", () => {
  it("rejects invalid transition (draft → mark_resolved)", async () => {
    await seedAuthenticatedUser();
    seedClaim({ claim_id: "c6", tenant_id: "t1", status: "draft" });

    const err = await claimTransition("c6", { event: "mark_resolved" }).catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).message).toMatch(/cannot/);
  });

  it("rejects transition on nonexistent claim", async () => {
    await seedAuthenticatedUser();

    const err = await claimTransition("nope", { event: "submit" }).catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).message).toMatch(/not found/);
  });

  it("rejects unauthenticated callers", async () => {
    seedClaim({ claim_id: "c7", tenant_id: "t1" });
    const err = await claimTransition("c7", { event: "submit" }).catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });
});

// ── Matching ─────────────────────────────────────────────────────────

describe("claim matching", () => {
  it("finds matches for claims in the same tenant", async () => {
    await seedAuthenticatedUser();
    seedClaim({ claim_id: "c10", tenant_id: "t1", description: "broken door" });
    seedClaim({ claim_id: "c11", tenant_id: "t1", description: "broken window" });
    seedClaim({ claim_id: "c12", tenant_id: "t2", description: "different tenant" });

    const matches = await findMatches("c10");
    expect(matches).toHaveLength(1); // only c11 (same tenant)
    expect(matches[0].claim_id).toBe("c11");
    expect(matches[0].score).toBeGreaterThan(0);
    expect(matches[0].breakdown).toHaveProperty("category");
    expect(matches[0].breakdown).toHaveProperty("address");
    expect(matches[0].breakdown).toHaveProperty("time");
    expect(matches[0].breakdown).toHaveProperty("keywords");
  });

  it("returns empty matches when claim is the only one", async () => {
    await seedAuthenticatedUser();
    seedClaim({ claim_id: "c13", tenant_id: "t1" });

    const matches = await findMatches("c13");
    expect(matches).toEqual([]);
  });
});

// ── Event bus ────────────────────────────────────────────────────────

describe("claim auto-cancel event bus", () => {
  it("onClaimAutoCancelled delivers events emitted onto claim://auto_cancelled", async () => {
    const received: Array<{ claim_id: string }> = [];
    await onClaimAutoCancelled((e) => received.push(e));

    emit("claim://auto_cancelled", { claim_id: "c20", tenant_id: "t1", at_unix: 123 });
    expect(received).toHaveLength(1);
    expect(received[0].claim_id).toBe("c20");
  });
});
