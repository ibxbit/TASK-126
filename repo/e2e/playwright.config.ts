import { defineConfig } from "@playwright/test";

/**
 * Playwright config for E2E testing the Shoreline Property Ops frontend.
 *
 * The tests run against the Vite dev server (port 1420). Tauri IPC is
 * not available in a browser, so the tests inject a mock of
 * `window.__TAURI_INTERNALS__` that routes all `invoke()` calls to an
 * in-page fake backend — same contract as the Vitest fake-backend but
 * executed in the browser context.
 */
export default defineConfig({
  testDir: ".",
  testMatch: "**/*.e2e.ts",
  timeout: 30_000,
  retries: 0,
  use: {
    baseURL: "http://localhost:1420",
    headless: true,
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "chromium", use: { browserName: "chromium" } },
  ],
  webServer: {
    command: "npm run dev",
    port: 1420,
    reuseExistingServer: true,
    timeout: 30_000,
  },
  reporter: [["html", { open: "never", outputFolder: "playwright-report" }]],
  outputDir: "playwright-results",
});
