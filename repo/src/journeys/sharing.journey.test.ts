// Cross-boundary journey test for sharing workflows.
// Covers watermarking, package build, verify access, revoke, and sweep.

import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  resetFakeBackend,
  seedUser,
  seedPackage,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
  IpcError,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import { login } from "../ipc/auth";
import {
  wrapWithWatermark,
  buildSharePackage,
  verifyPackageAccess,
  revokePackage,
  sweepExpiredPackages,
  defaultExpiryUnix,
  downloadPackage,
} from "../ipc/sharing";

beforeEach(() => resetFakeBackend());

function seedAuthenticatedUser() {
  seedUser({ username: "sharer", password: "pw", role: "property_manager", tenant_ids: ["t1"], active: true });
  return login("sharer", "pw");
}

// ── Watermark ────────────────────────────────────────────────────────

describe("watermarking", () => {
  it("wraps bytes into a watermarked HTML document", async () => {
    await seedAuthenticatedUser();

    const html = await wrapWithWatermark(
      new Uint8Array([0xff, 0xd8]),
      "image/jpeg",
      { username: "alice", generated_at_unix: 1700000000 },
    );
    expect(html).toContain("alice");
    expect(html).toContain("image/jpeg");
    expect(html).toContain("<html>");
  });

  it("rejects unauthenticated watermark request", async () => {
    const err = await wrapWithWatermark(
      new Uint8Array([1]), "text/plain",
      { username: "x", generated_at_unix: 0 },
    ).catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });
});

// ── Package build + verify + revoke ──────────────────────────────────

describe("share package lifecycle", () => {
  it("builds a package, verifies access with correct password, then revokes", async () => {
    await seedAuthenticatedUser();

    const outcome = await buildSharePackage({
      tenant_id: "t1",
      items: [{ filename: "doc.pdf", mime_type: "application/pdf", bytes: [1, 2, 3] }],
      password: "secret123",
      expires_at_unix: Math.floor(Date.now() / 1000) + 86400,
      created_at_unix: Math.floor(Date.now() / 1000),
    });
    expect(outcome.package_id).toBeTruthy();
    expect(outcome.zip_bytes.length).toBeGreaterThan(0);
    expect(outcome.sha256_hex).toBeTruthy();
    expect(outcome.contents_summary).toContain("1 items");

    // Verify with correct password
    const ok = await verifyPackageAccess(outcome.package_id, "secret123");
    expect(ok.ok).toBe(true);

    // Verify with wrong password
    const bad = await verifyPackageAccess(outcome.package_id, "wrong");
    expect(bad.ok).toBe(false);
    expect(bad.reason).toBe("bad_password");

    // Revoke
    await revokePackage(outcome.package_id);

    // Verify after revoke — should be denied
    const revoked = await verifyPackageAccess(outcome.package_id, "secret123");
    expect(revoked.ok).toBe(false);
    expect(revoked.reason).toBe("revoked");
  });

  it("verifyPackageAccess returns not_found for unknown package", async () => {
    await seedAuthenticatedUser();
    const result = await verifyPackageAccess("pkg-nonexistent", "pw");
    expect(result.ok).toBe(false);
    expect(result.reason).toBe("not_found");
  });
});

// ── Expiry ───────────────────────────────────────────────────────────

describe("share package expiry", () => {
  it("sweepExpiredPackages removes only expired packages", async () => {
    await seedAuthenticatedUser();
    const now = Math.floor(Date.now() / 1000);

    // Seed one expired and one active package
    seedPackage({ package_id: "pkg-expired", tenant_id: "t1", password: "pw", expires_at_unix: now - 1000 });
    seedPackage({ package_id: "pkg-active", tenant_id: "t1", password: "pw", expires_at_unix: now + 86400 });

    const count = await sweepExpiredPackages();
    expect(count).toBe(1); // only the expired one

    // Active package still accessible
    const result = await verifyPackageAccess("pkg-active", "pw");
    expect(result.ok).toBe(true);
  });
});

// ── Pure helpers ─────────────────────────────────────────────────────

describe("sharing pure helpers", () => {
  it("defaultExpiryUnix adds 7 days", () => {
    const from = 1700000000;
    expect(defaultExpiryUnix(from)).toBe(from + 7 * 24 * 3600);
  });

  it("downloadPackage creates and clicks a download link", () => {
    const clicks: string[] = [];
    const appendChild = vi.fn();
    const removeChild = vi.fn();
    const revokeUrl = vi.fn();

    globalThis.URL.createObjectURL = vi.fn().mockReturnValue("blob:test");
    globalThis.URL.revokeObjectURL = revokeUrl;
    vi.spyOn(document.body, "appendChild").mockImplementation(appendChild);

    // Mock createElement to track click
    const origCreate = document.createElement.bind(document);
    vi.spyOn(document, "createElement").mockImplementation((tag: string) => {
      const el = origCreate(tag);
      if (tag === "a") {
        el.click = () => clicks.push(el.getAttribute("href") ?? "");
        el.remove = removeChild;
      }
      return el;
    });

    downloadPackage("share.zip", [0x50, 0x4b]);

    expect(appendChild).toHaveBeenCalled();
    expect(clicks).toHaveLength(1);
    expect(removeChild).toHaveBeenCalled();
    expect(revokeUrl).toHaveBeenCalledWith("blob:test");

    vi.restoreAllMocks();
  });
});
