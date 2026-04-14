// Subscribe to OS-level shortcut events dispatched by the Rust
// backend. Components call `useShortcuts({ quick_search: () => ... })`
// and receive callbacks only while mounted.

import { useEffect } from "react";
import { onShortcut, ShortcutAction } from "../ipc/desktop";

export type ShortcutHandlers = Partial<Record<ShortcutAction, () => void>>;

export function useShortcuts(handlers: ShortcutHandlers): void {
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let mounted = true;

    onShortcut((action) => {
      const fn = handlers[action];
      if (fn) fn();
    }).then((u) => {
      if (!mounted) {
        u();
        return;
      }
      unlisten = u;
    });

    return () => {
      mounted = false;
      if (unlisten) unlisten();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    handlers.quick_search,
    handlers.new_case,
    handlers.rename_tag,
  ]);
}
