import { describe, expect, it } from "vitest";
import type {
  RuleKind,
  Severity,
  DistributionMode,
  RuleSpec,
  Demand,
  Assignment,
  TimeWindow,
} from "./scheduling";

describe("Scheduling type contracts", () => {
  it("RuleKind covers all four variants", () => {
    const kinds: RuleKind[] = [
      "unavailable_window",
      "capacity_limit",
      "required_duration",
      "distribution",
    ];
    expect(kinds).toHaveLength(4);
  });

  it("Severity covers hard and soft", () => {
    const severities: Severity[] = ["hard", "soft"];
    expect(severities).toHaveLength(2);
  });

  it("DistributionMode covers consecutive and distributed", () => {
    const modes: DistributionMode[] = ["consecutive", "distributed"];
    expect(modes).toHaveLength(2);
  });

  it("unavailable_window RuleSpec has correct shape", () => {
    const spec: RuleSpec = {
      kind: "unavailable_window",
      resource_id: null,
      windows: [{ start_unix: 1000, end_unix: 2000 }],
    };
    expect(spec.kind).toBe("unavailable_window");
  });

  it("capacity_limit RuleSpec has correct shape", () => {
    const spec: RuleSpec = {
      kind: "capacity_limit",
      resource_id: "room-a",
      max_concurrent: 3,
    };
    expect(spec.kind).toBe("capacity_limit");
    if (spec.kind === "capacity_limit") {
      expect(spec.max_concurrent).toBe(3);
    }
  });

  it("Demand interface can be constructed", () => {
    const demand: Demand = {
      demand_id: "d1",
      duration_seconds: 3600,
      earliest_unix: 1000,
      latest_unix: 5000,
      eligible_resources: ["room-a", "room-b"],
    };
    expect(demand.demand_id).toBe("d1");
    expect(demand.eligible_resources).toHaveLength(2);
  });

  it("Assignment interface can be constructed", () => {
    const assignment: Assignment = {
      resource_id: "room-a",
      window: { start_unix: 1000, end_unix: 2000 },
    };
    expect(assignment.resource_id).toBe("room-a");
    expect(assignment.window.end_unix - assignment.window.start_unix).toBe(1000);
  });

  it("TimeWindow calculates duration correctly", () => {
    const w: TimeWindow = { start_unix: 100, end_unix: 3700 };
    expect(w.end_unix - w.start_unix).toBe(3600);
  });
});
