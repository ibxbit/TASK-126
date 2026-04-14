import { describe, expect, it, vi, beforeAll } from "vitest";
import { DEFAULT_CHUNK_SIZE, previewToBlobUrl, type PreviewPayload } from "./docs";

// jsdom does not implement URL.createObjectURL — polyfill for tests.
beforeAll(() => {
  if (typeof URL.createObjectURL !== "function") {
    URL.createObjectURL = vi.fn().mockReturnValue("blob:mock-url");
  }
  if (typeof URL.revokeObjectURL !== "function") {
    URL.revokeObjectURL = vi.fn();
  }
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
