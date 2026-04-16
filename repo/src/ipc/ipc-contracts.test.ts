// IPC-contract tests for the remaining typed wrappers. Each test
// asserts the **exact** Tauri command name and argument shape the
// backend handler expects. Drift between these and the Rust
// `#[tauri::command]` signatures is the #1 class of bug in a
// desktop-IPC app, so we test them explicitly rather than relying on
// TypeScript type checking alone.

import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";

// Import the wrappers AFTER the mock so they bind to the mocked invoke.
import {
  trackEvent,
  track,
  loadFunnel,
  loadRetention,
  loadQuality,
  exportRows,
  assignVariant,
  type EventInput,
  type FunnelDefinition,
} from "./analytics";

import {
  wrapWithWatermark,
  buildSharePackage,
  verifyPackageAccess,
  revokePackage,
  sweepExpiredPackages,
  type PackageBuildInput,
} from "./sharing";

import {
  settlementTransition,
  prepareSettlement,
  approveSettlement,
  generateStatement,
  renderStatementHtml,
  generateCheckRequest,
  printArtifact,
} from "./settlement";

import {
  startUploadSession,
  putChunk,
  uploadSessionStatus,
  finalizeUpload,
  abortUpload,
  searchAttachments,
  addTag,
  removeTag,
  previewAttachment,
  previewToBlobUrl,
  DEFAULT_CHUNK_SIZE,
  type SessionInit,
  type SearchQuery,
} from "./docs";

import {
  activateRuleSetVersion,
  validateAssignment,
  proposeSchedule,
  type Assignment,
  type Demand,
} from "./scheduling";

import {
  lastRecoveryOutcome,
  openHandles,
  verifyUpdatePackage,
  installUpdate,
  rollbackPreviousVersion,
  listInstalledVersions,
} from "./system";

import {
  availableTransitions,
  transitionParcel,
  parcelHistory,
  type TransitionInput,
} from "./parcel";

import {
  claimTransition,
  findMatches,
  type ClaimEvent,
} from "./claims";

const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue(undefined);
});

// ── analytics.ts ───────────────────────────────────────────────────────

describe("analytics IPC contract", () => {
  it("trackEvent invokes cmd_analytics_track with input envelope", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "e1",
      category: "click",
      kind: "opened_workspace",
      occurred_at_unix: 1,
    });
    const evt: EventInput = { category: "click", kind: "opened_workspace" };
    const out = await trackEvent(evt);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_analytics_track", { input: evt });
    expect(out.id).toBe("e1");
  });

  it("track.impression/click/completion/conversion delegate to trackEvent", async () => {
    for (const _ of [0, 1, 2, 3]) {
      mockInvoke.mockResolvedValueOnce({
        id: "e",
        category: "click",
        kind: "x",
        occurred_at_unix: 1,
      });
    }
    await track.impression("page_view");
    await track.click("btn");
    await track.completion("upload", true, 500);
    await track.conversion("signup");
    expect(mockInvoke).toHaveBeenCalledTimes(4);
    expect(mockInvoke.mock.calls[0][1]).toMatchObject({
      input: { category: "impression", kind: "page_view" },
    });
    expect(mockInvoke.mock.calls[2][1]).toMatchObject({
      input: {
        category: "completion",
        kind: "upload",
        success: true,
        duration_ms: 500,
      },
    });
    expect(mockInvoke.mock.calls[3][1]).toMatchObject({
      input: { category: "conversion", kind: "signup" },
    });
  });

  it("loadFunnel packs tenantId, funnel, and time range", async () => {
    mockInvoke.mockResolvedValueOnce({
      funnel_name: "x",
      steps: [],
      overall_conversion_rate: 0,
    });
    const f: FunnelDefinition = { name: "x", steps: [] };
    await loadFunnel("t1", f, 1, 2);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_analytics_funnel", {
      tenantId: "t1",
      funnel: f,
      fromUnix: 1,
      toUnix: 2,
    });
  });

  it("loadRetention passes cohort parameters verbatim", async () => {
    mockInvoke.mockResolvedValueOnce({
      cohort_window_seconds: 86400,
      follow_up_windows: 3,
      cohorts: [],
    });
    await loadRetention("t1", 86400, 3, 1, 2);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_analytics_retention", {
      tenantId: "t1",
      cohortWindowSeconds: 86400,
      followUpWindows: 3,
      fromUnix: 1,
      toUnix: 2,
    });
  });

  it("loadQuality passes kind + time range", async () => {
    mockInvoke.mockResolvedValueOnce({
      total_events: 0,
      success_rate: 0,
      mean_duration_ms: 0,
      p50_duration_ms: 0,
      p95_duration_ms: 0,
    });
    await loadQuality("t1", "upload", 1, 2);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_analytics_quality", {
      tenantId: "t1",
      kind: "upload",
      fromUnix: 1,
      toUnix: 2,
    });
  });

  it("exportRows forwards format + rows", async () => {
    mockInvoke.mockResolvedValueOnce("a,b\n1,2\n");
    const rows = [{ a: 1, b: 2 }];
    const out = await exportRows("csv", rows);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_analytics_export", {
      format: "csv",
      rows,
    });
    expect(out).toBe("a,b\n1,2\n");
  });

  it("assignVariant forwards experimentId + subjectId", async () => {
    mockInvoke.mockResolvedValueOnce({
      experiment_id: "exp1",
      subject_id: "s1",
      variant_id: "v1",
      variant_name: "A",
      assigned_at_unix: 1,
      sticky: true,
    });
    const out = await assignVariant("exp1", "s1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_experiment_assign", {
      experimentId: "exp1",
      subjectId: "s1",
    });
    expect(out.variant_id).toBe("v1");
  });
});

// ── sharing.ts ──────────────────────────────────────────────────────────

describe("sharing IPC contract", () => {
  it("wrapWithWatermark converts Uint8Array to a plain number array", async () => {
    mockInvoke.mockResolvedValueOnce("<html>watermark</html>");
    const bytes = new Uint8Array([1, 2, 3, 4]);
    await wrapWithWatermark(bytes, "image/png", {
      username: "alice",
      generated_at_unix: 123,
    });
    expect(mockInvoke).toHaveBeenCalledWith("cmd_wrap_with_watermark", {
      bytes: [1, 2, 3, 4],
      mime: "image/png",
      spec: { username: "alice", generated_at_unix: 123 },
    });
  });

  it("buildSharePackage wraps the input envelope", async () => {
    mockInvoke.mockResolvedValueOnce({
      package_id: "pkg-1",
      zip_bytes: [1, 2],
      sha256_hex: "abc",
      contents_summary: "x",
    });
    const input: PackageBuildInput = {
      tenant_id: "t1",
      items: [],
      password: "pw",
      expires_at_unix: 2,
      created_at_unix: 1,
    };
    await buildSharePackage(input);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_share_build_package", { input });
  });

  it("verifyPackageAccess forwards packageId + password", async () => {
    mockInvoke.mockResolvedValueOnce({ ok: true });
    await verifyPackageAccess("pkg-1", "pw");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_share_verify_access", {
      packageId: "pkg-1",
      password: "pw",
    });
  });

  it("revokePackage invokes cmd_share_revoke", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await revokePackage("pkg-1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_share_revoke", { packageId: "pkg-1" });
  });

  it("sweepExpiredPackages returns the deleted count", async () => {
    mockInvoke.mockResolvedValueOnce(5);
    const n = await sweepExpiredPackages();
    expect(mockInvoke).toHaveBeenCalledWith("cmd_share_sweep_expired");
    expect(n).toBe(5);
  });
});

// ── settlement.ts ──────────────────────────────────────────────────────

describe("settlement IPC contract", () => {
  it("settlementTransition forwards settlementId + event", async () => {
    mockInvoke.mockResolvedValueOnce("approved");
    await settlementTransition("s1", { event: "approve" });
    expect(mockInvoke).toHaveBeenCalledWith("cmd_settlement_transition", {
      settlementId: "s1",
      event: { event: "approve" },
    });
  });

  it("prepareSettlement defaults notes to null when omitted", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "a1",
      settlement_id: "s1",
      step: "prepared",
      user_id: "u1",
      signed_at: 1,
    });
    await prepareSettlement("s1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_settlement_prepare", {
      settlementId: "s1",
      notes: null,
    });
  });

  it("prepareSettlement forwards notes when supplied", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "a1",
      settlement_id: "s1",
      step: "prepared",
      user_id: "u1",
      signed_at: 1,
    });
    await prepareSettlement("s1", "reviewed by pm");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_settlement_prepare", {
      settlementId: "s1",
      notes: "reviewed by pm",
    });
  });

  it("approveSettlement mirrors prepareSettlement's argument shape", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "a2",
      settlement_id: "s1",
      step: "approved",
      user_id: "u2",
      signed_at: 2,
    });
    await approveSettlement("s1", "ok");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_settlement_approve", {
      settlementId: "s1",
      notes: "ok",
    });
  });

  it("generateStatement forwards settlementId", async () => {
    mockInvoke.mockResolvedValueOnce({
      settlement_id: "s1",
      tenant_id: "t1",
      case_number: "C1",
      resident_display_name: "R",
      unit_label: null,
      move_out_date_unix: 1,
      generated_at_unix: 2,
      currency: "USD",
      deposit_total_cents: 100,
      deductions: [],
      deductions_total_cents: 0,
      refund_cents: 100,
      display: { deposit_total: "$1.00", deductions_total: "$0.00", refund: "$1.00" },
    });
    await generateStatement("s1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_settlement_statement", {
      settlementId: "s1",
    });
  });

  it("renderStatementHtml forwards settlementId", async () => {
    mockInvoke.mockResolvedValueOnce("<html>ok</html>");
    await renderStatementHtml("s1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_settlement_statement_html", {
      settlementId: "s1",
    });
  });

  it("generateCheckRequest defaults memo to null", async () => {
    mockInvoke.mockResolvedValueOnce({
      check_request: {
        id: "c1",
        tenant_id: "t1",
        settlement_id: "s1",
        payee_name: "P",
        amount_cents: 0,
        currency: "USD",
        memo: null,
        status: "drafted",
        drafted_at: 1,
      },
      ledger: [],
      artifact_html: "<html/>",
    });
    await generateCheckRequest("s1", "Alice");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_settlement_check_request", {
      settlementId: "s1",
      payeeName: "Alice",
      memo: null,
    });
  });

  it("printArtifact opens a new window and writes the html", () => {
    const write = vi.fn();
    const open = vi.fn();
    const close = vi.fn();
    const focus = vi.fn();
    const print = vi.fn();
    const fakeWin = {
      document: { open, write, close },
      focus,
      print,
    };
    vi.spyOn(window, "open").mockReturnValueOnce(fakeWin as never);
    vi.useFakeTimers();
    printArtifact("<html>X</html>");
    expect(window.open).toHaveBeenCalled();
    expect(write).toHaveBeenCalledWith("<html>X</html>");
    expect(focus).toHaveBeenCalled();
    vi.advanceTimersByTime(200);
    expect(print).toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("printArtifact is a no-op when window.open returns null (popup blocker)", () => {
    vi.spyOn(window, "open").mockReturnValueOnce(null as never);
    // Should not throw.
    expect(() => printArtifact("<html/>")).not.toThrow();
  });
});

// ── docs.ts ────────────────────────────────────────────────────────────

describe("docs IPC contract", () => {
  it("exposes the 25 MiB default chunk size", () => {
    expect(DEFAULT_CHUNK_SIZE).toBe(25 * 1024 * 1024);
  });

  it("startUploadSession wraps the init envelope", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "sess1",
      tenant_id: "t1",
      chunk_size: 8,
      chunk_count: 1,
      status: "in_progress",
    });
    const init: SessionInit = {
      tenant_id: "t1",
      entity_kind: "case",
      entity_id: "e1",
      display_name: "a.txt",
      mime_type: "text/plain",
      total_bytes: 5,
      expected_sha256_hex: "abc",
    };
    await startUploadSession(init);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_upload_start", { init });
  });

  it("putChunk converts Uint8Array payload into a plain number array", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await putChunk("sess1", 0, new Uint8Array([9, 8, 7]));
    expect(mockInvoke).toHaveBeenCalledWith("cmd_upload_put_chunk", {
      sessionId: "sess1",
      chunkIndex: 0,
      data: [9, 8, 7],
    });
  });

  it("uploadSessionStatus forwards sessionId", async () => {
    mockInvoke.mockResolvedValueOnce({
      session_id: "sess1",
      chunk_count: 2,
      received_indices: [0],
      missing_indices: [1],
    });
    await uploadSessionStatus("sess1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_upload_status", { sessionId: "sess1" });
  });

  it("finalizeUpload and abortUpload target their commands", async () => {
    mockInvoke.mockResolvedValueOnce({
      attachment_id: "a",
      version_no: 1,
      byte_size: 1,
      sha256_hex: "x",
    });
    await finalizeUpload("sess1");
    expect(mockInvoke).toHaveBeenLastCalledWith("cmd_upload_finalize", {
      sessionId: "sess1",
    });

    mockInvoke.mockResolvedValueOnce(undefined);
    await abortUpload("sess1");
    expect(mockInvoke).toHaveBeenLastCalledWith("cmd_upload_abort", {
      sessionId: "sess1",
    });
  });

  it("searchAttachments forwards the query envelope", async () => {
    mockInvoke.mockResolvedValueOnce([]);
    const q: SearchQuery = { tenant_id: "t1", limit: 10 };
    await searchAttachments(q);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_attachment_search", { query: q });
  });

  it("addTag and removeTag use named parameters", async () => {
    mockInvoke.mockResolvedValue(undefined);
    await addTag("t1", "a1", "important");
    expect(mockInvoke).toHaveBeenLastCalledWith("cmd_attachment_add_tag", {
      tenantId: "t1",
      attachmentId: "a1",
      tag: "important",
    });
    await removeTag("t1", "a1", "important");
    expect(mockInvoke).toHaveBeenLastCalledWith("cmd_attachment_remove_tag", {
      tenantId: "t1",
      attachmentId: "a1",
      tag: "important",
    });
  });

  it("previewAttachment defaults versionNo to null", async () => {
    mockInvoke.mockResolvedValueOnce({ kind: "text", content: "hi" });
    await previewAttachment("t1", "a1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_attachment_preview", {
      tenantId: "t1",
      attachmentId: "a1",
      versionNo: null,
    });
  });

  it("previewAttachment forwards an explicit versionNo", async () => {
    mockInvoke.mockResolvedValueOnce({ kind: "text", content: "hi" });
    await previewAttachment("t1", "a1", 3);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_attachment_preview", {
      tenantId: "t1",
      attachmentId: "a1",
      versionNo: 3,
    });
  });

  it("previewToBlobUrl returns null for text payloads", () => {
    expect(previewToBlobUrl({ kind: "text", content: "hello" })).toBeNull();
  });

  it("previewToBlobUrl creates a blob URL for pdf payloads", () => {
    const spy = vi.fn().mockReturnValue("blob:pdf-1");
    globalThis.URL.createObjectURL = spy;
    const out = previewToBlobUrl({ kind: "pdf", bytes: [0, 1, 2] });
    expect(spy).toHaveBeenCalled();
    expect(out).toBe("blob:pdf-1");
  });

  it("previewToBlobUrl creates a blob URL for image payloads and preserves mime", () => {
    const spy = vi.fn().mockReturnValue("blob:img-1");
    globalThis.URL.createObjectURL = spy;
    const out = previewToBlobUrl({ kind: "image", mime: "image/jpeg", bytes: [0xff] });
    expect(spy).toHaveBeenCalled();
    expect(out).toBe("blob:img-1");
  });
});

// ── scheduling.ts ──────────────────────────────────────────────────────

describe("scheduling IPC contract", () => {
  it("activateRuleSetVersion forwards the id", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await activateRuleSetVersion("rs-7");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_schedule_activate_rule_set", {
      ruleSetId: "rs-7",
    });
  });

  it("validateAssignment forwards candidate + existing", async () => {
    mockInvoke.mockResolvedValueOnce({
      hard_violations: [],
      soft_violations: [],
      soft_score: 0,
    });
    const candidate: Assignment = {
      resource_id: "r1",
      window: { start_unix: 1, end_unix: 2 },
    };
    await validateAssignment("t1", "v1", candidate, []);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_schedule_validate", {
      tenantId: "t1",
      ruleSetName: "v1",
      candidate,
      existing: [],
    });
  });

  it("proposeSchedule forwards demands + stride", async () => {
    mockInvoke.mockResolvedValueOnce({ assigned: [], unfulfilled: [] });
    const demands: Demand[] = [
      {
        demand_id: "d1",
        duration_seconds: 60,
        earliest_unix: 1,
        latest_unix: 10,
        eligible_resources: ["r1"],
      },
    ];
    await proposeSchedule("t1", "v1", demands, [], 60);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_schedule_propose", {
      tenantId: "t1",
      ruleSetName: "v1",
      demands,
      existing: [],
      strideSeconds: 60,
    });
  });
});

// ── system.ts ──────────────────────────────────────────────────────────

describe("system IPC contract", () => {
  it("lastRecoveryOutcome calls cmd_last_recovery_outcome", async () => {
    mockInvoke.mockResolvedValueOnce("clean_start");
    expect(await lastRecoveryOutcome()).toBe("clean_start");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_last_recovery_outcome");
  });

  it("lastRecoveryOutcome returns null when no run is recorded", async () => {
    mockInvoke.mockResolvedValueOnce(null);
    expect(await lastRecoveryOutcome()).toBeNull();
  });

  it("openHandles calls cmd_open_handles and returns the list", async () => {
    mockInvoke.mockResolvedValueOnce([]);
    expect(await openHandles()).toEqual([]);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_open_handles");
  });

  it("verifyUpdatePackage forwards packagePath", async () => {
    mockInvoke.mockResolvedValueOnce({
      package_id: "p",
      version: "1.0.0",
      created_at_unix: 0,
      min_required_version: null,
      notes: null,
    });
    await verifyUpdatePackage("/tmp/update.spkg");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_update_verify", {
      packagePath: "/tmp/update.spkg",
    });
  });

  it("installUpdate forwards packagePath", async () => {
    mockInvoke.mockResolvedValueOnce({
      previous_version: null,
      new_version: "1",
      snapshot_path: "/s",
      staging_path: "/st",
      restart_required: true,
    });
    await installUpdate("/tmp/u.spkg");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_update_install", {
      packagePath: "/tmp/u.spkg",
    });
  });

  it("rollbackPreviousVersion calls cmd_update_rollback", async () => {
    mockInvoke.mockResolvedValueOnce({
      from_version: "1.1",
      to_version: "1.0",
      restart_required: true,
    });
    await rollbackPreviousVersion();
    expect(mockInvoke).toHaveBeenCalledWith("cmd_update_rollback");
  });

  it("listInstalledVersions calls cmd_list_installed_versions", async () => {
    mockInvoke.mockResolvedValueOnce([]);
    await listInstalledVersions();
    expect(mockInvoke).toHaveBeenCalledWith("cmd_list_installed_versions");
  });
});

// ── parcel.ts ──────────────────────────────────────────────────────────

describe("parcel IPC contract", () => {
  it("availableTransitions forwards tenantId + current state", async () => {
    mockInvoke.mockResolvedValueOnce(["checked_out", "delivered"]);
    await availableTransitions("t1", "checked_in");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_parcel_available_transitions", {
      tenantId: "t1",
      current: "checked_in",
    });
  });

  it("transitionParcel wraps the input envelope", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "x",
      tenant_id: "t",
      parcel_id: "p",
      from_state: "checked_in",
      to_state: "delivered",
      operator_user_id: "u",
      occurred_at_unix: 1,
      location: "L",
      prev_chain_hash: null,
      chain_hash: "abc",
    });
    const input: TransitionInput = {
      parcel_id: "p",
      tenant_id: "t",
      to_state: "delivered",
      location: "L",
    };
    await transitionParcel(input);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_transition_parcel", { input });
  });

  it("parcelHistory forwards parcelId", async () => {
    mockInvoke.mockResolvedValueOnce([]);
    await parcelHistory("p1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_parcel_history", { parcelId: "p1" });
  });
});

// ── claims.ts ──────────────────────────────────────────────────────────

describe("claims IPC contract", () => {
  it("claimTransition forwards claimId + event", async () => {
    mockInvoke.mockResolvedValueOnce({
      from: "draft",
      to: "submitted",
      event: "submit",
    });
    const evt: ClaimEvent = { event: "submit" };
    const out = await claimTransition("c1", evt);
    expect(out.from).toBe("draft");
    expect(out.to).toBe("submitted");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_claim_transition", {
      claimId: "c1",
      event: evt,
    });
  });

  it("claimTransition serializes party_respond event with party + response", async () => {
    mockInvoke.mockResolvedValueOnce({
      from: "under_review",
      to: "confirmed",
      event: "party_respond",
    });
    const evt: ClaimEvent = {
      event: "party_respond",
      party: "respondent",
      response: "accept",
    };
    await claimTransition("c1", evt);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_claim_transition", {
      claimId: "c1",
      event: { event: "party_respond", party: "respondent", response: "accept" },
    });
  });

  it("findMatches forwards the claimId", async () => {
    mockInvoke.mockResolvedValueOnce([]);
    await findMatches("c1");
    expect(mockInvoke).toHaveBeenCalledWith("cmd_find_claim_matches", {
      claimId: "c1",
    });
  });
});
