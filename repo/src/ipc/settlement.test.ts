import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";
import { printArtifact } from "./settlement";

// Test the pure printArtifact helper — it uses window.open, which
// jsdom stubs.

describe("printArtifact", () => {
  let mockWindow: {
    document: { open: ReturnType<typeof vi.fn>; write: ReturnType<typeof vi.fn>; close: ReturnType<typeof vi.fn> };
    focus: ReturnType<typeof vi.fn>;
    print: ReturnType<typeof vi.fn>;
  };

  beforeEach(() => {
    vi.useFakeTimers();
    mockWindow = {
      document: {
        open: vi.fn(),
        write: vi.fn(),
        close: vi.fn(),
      },
      focus: vi.fn(),
      print: vi.fn(),
    };
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("opens a new window and writes HTML content", () => {
    vi.spyOn(window, "open").mockReturnValue(mockWindow as unknown as Window);

    printArtifact("<html><body>Statement</body></html>");

    expect(window.open).toHaveBeenCalledWith("", "_blank", "width=900,height=1100");
    expect(mockWindow.document.open).toHaveBeenCalled();
    expect(mockWindow.document.write).toHaveBeenCalledWith(
      "<html><body>Statement</body></html>",
    );
    expect(mockWindow.document.close).toHaveBeenCalled();
    expect(mockWindow.focus).toHaveBeenCalled();
  });

  it("calls print() after a short delay", () => {
    vi.spyOn(window, "open").mockReturnValue(mockWindow as unknown as Window);

    printArtifact("<p>test</p>");
    expect(mockWindow.print).not.toHaveBeenCalled();

    vi.advanceTimersByTime(50);
    expect(mockWindow.print).toHaveBeenCalledOnce();
  });

  it("handles popup blocker gracefully (window.open returns null)", () => {
    vi.spyOn(window, "open").mockReturnValue(null);

    // Should not throw
    expect(() => printArtifact("<p>test</p>")).not.toThrow();
  });
});
