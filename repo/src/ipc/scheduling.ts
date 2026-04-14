// IPC bindings for the scheduling engine.

import { invoke } from "@tauri-apps/api/core";

export type RuleKind =
  | "unavailable_window"
  | "capacity_limit"
  | "required_duration"
  | "distribution";

export type Severity = "hard" | "soft";

export type DistributionMode = "consecutive" | "distributed";

export interface TimeWindow {
  start_unix: number;
  end_unix: number;
}

export type RuleSpec =
  | { kind: "unavailable_window"; resource_id: string | null; windows: TimeWindow[] }
  | { kind: "capacity_limit"; resource_id: string; max_concurrent: number }
  | { kind: "required_duration"; min_seconds: number; max_seconds: number }
  | { kind: "distribution"; mode: DistributionMode; gap_seconds: number };

export interface Demand {
  demand_id: string;
  subject_id?: string | null;
  duration_seconds: number;
  earliest_unix: number;
  latest_unix: number;
  /** Resources eligible for this demand, in preference order. */
  eligible_resources: string[];
}

export interface Assignment {
  resource_id: string;
  subject_id?: string | null;
  window: TimeWindow;
}

export interface ProposedAssignment {
  demand_id: string;
  resource_id: string;
  window: TimeWindow;
  soft_score: number;
  notes: string[];
}

export interface ViolationDetail {
  rule_id: string;
  rule_kind: string;
  severity: Severity;
  message: string;
  weight: number;
}

export interface ConstraintReport {
  hard_violations: ViolationDetail[];
  soft_violations: ViolationDetail[];
  soft_score: number;
}

export interface UnfulfilledDemand {
  demand_id: string;
  best_attempt: ConstraintReport | null;
}

export interface Proposal {
  assigned: ProposedAssignment[];
  unfulfilled: UnfulfilledDemand[];
}

// ── Rule-set lifecycle ──────────────────────────────────────────────────

export async function activateRuleSetVersion(ruleSetId: string): Promise<void> {
  await invoke("cmd_schedule_activate_rule_set", { ruleSetId });
}

// ── Validation & proposal ───────────────────────────────────────────────

/** Validate a single candidate against the active rule set. */
export async function validateAssignment(
  tenantId: string,
  ruleSetName: string,
  candidate: Assignment,
  existing: Assignment[],
): Promise<ConstraintReport> {
  return invoke<ConstraintReport>("cmd_schedule_validate", {
    tenantId,
    ruleSetName,
    candidate,
    existing,
  });
}

/** Run the allocator across a batch of demands. */
export async function proposeSchedule(
  tenantId: string,
  ruleSetName: string,
  demands: Demand[],
  existing: Assignment[],
  strideSeconds: number,
): Promise<Proposal> {
  return invoke<Proposal>("cmd_schedule_propose", {
    tenantId,
    ruleSetName,
    demands,
    existing,
    strideSeconds,
  });
}
