// In-memory fake of the Tauri IPC backend.
//
// The fake mirrors the *contract* of the real Rust `#[tauri::command]`
// handlers: same command names, same argument shape, same response
// shape, same `IpcError` envelope. It is intentionally NOT a
// per-test-case mock — the same fake services every command in a
// journey, so a real cross-component flow (login → currentUser →
// openWorkspace → listWindows → logout) hits a single coherent
// backend.
//
// Tests register the fake with `installFakeBackend()` (which replaces
// the `invoke` symbol exported by `@tauri-apps/api/core`) and reset it
// between test files via `resetFakeBackend()`.
//
// Why this exists: a unit test that mocks invoke per-call is a wrapper
// test, not a journey test. Wiring this fake catches the same class
// of bug a real Tauri build would: typos in command names, drifted
// argument shapes, error envelopes the UI doesn't recognize.

import { vi } from "vitest";

// ─── Error envelope (mirrors Rust `IpcError`) ──────────────────────────

export type IpcErrorType =
  | "unauthenticated"
  | "permission_denied"
  | "tenant_scope_violation"
  | "internal";

export class IpcError extends Error {
  constructor(
    public readonly type: IpcErrorType,
    message: string,
    public readonly extras: Record<string, unknown> = {},
  ) {
    super(message);
  }
  /** Mirrors what the Tauri runtime hands the React side. */
  toJSON() {
    return { type: this.type, message: this.message, ...this.extras };
  }
}

// ─── Domain models — minimum needed for the journey ───────────────────

export interface FakeUser {
  user_id: string;
  username: string;
  password: string;
  role: string;
  tenant_ids: string[];
  active: boolean;
}

interface OpenedWindow {
  label: string;
  workspace: "move_out_case" | "parcel_queue" | "claims_inbox";
  instance_id: string;
}

// ─── Parcel domain ───────────────────────────────────────────────────

type ParcelState = "checked_in" | "checked_out" | "delivered" | "receipt_confirmed" | "returned_exception";

interface FakeParcel {
  parcel_id: string;
  tenant_id: string;
  current_state: ParcelState;
  history: Array<{
    id: string;
    tenant_id: string;
    parcel_id: string;
    from_state: ParcelState | null;
    to_state: ParcelState;
    operator_user_id: string;
    occurred_at_unix: number;
    location: string;
    prev_chain_hash: string | null;
    chain_hash: string;
  }>;
}

const PARCEL_TRANSITIONS: Record<string, ParcelState[]> = {
  "": ["checked_in"],
  checked_in: ["checked_out", "returned_exception"],
  checked_out: ["delivered", "returned_exception"],
  delivered: ["receipt_confirmed", "returned_exception"],
  receipt_confirmed: [],
  returned_exception: [],
};

// ─── Claims domain ──────────────────────────────────────────────────

type ClaimStatus = "draft" | "submitted" | "under_review" | "confirmed" | "resolved" | "contested" | "auto_cancelled" | "withdrawn" | "rejected_final" | "reopened";

interface FakeClaim {
  claim_id: string;
  tenant_id: string;
  status: ClaimStatus;
  kind: string;
  description: string;
}

const CLAIM_TRANSITIONS: Record<string, Record<string, ClaimStatus>> = {
  draft: { submit: "submitted" },
  submitted: { withdraw: "withdrawn", respondent_engaged: "under_review" },
  under_review: { party_respond: "confirmed", mark_resolved: "resolved", manager_reject: "rejected_final", auto_cancel: "auto_cancelled" },
  confirmed: { mark_resolved: "resolved" },
  contested: { manager_reject: "rejected_final", manager_reopen: "reopened" },
  reopened: { submit: "submitted" },
};

// ─── Settlement domain ──────────────────────────────────────────────

type SettlementStatus = "draft" | "pending_approval" | "approved" | "paid" | "reopened" | "void";

interface FakeSettlement {
  settlement_id: string;
  tenant_id: string;
  status: SettlementStatus;
  case_number: string;
  resident_name: string;
  deposit_cents: number;
  deductions_cents: number;
  prepared_by: string | null;
  approved_by: string | null;
}

// ─── Docs domain ────────────────────────────────────────────────────

interface FakeUploadSession {
  id: string;
  tenant_id: string;
  chunk_size: number;
  chunk_count: number;
  status: "in_progress" | "finalized" | "aborted";
  received_chunks: Set<number>;
  display_name: string;
  entity_kind: string;
  entity_id: string;
  expected_sha256: string;
}

interface FakeAttachment {
  attachment_id: string;
  tenant_id: string;
  entity_kind: string;
  entity_id: string;
  display_name: string;
  mime_type: string;
  byte_size: number;
  sha256_hex: string;
  tags: string[];
  version_no: number;
}

// ─── Sharing domain ─────────────────────────────────────────────────

interface FakePackage {
  package_id: string;
  tenant_id: string;
  password: string;
  expires_at_unix: number;
  created_at_unix: number;
  revoked: boolean;
  sha256_hex: string;
}

// ─── Analytics domain ───────────────────────────────────────────────

interface FakeEvent {
  id: string;
  category: string;
  kind: string;
  occurred_at_unix: number;
}

interface FakeExperiment {
  experiment_id: string;
  assignments: Map<string, { variant_id: string; variant_name: string }>;
}

// ─── System domain ──────────────────────────────────────────────────

interface FakeVersion {
  id: string;
  version: string;
  package_id: string | null;
  installed_at_unix: number;
  is_active: boolean;
  snapshot_path: string | null;
}

// ─── Aggregate state ────────────────────────────────────────────────

interface FakeState {
  users: FakeUser[];
  currentUser: { user_id: string; username: string; role: string; tenant_ids: string[] } | null;
  windows: OpenedWindow[];
  reminders: Map<string, { id: string; title: string; fire_at_unix: number }>;
  parcels: Map<string, FakeParcel>;
  claims: Map<string, FakeClaim>;
  settlements: Map<string, FakeSettlement>;
  uploadSessions: Map<string, FakeUploadSession>;
  attachments: Map<string, FakeAttachment>;
  packages: Map<string, FakePackage>;
  events: FakeEvent[];
  experiments: Map<string, FakeExperiment>;
  recoveryOutcome: string | null;
  versions: FakeVersion[];
  activeRuleSetId: string | null;
  /** Records every invoke() call for assertions. */
  callLog: Array<{ cmd: string; args: unknown }>;
}

let state: FakeState = freshState();

function freshState(): FakeState {
  return {
    users: [],
    currentUser: null,
    windows: [],
    reminders: new Map(),
    parcels: new Map(),
    claims: new Map(),
    settlements: new Map(),
    uploadSessions: new Map(),
    attachments: new Map(),
    packages: new Map(),
    events: [],
    experiments: new Map(),
    recoveryOutcome: null,
    versions: [],
    activeRuleSetId: null,
    callLog: [],
  };
}

export function resetFakeBackend(): void {
  state = freshState();
}

export function seedUser(u: Omit<FakeUser, "user_id"> & { user_id?: string }): FakeUser {
  const user: FakeUser = {
    user_id: u.user_id ?? `u-${state.users.length + 1}`,
    username: u.username,
    password: u.password,
    role: u.role,
    tenant_ids: u.tenant_ids,
    active: u.active,
  };
  state.users.push(user);
  return user;
}

export function getCallLog(): ReadonlyArray<{ cmd: string; args: unknown }> {
  return state.callLog;
}

export function seedParcel(p: { parcel_id: string; tenant_id: string; current_state?: ParcelState }): void {
  state.parcels.set(p.parcel_id, {
    parcel_id: p.parcel_id,
    tenant_id: p.tenant_id,
    current_state: p.current_state ?? "checked_in",
    history: [],
  });
}

export function seedClaim(c: { claim_id: string; tenant_id: string; status?: ClaimStatus; kind?: string; description?: string }): void {
  state.claims.set(c.claim_id, {
    claim_id: c.claim_id,
    tenant_id: c.tenant_id,
    status: c.status ?? "draft",
    kind: c.kind ?? "parcel_ownership",
    description: c.description ?? "test claim",
  });
}

export function seedSettlement(s: {
  settlement_id: string; tenant_id: string; status?: SettlementStatus;
  case_number?: string; resident_name?: string; deposit_cents?: number; deductions_cents?: number;
}): void {
  state.settlements.set(s.settlement_id, {
    settlement_id: s.settlement_id,
    tenant_id: s.tenant_id,
    status: s.status ?? "draft",
    case_number: s.case_number ?? "C-001",
    resident_name: s.resident_name ?? "Jane Doe",
    deposit_cents: s.deposit_cents ?? 200000,
    deductions_cents: s.deductions_cents ?? 50000,
    prepared_by: null,
    approved_by: null,
  });
}

export function seedPackage(p: {
  package_id: string; tenant_id: string; password: string;
  expires_at_unix: number; created_at_unix?: number;
}): void {
  state.packages.set(p.package_id, {
    package_id: p.package_id,
    tenant_id: p.tenant_id,
    password: p.password,
    expires_at_unix: p.expires_at_unix,
    created_at_unix: p.created_at_unix ?? 1700000000,
    revoked: false,
    sha256_hex: "abc123",
  });
}

export function seedExperiment(e: {
  experiment_id: string;
  variants: Array<{ variant_id: string; variant_name: string }>;
}): void {
  state.experiments.set(e.experiment_id, {
    experiment_id: e.experiment_id,
    assignments: new Map(),
  });
  // Store variant definitions on the experiment for assignment
  (state.experiments.get(e.experiment_id)! as unknown as { _variants: typeof e.variants })._variants = e.variants;
}

export function seedRecoveryOutcome(outcome: string): void {
  state.recoveryOutcome = outcome;
}

export function seedVersion(v: Omit<FakeVersion, "id"> & { id?: string }): void {
  state.versions.push({
    id: v.id ?? `v-${state.versions.length + 1}`,
    version: v.version,
    package_id: v.package_id,
    installed_at_unix: v.installed_at_unix,
    is_active: v.is_active,
    snapshot_path: v.snapshot_path,
  });
}

export function snapshotState() {
  return {
    currentUserName: state.currentUser?.username ?? null,
    windowCount: state.windows.length,
    reminderCount: state.reminders.size,
    parcelCount: state.parcels.size,
    claimCount: state.claims.size,
    settlementCount: state.settlements.size,
    uploadSessionCount: state.uploadSessions.size,
    attachmentCount: state.attachments.size,
    packageCount: state.packages.size,
    eventCount: state.events.length,
  };
}

// ─── Dispatcher ────────────────────────────────────────────────────────

type Handler = (args: Record<string, unknown>) => unknown | Promise<unknown>;

const handlers: Record<string, Handler> = {
  cmd_login: ({ username, password }) => {
    const user = state.users.find((u) => u.username === username);
    if (!user) throw new IpcError("internal", "invalid credentials");
    if (!user.active) throw new IpcError("internal", "account disabled");
    if (user.password !== password) throw new IpcError("internal", "invalid credentials");
    state.currentUser = {
      user_id: user.user_id,
      username: user.username,
      role: user.role,
      tenant_ids: user.tenant_ids,
    };
    return { ...state.currentUser };
  },

  cmd_logout: () => {
    state.currentUser = null;
    return undefined;
  },

  cmd_current_user: () => state.currentUser,

  cmd_open_workspace: ({ workspace, focusPayload }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    if (typeof workspace !== "string") {
      throw new IpcError("internal", "invalid workspace");
    }
    if (!["move_out_case", "parcel_queue", "claims_inbox"].includes(workspace)) {
      throw new IpcError("internal", `unknown workspace: ${workspace}`);
    }
    void focusPayload; // accepted but not stored — mirrors the Rust impl
    const instance_id = `inst-${state.windows.length + 1}`;
    const opened: OpenedWindow = {
      label: `${workspace}:${instance_id}`,
      workspace: workspace as OpenedWindow["workspace"],
      instance_id,
    };
    state.windows.push(opened);
    return opened;
  },

  cmd_focus_window: ({ label }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    if (!state.windows.some((w) => w.label === label)) {
      throw new IpcError("internal", `window not found: ${label}`);
    }
    return undefined;
  },

  cmd_close_window: ({ label }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const idx = state.windows.findIndex((w) => w.label === label);
    if (idx < 0) throw new IpcError("internal", `window not found: ${label}`);
    state.windows.splice(idx, 1);
    return undefined;
  },

  cmd_list_windows: () => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    return [...state.windows];
  },

  cmd_schedule_reminder: ({ reminder }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const r = reminder as { id: string; title: string; fire_at_unix: number };
    state.reminders.set(r.id, r);
    return undefined;
  },

  cmd_cancel_reminder: ({ id }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    state.reminders.delete(id as string);
    return undefined;
  },

  cmd_pending_reminder_count: () => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    return state.reminders.size;
  },

  cmd_show_context_menu: ({ windowLabel, spec }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    if (!state.windows.some((w) => w.label === windowLabel) && windowLabel !== "main") {
      throw new IpcError("internal", `window not found: ${windowLabel}`);
    }
    const s = spec as { target: string };
    return { target: s.target, chosen_id: null };
  },

  // ── Parcel handlers ────────────────────────────────────────────────

  cmd_parcel_available_transitions: ({ tenantId, current }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    void tenantId;
    const key = (current as string) ?? "";
    return PARCEL_TRANSITIONS[key] ?? [];
  },

  cmd_transition_parcel: ({ input }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const inp = input as { parcel_id: string; tenant_id: string; to_state: ParcelState; location: string; notes?: string | null };
    let parcel = state.parcels.get(inp.parcel_id);
    if (!parcel) {
      parcel = { parcel_id: inp.parcel_id, tenant_id: inp.tenant_id, current_state: "" as ParcelState, history: [] };
      state.parcels.set(inp.parcel_id, parcel);
    }
    const allowed = PARCEL_TRANSITIONS[parcel.current_state || ""] ?? [];
    if (!allowed.includes(inp.to_state)) {
      throw new IpcError("internal", `transition from ${parcel.current_state || "null"} to ${inp.to_state} not allowed`);
    }
    const prevHash = parcel.history.length > 0 ? parcel.history[parcel.history.length - 1].chain_hash : null;
    const rec = {
      id: `tr-${parcel.history.length + 1}`,
      tenant_id: inp.tenant_id,
      parcel_id: inp.parcel_id,
      from_state: parcel.current_state || null,
      to_state: inp.to_state,
      operator_user_id: state.currentUser!.user_id,
      occurred_at_unix: Math.floor(Date.now() / 1000),
      location: inp.location,
      prev_chain_hash: prevHash,
      chain_hash: `hash-${parcel.history.length + 1}`,
    };
    parcel.history.push(rec);
    parcel.current_state = inp.to_state;
    return { ...rec };
  },

  cmd_parcel_history: ({ parcelId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const parcel = state.parcels.get(parcelId as string);
    return parcel ? [...parcel.history] : [];
  },

  // ── Claims handlers ────────────────────────────────────────────────

  cmd_claim_transition: ({ claimId, event }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const claim = state.claims.get(claimId as string);
    if (!claim) throw new IpcError("internal", `claim not found: ${claimId}`);
    const evt = event as { event: string };
    const transitions = CLAIM_TRANSITIONS[claim.status];
    if (!transitions || !(evt.event in transitions)) {
      throw new IpcError("internal", `cannot ${evt.event} from ${claim.status}`);
    }
    const from = claim.status;
    claim.status = transitions[evt.event];
    return { from, to: claim.status, event: evt.event };
  },

  cmd_find_claim_matches: ({ claimId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const claim = state.claims.get(claimId as string);
    if (!claim) throw new IpcError("internal", `claim not found: ${claimId}`);
    // Return other claims in the same tenant as potential matches
    const matches: unknown[] = [];
    for (const [id, c] of state.claims) {
      if (id !== claimId && c.tenant_id === claim.tenant_id) {
        matches.push({
          claim_id: id,
          score: 0.75,
          breakdown: { category: 0.3, address: 0.2, time: 0.15, keywords: 0.1 },
        });
      }
    }
    return matches;
  },

  // ── Settlement handlers ────────────────────────────────────────────

  cmd_settlement_transition: ({ settlementId, event }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const s = state.settlements.get(settlementId as string);
    if (!s) throw new IpcError("internal", `settlement not found: ${settlementId}`);
    const evt = event as { event: string };
    const transitions: Record<string, SettlementStatus> = {
      "draft:prepare": "pending_approval",
      "pending_approval:approve": "approved",
      "pending_approval:withdraw": "draft",
      "approved:mark_paid": "paid",
      "approved:void": "void",
      "paid:reopen": "reopened",
      "reopened:prepare": "pending_approval",
    };
    const key = `${s.status}:${evt.event}`;
    const next = transitions[key];
    if (!next) throw new IpcError("internal", `cannot ${evt.event} from ${s.status}`);
    s.status = next;
    return s.status;
  },

  cmd_settlement_prepare: ({ settlementId, notes }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const s = state.settlements.get(settlementId as string);
    if (!s) throw new IpcError("internal", `settlement not found: ${settlementId}`);
    if (s.prepared_by) throw new IpcError("internal", "already prepared");
    s.prepared_by = state.currentUser!.user_id;
    void notes;
    return {
      id: `apr-${settlementId}-prep`,
      settlement_id: settlementId,
      step: "prepared",
      user_id: state.currentUser!.user_id,
      signed_at: Math.floor(Date.now() / 1000),
    };
  },

  cmd_settlement_approve: ({ settlementId, notes }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const s = state.settlements.get(settlementId as string);
    if (!s) throw new IpcError("internal", `settlement not found: ${settlementId}`);
    if (!s.prepared_by) throw new IpcError("internal", "must be prepared first");
    if (s.prepared_by === state.currentUser!.user_id) {
      throw new IpcError("permission_denied", "preparer cannot also approve");
    }
    s.approved_by = state.currentUser!.user_id;
    void notes;
    return {
      id: `apr-${settlementId}-appr`,
      settlement_id: settlementId,
      step: "approved",
      user_id: state.currentUser!.user_id,
      signed_at: Math.floor(Date.now() / 1000),
    };
  },

  cmd_settlement_statement: ({ settlementId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const s = state.settlements.get(settlementId as string);
    if (!s) throw new IpcError("internal", `settlement not found: ${settlementId}`);
    const refund = s.deposit_cents - s.deductions_cents;
    return {
      settlement_id: s.settlement_id,
      tenant_id: s.tenant_id,
      case_number: s.case_number,
      resident_display_name: s.resident_name,
      unit_label: null,
      move_out_date_unix: 1700000000,
      generated_at_unix: Math.floor(Date.now() / 1000),
      currency: "USD",
      deposit_total_cents: s.deposit_cents,
      deductions: [],
      deductions_total_cents: s.deductions_cents,
      refund_cents: refund,
      display: {
        deposit_total: `$${(s.deposit_cents / 100).toFixed(2)}`,
        deductions_total: `$${(s.deductions_cents / 100).toFixed(2)}`,
        refund: `$${(refund / 100).toFixed(2)}`,
      },
    };
  },

  cmd_settlement_statement_html: ({ settlementId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const s = state.settlements.get(settlementId as string);
    if (!s) throw new IpcError("internal", `settlement not found: ${settlementId}`);
    return `<html><body><h1>Statement for ${s.case_number}</h1></body></html>`;
  },

  cmd_settlement_check_request: ({ settlementId, payeeName, memo }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const s = state.settlements.get(settlementId as string);
    if (!s) throw new IpcError("internal", `settlement not found: ${settlementId}`);
    const refund = s.deposit_cents - s.deductions_cents;
    return {
      check_request: {
        id: `chk-${settlementId}`,
        tenant_id: s.tenant_id,
        settlement_id: s.settlement_id,
        payee_name: payeeName,
        amount_cents: refund,
        currency: "USD",
        memo: memo ?? null,
        status: "drafted",
        drafted_at: Math.floor(Date.now() / 1000),
      },
      ledger: [
        { id: "l1", tenant_id: s.tenant_id, journal_id: "j1", settlement_id: s.settlement_id, account: "deposit_liability", amount_cents: -refund, memo: null, occurred_at: Math.floor(Date.now() / 1000) },
        { id: "l2", tenant_id: s.tenant_id, journal_id: "j1", settlement_id: s.settlement_id, account: "refund_payable", amount_cents: refund, memo: null, occurred_at: Math.floor(Date.now() / 1000) },
      ],
      artifact_html: `<html><body>Check for ${payeeName}: $${(refund / 100).toFixed(2)}</body></html>`,
    };
  },

  // ── Docs handlers ──────────────────────────────────────────────────

  cmd_upload_start: ({ init }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const i = init as { tenant_id: string; entity_kind: string; entity_id: string; display_name: string; total_bytes: number; chunk_size?: number; expected_sha256_hex: string };
    const chunkSize = i.chunk_size ?? 25 * 1024 * 1024;
    const chunkCount = Math.max(1, Math.ceil(i.total_bytes / chunkSize));
    const session: FakeUploadSession = {
      id: `sess-${state.uploadSessions.size + 1}`,
      tenant_id: i.tenant_id,
      chunk_size: chunkSize,
      chunk_count: chunkCount,
      status: "in_progress",
      received_chunks: new Set(),
      display_name: i.display_name,
      entity_kind: i.entity_kind,
      entity_id: i.entity_id,
      expected_sha256: i.expected_sha256_hex,
    };
    state.uploadSessions.set(session.id, session);
    return { id: session.id, tenant_id: session.tenant_id, chunk_size: chunkSize, chunk_count: chunkCount, status: "in_progress" };
  },

  cmd_upload_put_chunk: ({ sessionId, chunkIndex }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const sess = state.uploadSessions.get(sessionId as string);
    if (!sess) throw new IpcError("internal", `session not found: ${sessionId}`);
    if (sess.status !== "in_progress") throw new IpcError("internal", "session not in progress");
    sess.received_chunks.add(chunkIndex as number);
    return undefined;
  },

  cmd_upload_status: ({ sessionId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const sess = state.uploadSessions.get(sessionId as string);
    if (!sess) throw new IpcError("internal", `session not found: ${sessionId}`);
    const received = Array.from(sess.received_chunks).sort((a, b) => a - b);
    const missing: number[] = [];
    for (let i = 0; i < sess.chunk_count; i++) {
      if (!sess.received_chunks.has(i)) missing.push(i);
    }
    return { session_id: sess.id, chunk_count: sess.chunk_count, received_indices: received, missing_indices: missing };
  },

  cmd_upload_finalize: ({ sessionId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const sess = state.uploadSessions.get(sessionId as string);
    if (!sess) throw new IpcError("internal", `session not found: ${sessionId}`);
    if (sess.status !== "in_progress") throw new IpcError("internal", "session not in progress");
    if (sess.received_chunks.size < sess.chunk_count) {
      throw new IpcError("internal", "missing chunks");
    }
    sess.status = "finalized";
    const att: FakeAttachment = {
      attachment_id: `att-${state.attachments.size + 1}`,
      tenant_id: sess.tenant_id,
      entity_kind: sess.entity_kind,
      entity_id: sess.entity_id,
      display_name: sess.display_name,
      mime_type: "application/octet-stream",
      byte_size: sess.chunk_size * sess.chunk_count,
      sha256_hex: sess.expected_sha256,
      tags: [],
      version_no: 1,
    };
    state.attachments.set(att.attachment_id, att);
    return { attachment_id: att.attachment_id, version_no: 1, byte_size: att.byte_size, sha256_hex: att.sha256_hex };
  },

  cmd_upload_abort: ({ sessionId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const sess = state.uploadSessions.get(sessionId as string);
    if (!sess) throw new IpcError("internal", `session not found: ${sessionId}`);
    sess.status = "aborted";
    return undefined;
  },

  cmd_attachment_search: ({ query }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const q = query as { tenant_id: string; text?: string; tag?: string; limit: number };
    const results: unknown[] = [];
    for (const att of state.attachments.values()) {
      if (att.tenant_id !== q.tenant_id) continue;
      if (q.tag && !att.tags.includes(q.tag)) continue;
      if (q.text && !att.display_name.toLowerCase().includes(q.text.toLowerCase())) continue;
      results.push({
        attachment_id: att.attachment_id,
        entity_kind: att.entity_kind,
        entity_id: att.entity_id,
        display_name: att.display_name,
        mime_type: att.mime_type,
        byte_size: att.byte_size,
        sha256_hex: att.sha256_hex,
        tags: att.tags,
        latest_version_no: att.version_no,
        created_at: 1700000000,
      });
      if (results.length >= q.limit) break;
    }
    return results;
  },

  cmd_attachment_add_tag: ({ tenantId, attachmentId, tag }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const att = state.attachments.get(attachmentId as string);
    if (!att || att.tenant_id !== tenantId) throw new IpcError("internal", `attachment not found`);
    if (!att.tags.includes(tag as string)) att.tags.push(tag as string);
    return undefined;
  },

  cmd_attachment_remove_tag: ({ tenantId, attachmentId, tag }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const att = state.attachments.get(attachmentId as string);
    if (!att || att.tenant_id !== tenantId) throw new IpcError("internal", `attachment not found`);
    att.tags = att.tags.filter((t) => t !== tag);
    return undefined;
  },

  cmd_attachment_preview: ({ tenantId, attachmentId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const att = state.attachments.get(attachmentId as string);
    if (!att || att.tenant_id !== tenantId) throw new IpcError("internal", `attachment not found`);
    return { kind: "text", content: `Preview of ${att.display_name}` };
  },

  // ── Sharing handlers ──────────────────────────────────────────────

  cmd_wrap_with_watermark: ({ bytes, mime, spec }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const s = spec as { username: string; generated_at_unix: number };
    return `<html><body>Watermarked by ${s.username} (${mime}, ${(bytes as number[]).length} bytes)</body></html>`;
  },

  cmd_share_build_package: ({ input }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const i = input as { tenant_id: string; items: unknown[]; password: string; expires_at_unix: number; created_at_unix: number };
    const pkg: FakePackage = {
      package_id: `pkg-${state.packages.size + 1}`,
      tenant_id: i.tenant_id,
      password: i.password,
      expires_at_unix: i.expires_at_unix,
      created_at_unix: i.created_at_unix,
      revoked: false,
      sha256_hex: `sha-${state.packages.size + 1}`,
    };
    state.packages.set(pkg.package_id, pkg);
    return {
      package_id: pkg.package_id,
      zip_bytes: [0x50, 0x4b, 0x03, 0x04],
      sha256_hex: pkg.sha256_hex,
      contents_summary: `${(i.items as unknown[]).length} items`,
    };
  },

  cmd_share_verify_access: ({ packageId, password }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const pkg = state.packages.get(packageId as string);
    if (!pkg) return { ok: false, reason: "not_found" };
    if (pkg.revoked) return { ok: false, reason: "revoked" };
    const now = Math.floor(Date.now() / 1000);
    if (now > pkg.expires_at_unix) return { ok: false, reason: "expired" };
    if (pkg.password !== password) return { ok: false, reason: "bad_password" };
    return { ok: true };
  },

  cmd_share_revoke: ({ packageId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const pkg = state.packages.get(packageId as string);
    if (!pkg) throw new IpcError("internal", `package not found: ${packageId}`);
    pkg.revoked = true;
    return undefined;
  },

  cmd_share_sweep_expired: () => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const now = Math.floor(Date.now() / 1000);
    let count = 0;
    for (const [id, pkg] of state.packages) {
      if (now > pkg.expires_at_unix && !pkg.revoked) {
        state.packages.delete(id);
        count++;
      }
    }
    return count;
  },

  // ── Scheduling handlers ───────────────────────────────────────────

  cmd_schedule_activate_rule_set: ({ ruleSetId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    state.activeRuleSetId = ruleSetId as string;
    return undefined;
  },

  cmd_schedule_validate: ({ tenantId, ruleSetName, candidate, existing }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    void tenantId; void ruleSetName;
    const c = candidate as { resource_id: string; window: { start_unix: number; end_unix: number } };
    const ex = existing as Array<{ resource_id: string; window: { start_unix: number; end_unix: number } }>;
    const hardViolations: unknown[] = [];
    // Check overlap with existing
    for (const e of ex) {
      if (e.resource_id === c.resource_id && c.window.start_unix < e.window.end_unix && c.window.end_unix > e.window.start_unix) {
        hardViolations.push({ rule_id: "overlap", rule_kind: "capacity_limit", severity: "hard", message: "overlapping assignment", weight: 1 });
      }
    }
    return { hard_violations: hardViolations, soft_violations: [], soft_score: 0 };
  },

  cmd_schedule_propose: ({ tenantId, ruleSetName, demands, existing, strideSeconds }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    void tenantId; void ruleSetName; void strideSeconds;
    const ds = demands as Array<{ demand_id: string; duration_seconds: number; earliest_unix: number; eligible_resources: string[] }>;
    const ex = existing as Array<{ resource_id: string; window: { start_unix: number; end_unix: number } }>;
    const assigned: unknown[] = [];
    const unfulfilled: unknown[] = [];
    for (const d of ds) {
      if (d.eligible_resources.length > 0) {
        assigned.push({
          demand_id: d.demand_id,
          resource_id: d.eligible_resources[0],
          window: { start_unix: d.earliest_unix, end_unix: d.earliest_unix + d.duration_seconds },
          soft_score: 0,
          notes: [],
        });
      } else {
        unfulfilled.push({ demand_id: d.demand_id, best_attempt: null });
      }
    }
    void ex;
    return { assigned, unfulfilled };
  },

  // ── Analytics handlers ────────────────────────────────────────────

  cmd_analytics_track: ({ input }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const i = input as { category: string; kind: string };
    const evt: FakeEvent = {
      id: `evt-${state.events.length + 1}`,
      category: i.category,
      kind: i.kind,
      occurred_at_unix: Math.floor(Date.now() / 1000),
    };
    state.events.push(evt);
    return { ...evt };
  },

  cmd_analytics_funnel: ({ tenantId, funnel, fromUnix, toUnix }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    void tenantId; void fromUnix; void toUnix;
    const f = funnel as { name: string; steps: Array<{ event_kind: string; label: string }> };
    return {
      funnel_name: f.name,
      steps: f.steps.map((s, i) => ({ step_no: i, label: s.label, event_kind: s.event_kind, user_count: 10 - i * 2, conversion_rate: (10 - i * 2) / 10 })),
      overall_conversion_rate: f.steps.length > 0 ? (10 - (f.steps.length - 1) * 2) / 10 : 1,
    };
  },

  cmd_analytics_retention: ({ tenantId, cohortWindowSeconds, followUpWindows, fromUnix, toUnix }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    void tenantId; void fromUnix; void toUnix;
    return {
      cohort_window_seconds: cohortWindowSeconds,
      follow_up_windows: followUpWindows,
      cohorts: [{ cohort_unix: 1700000000, cohort_size: 100, retained: Array.from({ length: followUpWindows as number }, (_, i) => 100 - i * 10) }],
    };
  },

  cmd_analytics_quality: ({ tenantId, kind, fromUnix, toUnix }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    void tenantId; void fromUnix; void toUnix;
    return { total_events: state.events.filter((e) => e.kind === kind).length, success_rate: 0.95, mean_duration_ms: 120, p50_duration_ms: 100, p95_duration_ms: 350 };
  },

  cmd_analytics_export: ({ format, rows }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const r = rows as Array<Record<string, unknown>>;
    if (r.length === 0) return "";
    if (format === "csv") {
      const headers = Object.keys(r[0]);
      return [headers.join(","), ...r.map((row) => headers.map((h) => String(row[h] ?? "")).join(","))].join("\n") + "\n";
    }
    return r.map((row) => JSON.stringify(row)).join("\n") + "\n";
  },

  cmd_experiment_assign: ({ experimentId, subjectId }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const exp = state.experiments.get(experimentId as string);
    if (!exp) throw new IpcError("internal", `experiment not found: ${experimentId}`);
    const existing = exp.assignments.get(subjectId as string);
    if (existing) {
      return { experiment_id: experimentId, subject_id: subjectId, variant_id: existing.variant_id, variant_name: existing.variant_name, assigned_at_unix: Math.floor(Date.now() / 1000), sticky: true };
    }
    const variants = ((exp as unknown as { _variants: Array<{ variant_id: string; variant_name: string }> })._variants) ?? [{ variant_id: "v-default", variant_name: "Control" }];
    const variant = variants[Math.abs(hashCode(subjectId as string)) % variants.length];
    exp.assignments.set(subjectId as string, variant);
    return { experiment_id: experimentId, subject_id: subjectId, variant_id: variant.variant_id, variant_name: variant.variant_name, assigned_at_unix: Math.floor(Date.now() / 1000), sticky: true };
  },

  // ── System handlers ───────────────────────────────────────────────

  cmd_last_recovery_outcome: () => state.recoveryOutcome,

  cmd_open_handles: () => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    return [];
  },

  cmd_update_verify: ({ packagePath }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    if (!(packagePath as string).endsWith(".spkg")) throw new IpcError("internal", "unsupported format");
    return { package_id: "upd-1", version: "1.1.0", created_at_unix: 1700000000, min_required_version: null, notes: null };
  },

  cmd_update_install: ({ packagePath }) => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    void packagePath;
    const prev = state.versions.find((v) => v.is_active);
    if (prev) prev.is_active = false;
    const v: FakeVersion = { id: `v-${state.versions.length + 1}`, version: "1.1.0", package_id: "upd-1", installed_at_unix: Math.floor(Date.now() / 1000), is_active: true, snapshot_path: "/snapshots/1.0.0" };
    state.versions.push(v);
    return { previous_version: prev?.version ?? null, new_version: v.version, snapshot_path: v.snapshot_path, staging_path: "/staging", restart_required: true };
  },

  cmd_update_rollback: () => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    const active = state.versions.find((v) => v.is_active);
    const previous = state.versions.filter((v) => !v.is_active).pop();
    if (!active || !previous) throw new IpcError("internal", "no previous version to rollback to");
    active.is_active = false;
    previous.is_active = true;
    return { from_version: active.version, to_version: previous.version, restart_required: true };
  },

  cmd_list_installed_versions: () => {
    if (!state.currentUser) throw new IpcError("unauthenticated", "session has no principal");
    return state.versions.map((v) => ({ ...v }));
  },
};

function hashCode(s: string): number {
  let hash = 0;
  for (let i = 0; i < s.length; i++) {
    hash = ((hash << 5) - hash + s.charCodeAt(i)) | 0;
  }
  return hash;
}

/**
 * Replace `invoke` from `@tauri-apps/api/core` with a dispatcher into
 * the fake backend. Call from `vi.mock("@tauri-apps/api/core", ...)`
 * factory.
 *
 * Returns the dispatcher so a test can call it directly if desired.
 */
export function fakeInvoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  state.callLog.push({ cmd, args });
  const h = handlers[cmd];
  if (!h) {
    return Promise.reject(
      new IpcError("internal", `fake backend has no handler for ${cmd}`),
    );
  }
  try {
    const out = h(args ?? {});
    return Promise.resolve(out);
  } catch (e) {
    return Promise.reject(e);
  }
}

// ─── Tauri runtime stubs (window + event) ──────────────────────────────

let currentWindowLabel = "main";
export function setCurrentWindowLabel(label: string) {
  currentWindowLabel = label;
}

export function fakeGetCurrentWindow() {
  return { label: currentWindowLabel };
}

type Listener<T> = (e: { payload: T }) => void;
const subscribers: Map<string, Set<Listener<unknown>>> = new Map();

export async function fakeListen<T>(
  event: string,
  cb: Listener<T>,
): Promise<() => void> {
  if (!subscribers.has(event)) subscribers.set(event, new Set());
  const set = subscribers.get(event)!;
  set.add(cb as Listener<unknown>);
  return () => set.delete(cb as Listener<unknown>);
}

/** Emit an event into the fake bus — useful for testing reminder + shortcut handlers. */
export function emit(event: string, payload: unknown): number {
  const set = subscribers.get(event);
  if (!set) return 0;
  for (const cb of set) cb({ payload });
  return set.size;
}

// ─── Test helpers to compose mocks consistently ────────────────────────

/** Standard vi.mock factories so tests don't repeat boilerplate. */
export const tauriMockFactories = {
  core: () => ({ invoke: vi.fn(fakeInvoke) }),
  window: () => ({ getCurrentWindow: vi.fn(fakeGetCurrentWindow) }),
  event: () => ({ listen: vi.fn(fakeListen) }),
};
