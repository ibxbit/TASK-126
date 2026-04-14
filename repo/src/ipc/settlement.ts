// IPC bindings for move-out / deposit settlement workflows.

import { invoke } from "@tauri-apps/api/core";

export type SettlementStatus =
  | "draft"
  | "pending_approval"
  | "approved"
  | "paid"
  | "reopened"
  | "void";

export type SettlementEvent =
  | { event: "prepare" }
  | { event: "approve" }
  | { event: "withdraw" }
  | { event: "mark_paid" }
  | { event: "void" }
  | { event: "reopen" };

export interface ApprovalRecord {
  id: string;
  settlement_id: string;
  step: "prepared" | "approved";
  user_id: string;
  signed_at: number;
}

export interface DeductionLine {
  id: string;
  category: string;
  description: string;
  amount_cents: number;
  evidence_count: number;
}

export interface SettlementStatement {
  settlement_id: string;
  tenant_id: string;
  case_number: string;
  resident_display_name: string;
  unit_label: string | null;
  move_out_date_unix: number;
  generated_at_unix: number;
  currency: string;
  deposit_total_cents: number;
  deductions: DeductionLine[];
  deductions_total_cents: number;
  refund_cents: number;
  display: {
    deposit_total: string;
    deductions_total: string;
    refund: string;
  };
}

export interface CheckRequest {
  id: string;
  tenant_id: string;
  settlement_id: string;
  payee_name: string;
  amount_cents: number;
  currency: string;
  memo: string | null;
  status: "drafted" | "printed" | "voided";
  drafted_at: number;
}

export interface LedgerEntry {
  id: string;
  tenant_id: string;
  journal_id: string;
  settlement_id: string | null;
  account: "deposit_liability" | "refund_payable" | "forfeited_revenue" | "clearing";
  amount_cents: number;
  memo: string | null;
  occurred_at: number;
}

export interface PayoutOutcome {
  check_request: CheckRequest;
  ledger: LedgerEntry[];
  /** Self-contained printable HTML for the check request. */
  artifact_html: string;
}

// ── Workflow ────────────────────────────────────────────────────────────

export async function settlementTransition(
  settlementId: string,
  event: SettlementEvent,
): Promise<SettlementStatus> {
  return invoke<SettlementStatus>("cmd_settlement_transition", {
    settlementId,
    event,
  });
}

export async function prepareSettlement(
  settlementId: string,
  notes?: string,
): Promise<ApprovalRecord> {
  return invoke<ApprovalRecord>("cmd_settlement_prepare", {
    settlementId,
    notes: notes ?? null,
  });
}

export async function approveSettlement(
  settlementId: string,
  notes?: string,
): Promise<ApprovalRecord> {
  return invoke<ApprovalRecord>("cmd_settlement_approve", {
    settlementId,
    notes: notes ?? null,
  });
}

// ── Statement ──────────────────────────────────────────────────────────

export async function generateStatement(
  settlementId: string,
): Promise<SettlementStatement> {
  return invoke<SettlementStatement>("cmd_settlement_statement", {
    settlementId,
  });
}

export async function renderStatementHtml(
  settlementId: string,
): Promise<string> {
  return invoke<string>("cmd_settlement_statement_html", { settlementId });
}

// ── Payout ─────────────────────────────────────────────────────────────

export async function generateCheckRequest(
  settlementId: string,
  payeeName: string,
  memo?: string,
): Promise<PayoutOutcome> {
  return invoke<PayoutOutcome>("cmd_settlement_check_request", {
    settlementId,
    payeeName,
    memo: memo ?? null,
  });
}

/**
 * Open the printable artifact in the current window's print dialog.
 * Self-contained HTML — no network fetch required.
 */
export function printArtifact(html: string): void {
  const w = window.open("", "_blank", "width=900,height=1100");
  if (!w) return;
  w.document.open();
  w.document.write(html);
  w.document.close();
  w.focus();
  // Slight defer to let the WebView lay out before invoking print().
  setTimeout(() => w.print(), 50);
}
