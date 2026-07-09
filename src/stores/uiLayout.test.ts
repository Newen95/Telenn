import { describe, expect, it } from "vitest";

import { BOUNDS, defaultLayout, loadLayout, sanitizeLayout } from "./uiLayout";

// The persisted layout is untrusted input (older versions, other screens,
// hand edits): every field must fall back independently and sizes must land
// inside today's bounds.

describe("sanitizeLayout", () => {
  it("returns defaults for garbage", () => {
    expect(sanitizeLayout(null)).toEqual(defaultLayout());
    expect(sanitizeLayout("nope")).toEqual(defaultLayout());
    expect(sanitizeLayout(42)).toEqual(defaultLayout());
  });

  it("keeps valid fields and defaults missing ones", () => {
    const layout = sanitizeLayout({ browser: { open: false, width: 300 } });
    expect(layout.browser).toEqual({ open: false, width: 300 });
    expect(layout.bottom).toEqual(defaultLayout().bottom);
    expect(layout.timelineHeight).toBe(BOUNDS.timelineHeight.default);
  });

  it("clamps sizes saved on another screen into today's bounds", () => {
    const layout = sanitizeLayout({
      browser: { open: true, width: 9000 },
      bottom: { open: true, height: -50, tab: "mixer" },
      timelineHeight: 3,
    });
    expect(layout.browser.width).toBe(BOUNDS.browserWidth.max);
    expect(layout.bottom.height).toBe(BOUNDS.bottomHeight.min);
    expect(layout.timelineHeight).toBe(BOUNDS.timelineHeight.min);
  });

  it("rejects unknown tabs and non-finite sizes", () => {
    const layout = sanitizeLayout({
      bottom: { open: true, height: Number.NaN, tab: "banana" },
    });
    expect(layout.bottom.tab).toBe("chain");
    expect(layout.bottom.height).toBe(BOUNDS.bottomHeight.default);
  });

  it("never restores the dev overlay as open", () => {
    expect(sanitizeLayout({ devOpen: true }).devOpen).toBe(false);
  });
});

describe("loadLayout", () => {
  it("parses a stored layout and survives bad JSON", () => {
    const stored = JSON.stringify({ timelineHeight: 200 });
    expect(loadLayout(stored).timelineHeight).toBe(200);
    expect(loadLayout("{oops")).toEqual(defaultLayout());
    expect(loadLayout(null)).toEqual(defaultLayout());
  });
});
