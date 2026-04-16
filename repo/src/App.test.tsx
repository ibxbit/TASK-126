import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import "@testing-library/jest-dom";
import App from "./App";

// Mock all Tauri IPC modules used by App.
vi.mock("./ipc/auth", () => ({
  currentUser: vi.fn(),
  login: vi.fn(),
  logout: vi.fn(),
}));

vi.mock("./ipc/desktop", () => ({
  openWorkspace: vi.fn(),
  onShortcut: vi.fn(() => Promise.resolve(() => {})),
}));

import { currentUser } from "./ipc/auth";
import { openWorkspace } from "./ipc/desktop";
const mockCurrentUser = vi.mocked(currentUser);
const mockOpenWorkspace = vi.mocked(openWorkspace);

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Default: simulate being at root path
    Object.defineProperty(window, "location", {
      value: { pathname: "/" },
      writable: true,
    });
  });

  it("shows login form when no user is authenticated", async () => {
    mockCurrentUser.mockResolvedValueOnce(null);

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText(/sign in to continue/i)).toBeInTheDocument();
    });
  });

  it("shows dashboard when user is authenticated", async () => {
    mockCurrentUser.mockResolvedValueOnce({
      user_id: "u1",
      username: "admin",
      role: "Administrator",
      tenant_ids: ["t1"],
    });

    render(<App />);

    await waitFor(() => {
      expect(
        screen.getByText("Shoreline Property Operations Console"),
      ).toBeInTheDocument();
      expect(screen.getByText(/move-out case/i)).toBeInTheDocument();
      expect(screen.getByText(/parcel queue/i)).toBeInTheDocument();
      expect(screen.getByText(/claims inbox/i)).toBeInTheDocument();
    });
  });

  it("renders workspace view for /workspace/move-out path", async () => {
    mockCurrentUser.mockResolvedValueOnce(null);
    Object.defineProperty(window, "location", {
      value: { pathname: "/workspace/move-out" },
      writable: true,
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Move-Out Case")).toBeInTheDocument();
    });
  });

  it("renders workspace view for /workspace/parcel-queue path", async () => {
    mockCurrentUser.mockResolvedValueOnce(null);
    Object.defineProperty(window, "location", {
      value: { pathname: "/workspace/parcel-queue" },
      writable: true,
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Parcel Queue")).toBeInTheDocument();
    });
  });

  it("renders workspace view for /workspace/claims-inbox path", async () => {
    mockCurrentUser.mockResolvedValueOnce(null);
    Object.defineProperty(window, "location", {
      value: { pathname: "/workspace/claims-inbox" },
      writable: true,
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Claims Inbox")).toBeInTheDocument();
    });
  });

  it("displays username and role when authenticated", async () => {
    mockCurrentUser.mockResolvedValueOnce({
      user_id: "u2",
      username: "jdoe",
      role: "Staff",
      tenant_ids: ["t1"],
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText(/jdoe/)).toBeInTheDocument();
      expect(screen.getByText(/Staff/)).toBeInTheDocument();
    });
  });

  it("shows sign out button when authenticated", async () => {
    mockCurrentUser.mockResolvedValueOnce({
      user_id: "u1",
      username: "admin",
      role: "Administrator",
      tenant_ids: ["t1"],
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText(/sign out/i)).toBeInTheDocument();
    });
  });

  it("logs error to console when openWorkspace fails", async () => {
    mockCurrentUser.mockResolvedValueOnce({
      user_id: "u1",
      username: "admin",
      role: "Administrator",
      tenant_ids: ["t1"],
    });
    mockOpenWorkspace.mockRejectedValueOnce(new Error("IPC failed"));

    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    render(<App />);

    // Wait for dashboard to render
    await waitFor(() => {
      expect(screen.getByText(/move-out case/i)).toBeInTheDocument();
    });

    // Click the Move-Out Case card button to trigger the error path.
    // The card is a <button> wrapping the title div.
    const card = screen.getByText(/move-out case/i).closest("button");
    expect(card).toBeTruthy();
    fireEvent.click(card!);

    await waitFor(() => {
      expect(consoleSpy).toHaveBeenCalledWith("openWorkspace failed", expect.any(Error));
    });

    consoleSpy.mockRestore();
  });

  it("handles currentUser rejection gracefully", async () => {
    // Simulate currentUser() throwing — App.tsx catches and sets user=null → login form.
    mockCurrentUser.mockReturnValueOnce(Promise.reject(new Error("network")) as never);

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Sign in to continue")).toBeInTheDocument();
    });
  });
});
