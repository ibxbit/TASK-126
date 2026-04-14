import { describe, expect, it, vi, beforeEach } from "vitest";
import { defaultExpiryUnix, downloadPackage } from "./sharing";

describe("defaultExpiryUnix", () => {
  it("adds exactly 7 days (in seconds) to the input", () => {
    const from = 1700000000;
    const result = defaultExpiryUnix(from);
    expect(result).toBe(from + 7 * 24 * 3600);
  });

  it("works with zero", () => {
    expect(defaultExpiryUnix(0)).toBe(604800);
  });

  it("preserves precision for large timestamps", () => {
    const from = 2000000000;
    expect(defaultExpiryUnix(from)).toBe(2000000000 + 604800);
  });
});

describe("downloadPackage", () => {
  let createObjUrl: ReturnType<typeof vi.fn>;
  let revokeObjUrl: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.spyOn(document.body, "appendChild").mockImplementation((n) => n as Node);
    createObjUrl = vi.fn().mockReturnValue("blob:zip-url");
    revokeObjUrl = vi.fn();
    globalThis.URL.createObjectURL = createObjUrl;
    globalThis.URL.revokeObjectURL = revokeObjUrl;
  });

  it("creates a zip blob and triggers a download", () => {
    const fakeAnchor = {
      href: "",
      download: "",
      click: vi.fn(),
      remove: vi.fn(),
    };
    vi.spyOn(document, "createElement").mockReturnValue(
      fakeAnchor as unknown as HTMLAnchorElement,
    );

    downloadPackage("export_2024.zip", [0x50, 0x4b, 0x03, 0x04]);

    expect(createObjUrl).toHaveBeenCalledOnce();
    expect(fakeAnchor.click).toHaveBeenCalledOnce();
    expect(fakeAnchor.remove).toHaveBeenCalledOnce();
    expect(revokeObjUrl).toHaveBeenCalledWith("blob:zip-url");
    expect(fakeAnchor.download).toBe("export_2024.zip");
  });
});
