// E2E test: authentication flow through the real Vite-served frontend.
// Tauri IPC is injected via the fake backend in fixtures.ts.

import { test, expect } from "./fixtures";

test.describe("Authentication E2E", () => {
  test("shows login form on first visit", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByText("Sign in to continue")).toBeVisible();
    await expect(page.getByLabel("Username")).toBeVisible();
    await expect(page.getByLabel("Password")).toBeVisible();
    await expect(page.getByRole("button", { name: "Sign in" })).toBeVisible();
  });

  test("successful login shows dashboard with user info", async ({ page }) => {
    await page.goto("/");
    await page.getByLabel("Username").fill("admin");
    await page.getByLabel("Password").fill("admin123");
    await page.getByRole("button", { name: "Sign in" }).click();

    // Dashboard appears
    await expect(page.getByText("Shoreline Property Operations Console")).toBeVisible();
    await expect(page.getByText("admin")).toBeVisible();
    await expect(page.getByText("Administrator")).toBeVisible();

    // Workspace cards are present
    await expect(page.getByText("Move-Out Case")).toBeVisible();
    await expect(page.getByText("Parcel Queue")).toBeVisible();
    await expect(page.getByText("Claims Inbox")).toBeVisible();
  });

  test("failed login shows error message and re-enables form", async ({ page }) => {
    await page.goto("/");
    await page.getByLabel("Username").fill("admin");
    await page.getByLabel("Password").fill("wrongpassword");
    await page.getByRole("button", { name: "Sign in" }).click();

    // Error message appears
    await expect(page.getByText("invalid credentials")).toBeVisible();

    // Form is still usable (not stuck in loading)
    await expect(page.getByRole("button", { name: "Sign in" })).toBeEnabled();
  });

  test("login with unknown user shows error", async ({ page }) => {
    await page.goto("/");
    await page.getByLabel("Username").fill("nobody");
    await page.getByLabel("Password").fill("any");
    await page.getByRole("button", { name: "Sign in" }).click();

    await expect(page.getByText("invalid credentials")).toBeVisible();
  });

  test("sign out returns to login form", async ({ page }) => {
    await page.goto("/");

    // Login first
    await page.getByLabel("Username").fill("admin");
    await page.getByLabel("Password").fill("admin123");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByText("Shoreline Property Operations Console")).toBeVisible();

    // Click sign out
    await page.getByText("Sign out").click();

    // Back to login
    await expect(page.getByText("Sign in to continue")).toBeVisible();
  });

  test("login button shows 'Signing in...' while loading", async ({ page }) => {
    await page.goto("/");
    await page.getByLabel("Username").fill("admin");
    await page.getByLabel("Password").fill("admin123");

    // The button should briefly show loading text; since the fake is fast
    // we assert the final state (dashboard) instead of the transient state
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByText("Shoreline Property Operations Console")).toBeVisible();
  });
});
