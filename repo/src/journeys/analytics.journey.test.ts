// Cross-boundary journey test for the analytics domain.
// Covers event tracking, funnel/retention/quality dashboards,
// exports, and A/B experiment assignment.

import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  resetFakeBackend,
  seedUser,
  seedExperiment,
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
  trackEvent,
  track,
  loadFunnel,
  loadRetention,
  loadQuality,
  exportRows,
  assignVariant,
  downloadAs,
} from "../ipc/analytics";

beforeEach(() => resetFakeBackend());

function seedAuthenticatedUser() {
  seedUser({ username: "analyst", password: "pw", role: "property_manager", tenant_ids: ["t1"], active: true });
  return login("analyst", "pw");
}

// ── Event tracking ───────────────────────────────────────────────────

describe("analytics event tracking", () => {
  it("tracks events and returns event records", async () => {
    await seedAuthenticatedUser();

    const evt = await trackEvent({ category: "click", kind: "open_workspace" });
    expect(evt.id).toBeTruthy();
    expect(evt.category).toBe("click");
    expect(evt.kind).toBe("open_workspace");
    expect(evt.occurred_at_unix).toBeGreaterThan(0);
  });

  it("convenience track builders emit correct categories", async () => {
    await seedAuthenticatedUser();

    const imp = await track.impression("page_view");
    expect(imp.category).toBe("impression");

    const clk = await track.click("btn_save");
    expect(clk.category).toBe("click");

    const comp = await track.completion("upload", true, 500);
    expect(comp.category).toBe("completion");

    const conv = await track.conversion("signup");
    expect(conv.category).toBe("conversion");

    // Verify all four calls hit the backend
    const trackCalls = getCallLog().filter((c) => c.cmd === "cmd_analytics_track");
    expect(trackCalls).toHaveLength(4);
  });

  it("rejects unauthenticated tracking", async () => {
    const err = await trackEvent({ category: "click", kind: "x" }).catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });
});

// ── Dashboards ───────────────────────────────────────────────────────

describe("analytics dashboards", () => {
  it("loadFunnel returns step-by-step funnel results", async () => {
    await seedAuthenticatedUser();

    const funnel = await loadFunnel(
      "t1",
      { name: "onboarding", steps: [
        { event_kind: "signup", label: "Sign up" },
        { event_kind: "activate", label: "Activate" },
        { event_kind: "convert", label: "Convert" },
      ]},
      1700000000, 1700100000,
    );
    expect(funnel.funnel_name).toBe("onboarding");
    expect(funnel.steps).toHaveLength(3);
    expect(funnel.steps[0].label).toBe("Sign up");
    expect(funnel.steps[0].user_count).toBeGreaterThan(0);
    expect(funnel.overall_conversion_rate).toBeGreaterThan(0);
    expect(funnel.overall_conversion_rate).toBeLessThanOrEqual(1);
  });

  it("loadRetention returns cohort data", async () => {
    await seedAuthenticatedUser();

    const ret = await loadRetention("t1", 86400, 3, 1700000000, 1700300000);
    expect(ret.cohort_window_seconds).toBe(86400);
    expect(ret.follow_up_windows).toBe(3);
    expect(ret.cohorts).toHaveLength(1);
    expect(ret.cohorts[0].cohort_size).toBe(100);
    expect(ret.cohorts[0].retained).toHaveLength(3);
  });

  it("loadQuality returns performance metrics", async () => {
    await seedAuthenticatedUser();

    const q = await loadQuality("t1", "upload", 1700000000, 1700100000);
    expect(q.success_rate).toBeGreaterThan(0);
    expect(q.mean_duration_ms).toBeGreaterThan(0);
    expect(q.p50_duration_ms).toBeGreaterThan(0);
    expect(q.p95_duration_ms).toBeGreaterThanOrEqual(q.p50_duration_ms);
  });
});

// ── Exports ──────────────────────────────────────────────────────────

describe("analytics exports", () => {
  it("exports CSV with headers and rows", async () => {
    await seedAuthenticatedUser();

    const csv = await exportRows("csv", [
      { name: "Alice", score: 95 },
      { name: "Bob", score: 87 },
    ]);
    expect(csv).toContain("name,score");
    expect(csv).toContain("Alice,95");
    expect(csv).toContain("Bob,87");
  });

  it("exports JSONL with one JSON object per line", async () => {
    await seedAuthenticatedUser();

    const jsonl = await exportRows("jsonl", [
      { event: "click", count: 10 },
    ]);
    const parsed = JSON.parse(jsonl.trim());
    expect(parsed.event).toBe("click");
    expect(parsed.count).toBe(10);
  });

  it("exports empty string for empty rows", async () => {
    await seedAuthenticatedUser();
    const result = await exportRows("csv", []);
    expect(result).toBe("");
  });
});

// ── A/B experiments ──────────────────────────────────────────────────

describe("analytics experiments", () => {
  it("assigns a variant that is sticky per subject_id", async () => {
    await seedAuthenticatedUser();
    seedExperiment({
      experiment_id: "exp-1",
      variants: [
        { variant_id: "v-a", variant_name: "Control" },
        { variant_id: "v-b", variant_name: "Treatment" },
      ],
    });

    const first = await assignVariant("exp-1", "user-42");
    expect(first.experiment_id).toBe("exp-1");
    expect(first.subject_id).toBe("user-42");
    expect(first.sticky).toBe(true);
    expect(["v-a", "v-b"]).toContain(first.variant_id);

    // Second assignment for the same subject must return the same variant
    const second = await assignVariant("exp-1", "user-42");
    expect(second.variant_id).toBe(first.variant_id);
    expect(second.variant_name).toBe(first.variant_name);
  });

  it("rejects assignment for nonexistent experiment", async () => {
    await seedAuthenticatedUser();
    const err = await assignVariant("exp-nope", "user-1").catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).message).toMatch(/not found/);
  });
});

// ── downloadAs (browser helper) ──────────────────────────────────────

describe("downloadAs helper", () => {
  it("creates a blob download link and cleans up", () => {
    const clicks: string[] = [];
    const revokeUrl = vi.fn();
    globalThis.URL.createObjectURL = vi.fn().mockReturnValue("blob:export");
    globalThis.URL.revokeObjectURL = revokeUrl;

    const origCreate = document.createElement.bind(document);
    vi.spyOn(document, "createElement").mockImplementation((tag: string) => {
      const el = origCreate(tag);
      if (tag === "a") {
        el.click = () => clicks.push("clicked");
        el.remove = vi.fn();
      }
      return el;
    });
    vi.spyOn(document.body, "appendChild").mockImplementation(vi.fn() as never);

    downloadAs("data.csv", "a,b\n1,2\n", "text/csv");

    expect(clicks).toHaveLength(1);
    expect(revokeUrl).toHaveBeenCalledWith("blob:export");

    vi.restoreAllMocks();
  });
});
