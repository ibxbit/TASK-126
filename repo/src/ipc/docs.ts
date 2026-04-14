// IPC bindings for local document management: chunked uploads,
// metadata search, tagging, and offline previews.

import { invoke } from "@tauri-apps/api/core";

export const DEFAULT_CHUNK_SIZE = 25 * 1024 * 1024;

// ── Session lifecycle ───────────────────────────────────────────────────

export interface SessionInit {
  tenant_id: string;
  entity_kind: "case" | "settlement" | "parcel" | "claim" | "resident";
  entity_id: string;
  display_name: string;
  mime_type: string;
  total_bytes: number;
  chunk_size?: number;
  expected_sha256_hex: string;
  /** Omit / null to create a new attachment; set to an existing id to upload a new version. */
  target_attachment_id?: string | null;
}

export interface UploadSession {
  id: string;
  tenant_id: string;
  chunk_size: number;
  chunk_count: number;
  status: "in_progress" | "finalized" | "aborted";
}

export interface ChunkStatus {
  session_id: string;
  chunk_count: number;
  received_indices: number[];
  missing_indices: number[];
}

export interface FinalizeOutcome {
  attachment_id: string;
  version_no: number;
  byte_size: number;
  sha256_hex: string;
}

export async function startUploadSession(init: SessionInit): Promise<UploadSession> {
  return invoke<UploadSession>("cmd_upload_start", { init });
}

export async function putChunk(
  sessionId: string,
  chunkIndex: number,
  data: Uint8Array,
): Promise<void> {
  // Tauri serializes Uint8Array as an array of numbers; the backend
  // accepts Vec<u8>. For production volumes a dedicated tauri stream
  // command would be preferable, but this keeps the surface minimal.
  await invoke("cmd_upload_put_chunk", {
    sessionId,
    chunkIndex,
    data: Array.from(data),
  });
}

export async function uploadSessionStatus(sessionId: string): Promise<ChunkStatus> {
  return invoke<ChunkStatus>("cmd_upload_status", { sessionId });
}

export async function finalizeUpload(sessionId: string): Promise<FinalizeOutcome> {
  return invoke<FinalizeOutcome>("cmd_upload_finalize", { sessionId });
}

export async function abortUpload(sessionId: string): Promise<void> {
  await invoke("cmd_upload_abort", { sessionId });
}

/**
 * Resume-aware high-level upload. Computes missing chunks, streams
 * only those, then finalizes. Safe to re-invoke with the same session
 * id after an app restart — the backend holds the session state.
 */
export async function resumeAndFinalize(
  sessionId: string,
  file: Blob,
  chunkSize: number,
  onProgress?: (pct: number) => void,
): Promise<FinalizeOutcome> {
  const status = await uploadSessionStatus(sessionId);
  const received = new Set(status.received_indices);

  for (let i = 0; i < status.chunk_count; i++) {
    if (received.has(i)) continue;
    const start = i * chunkSize;
    const end = Math.min(start + chunkSize, file.size);
    const slice = await file.slice(start, end).arrayBuffer();
    await putChunk(sessionId, i, new Uint8Array(slice));
    onProgress?.((i + 1) / status.chunk_count);
  }
  return finalizeUpload(sessionId);
}

// ── Search & tagging ────────────────────────────────────────────────────

export interface SearchQuery {
  tenant_id: string;
  text?: string | null;
  tag?: string | null;
  mime_type?: string | null;
  entity_kind?: string | null;
  entity_id?: string | null;
  limit: number;
}

export interface SearchHit {
  attachment_id: string;
  entity_kind: string;
  entity_id: string;
  display_name: string;
  mime_type: string;
  byte_size: number;
  sha256_hex: string;
  tags: string[];
  latest_version_no: number;
  created_at: number;
}

export async function searchAttachments(q: SearchQuery): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("cmd_attachment_search", { query: q });
}

export async function addTag(
  tenantId: string, attachmentId: string, tag: string,
): Promise<void> {
  await invoke("cmd_attachment_add_tag", { tenantId, attachmentId, tag });
}

export async function removeTag(
  tenantId: string, attachmentId: string, tag: string,
): Promise<void> {
  await invoke("cmd_attachment_remove_tag", { tenantId, attachmentId, tag });
}

// ── Preview ─────────────────────────────────────────────────────────────

export type PreviewPayload =
  | { kind: "pdf"; bytes: number[] }
  | { kind: "image"; mime: string; bytes: number[] }
  | { kind: "text"; content: string };

export async function previewAttachment(
  tenantId: string,
  attachmentId: string,
  versionNo?: number,
): Promise<PreviewPayload> {
  return invoke<PreviewPayload>("cmd_attachment_preview", {
    tenantId,
    attachmentId,
    versionNo: versionNo ?? null,
  });
}

/** Convert a preview byte array into a blob URL the UI can display. */
export function previewToBlobUrl(p: PreviewPayload): string | null {
  if (p.kind === "pdf") {
    const u8 = new Uint8Array(p.bytes);
    return URL.createObjectURL(new Blob([u8], { type: "application/pdf" }));
  }
  if (p.kind === "image") {
    const u8 = new Uint8Array(p.bytes);
    return URL.createObjectURL(new Blob([u8], { type: p.mime }));
  }
  return null;
}
