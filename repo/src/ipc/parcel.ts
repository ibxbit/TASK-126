// Typed IPC wrappers for parcel lifecycle. UI components should import
// from here — never call `invoke` directly for parcel flows.

import { invoke } from "@tauri-apps/api/core";

export type ParcelState =
  | "checked_in"
  | "checked_out"
  | "delivered"
  | "receipt_confirmed"
  | "returned_exception";

export interface TransitionInput {
  parcel_id: string;
  tenant_id: string;
  to_state: ParcelState;
  location: string;
  notes?: string | null;
  /** Unix seconds UTC. UI formats the display string from this. */
  occurred_at_unix?: number | null;
}

export interface TransitionRecord {
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
}

/** States the user is permitted to move to from `current`. */
export async function availableTransitions(
  tenantId: string,
  current: ParcelState | null,
): Promise<ParcelState[]> {
  return invoke<ParcelState[]>("cmd_parcel_available_transitions", {
    tenantId,
    current,
  });
}

/** Apply a transition. Throws on validation / permission / guard failure. */
export async function transitionParcel(
  input: TransitionInput,
): Promise<TransitionRecord> {
  return invoke<TransitionRecord>("cmd_transition_parcel", { input });
}

/** Full immutable history, oldest → newest. */
export async function parcelHistory(parcelId: string): Promise<TransitionRecord[]> {
  return invoke<TransitionRecord[]>("cmd_parcel_history", { parcelId });
}

// ── Display helpers ─────────────────────────────────────────────────────

/** Format a Unix-seconds timestamp as "MM/DD/YYYY hh:mm AM/PM" (local time). */
export function formatParcelTimestamp(unixSeconds: number): string {
  const d = new Date(unixSeconds * 1000);
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const yyyy = d.getFullYear();

  let hours = d.getHours();
  const ampm = hours >= 12 ? "PM" : "AM";
  hours = hours % 12 || 12;
  const mins = String(d.getMinutes()).padStart(2, "0");

  return `${mm}/${dd}/${yyyy} ${String(hours).padStart(2, "0")}:${mins} ${ampm}`;
}

/** Human-readable state label for UI chips / buttons. */
export function parcelStateLabel(s: ParcelState): string {
  switch (s) {
    case "checked_in": return "Checked-in";
    case "checked_out": return "Checked-out";
    case "delivered": return "Delivered";
    case "receipt_confirmed": return "Receipt Confirmed";
    case "returned_exception": return "Returned / Exception";
  }
}
