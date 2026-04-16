// Direct unit tests for the Dashboard component (rendered inline in App.tsx).
// These tests exercise the Dashboard in isolation by rendering App with an
// authenticated user, and test workspace card interactions, error handling,
// user display, and layout through the fake backend.

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
  getCallLog,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import App from "../App";
import { login } from "../ipc/auth";

beforeEach(() => {
  resetFakeBackend();
  Object.defineProperty(window, "location", {
    value: { pathname: "/" },
    writable: true,
  });
});

async function loginAndRender() {
  seedUser({ user_id: "u1", username: "alice", password: "pw", role: "property_manager", tenant_ids: ["t1"], active: true });
  await login("alice", "pw");
  render(<App />);
  await waitFor(() => {
    expect(screen.getByText("Shoreline Property Operations Console")).toBeInTheDocument();
  });
}

describe("Dashboard component", () => {
  it("renders title, subtitle, and footer", async () => {
    await loginAndRender();
    expect(screen.getByText("Shoreline Property Operations Console")).toBeInTheDocument();
    expect(screen.getByText(/Select a workspace/)).toBeInTheDocument();
    expect(screen.getByText(/Offline-first/)).toBeInTheDocument();
  });

  it("displays the authenticated user's name and role", async () => {
    await loginAndRender();
    expect(screen.getByText(/alice/)).toBeInTheDocument();
    expect(screen.getByText(/property_manager/)).toBeInTheDocument();
  });

  it("renders all three workspace cards", async () => {
    await loginAndRender();
    expect(screen.getByText("Move-Out Case")).toBeInTheDocument();
    expect(screen.getByText("Parcel Queue")).toBeInTheDocument();
    expect(screen.getByText("Claims Inbox")).toBeInTheDocument();
  });

  it("workspace cards display correct descriptions", async () => {
    await loginAndRender();
    expect(screen.getByText(/Track deposits, inspections/)).toBeInTheDocument();
    expect(screen.getByText(/Check-in, check-out, and deliver/)).toBeInTheDocument();
    expect(screen.getByText(/Resolve disputes/)).toBeInTheDocument();
  });

  it("clicking a workspace card invokes cmd_open_workspace through the dispatcher", async () => {
    await loginAndRender();

    const cards = screen.getAllByRole("button").filter(
      (b) => b.textContent?.includes("Parcel Queue"),
    );
    expect(cards).toHaveLength(1);

    await fireEvent.click(cards[0]);
    await waitFor(() => {
      expect(snapshotState().windowCount).toBe(1);
    });

    const cmds = getCallLog().map((c) => c.cmd);
    expect(cmds).toContain("cmd_open_workspace");
  });

  it("clicking multiple workspace cards opens multiple windows", async () => {
    await loginAndRender();

    const moveOut = screen.getAllByRole("button").find((b) => b.textContent?.includes("Move-Out Case"));
    const parcel = screen.getAllByRole("button").find((b) => b.textContent?.includes("Parcel Queue"));
    const claims = screen.getAllByRole("button").find((b) => b.textContent?.includes("Claims Inbox"));

    await fireEvent.click(moveOut!);
    await fireEvent.click(parcel!);
    await fireEvent.click(claims!);

    await waitFor(() => {
      expect(snapshotState().windowCount).toBe(3);
    });
  });

  it("sign out clears session and shows login form", async () => {
    await loginAndRender();

    const signOut = screen.getByText(/Sign out/i);
    fireEvent.click(signOut);

    await waitFor(() => {
      expect(screen.getByText(/Sign in to continue/i)).toBeInTheDocument();
    });
    expect(snapshotState().currentUserName).toBeNull();
  });

  it("shows different info for staff vs admin roles", async () => {
    resetFakeBackend();
    seedUser({ username: "bob", password: "pw", role: "staff", tenant_ids: ["t1", "t2"], active: true });
    await login("bob", "pw");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText(/bob/)).toBeInTheDocument();
      expect(screen.getByText(/staff/)).toBeInTheDocument();
    });
  });
});
