// Smoke test: confirms the vitest runner is wired and TypeScript
// compiles. Domain-level frontend tests will live next to their
// components as they're added.

import { describe, expect, it } from "vitest";
import { formatParcelTimestamp, parcelStateLabel } from "./ipc/parcel";

describe("ipc helpers", () => {
  it("formats a unix timestamp as US 12-hour", () => {
    // Fixed timestamp → deterministic output regardless of locale.
    const s = formatParcelTimestamp(1_700_000_000);
    // Contains MM/DD/YYYY pieces and AM or PM suffix.
    expect(s).toMatch(/\d{2}\/\d{2}\/\d{4}/);
    expect(s).toMatch(/AM|PM/);
  });

  it("labels every parcel state", () => {
    expect(parcelStateLabel("checked_in")).toBe("Checked-in");
    expect(parcelStateLabel("checked_out")).toBe("Checked-out");
    expect(parcelStateLabel("delivered")).toBe("Delivered");
    expect(parcelStateLabel("receipt_confirmed")).toBe("Receipt Confirmed");
    expect(parcelStateLabel("returned_exception")).toBe("Returned / Exception");
  });
});
