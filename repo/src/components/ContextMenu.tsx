// Right-click helper. Components attach `useContextMenu(...)` to any
// element and receive an `onContextMenu` handler that pops a native
// menu via the backend and invokes the matching action callback.

import { MouseEvent, useCallback } from "react";
import {
  ContextMenuItem,
  ContextMenuSpec,
  showContextMenu,
} from "../ipc/desktop";

export type ActionHandler = (target: string) => void | Promise<void>;

export interface ContextMenuConfig {
  /** Stable identifier for the right-clicked entity (e.g. "case:<uuid>"). */
  target: string;
  items: ContextMenuItem[];
  /** Map of action id → handler. Ids must match `ContextMenuItem.action.id`. */
  handlers: Record<string, ActionHandler>;
}

/**
 * Returns an `onContextMenu` handler that shows a native context menu
 * and dispatches the chosen action. Suppresses the default browser
 * right-click menu.
 */
export function useContextMenu(
  config: ContextMenuConfig,
): (e: MouseEvent) => void {
  return useCallback(
    async (e: MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();

      const spec: ContextMenuSpec = {
        target: config.target,
        items: config.items,
      };
      const result = await showContextMenu(spec);
      if (!result.chosen_id) return;

      const handler = config.handlers[result.chosen_id];
      if (handler) await handler(result.target);
    },
    [config.target, config.items, config.handlers],
  );
}

// ─── Canonical item builders ────────────────────────────────────────────
// Shared catalog so status-transition and attachment menus look
// identical across workspaces.

export const statusTransitionItems = (opts: {
  canReopen: boolean;
  canClose: boolean;
  canApprove: boolean;
}): ContextMenuItem[] => [
  {
    kind: "action",
    id: "status.approve",
    label: "Approve",
    enabled: opts.canApprove,
  },
  {
    kind: "action",
    id: "status.reopen",
    label: "Reopen",
    enabled: opts.canReopen,
  },
  {
    kind: "action",
    id: "status.close",
    label: "Close",
    enabled: opts.canClose,
  },
];

export const attachmentItems = (): ContextMenuItem[] => [
  { kind: "action", id: "attach.open", label: "Open", enabled: true },
  { kind: "action", id: "attach.reveal", label: "Reveal in Folder", enabled: true },
  { kind: "separator" },
  { kind: "action", id: "attach.rename", label: "Rename", enabled: true, accelerator: "F2" },
  { kind: "action", id: "attach.remove", label: "Remove", enabled: true },
];
