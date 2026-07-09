//! Pointer-based drag & drop for the effect chain (M6).
//!
//! Native HTML5 drag & drop is unusable here (ghost image, inconsistent events),
//! so we drive it with pointer events and a small shared state. The *decision* —
//! what a drop means — is the pure `resolveDrop`, kept free of the DOM so it is
//! unit-tested; the Vue components own the pointer plumbing and the ghost.

import { reactive } from "vue";

/** What is being dragged: an existing effect, or a browser item (module/preset). */
export type DragSource =
  | { kind: "node"; nodeId: string; module: string; fromTrack: string; fromIndex: number }
  | { kind: "module"; module: string; label: string; category: "instrument" | "effect" }
  | { kind: "preset"; name: string; module: string; params: Record<string, number> };

/** Where a drop can land: a gap in a chain's effects region, a track's
 *  instrument slot (chain head), or the new-track zone below the rows. */
export type DropTarget =
  | { kind: "gap"; trackId: string; index: number }
  | { kind: "instrument"; trackId: string }
  | { kind: "newTrack" };

/** The edit a drop resolves to (executed by the view through the store). */
export type DropIntent =
  | { kind: "move"; nodeId: string; toTrack: string; index: number }
  | { kind: "copy"; nodeId: string; toTrack: string; index: number }
  | { kind: "insert"; trackId: string; index: number; module: string; params: Record<string, number> }
  | { kind: "setInstrument"; trackId: string; module: string }
  | { kind: "addTrack"; module: string; name: string };

/**
 * What dropping `source` on `target` means, or `null` for a no-op. A node dropped
 * in the gap just before or after itself (same track) does not move — returning
 * `null` keeps the graph (and its `GraphDesc`) strictly unchanged. Typed slots:
 * only an instrument module can land on an instrument slot or the new-track
 * zone, and it can never land in the effects region.
 */
export function resolveDrop(
  source: DragSource,
  target: DropTarget,
  copy: boolean,
): DropIntent | null {
  const instrument = source.kind === "module" && source.category === "instrument";
  if (target.kind === "instrument") {
    return instrument && source.kind === "module"
      ? { kind: "setInstrument", trackId: target.trackId, module: source.module }
      : null;
  }
  if (target.kind === "newTrack") {
    return instrument && source.kind === "module"
      ? { kind: "addTrack", module: source.module, name: source.label }
      : null;
  }
  switch (source.kind) {
    case "module":
      if (instrument) return null; // an instrument is a chain head, never an effect
      return { kind: "insert", trackId: target.trackId, index: target.index, module: source.module, params: {} };
    case "preset":
      return {
        kind: "insert",
        trackId: target.trackId,
        index: target.index,
        module: source.module,
        params: source.params,
      };
    case "node": {
      const sameTrack = target.trackId === source.fromTrack;
      const samePlace = sameTrack && (target.index === source.fromIndex || target.index === source.fromIndex + 1);
      if (!copy && samePlace) return null;
      return { kind: copy ? "copy" : "move", nodeId: source.nodeId, toTrack: target.trackId, index: target.index };
    }
  }
}

interface DragState {
  source: DragSource | null;
  /** A press is held on a draggable but no drag has started yet (pre-threshold). */
  pressing: boolean;
  hovered: DropTarget | null;
  copy: boolean;
  /** Pointer position, for the floating ghost. */
  x: number;
  y: number;
}

/** Pixels the pointer must travel after a press before it becomes a real drag. A
 *  shorter move is a plain click — for a chain node, just a selection. */
const DRAG_THRESHOLD = 4;

/** Milliseconds a press must be held still before it shows the grab cursor. A
 *  quick click releases before this, so it never flashes the grab affordance. */
const HOLD_DELAY = 160;

// One shared drag across the chain panel and the browser (singleton).
const state = reactive<DragState>({
  source: null,
  pressing: false,
  hovered: null,
  copy: false,
  x: 0,
  y: 0,
});

// The origin of an armed press, kept to measure the threshold. The fact that a
// press is armed lives in the reactive `state.pressing` (it drives the cursor).
let pressOrigin: { source: DragSource; x: number; y: number } | null = null;
// Timer that flips on the grab cursor after HOLD_DELAY; cancelled by an early
// release (a click), by a real drag starting, or by any drag teardown.
let pressTimer: ReturnType<typeof setTimeout> | null = null;

function clearPressTimer(): void {
  if (pressTimer !== null) {
    clearTimeout(pressTimer);
    pressTimer = null;
  }
}

export function useChainDrag() {
  return {
    state,
    /** Arm a press: a drag candidate that starts only past the move threshold. */
    arm(source: DragSource, x: number, y: number): void {
      pressOrigin = { source, x, y };
      state.hovered = null;
      state.x = x;
      state.y = y;
      // Show the grab cursor only after a brief hold, so a quick click stays an
      // arrow (it releases before the timer fires).
      clearPressTimer();
      pressTimer = setTimeout(() => {
        if (pressOrigin) state.pressing = true;
        pressTimer = null;
      }, HOLD_DELAY);
    },
    move(x: number, y: number): void {
      // Promote an armed press to a real drag once it travels past the threshold.
      if (pressOrigin && !state.source) {
        if (Math.hypot(x - pressOrigin.x, y - pressOrigin.y) < DRAG_THRESHOLD) return;
        clearPressTimer();
        state.source = pressOrigin.source;
        state.pressing = false; // the real drag's `chain-dragging` carries the cursor now
        pressOrigin = null;
      }
      state.x = x;
      state.y = y;
    },
    hover(target: DropTarget | null): void {
      state.hovered = target;
    },
    setCopy(copy: boolean): void {
      state.copy = copy;
    },
    /** A release that never became a drag: drop the armed press (it was a click). */
    clearPress(): void {
      clearPressTimer();
      pressOrigin = null;
      state.pressing = false;
    },
    cancel(): void {
      clearPressTimer();
      pressOrigin = null;
      state.pressing = false;
      state.source = null;
      state.hovered = null;
    },
    /** Resolve the drop on the hovered target and clear the drag. */
    drop(): DropIntent | null {
      const { source, hovered, copy } = state;
      clearPressTimer();
      pressOrigin = null;
      state.pressing = false;
      state.source = null;
      state.hovered = null;
      if (!source || !hovered) return null;
      return resolveDrop(source, hovered, copy);
    },
  };
}
