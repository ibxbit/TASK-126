// Cross-boundary journey test for system, recovery, and update domains.
// Exercises real IPC wrappers against the fake backend.

import { describe, expect, it, vi, beforeEach } from "vitest";
import {
  resetFakeBackend,
  seedUser,
  seedRecoveryOutcome,
  seedVersion,
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
  lastRecoveryOutcome,
  openHandles,
  verifyUpdatePackage,
  installUpdate,
  rollbackPreviousVersion,
  listInstalledVersions,
} from "../ipc/system";

beforeEach(() => resetFakeBackend());

function seedAuthenticatedUser() {
  seedUser({ username: "admin", password: "pw", role: "administrator", tenant_ids: ["t1"], active: true });
  return login("admin", "pw");
}

// ── Recovery ─────────────────────────────────────────────────────────

describe("recovery outcome", () => {
  it("returns null when no recovery has been recorded", async () => {
    const result = await lastRecoveryOutcome();
    expect(result).toBeNull();
  });

  it("returns the seeded recovery outcome", async () => {
    seedRecoveryOutcome("unclean_repaired");
    const result = await lastRecoveryOutcome();
    expect(result).toBe("unclean_repaired");
  });

  it("reflects clean_start outcome", async () => {
    seedRecoveryOutcome("clean_start");
    expect(await lastRecoveryOutcome()).toBe("clean_start");
  });
});

// ── Handles ──────────────────────────────────────────────────────────

describe("open handles", () => {
  it("returns empty list when no handles are open", async () => {
    await seedAuthenticatedUser();
    const handles = await openHandles();
    expect(handles).toEqual([]);
  });

  it("rejects unauthenticated caller", async () => {
    const err = await openHandles().catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });
});

// ── Update verify + install ──────────────────────────────────────────

describe("update verification and installation", () => {
  it("verifies a valid .spkg package", async () => {
    await seedAuthenticatedUser();

    const info = await verifyUpdatePackage("/tmp/update-1.1.0.spkg");
    expect(info.package_id).toBeTruthy();
    expect(info.version).toBe("1.1.0");
    expect(info.created_at_unix).toBeGreaterThan(0);
  });

  it("rejects non-.spkg packages", async () => {
    await seedAuthenticatedUser();

    const err = await verifyUpdatePackage("/tmp/update.zip").catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).message).toMatch(/unsupported format/);
  });

  it("installs an update and records the version", async () => {
    await seedAuthenticatedUser();
    seedVersion({ version: "1.0.0", package_id: null, installed_at_unix: 1700000000, is_active: true, snapshot_path: null });

    const result = await installUpdate("/tmp/update.spkg");
    expect(result.previous_version).toBe("1.0.0");
    expect(result.new_version).toBe("1.1.0");
    expect(result.restart_required).toBe(true);
    expect(result.snapshot_path).toBeTruthy();

    // Verify the version list
    const versions = await listInstalledVersions();
    expect(versions).toHaveLength(2);
    expect(versions.find((v) => v.is_active)?.version).toBe("1.1.0");
  });

  it("install with no prior version sets previous_version to null", async () => {
    await seedAuthenticatedUser();

    const result = await installUpdate("/tmp/fresh.spkg");
    expect(result.previous_version).toBeNull();
    expect(result.new_version).toBe("1.1.0");
  });
});

// ── Rollback ─────────────────────────────────────────────────────────

describe("update rollback", () => {
  it("rolls back to the previous version", async () => {
    await seedAuthenticatedUser();
    seedVersion({ version: "1.0.0", package_id: null, installed_at_unix: 1700000000, is_active: false, snapshot_path: "/snap/1.0" });
    seedVersion({ version: "1.1.0", package_id: "upd-1", installed_at_unix: 1700001000, is_active: true, snapshot_path: "/snap/1.1" });

    const result = await rollbackPreviousVersion();
    expect(result.from_version).toBe("1.1.0");
    expect(result.to_version).toBe("1.0.0");
    expect(result.restart_required).toBe(true);

    // Verify version list
    const versions = await listInstalledVersions();
    expect(versions.find((v) => v.is_active)?.version).toBe("1.0.0");
  });

  it("rollback fails when there is no previous version", async () => {
    await seedAuthenticatedUser();
    seedVersion({ version: "1.0.0", package_id: null, installed_at_unix: 1700000000, is_active: true, snapshot_path: null });

    const err = await rollbackPreviousVersion().catch((e) => e);
    expect(err).toBeInstanceOf(IpcError);
    expect((err as IpcError).message).toMatch(/no previous/);
  });

  it("rejects unauthenticated rollback", async () => {
    const err = await rollbackPreviousVersion().catch((e) => e);
    expect((err as IpcError).type).toBe("unauthenticated");
  });
});

// ── Installed versions ───────────────────────────────────────────────

describe("installed versions", () => {
  it("lists seeded versions", async () => {
    await seedAuthenticatedUser();
    seedVersion({ version: "0.9.0", package_id: null, installed_at_unix: 1699000000, is_active: false, snapshot_path: null });
    seedVersion({ version: "1.0.0", package_id: null, installed_at_unix: 1700000000, is_active: true, snapshot_path: "/snap/1.0" });

    const versions = await listInstalledVersions();
    expect(versions).toHaveLength(2);
    expect(versions.map((v) => v.version)).toEqual(["0.9.0", "1.0.0"]);
  });

  it("returns empty when no versions installed", async () => {
    await seedAuthenticatedUser();
    const versions = await listInstalledVersions();
    expect(versions).toEqual([]);
  });

  it("verifyUpdatePackage command name is exact", async () => {
    await seedAuthenticatedUser();
    await verifyUpdatePackage("/tmp/x.spkg");
    const cmds = getCallLog().map((c) => c.cmd);
    expect(cmds).toContain("cmd_update_verify");
  });
});
