// Integration test for LoginForm using the fake backend instead of mocks.
// This verifies the form works end-to-end through real IPC wrappers.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import {
  resetFakeBackend,
  seedUser,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
  snapshotState,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import LoginForm from "./LoginForm";

beforeEach(() => resetFakeBackend());

describe("LoginForm integration (fake backend)", () => {
  it("successful login calls onLogin with user data from the backend", async () => {
    seedUser({ username: "alice", password: "shore123", role: "property_manager", tenant_ids: ["t1"], active: true });
    const onLogin = vi.fn();

    render(<LoginForm onLogin={onLogin} />);

    fireEvent.change(screen.getByLabelText(/username/i), { target: { value: "alice" } });
    fireEvent.change(screen.getByLabelText(/password/i), { target: { value: "shore123" } });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() => {
      expect(onLogin).toHaveBeenCalledWith(
        expect.objectContaining({
          username: "alice",
          role: "property_manager",
          tenant_ids: ["t1"],
        }),
      );
    });
    expect(snapshotState().currentUserName).toBe("alice");
  });

  it("wrong password shows the backend's error message", async () => {
    seedUser({ username: "alice", password: "correct", role: "staff", tenant_ids: ["t1"], active: true });
    const onLogin = vi.fn();

    render(<LoginForm onLogin={onLogin} />);

    fireEvent.change(screen.getByLabelText(/username/i), { target: { value: "alice" } });
    fireEvent.change(screen.getByLabelText(/password/i), { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() => {
      expect(screen.getByText("invalid credentials")).toBeInTheDocument();
    });
    expect(onLogin).not.toHaveBeenCalled();
    expect(snapshotState().currentUserName).toBeNull();
  });

  it("disabled account shows account disabled error", async () => {
    seedUser({ username: "frozen", password: "pw", role: "staff", tenant_ids: ["t1"], active: false });
    const onLogin = vi.fn();

    render(<LoginForm onLogin={onLogin} />);

    fireEvent.change(screen.getByLabelText(/username/i), { target: { value: "frozen" } });
    fireEvent.change(screen.getByLabelText(/password/i), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() => {
      expect(screen.getByText("account disabled")).toBeInTheDocument();
    });
    expect(onLogin).not.toHaveBeenCalled();
  });

  it("unknown user shows invalid credentials", async () => {
    const onLogin = vi.fn();
    render(<LoginForm onLogin={onLogin} />);

    fireEvent.change(screen.getByLabelText(/username/i), { target: { value: "nobody" } });
    fireEvent.change(screen.getByLabelText(/password/i), { target: { value: "any" } });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() => {
      expect(screen.getByText("invalid credentials")).toBeInTheDocument();
    });
  });

  it("button re-enables after failed login attempt", async () => {
    seedUser({ username: "u", password: "right", role: "staff", tenant_ids: ["t1"], active: true });
    render(<LoginForm onLogin={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/username/i), { target: { value: "u" } });
    fireEvent.change(screen.getByLabelText(/password/i), { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() => {
      expect(screen.getByText("invalid credentials")).toBeInTheDocument();
    });

    const btn = screen.getByRole("button", { name: /sign in/i });
    expect(btn).not.toBeDisabled();
  });
});
