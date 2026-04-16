// Cross-boundary journey test for settlement workflows.
// Exercises two-step approval, statement generation, check requests,
// and payout through real IPC wrappers against the fake backend.

import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  resetFakeBackend,
  seedUser,
  seedSettlement,
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

import { login, logout } from "../ipc/auth";
import {
  settlementTransition,
  prepareSettlement,
  approveSettlement,
  generateStatement,
  renderStatementHtml,
  generateCheckRequest,
  printArtifact,
} from "../ipc/settlement";

beforeEach(() => resetFakeBackend());

// ── Two-step approval journey ────────────────────────────────────────

describe("settlement two-step approval", () => {
  it("preparer and approver must be different users", async () => {
    // Seed two users
    seedUser({ user_id: "u-prep", username: "preparer", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    seedUser({ user_id: "u-appr", username: "approver", password: "pw", role: "property_manager", tenant_ids: ["t1"], active: true });
    seedSettlement({ settlement_id: "s1", tenant_id: "t1", deposit_cents: 200000, deductions_cents: 50000 });

    // Preparer logs in and prepares
    await login("preparer", "pw");
    const prep = await prepareSettlement("s1", "reviewed all deductions");
    expect(prep.step).toBe("prepared");
    expect(prep.user_id).toBe("u-prep");

    // Preparer tries to also approve — must be denied
    const err = await approveSettlement("s1", "lgtm").catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).type).toBe("permission_denied");
    expect((err as IpcError).message).toMatch(/preparer cannot also approve/);

    // Approver logs in and approves
    await logout();
    await login("approver", "pw");
    const appr = await approveSettlement("s1", "approved");
    expect(appr.step).toBe("approved");
    expect(appr.user_id).toBe("u-appr");
  });

  it("approval requires prepare to be done first", async () => {
    seedUser({ username: "mgr", password: "pw", role: "property_manager", tenant_ids: ["t1"], active: true });
    seedSettlement({ settlement_id: "s2", tenant_id: "t1" });
    await login("mgr", "pw");

    const err = await approveSettlement("s2").catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).message).toMatch(/prepared first/);
  });
});

// ── Settlement workflow transitions ──────────────────────────────────

describe("settlement workflow transitions", () => {
  it("draft → pending_approval → approved → paid full lifecycle", async () => {
    seedUser({ user_id: "u-a", username: "user-a", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    seedSettlement({ settlement_id: "s3", tenant_id: "t1" });
    await login("user-a", "pw");

    const r1 = await settlementTransition("s3", { event: "prepare" });
    expect(r1).toBe("pending_approval");

    const r2 = await settlementTransition("s3", { event: "approve" });
    expect(r2).toBe("approved");

    const r3 = await settlementTransition("s3", { event: "mark_paid" });
    expect(r3).toBe("paid");
  });

  it("withdraw returns to draft from pending_approval", async () => {
    seedUser({ username: "u", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    seedSettlement({ settlement_id: "s4", tenant_id: "t1", status: "pending_approval" });
    await login("u", "pw");

    const r = await settlementTransition("s4", { event: "withdraw" });
    expect(r).toBe("draft");
  });

  it("void from approved", async () => {
    seedUser({ username: "u", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    seedSettlement({ settlement_id: "s5", tenant_id: "t1", status: "approved" });
    await login("u", "pw");

    const r = await settlementTransition("s5", { event: "void" });
    expect(r).toBe("void");
  });

  it("rejects invalid transition", async () => {
    seedUser({ username: "u", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    seedSettlement({ settlement_id: "s6", tenant_id: "t1", status: "draft" });
    await login("u", "pw");

    const err = await settlementTransition("s6", { event: "approve" }).catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
  });
});

// ── Statement generation ─────────────────────────────────────────────

describe("settlement statement", () => {
  it("generates a statement with correct refund calculation", async () => {
    seedUser({ username: "u", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    seedSettlement({
      settlement_id: "s7", tenant_id: "t1",
      deposit_cents: 200000, deductions_cents: 50000,
      case_number: "C-100", resident_name: "Alice Smith",
    });
    await login("u", "pw");

    const stmt = await generateStatement("s7");
    expect(stmt.settlement_id).toBe("s7");
    expect(stmt.case_number).toBe("C-100");
    expect(stmt.resident_display_name).toBe("Alice Smith");
    expect(stmt.deposit_total_cents).toBe(200000);
    expect(stmt.deductions_total_cents).toBe(50000);
    expect(stmt.refund_cents).toBe(150000);
    expect(stmt.currency).toBe("USD");
    expect(stmt.display.deposit_total).toBe("$2000.00");
    expect(stmt.display.refund).toBe("$1500.00");
  });

  it("renders statement HTML", async () => {
    seedUser({ username: "u", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    seedSettlement({ settlement_id: "s8", tenant_id: "t1", case_number: "C-200" });
    await login("u", "pw");

    const html = await renderStatementHtml("s8");
    expect(html).toContain("C-200");
    expect(html).toContain("<html>");
  });
});

// ── Check request + payout ───────────────────────────────────────────

describe("settlement check request", () => {
  it("generates a check request with balanced ledger entries", async () => {
    seedUser({ username: "u", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    seedSettlement({ settlement_id: "s9", tenant_id: "t1", deposit_cents: 100000, deductions_cents: 20000 });
    await login("u", "pw");

    const out = await generateCheckRequest("s9", "Jane Doe", "deposit refund");
    expect(out.check_request.payee_name).toBe("Jane Doe");
    expect(out.check_request.amount_cents).toBe(80000);
    expect(out.check_request.currency).toBe("USD");
    expect(out.check_request.status).toBe("drafted");

    // Ledger must be balanced (Σ = 0)
    const sum = out.ledger.reduce((acc, e) => acc + e.amount_cents, 0);
    expect(sum).toBe(0);

    // Artifact HTML is self-contained
    expect(out.artifact_html).toContain("Jane Doe");
    expect(out.artifact_html).toContain("$800.00");
  });

  it("defaults memo to null when omitted", async () => {
    seedUser({ username: "u", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
    seedSettlement({ settlement_id: "s10", tenant_id: "t1" });
    await login("u", "pw");

    const out = await generateCheckRequest("s10", "Bob");
    expect(out.check_request.memo).toBeNull();
  });
});

// ── printArtifact (browser helper) ───────────────────────────────────

describe("printArtifact", () => {
  it("opens a print window with the artifact HTML", () => {
    const write = vi.fn();
    const open = vi.fn();
    const close = vi.fn();
    const focus = vi.fn();
    const print = vi.fn();
    vi.spyOn(window, "open").mockReturnValueOnce({
      document: { open, write, close },
      focus,
      print,
    } as never);
    vi.useFakeTimers();

    printArtifact("<html>test</html>");
    expect(write).toHaveBeenCalledWith("<html>test</html>");
    expect(focus).toHaveBeenCalled();
    vi.advanceTimersByTime(200);
    expect(print).toHaveBeenCalled();

    vi.useRealTimers();
  });

  it("handles popup blocker gracefully", () => {
    vi.spyOn(window, "open").mockReturnValueOnce(null as never);
    expect(() => printArtifact("<html/>")).not.toThrow();
  });
});
