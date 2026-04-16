import { describe, expect, it, vi, beforeAll, beforeEach } from "vitest";
import {
  DEFAULT_CHUNK_SIZE,
  previewToBlobUrl,
  resumeAndFinalize,
  type PreviewPayload,
} from "./docs";

// ── Mocks ──────────────────────────────────────────────────────────────
// Mock the individual IPC functions that resumeAndFinalize depends on,
// rather than the raw invoke, so we avoid jsdom Blob limitations.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const mockInvoke = vi.mocked(invoke);

// jsdom does not implement URL.createObjectURL — polyfill for tests.
beforeAll(() => {
  if (typeof URL.createObjectURL !== "function") {
    URL.createObjectURL = vi.fn().mockReturnValue("blob:mock-url");
  }
  if (typeof URL.revokeObjectURL !== "function") {
    URL.revokeObjectURL = vi.fn();
  }
});

beforeEach(() => {
  vi.clearAllMocks();
});

describe("DEFAULT_CHUNK_SIZE", () => {
  it("equals 25 MB", () => {
    expect(DEFAULT_CHUNK_SIZE).toBe(25 * 1024 * 1024);
  });
});

describe("previewToBlobUrl", () => {
  it("returns a blob URL for a PDF payload", () => {
    const payload: PreviewPayload = { kind: "pdf", bytes: [0x25, 0x50, 0x44, 0x46] };
    const url = previewToBlobUrl(payload);
    expect(url).toBeTruthy();
    expect(typeof url).toBe("string");
  });

  it("returns a blob URL for an image payload", () => {
    const payload: PreviewPayload = { kind: "image", mime: "image/png", bytes: [0x89, 0x50, 0x4e, 0x47] };
    const url = previewToBlobUrl(payload);
    expect(url).toBeTruthy();
    expect(typeof url).toBe("string");
  });

  it("returns null for a text payload", () => {
    const payload: PreviewPayload = { kind: "text", content: "Hello, world" };
    const url = previewToBlobUrl(payload);
    expect(url).toBeNull();
  });
});

describe("resumeAndFinalize", () => {
  // jsdom's Blob.slice().arrayBuffer() is not always available.
  // We create a minimal Blob-like object that supports slice + arrayBuffer.
  function createFakeBlob(size: number) {
    const data = new Uint8Array(size);
    const blob = {
      size,
      slice(start: number, end: number) {
        return {
          arrayBuffer() {
            return Promise.resolve(data.slice(start, end).buffer);
          },
        };
      },
    };
    return blob as unknown as Blob;
  }

  it("uploads only missing chunks and calls finalize", async () => {
    const chunkSize = 10;

    mockInvoke
      // cmd_upload_status
      .mockResolvedValueOnce({
        session_id: "s1",
        chunk_count: 3,
        received_indices: [1],
        missing_indices: [0, 2],
      })
      // cmd_upload_put_chunk for chunk 0
      .mockResolvedValueOnce(undefined)
      // cmd_upload_put_chunk for chunk 2
      .mockResolvedValueOnce(undefined)
      // cmd_upload_finalize
      .mockResolvedValueOnce({
        attachment_id: "att-1",
        version_no: 1,
        byte_size: 25,
        sha256_hex: "abc",
      });

    const onProgress = vi.fn();
    const blob = createFakeBlob(25);
    const result = await resumeAndFinalize("s1", blob, chunkSize, onProgress);

    expect(result.attachment_id).toBe("att-1");
    // Should have called: status, put_chunk x2, finalize = 4 calls
    expect(mockInvoke).toHaveBeenCalledTimes(4);
    // Progress callback was called for chunks 0 and 2 (indices in loop)
    expect(onProgress).toHaveBeenCalledTimes(2);
    expect(onProgress).toHaveBeenCalledWith(1 / 3); // after chunk 0
    expect(onProgress).toHaveBeenCalledWith(3 / 3); // after chunk 2
  });

  it("skips all uploads when every chunk is already received", async () => {
    mockInvoke
      .mockResolvedValueOnce({
        session_id: "s2",
        chunk_count: 2,
        received_indices: [0, 1],
        missing_indices: [],
      })
      .mockResolvedValueOnce({
        attachment_id: "att-2",
        version_no: 1,
        byte_size: 20,
        sha256_hex: "def",
      });

    const blob = createFakeBlob(20);
    const result = await resumeAndFinalize("s2", blob, 10);

    expect(result.attachment_id).toBe("att-2");
    // Only status + finalize, no put_chunk calls
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });

  it("works without an onProgress callback", async () => {
    mockInvoke
      .mockResolvedValueOnce({
        session_id: "s3",
        chunk_count: 1,
        received_indices: [],
        missing_indices: [0],
      })
      .mockResolvedValueOnce(undefined) // put_chunk
      .mockResolvedValueOnce({
        attachment_id: "att-3",
        version_no: 1,
        byte_size: 5,
        sha256_hex: "ghi",
      });

    const blob = createFakeBlob(5);
    // No onProgress — should not throw
    const result = await resumeAndFinalize("s3", blob, 10);
    expect(result.attachment_id).toBe("att-3");
  });
});
