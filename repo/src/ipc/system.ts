// IPC bindings for recovery, updates, and rollback. These commands
// are admin-only and gated by `Permission::ConfigurePermissions` in
// the backend.

import { invoke } from "@tauri-apps/api/core";

export type RecoveryOutcome =
  | "clean_start"
  | "unclean_repaired"
  | "integrity_failed";

/**
 * Returns the most recent recovery outcome (recorded at startup).
 * The UI uses this to surface a "previous run did not exit cleanly"
 * banner or a rollback prompt on integrity failure.
 */
export async function lastRecoveryOutcome(): Promise<RecoveryOutcome | null> {
  return invoke<RecoveryOutcome | null>("cmd_last_recovery_outcome");
}

/** Snapshot of currently-tracked file/db handles (admin diagnostics). */
export interface HandleEntry {
  id: string;
  kind: "file" | "db_connection" | "upload_chunk" | "other";
  label: string;
  opened_at_unix: number;
}
export async function openHandles(): Promise<HandleEntry[]> {
  return invoke<HandleEntry[]>("cmd_open_handles");
}

// ── Updates ─────────────────────────────────────────────────────────────

export interface VerifiedPackageInfo {
  package_id: string;
  version: string;
  created_at_unix: number;
  min_required_version: string | null;
  notes: string | null;
}

/** Verify a `.spkg` at `packagePath` against the embedded public key. */
export async function verifyUpdatePackage(
  packagePath: string,
): Promise<VerifiedPackageInfo> {
  return invoke<VerifiedPackageInfo>("cmd_update_verify", { packagePath });
}

export interface InstallOutcome {
  previous_version: string | null;
  new_version: string;
  snapshot_path: string;
  staging_path: string;
  restart_required: boolean;
}
export async function installUpdate(
  packagePath: string,
): Promise<InstallOutcome> {
  return invoke<InstallOutcome>("cmd_update_install", { packagePath });
}

// ── Rollback ────────────────────────────────────────────────────────────

export interface RollbackOutcome {
  from_version: string;
  to_version: string;
  restart_required: boolean;
}

/** Roll back to the immediately-previous installed version. */
export async function rollbackPreviousVersion(): Promise<RollbackOutcome> {
  return invoke<RollbackOutcome>("cmd_update_rollback");
}

export interface InstalledVersion {
  id: string;
  version: string;
  package_id: string | null;
  installed_at_unix: number;
  is_active: boolean;
  snapshot_path: string | null;
}

export async function listInstalledVersions(): Promise<InstalledVersion[]> {
  return invoke<InstalledVersion[]>("cmd_list_installed_versions");
}
