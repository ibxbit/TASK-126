import { describe, expect, it } from "vitest";
import { statusTransitionItems, attachmentItems } from "./ContextMenu";

// We test the pure helper functions (statusTransitionItems,
// attachmentItems) which don't require Tauri. The useContextMenu hook
// calls invoke() under the hood so it's covered by integration / E2E.

describe("statusTransitionItems", () => {
  it("returns three items: Approve, Reopen, Close", () => {
    const items = statusTransitionItems({
      canApprove: true,
      canReopen: true,
      canClose: true,
    });
    expect(items).toHaveLength(3);
    expect(items.map((i) => ("id" in i ? i.id : null))).toEqual([
      "status.approve",
      "status.reopen",
      "status.close",
    ]);
  });

  it("disables items when permission flags are false", () => {
    const items = statusTransitionItems({
      canApprove: false,
      canReopen: false,
      canClose: true,
    });
    const approve = items.find(
      (i) => i.kind === "action" && i.id === "status.approve",
    );
    const reopen = items.find(
      (i) => i.kind === "action" && i.id === "status.reopen",
    );
    const close = items.find(
      (i) => i.kind === "action" && i.id === "status.close",
    );
    expect(approve && "enabled" in approve && approve.enabled).toBe(false);
    expect(reopen && "enabled" in reopen && reopen.enabled).toBe(false);
    expect(close && "enabled" in close && close.enabled).toBe(true);
  });
});

describe("attachmentItems", () => {
  it("returns 5 items including a separator", () => {
    const items = attachmentItems();
    expect(items).toHaveLength(5);
  });

  it("contains Open, Reveal, Rename, Remove actions", () => {
    const items = attachmentItems();
    const ids = items
      .filter((i) => i.kind === "action")
      .map((i) => (i as { id: string }).id);
    expect(ids).toContain("attach.open");
    expect(ids).toContain("attach.reveal");
    expect(ids).toContain("attach.rename");
    expect(ids).toContain("attach.remove");
  });

  it("includes a separator between reveal and rename", () => {
    const items = attachmentItems();
    expect(items[2]).toEqual({ kind: "separator" });
  });

  it("Rename has F2 accelerator", () => {
    const items = attachmentItems();
    const rename = items.find(
      (i) => i.kind === "action" && (i as { id: string }).id === "attach.rename",
    );
    expect(rename).toBeDefined();
    expect(rename && "accelerator" in rename && rename.accelerator).toBe("F2");
  });
});
