// Cross-boundary journey test for the scheduling engine.
// Covers rule-set activation, constraint validation, and proposal generation.

import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  resetFakeBackend,
  seedUser,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
  IpcError,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import { login } from "../ipc/auth";
import {
  activateRuleSetVersion,
  validateAssignment,
  proposeSchedule,
} from "../ipc/scheduling";

beforeEach(() => resetFakeBackend());

function seedAuthenticatedUser() {
  seedUser({ username: "scheduler", password: "pw", role: "property_manager", tenant_ids: ["t1"], active: true });
  return login("scheduler", "pw");
}

// ── Rule-set activation ──────────────────────────────────────────────

describe("scheduling rule-set lifecycle", () => {
  it("activates a rule set version", async () => {
    await seedAuthenticatedUser();
    await expect(activateRuleSetVersion("rs-7")).resolves.toBeUndefined();
  });

  it("rejects unauthenticated activation", async () => {
    const err = await activateRuleSetVersion("rs-1").catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });
});

// ── Constraint validation ────────────────────────────────────────────

describe("scheduling constraint validation", () => {
  it("validates a non-overlapping assignment as clean", async () => {
    await seedAuthenticatedUser();

    const report = await validateAssignment(
      "t1", "default",
      { resource_id: "r1", window: { start_unix: 100, end_unix: 200 } },
      [{ resource_id: "r1", window: { start_unix: 0, end_unix: 50 } }],
    );
    expect(report.hard_violations).toEqual([]);
    expect(report.soft_violations).toEqual([]);
    expect(report.soft_score).toBe(0);
  });

  it("detects overlapping assignments as hard violations", async () => {
    await seedAuthenticatedUser();

    const report = await validateAssignment(
      "t1", "default",
      { resource_id: "r1", window: { start_unix: 30, end_unix: 100 } },
      [{ resource_id: "r1", window: { start_unix: 0, end_unix: 50 } }],
    );
    expect(report.hard_violations).toHaveLength(1);
    expect(report.hard_violations[0].rule_kind).toBe("capacity_limit");
    expect(report.hard_violations[0].severity).toBe("hard");
  });

  it("allows overlapping on different resources", async () => {
    await seedAuthenticatedUser();

    const report = await validateAssignment(
      "t1", "default",
      { resource_id: "r2", window: { start_unix: 30, end_unix: 100 } },
      [{ resource_id: "r1", window: { start_unix: 0, end_unix: 50 } }],
    );
    expect(report.hard_violations).toEqual([]);
  });
});

// ── Proposal generation ──────────────────────────────────────────────

describe("scheduling proposal", () => {
  it("assigns demands to eligible resources", async () => {
    await seedAuthenticatedUser();

    const proposal = await proposeSchedule(
      "t1", "default",
      [
        { demand_id: "d1", duration_seconds: 3600, earliest_unix: 0, latest_unix: 86400, eligible_resources: ["r1", "r2"] },
        { demand_id: "d2", duration_seconds: 1800, earliest_unix: 0, latest_unix: 86400, eligible_resources: ["r2"] },
      ],
      [],
      300,
    );
    expect(proposal.assigned).toHaveLength(2);
    expect(proposal.unfulfilled).toEqual([]);

    // d1 should get r1 (first eligible), d2 should get r2
    expect(proposal.assigned[0].demand_id).toBe("d1");
    expect(proposal.assigned[0].resource_id).toBe("r1");
    expect(proposal.assigned[1].demand_id).toBe("d2");
    expect(proposal.assigned[1].resource_id).toBe("r2");

    // Windows should have correct durations
    expect(proposal.assigned[0].window.end_unix - proposal.assigned[0].window.start_unix).toBe(3600);
  });

  it("marks demands with no eligible resources as unfulfilled", async () => {
    await seedAuthenticatedUser();

    const proposal = await proposeSchedule(
      "t1", "default",
      [{ demand_id: "d3", duration_seconds: 60, earliest_unix: 0, latest_unix: 100, eligible_resources: [] }],
      [],
      60,
    );
    expect(proposal.assigned).toEqual([]);
    expect(proposal.unfulfilled).toHaveLength(1);
    expect(proposal.unfulfilled[0].demand_id).toBe("d3");
  });
});
