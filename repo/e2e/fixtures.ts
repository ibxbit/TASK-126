/**
 * Shared Playwright fixtures: injects a fake Tauri IPC layer into the
 * browser page so frontend code that calls `invoke()` works without a
 * real Tauri WebView.
 *
 * The fake mirrors the same contract as `src/test/fake-backend.ts` but
 * runs inside the browser (Playwright `page.addInitScript`).
 */
import { test as base, expect } from "@playwright/test";

export const test = base.extend<{ injectFakeBackend: void }>({
  // eslint-disable-next-line no-empty-pattern
  injectFakeBackend: [async ({ page }, use) => {
    await page.addInitScript(() => {
      // Minimal in-browser fake of Tauri IPC
      const state = {
        currentUser: null as null | { user_id: string; username: string; role: string; tenant_ids: string[] },
        users: [
          { user_id: "u1", username: "admin", password: "admin123", role: "Administrator", tenant_ids: ["t1"], active: true },
          { user_id: "u2", username: "staff", password: "staff123", role: "Staff", tenant_ids: ["t1"], active: true },
        ],
        windows: [] as Array<{ label: string; workspace: string; instance_id: string }>,
      };

      const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
        cmd_login: ({ username, password }) => {
          const user = state.users.find((u) => u.username === username);
          if (!user || user.password !== password) throw { type: "internal", message: "invalid credentials" };
          if (!user.active) throw { type: "internal", message: "account disabled" };
          state.currentUser = { user_id: user.user_id, username: user.username, role: user.role, tenant_ids: user.tenant_ids };
          return { ...state.currentUser };
        },
        cmd_logout: () => { state.currentUser = null; },
        cmd_current_user: () => state.currentUser,
        cmd_open_workspace: ({ workspace }) => {
          if (!state.currentUser) throw { type: "unauthenticated", message: "no session" };
          const id = `inst-${state.windows.length + 1}`;
          const w = { label: `${workspace}:${id}`, workspace: workspace as string, instance_id: id };
          state.windows.push(w);
          return w;
        },
        cmd_list_windows: () => {
          if (!state.currentUser) throw { type: "unauthenticated", message: "no session" };
          return [...state.windows];
        },
        cmd_close_window: ({ label }) => {
          if (!state.currentUser) throw { type: "unauthenticated", message: "no session" };
          const idx = state.windows.findIndex((w) => w.label === label);
          if (idx >= 0) state.windows.splice(idx, 1);
        },
        cmd_schedule_reminder: () => {},
        cmd_cancel_reminder: () => {},
        cmd_pending_reminder_count: () => 0,
        cmd_focus_window: () => {},
        cmd_show_context_menu: () => ({ target: "", chosen_id: null }),
      };

      // Inject Tauri internals mock
      (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
        invoke: (cmd: string, args?: Record<string, unknown>) => {
          const handler = handlers[cmd];
          if (!handler) return Promise.reject({ type: "internal", message: `no handler for ${cmd}` });
          try {
            return Promise.resolve(handler(args ?? {}));
          } catch (e) {
            return Promise.reject(e);
          }
        },
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      };
    });
    await use();
  }, { auto: true }],
});

export { expect };
