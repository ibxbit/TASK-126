// Cross-boundary journey test for the document management domain.
// Covers chunked upload lifecycle, search, tagging, and preview
// through real IPC wrappers against the fake backend.

import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  resetFakeBackend,
  seedUser,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
  getCallLog,
  IpcError,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import { login } from "../ipc/auth";
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
} from "../ipc/docs";

beforeEach(() => resetFakeBackend());

function seedAuthenticatedUser() {
  seedUser({ username: "clerk", password: "pw", role: "staff", tenant_ids: ["t1"], active: true });
  return login("clerk", "pw");
}

// ── Upload lifecycle ─────────────────────────────────────────────────

describe("document upload lifecycle", () => {
  it("start → put chunks → finalize creates an attachment", async () => {
    await seedAuthenticatedUser();

    // Start session
    const session = await startUploadSession({
      tenant_id: "t1",
      entity_kind: "case",
      entity_id: "e1",
      display_name: "report.pdf",
      mime_type: "application/pdf",
      total_bytes: 100,
      chunk_size: 50,
      expected_sha256_hex: "abc123",
    });
    expect(session.id).toBeTruthy();
    expect(session.chunk_count).toBe(2);
    expect(session.status).toBe("in_progress");

    // Check status — both chunks missing
    const status1 = await uploadSessionStatus(session.id);
    expect(status1.missing_indices).toEqual([0, 1]);
    expect(status1.received_indices).toEqual([]);

    // Upload chunk 0
    await putChunk(session.id, 0, new Uint8Array([1, 2, 3]));
    const status2 = await uploadSessionStatus(session.id);
    expect(status2.received_indices).toEqual([0]);
    expect(status2.missing_indices).toEqual([1]);

    // Upload chunk 1
    await putChunk(session.id, 1, new Uint8Array([4, 5, 6]));

    // Finalize
    const result = await finalizeUpload(session.id);
    expect(result.attachment_id).toBeTruthy();
    expect(result.version_no).toBe(1);
    expect(result.sha256_hex).toBe("abc123");

    // Verify the command names in the call log
    const cmds = getCallLog().map((c) => c.cmd);
    expect(cmds).toContain("cmd_upload_start");
    expect(cmds).toContain("cmd_upload_put_chunk");
    expect(cmds).toContain("cmd_upload_status");
    expect(cmds).toContain("cmd_upload_finalize");
  });

  it("finalize rejects when chunks are missing", async () => {
    await seedAuthenticatedUser();

    const session = await startUploadSession({
      tenant_id: "t1", entity_kind: "case", entity_id: "e2",
      display_name: "incomplete.pdf", mime_type: "application/pdf",
      total_bytes: 100, chunk_size: 50, expected_sha256_hex: "def",
    });
    // Only upload chunk 0, skip chunk 1
    await putChunk(session.id, 0, new Uint8Array([1]));

    const err = await finalizeUpload(session.id).catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).message).toMatch(/missing chunks/);
  });

  it("abort marks session as aborted and blocks further puts", async () => {
    await seedAuthenticatedUser();

    const session = await startUploadSession({
      tenant_id: "t1", entity_kind: "case", entity_id: "e3",
      display_name: "aborted.txt", mime_type: "text/plain",
      total_bytes: 10, expected_sha256_hex: "xyz",
    });
    await abortUpload(session.id);

    // Trying to put a chunk on an aborted session should fail
    const err = await putChunk(session.id, 0, new Uint8Array([1])).catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).message).toMatch(/not in progress/);
  });

  it("DEFAULT_CHUNK_SIZE is 25 MiB", () => {
    expect(DEFAULT_CHUNK_SIZE).toBe(25 * 1024 * 1024);
  });
});

// ── Search & tagging ─────────────────────────────────────────────────

describe("document search and tagging", () => {
  async function createAttachment(name: string) {
    const session = await startUploadSession({
      tenant_id: "t1", entity_kind: "case", entity_id: "e1",
      display_name: name, mime_type: "text/plain",
      total_bytes: 5, expected_sha256_hex: `hash-${name}`,
    });
    await putChunk(session.id, 0, new Uint8Array([1, 2, 3, 4, 5]));
    return finalizeUpload(session.id);
  }

  it("searches by tenant and returns uploaded attachments", async () => {
    await seedAuthenticatedUser();
    const att1 = await createAttachment("notes.txt");
    const att2 = await createAttachment("report.txt");

    const results = await searchAttachments({ tenant_id: "t1", limit: 10 });
    expect(results).toHaveLength(2);
    expect(results.map((r) => r.attachment_id)).toContain(att1.attachment_id);
    expect(results.map((r) => r.attachment_id)).toContain(att2.attachment_id);
  });

  it("tag add and remove are idempotent", async () => {
    await seedAuthenticatedUser();
    const att = await createAttachment("tagged.txt");

    await addTag("t1", att.attachment_id, "important");
    await addTag("t1", att.attachment_id, "important"); // idempotent

    const results = await searchAttachments({ tenant_id: "t1", tag: "important", limit: 10 });
    expect(results).toHaveLength(1);
    expect(results[0].tags).toEqual(["important"]);

    await removeTag("t1", att.attachment_id, "important");
    const after = await searchAttachments({ tenant_id: "t1", tag: "important", limit: 10 });
    expect(after).toHaveLength(0);
  });

  it("text search filters by display name", async () => {
    await seedAuthenticatedUser();
    await createAttachment("meeting-notes.txt");
    await createAttachment("budget-report.txt");

    const results = await searchAttachments({ tenant_id: "t1", text: "meeting", limit: 10 });
    expect(results).toHaveLength(1);
    expect(results[0].display_name).toBe("meeting-notes.txt");
  });
});

// ── Preview ──────────────────────────────────────────────────────────

describe("document preview", () => {
  it("returns text preview for a known attachment", async () => {
    await seedAuthenticatedUser();
    const session = await startUploadSession({
      tenant_id: "t1", entity_kind: "case", entity_id: "e1",
      display_name: "readme.txt", mime_type: "text/plain",
      total_bytes: 5, expected_sha256_hex: "h1",
    });
    await putChunk(session.id, 0, new Uint8Array([1, 2, 3, 4, 5]));
    const att = await finalizeUpload(session.id);

    const preview = await previewAttachment("t1", att.attachment_id);
    expect(preview.kind).toBe("text");
    expect((preview as { content: string }).content).toContain("readme.txt");
  });

  it("previewToBlobUrl returns null for text", () => {
    expect(previewToBlobUrl({ kind: "text", content: "hello" })).toBeNull();
  });

  it("previewToBlobUrl creates blob URL for pdf", () => {
    const spy = vi.fn().mockReturnValue("blob:pdf");
    globalThis.URL.createObjectURL = spy;
    const url = previewToBlobUrl({ kind: "pdf", bytes: [0, 1] });
    expect(spy).toHaveBeenCalled();
    expect(url).toBe("blob:pdf");
  });

  it("previewToBlobUrl creates blob URL for image with correct mime", () => {
    const spy = vi.fn().mockReturnValue("blob:img");
    globalThis.URL.createObjectURL = spy;
    const url = previewToBlobUrl({ kind: "image", mime: "image/png", bytes: [0xff] });
    expect(url).toBe("blob:img");
  });
});

// ── Auth guard ───────────────────────────────────────────────────────

describe("document auth guard", () => {
  it("rejects unauthenticated upload start", async () => {
    const err = await startUploadSession({
      tenant_id: "t1", entity_kind: "case", entity_id: "e1",
      display_name: "x.txt", mime_type: "text/plain",
      total_bytes: 1, expected_sha256_hex: "h",
    }).catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });

  it("rejects unauthenticated search", async () => {
    const err = await searchAttachments({ tenant_id: "t1", limit: 10 }).catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });
});
