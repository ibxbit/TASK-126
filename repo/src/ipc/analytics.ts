// IPC bindings for the analytics framework: tracking, dashboards,
// exports, A/B experiments.

import { invoke } from "@tauri-apps/api/core";

// ── Tracking ────────────────────────────────────────────────────────────

export type EventCategory = "impression" | "click" | "completion" | "conversion";

export interface EventInput {
  tenant_id?: string | null;
  actor_user_id?: string | null;
  session_id?: string | null;
  category: EventCategory;
  kind: string;
  entity_kind?: string | null;
  entity_id?: string | null;
  funnel?: string | null;
  funnel_step?: number | null;
  duration_ms?: number | null;
  success?: boolean | null;
  payload_json?: string | null;
  experiment_id?: string | null;
  variant_id?: string | null;
  occurred_at_unix?: number | null;
}

export interface TrackedEvent {
  id: string;
  category: EventCategory;
  kind: string;
  occurred_at_unix: number;
}

export async function trackEvent(input: EventInput): Promise<TrackedEvent> {
  return invoke<TrackedEvent>("cmd_analytics_track", { input });
}

/** Convenience builders. */
export const track = {
  impression: (kind: string, ctx?: Partial<EventInput>) =>
    trackEvent({ category: "impression", kind, ...ctx }),
  click: (kind: string, ctx?: Partial<EventInput>) =>
    trackEvent({ category: "click", kind, ...ctx }),
  completion: (kind: string, success: boolean, durationMs: number, ctx?: Partial<EventInput>) =>
    trackEvent({ category: "completion", kind, success, duration_ms: durationMs, ...ctx }),
  conversion: (kind: string, ctx?: Partial<EventInput>) =>
    trackEvent({ category: "conversion", kind, ...ctx }),
};

// ── Dashboards ──────────────────────────────────────────────────────────

export interface FunnelStepDef { event_kind: string; label: string; }
export interface FunnelDefinition { name: string; steps: FunnelStepDef[]; }

export interface FunnelStepResult {
  step_no: number;
  label: string;
  event_kind: string;
  user_count: number;
  conversion_rate: number;
}
export interface FunnelResult {
  funnel_name: string;
  steps: FunnelStepResult[];
  overall_conversion_rate: number;
}

export interface RetentionCohort {
  cohort_unix: number;
  cohort_size: number;
  retained: number[];
}
export interface RetentionResult {
  cohort_window_seconds: number;
  follow_up_windows: number;
  cohorts: RetentionCohort[];
}

export interface QualityMetrics {
  total_events: number;
  success_rate: number;
  mean_duration_ms: number;
  p50_duration_ms: number;
  p95_duration_ms: number;
}

export async function loadFunnel(
  tenantId: string,
  funnel: FunnelDefinition,
  fromUnix: number,
  toUnix: number,
): Promise<FunnelResult> {
  return invoke<FunnelResult>("cmd_analytics_funnel", {
    tenantId, funnel, fromUnix, toUnix,
  });
}

export async function loadRetention(
  tenantId: string,
  cohortWindowSeconds: number,
  followUpWindows: number,
  fromUnix: number,
  toUnix: number,
): Promise<RetentionResult> {
  return invoke<RetentionResult>("cmd_analytics_retention", {
    tenantId, cohortWindowSeconds, followUpWindows, fromUnix, toUnix,
  });
}

export async function loadQuality(
  tenantId: string,
  kind: string,
  fromUnix: number,
  toUnix: number,
): Promise<QualityMetrics> {
  return invoke<QualityMetrics>("cmd_analytics_quality", {
    tenantId, kind, fromUnix, toUnix,
  });
}

// ── Exports ─────────────────────────────────────────────────────────────

export type ExportFormat = "csv" | "jsonl";

export async function exportRows(
  format: ExportFormat,
  rows: unknown[],
): Promise<string> {
  return invoke<string>("cmd_analytics_export", { format, rows });
}

/** Trigger a browser-style download of an exported string. */
export function downloadAs(filename: string, content: string, mime: string): void {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

// ── Experiments ─────────────────────────────────────────────────────────

export interface VariantAssignment {
  experiment_id: string;
  subject_id: string;
  variant_id: string;
  variant_name: string;
  assigned_at_unix: number;
  sticky: boolean;
}

export async function assignVariant(
  experimentId: string,
  subjectId: string,
): Promise<VariantAssignment> {
  return invoke<VariantAssignment>("cmd_experiment_assign", {
    experimentId, subjectId,
  });
}
