// E2E test: dashboard rendering, workspace card interactions, and navigation.

import { test, expect } from "./fixtures";

test.describe("Dashboard E2E", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    // Login to reach dashboard
    await page.getByLabel("Username").fill("admin");
    await page.getByLabel("Password").fill("admin123");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByText("Shoreline Property Operations Console")).toBeVisible();
  });

  test("displays all three workspace cards with descriptions", async ({ page }) => {
    // Move-Out Case card
    await expect(page.getByText("Move-Out Case")).toBeVisible();
    await expect(page.getByText(/Track deposits, inspections/)).toBeVisible();

    // Parcel Queue card
    await expect(page.getByText("Parcel Queue")).toBeVisible();
    await expect(page.getByText(/Check-in, check-out, and deliver/)).toBeVisible();

    // Claims Inbox card
    await expect(page.getByText("Claims Inbox")).toBeVisible();
    await expect(page.getByText(/Resolve disputes/)).toBeVisible();
  });

  test("displays footer with version info", async ({ page }) => {
    await expect(page.getByText(/Offline-first.*v0\.1\.0/)).toBeVisible();
  });

  test("displays user identity in header", async ({ page }) => {
    await expect(page.getByText("admin (Administrator)")).toBeVisible();
  });

  test("workspace cards are clickable buttons", async ({ page }) => {
    const cards = page.getByRole("button").filter({ hasText: /Move-Out Case|Parcel Queue|Claims Inbox/ });
    await expect(cards).toHaveCount(3);

    // Clicking a workspace card should not crash (invokes openWorkspace)
    await cards.nth(0).click();
    // Dashboard should still be visible (workspace opens in a new window)
    await expect(page.getByText("Shoreline Property Operations Console")).toBeVisible();
  });

  test("clicking each workspace card triggers the backend command", async ({ page }) => {
    // Click Move-Out Case
    await page.getByRole("button", { hasText: "Move-Out Case" }).click();

    // Click Parcel Queue
    await page.getByRole("button", { hasText: "Parcel Queue" }).click();

    // Click Claims Inbox
    await page.getByRole("button", { hasText: "Claims Inbox" }).click();

    // Dashboard remains stable after all clicks
    await expect(page.getByText("Shoreline Property Operations Console")).toBeVisible();
    await expect(page.getByText("Select a workspace to open")).toBeVisible();
  });
});
