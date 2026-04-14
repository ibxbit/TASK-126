import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import LoginForm from "./LoginForm";

// Mock the auth IPC — we cannot call Tauri in jsdom.
vi.mock("../ipc/auth", () => ({
  login: vi.fn(),
}));

import { login } from "../ipc/auth";
const mockLogin = vi.mocked(login);

describe("LoginForm", () => {
  const onLogin = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders username and password fields", () => {
    render(<LoginForm onLogin={onLogin} />);
    expect(screen.getByLabelText(/username/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/password/i)).toBeInTheDocument();
  });

  it("renders a sign-in button", () => {
    render(<LoginForm onLogin={onLogin} />);
    expect(screen.getByRole("button", { name: /sign in/i })).toBeInTheDocument();
  });

  it("requires both fields (HTML required attribute)", () => {
    render(<LoginForm onLogin={onLogin} />);
    const username = screen.getByLabelText(/username/i) as HTMLInputElement;
    const password = screen.getByLabelText(/password/i) as HTMLInputElement;
    expect(username).toBeRequired();
    expect(password).toBeRequired();
  });

  it("password field has type=password", () => {
    render(<LoginForm onLogin={onLogin} />);
    const password = screen.getByLabelText(/password/i) as HTMLInputElement;
    expect(password.type).toBe("password");
  });

  it("calls login IPC and onLogin on successful submit", async () => {
    const fakeUser = {
      user_id: "u1",
      username: "admin",
      role: "Administrator",
      tenant_ids: ["t1"],
    };
    mockLogin.mockResolvedValueOnce(fakeUser);

    render(<LoginForm onLogin={onLogin} />);
    fireEvent.change(screen.getByLabelText(/username/i), {
      target: { value: "admin" },
    });
    fireEvent.change(screen.getByLabelText(/password/i), {
      target: { value: "secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() => {
      expect(mockLogin).toHaveBeenCalledWith("admin", "secret");
      expect(onLogin).toHaveBeenCalledWith(fakeUser);
    });
  });

  it("shows error message on login failure (string error)", async () => {
    mockLogin.mockRejectedValueOnce("Invalid credentials");

    render(<LoginForm onLogin={onLogin} />);
    fireEvent.change(screen.getByLabelText(/username/i), {
      target: { value: "bad" },
    });
    fireEvent.change(screen.getByLabelText(/password/i), {
      target: { value: "wrong" },
    });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() => {
      expect(screen.getByText("Invalid credentials")).toBeInTheDocument();
    });
    expect(onLogin).not.toHaveBeenCalled();
  });

  it("shows error message on login failure (object with message)", async () => {
    mockLogin.mockRejectedValueOnce({ message: "Account locked" });

    render(<LoginForm onLogin={onLogin} />);
    fireEvent.change(screen.getByLabelText(/username/i), {
      target: { value: "locked" },
    });
    fireEvent.change(screen.getByLabelText(/password/i), {
      target: { value: "any" },
    });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() => {
      expect(screen.getByText("Account locked")).toBeInTheDocument();
    });
  });

  it("disables submit button while loading", async () => {
    // Never-resolving promise to keep loading state
    mockLogin.mockReturnValueOnce(new Promise(() => {}));

    render(<LoginForm onLogin={onLogin} />);
    fireEvent.change(screen.getByLabelText(/username/i), {
      target: { value: "user" },
    });
    fireEvent.change(screen.getByLabelText(/password/i), {
      target: { value: "pass" },
    });
    fireEvent.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() => {
      const btn = screen.getByRole("button");
      expect(btn).toBeDisabled();
      expect(btn).toHaveTextContent(/signing in/i);
    });
  });

  it("autofocuses the username field", () => {
    render(<LoginForm onLogin={onLogin} />);
    const username = screen.getByLabelText(/username/i);
    // React's autoFocus prop sets the DOM property, not the HTML attribute.
    // In jsdom, this results in the element receiving focus.
    expect(document.activeElement).toBe(username);
  });
});
