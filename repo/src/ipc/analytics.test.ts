import { describe, expect, it, vi, beforeEach } from "vitest";
import { downloadAs } from "./analytics";

describe("downloadAs", () => {
  let createObjUrl: ReturnType<typeof vi.fn>;
  let revokeObjUrl: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.spyOn(document.body, "appendChild").mockImplementation((n) => n as Node);
    createObjUrl = vi.fn().mockReturnValue("blob:test-url");
    revokeObjUrl = vi.fn();
    globalThis.URL.createObjectURL = createObjUrl;
    globalThis.URL.revokeObjectURL = revokeObjUrl;
  });

  it("creates a blob and triggers a download link", () => {
    const clickSpy = vi.fn();
    const removeSpy = vi.fn();
    vi.spyOn(document, "createElement").mockReturnValue({
      href: "",
      download: "",
      click: clickSpy,
      remove: removeSpy,
    } as unknown as HTMLAnchorElement);

    downloadAs("report.csv", "a,b,c\n1,2,3", "text/csv");

    expect(createObjUrl).toHaveBeenCalledOnce();
    expect(clickSpy).toHaveBeenCalledOnce();
    expect(removeSpy).toHaveBeenCalledOnce();
    expect(revokeObjUrl).toHaveBeenCalledWith("blob:test-url");
  });

  it("sets the correct filename on the anchor element", () => {
    const fakeAnchor = {
      href: "",
      download: "",
      click: vi.fn(),
      remove: vi.fn(),
    };
    vi.spyOn(document, "createElement").mockReturnValue(
      fakeAnchor as unknown as HTMLAnchorElement,
    );

    downloadAs("events.jsonl", '{"a":1}\n', "application/jsonl");

    expect(fakeAnchor.href).toBe("blob:test-url");
    expect(fakeAnchor.download).toBe("events.jsonl");
  });
});
