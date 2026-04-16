// E2E test: workspace window routes render the correct views.

import { test, expect } from "./fixtures";

test.describe("Workspace Windows E2E", () => {
  test("move-out workspace renders without login gate", async ({ page }) => {
    await page.goto("/workspace/move-out");
    await expect(page.getByText("Move-Out Case")).toBeVisible();
    await expect(page.getByText(/This workspace window is ready/)).toBeVisible();
  });

  test("parcel-queue workspace renders without login gate", async ({ page }) => {
    await page.goto("/workspace/parcel-queue");
    await expect(page.getByText("Parcel Queue")).toBeVisible();
    await expect(page.getByText(/This workspace window is ready/)).toBeVisible();
  });

  test("claims-inbox workspace renders without login gate", async ({ page }) => {
    await page.goto("/workspace/claims-inbox");
    await expect(page.getByText("Claims Inbox")).toBeVisible();
    await expect(page.getByText(/This workspace window is ready/)).toBeVisible();
  });

  test("workspace routes skip the auth check (child windows inherit session)", async ({ page }) => {
    // Navigate directly to a workspace path — should NOT show login form
    await page.goto("/workspace/parcel-queue");
    await expect(page.getByText("Parcel Queue")).toBeVisible();

    // Login form elements should NOT be present
    await expect(page.getByLabel("Username")).not.toBeVisible();
    await expect(page.getByLabel("Password")).not.toBeVisible();
  });

  test("workspace view shows ready message with domain context", async ({ page }) => {
    await page.goto("/workspace/claims-inbox");
    // The subtitle mentions SQLite repositories
    await expect(page.getByText(/Domain views attach here/)).toBeVisible();
  });

  test("each workspace has correct styling (min-height viewport)", async ({ page }) => {
    await page.goto("/workspace/move-out");
    const main = page.locator("main");
    const box = await main.boundingBox();
    expect(box).not.toBeNull();
    // Should fill the viewport
    expect(box!.height).toBeGreaterThan(200);
  });
});
