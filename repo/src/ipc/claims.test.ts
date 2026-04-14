import { describe, expect, it } from "vitest";
import type { ClaimEvent, ClaimStatus } from "./claims";

describe("ClaimEvent type union", () => {
  it("submit event has correct shape", () => {
    const event: ClaimEvent = { event: "submit" };
    expect(event.event).toBe("submit");
  });

  it("withdraw event has correct shape", () => {
    const event: ClaimEvent = { event: "withdraw" };
    expect(event.event).toBe("withdraw");
  });

  it("party_respond event carries party and response", () => {
    const event: ClaimEvent = {
      event: "party_respond",
      party: "claimant",
      response: "accept",
    };
    expect(event.event).toBe("party_respond");
    if (event.event === "party_respond") {
      expect(event.party).toBe("claimant");
      expect(event.response).toBe("accept");
    }
  });

  it("manager_reopen event has correct shape", () => {
    const event: ClaimEvent = { event: "manager_reopen" };
    expect(event.event).toBe("manager_reopen");
  });

  it("auto_cancel event has correct shape", () => {
    const event: ClaimEvent = { event: "auto_cancel" };
    expect(event.event).toBe("auto_cancel");
  });
});

describe("ClaimStatus type", () => {
  it("all ten statuses are valid string literals", () => {
    const statuses: ClaimStatus[] = [
      "draft", "submitted", "under_review", "confirmed",
      "resolved", "contested", "auto_cancelled", "withdrawn",
      "rejected_final", "reopened",
    ];
    expect(statuses).toHaveLength(10);
    // Each status should be a non-empty string
    for (const s of statuses) {
      expect(typeof s).toBe("string");
      expect(s.length).toBeGreaterThan(0);
    }
  });
});
