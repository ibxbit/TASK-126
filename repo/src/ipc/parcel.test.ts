import { describe, expect, it } from "vitest";
import { formatParcelTimestamp, parcelStateLabel } from "./parcel";

// Pure-function tests — no Tauri runtime needed.

describe("formatParcelTimestamp", () => {
  it("formats epoch zero as a valid date string", () => {
    const s = formatParcelTimestamp(0);
    // Should contain a date pattern and AM/PM
    expect(s).toMatch(/\d{2}\/\d{2}\/\d{4}/);
    expect(s).toMatch(/AM|PM/);
  });

  it("formats a known timestamp deterministically", () => {
    // 2023-11-14 22:13:20 UTC = 1700000000
    const s = formatParcelTimestamp(1_700_000_000);
    expect(s).toMatch(/\d{2}\/\d{2}\/2023/);
    expect(s).toMatch(/AM|PM/);
  });

  it("includes zero-padded hours and minutes", () => {
    const s = formatParcelTimestamp(1_700_000_000);
    // Format: MM/DD/YYYY HH:MM AM/PM
    expect(s).toMatch(/\d{2}:\d{2}/);
  });

  it("handles midnight timestamps", () => {
    // 2024-01-01 00:00:00 UTC = 1704067200
    const s = formatParcelTimestamp(1_704_067_200);
    expect(s).toMatch(/\d{2}\/\d{2}\/202[34]/);
    expect(s).toMatch(/AM|PM/);
  });
});

describe("parcelStateLabel", () => {
  it("maps all five states to human-readable labels", () => {
    const cases: [string, string][] = [
      ["checked_in", "Checked-in"],
      ["checked_out", "Checked-out"],
      ["delivered", "Delivered"],
      ["receipt_confirmed", "Receipt Confirmed"],
      ["returned_exception", "Returned / Exception"],
    ];
    for (const [input, expected] of cases) {
      expect(parcelStateLabel(input as never)).toBe(expected);
    }
  });

  it("returns consistent results on repeated calls", () => {
    const a = parcelStateLabel("delivered");
    const b = parcelStateLabel("delivered");
    expect(a).toBe(b);
  });
});
