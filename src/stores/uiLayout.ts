// Pure layout-state logic for the zoned shell: defaults, bounds and the
// sanitizer that turns whatever localStorage holds back into a valid layout.
// Kept DOM-free so it is unit-tested (repo convention: test pure logic, not
// component mounts).

export type BottomTab = "chain" | "mixer";

export interface UiLayout {
  /** Left browser zone. */
  browser: { open: boolean; width: number };
  /** Bottom zone (chain/mixer tabs). */
  bottom: { open: boolean; height: number; tab: BottomTab };
  /** Height of the timeline pane (top half of the central split). */
  timelineHeight: number;
  /** Dev overlay (Ctrl+Alt+D). Never persisted as open. */
  devOpen: boolean;
}

/** Versioned storage key: bump when the shape changes incompatibly. */
export const LAYOUT_KEY = "telenn.ui.v1";

export const BOUNDS = {
  browserWidth: { min: 180, max: 360, default: 220 },
  bottomHeight: { min: 160, max: 560, default: 260 },
  timelineHeight: { min: 64, max: 480, default: 140 },
} as const;

export function defaultLayout(): UiLayout {
  return {
    browser: { open: true, width: BOUNDS.browserWidth.default },
    bottom: { open: true, height: BOUNDS.bottomHeight.default, tab: "chain" },
    timelineHeight: BOUNDS.timelineHeight.default,
    devOpen: false,
  };
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

const num = (v: unknown, bounds: { min: number; max: number; default: number }): number =>
  typeof v === "number" && Number.isFinite(v) ? clamp(v, bounds.min, bounds.max) : bounds.default;

const bool = (v: unknown, fallback: boolean): boolean => (typeof v === "boolean" ? v : fallback);

/**
 * Turn a parsed (untrusted) value into a valid layout: every field falls back
 * to its default independently, sizes are clamped to today's bounds — so a
 * layout saved on a bigger screen or by an older version always loads sane.
 */
export function sanitizeLayout(raw: unknown): UiLayout {
  const d = defaultLayout();
  if (typeof raw !== "object" || raw === null) return d;
  const r = raw as Record<string, unknown>;
  const browser = (r.browser ?? {}) as Record<string, unknown>;
  const bottom = (r.bottom ?? {}) as Record<string, unknown>;
  return {
    browser: {
      open: bool(browser.open, d.browser.open),
      width: num(browser.width, BOUNDS.browserWidth),
    },
    bottom: {
      open: bool(bottom.open, d.bottom.open),
      height: num(bottom.height, BOUNDS.bottomHeight),
      tab: bottom.tab === "mixer" ? "mixer" : "chain",
    },
    timelineHeight: num(r.timelineHeight, BOUNDS.timelineHeight),
    // The dev overlay is session state: it never reopens from a saved layout.
    devOpen: false,
  };
}

/** Parse a stored JSON string into a valid layout (bad JSON → defaults). */
export function loadLayout(stored: string | null): UiLayout {
  if (!stored) return defaultLayout();
  try {
    return sanitizeLayout(JSON.parse(stored));
  } catch {
    return defaultLayout();
  }
}
