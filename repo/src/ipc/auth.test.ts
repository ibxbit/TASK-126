// Tests for the auth IPC wrapper. We mock `@tauri-apps/api/core` at
// the module boundary so the test exercises the wrapper's *contract*
// (correct command name, correct args packaging) without booting Tauri.

import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { login, logout, currentUser, type LoginResponse } from "./auth";

const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
  mockInvoke.mockReset();
});

describe("login()", () => {
  it("invokes cmd_login with username and password", async () => {
    const fake: LoginResponse = {
      user_id: "u1",
      username: "alice",
      role: "property_manager",
      tenant_ids: ["t1"],
    };
    mockInvoke.mockResolvedValueOnce(fake);
    const out = await login("alice", "secret");
    expect(out).toEqual(fake);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("cmd_login", {
      username: "alice",
      password: "secret",
    });
  });

  it("propagates rejection from the backend (invalid credentials)", async () => {
    mockInvoke.mockRejectedValueOnce({ type: "internal", message: "invalid credentials" });
    await expect(login("alice", "wrong")).rejects.toMatchObject({
      type: "internal",
    });
  });

  it("does not swallow string errors", async () => {
    mockInvoke.mockRejectedValueOnce("Login failed");
    await expect(login("a", "b")).rejects.toBe("Login failed");
  });

  it("returns the verbatim shape — no client-side normalization", async () => {
    const fake: LoginResponse = {
      user_id: "u2",
      username: "root",
      role: "administrator",
      tenant_ids: ["*"], // global scope marker
    };
    mockInvoke.mockResolvedValueOnce(fake);
    const out = await login("root", "pw");
    expect(out.tenant_ids).toEqual(["*"]);
  });
});

describe("logout()", () => {
  it("invokes cmd_logout with no args", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await logout();
    expect(mockInvoke).toHaveBeenCalledWith("cmd_logout");
  });

  it("resolves to undefined", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await expect(logout()).resolves.toBeUndefined();
  });

  it("propagates errors from the backend", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("session lock poisoned"));
    await expect(logout()).rejects.toThrow("session lock poisoned");
  });
});

describe("currentUser()", () => {
  it("invokes cmd_current_user", async () => {
    mockInvoke.mockResolvedValueOnce(null);
    const out = await currentUser();
    expect(out).toBeNull();
    expect(mockInvoke).toHaveBeenCalledWith("cmd_current_user");
  });

  it("returns a populated principal when the session is set", async () => {
    const fake: LoginResponse = {
      user_id: "u9",
      username: "carla",
      role: "reviewer",
      tenant_ids: ["t1", "t2"],
    };
    mockInvoke.mockResolvedValueOnce(fake);
    const out = await currentUser();
    expect(out).toEqual(fake);
    expect(out?.tenant_ids).toHaveLength(2);
  });

  it("preserves the null-sentinel when there is no session", async () => {
    mockInvoke.mockResolvedValueOnce(null);
    const out = await currentUser();
    expect(out).toBeNull();
  });
});
