// IPC bindings for watermarked downloads + local share packages.

import { invoke } from "@tauri-apps/api/core";

// ── Watermark ───────────────────────────────────────────────────────────

export interface WatermarkSpec {
  username: string;
  generated_at_unix: number;
  label?: string | null;
}

/**
 * Wrap an asset (PDF / PNG / JPG / TXT) in a watermarked HTML
 * document. Returns the rendered HTML string for in-app preview /
 * print or for embedding in a share package.
 */
export async function wrapWithWatermark(
  bytes: Uint8Array,
  mime: string,
  spec: WatermarkSpec,
): Promise<string> {
  return invoke<string>("cmd_wrap_with_watermark", {
    bytes: Array.from(bytes),
    mime,
    spec,
  });
}

// ── Share package ──────────────────────────────────────────────────────

export interface PackageItem {
  filename: string;
  mime_type: string;
  bytes: number[];           // serialized as Vec<u8> on the Rust side
}

export interface PackageBuildInput {
  tenant_id: string;
  recipient_label?: string | null;
  items: PackageItem[];
  password: string;          // never persisted
  expires_at_unix: number;
  created_at_unix: number;
}

export interface PackageBuildOutcome {
  package_id: string;
  /** AES-encrypted ZIP bytes. The UI typically saves these to disk. */
  zip_bytes: number[];
  sha256_hex: string;
  contents_summary: string;
}

export async function buildSharePackage(
  input: PackageBuildInput,
): Promise<PackageBuildOutcome> {
  return invoke<PackageBuildOutcome>("cmd_share_build_package", { input });
}

/** Default expiry: 7 days from `from`. */
export function defaultExpiryUnix(fromUnix: number): number {
  return fromUnix + 7 * 24 * 3600;
}

/** Trigger a save dialog / download for the produced ZIP bytes. */
export function downloadPackage(filename: string, zipBytes: number[]): void {
  const blob = new Blob([new Uint8Array(zipBytes)], { type: "application/zip" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

// ── Expiry / access ─────────────────────────────────────────────────────

export interface VerifyAccessResult {
  ok: boolean;
  reason?: "expired" | "revoked" | "bad_password" | "not_found";
}

export async function verifyPackageAccess(
  packageId: string,
  password: string,
): Promise<VerifyAccessResult> {
  return invoke<VerifyAccessResult>("cmd_share_verify_access", {
    packageId,
    password,
  });
}

export async function revokePackage(packageId: string): Promise<void> {
  await invoke("cmd_share_revoke", { packageId });
}

/** Manually trigger an expiry sweep (admin tool). */
export async function sweepExpiredPackages(): Promise<number> {
  return invoke<number>("cmd_share_sweep_expired");
}
