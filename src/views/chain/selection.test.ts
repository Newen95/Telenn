import { describe, expect, it } from "vitest";

import { ChainSelection } from "./selection";

const NONE = { ctrl: false, meta: false };
const CTRL = { ctrl: true, meta: false };

describe("ChainSelection", () => {
  it("a click selects the node", () => {
    const sel = new ChainSelection();
    expect(sel.selected).toBeNull();
    sel.select("a");
    expect(sel.selected).toBe("a");
  });

  it("Delete targets the selected node when the chain is focused", () => {
    const sel = new ChainSelection();
    sel.select("b");
    expect(sel.action("Delete", NONE, true)).toBe("delete");
    expect(sel.action("Backspace", NONE, true)).toBe("delete");
    // The caller acts on `sel.selected`, so deletion hits the right node.
    expect(sel.selected).toBe("b");
  });

  it("Ctrl/Cmd+D duplicates the selected node", () => {
    const sel = new ChainSelection();
    sel.select("b");
    expect(sel.action("d", CTRL, true)).toBe("duplicate");
    expect(sel.action("d", { ctrl: false, meta: true }, true)).toBe("duplicate");
    // Plain "d" without a modifier is not a chain shortcut.
    expect(sel.action("d", NONE, true)).toBeNull();
  });

  it("is inert when the focus is elsewhere", () => {
    const sel = new ChainSelection();
    sel.select("b");
    expect(sel.action("Delete", NONE, false)).toBeNull();
    expect(sel.action("d", CTRL, false)).toBeNull();
  });

  it("is inert when nothing is selected", () => {
    const sel = new ChainSelection();
    expect(sel.action("Delete", NONE, true)).toBeNull();
    sel.select("b");
    sel.clear();
    expect(sel.action("Delete", NONE, true)).toBeNull();
  });
});
