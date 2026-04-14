// IPC bindings for dispute / claim workflows.

import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export type ClaimStatus =
  | "draft"
  | "submitted"
  | "under_review"
  | "confirmed"
  | "resolved"
  | "contested"
  | "auto_cancelled"
  | "withdrawn"
  | "rejected_final"
  | "reopened";

export type ClaimKind = "parcel_ownership" | "deposit_deduction";
export type PartyRole = "claimant" | "respondent";
export type PartyResponse = "accept" | "reject";

export type ClaimEvent =
  | { event: "submit" }
  | { event: "withdraw" }
  | { event: "respondent_engaged" }
  | { event: "party_respond"; party: PartyRole; response: PartyResponse }
  | { event: "mark_resolved" }
  | { event: "manager_reject" }
  | { event: "auto_cancel" }
  | { event: "manager_reopen" };

export interface TransitionOutcome {
  from: ClaimStatus;
  to: ClaimStatus;
  event: string;
}

export async function claimTransition(
  claimId: string,
  event: ClaimEvent,
): Promise<TransitionOutcome> {
  return invoke<TransitionOutcome>("cmd_claim_transition", { claimId, event });
}

// ── Matching ────────────────────────────────────────────────────────────

export interface MatchBreakdown {
  category: number;
  address: number;
  time: number;
  keywords: number;
}

export interface MatchCandidate {
  claim_id: string;
  score: number;
  breakdown: MatchBreakdown;
}

/** Returns candidate claims ranked by similarity, above the configured threshold. */
export async function findMatches(claimId: string): Promise<MatchCandidate[]> {
  return invoke<MatchCandidate[]>("cmd_find_claim_matches", { claimId });
}

// ── Timeout events ──────────────────────────────────────────────────────

export interface AutoCancelEvent {
  claim_id: string;
  tenant_id: string;
  at_unix: number;
}

export async function onClaimAutoCancelled(
  handler: (e: AutoCancelEvent) => void,
): Promise<UnlistenFn> {
  return listen<AutoCancelEvent>(
    "claim://auto_cancelled",
    (evt) => handler(evt.payload),
  );
}
