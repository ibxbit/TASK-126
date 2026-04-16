// Direct unit tests for the WorkspaceView component (rendered inline in App.tsx).
// WorkspaceView renders when the URL matches /workspace/<name>. These tests
// verify each workspace route renders the correct title and ready state,
// and that workspace windows bypass the auth gate.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import {
  resetFakeBackend,
  fakeInvoke,
  fakeGetCurrentWindow,
  fakeListen,
} from "../test/fake-backend";

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: Record<string, unknown>) => fakeInvoke(cmd, args) }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => fakeGetCurrentWindow() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (e: { payload: unknown }) => void) => fakeListen(event, cb),
}));

import App from "../App";

beforeEach(() => {
  resetFakeBackend();
});

function setPathname(path: string) {
  Object.defineProperty(window, "location", {
    value: { pathname: path },
    writable: true,
  });
}

describe("WorkspaceView component", () => {
  it("renders Move-Out Case for /workspace/move-out", async () => {
    setPathname("/workspace/move-out");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText("Move-Out Case")).toBeInTheDocument();
    });
    expect(screen.getByText(/This workspace window is ready/)).toBeInTheDocument();
  });

  it("renders Parcel Queue for /workspace/parcel-queue", async () => {
    setPathname("/workspace/parcel-queue");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText("Parcel Queue")).toBeInTheDocument();
    });
  });

  it("renders Claims Inbox for /workspace/claims-inbox", async () => {
    setPathname("/workspace/claims-inbox");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText("Claims Inbox")).toBeInTheDocument();
    });
  });

  it("workspace views do not show login form", async () => {
    setPathname("/workspace/parcel-queue");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText("Parcel Queue")).toBeInTheDocument();
    });
    expect(screen.queryByText(/Sign in to continue/)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/Username/)).not.toBeInTheDocument();
  });

  it("workspace view contains domain-ready subtitle", async () => {
    setPathname("/workspace/claims-inbox");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText(/Domain views attach here/)).toBeInTheDocument();
    });
  });

  it("workspace view renders even without an authenticated user", async () => {
    // No user seeded, no login — workspace should still render
    setPathname("/workspace/move-out");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText("Move-Out Case")).toBeInTheDocument();
    });
  });

  it("handles subpath under workspace route", async () => {
    setPathname("/workspace/move-out/case-123");
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText("Move-Out Case")).toBeInTheDocument();
    });
  });
});
