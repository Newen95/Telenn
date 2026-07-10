//! Shared editing gesture state machine for the grid views.
//!
//! Timeline and piano roll reuse this exact machine; only the *mapping* differs,
//! supplied through an `EditAdapter` (how a (tick, lane) maps to a note/clip edit,
//! whether lanes are movable, default lengths). The machine works purely in
//! (tick, lane) space using the shared `Viewport` and `hitTest`, holds a local
//! ghost during a drag, and commits only on release — so it is framework-free and
//! unit-testable.

import { hitTest, type Block } from "./renderer";
import { tickToX, xToTick, yToLane, type Viewport } from "./viewport";

/** What a drag is doing. A resize acts on the left or right edge of a block. */
export type DragKind = "move" | "resize";

/** Which edge a resize drag grabbed. */
export type ResizeEdge = "left" | "right";

/** Per-view mapping from generic grid edits to project edits. */
export interface EditAdapter {
  /** Create a block at a snapped (tick, lane). */
  create(tick: number, lane: number): void;
  /** Commit a move: new start tick and (if `laneMovable`) new lane. */
  move(block: Block, startTick: number, lane: number): void;
  /** Commit a resize: new start tick and length (ticks). A right-edge resize
   *  passes the unchanged start; a left-edge resize moves the start, end fixed. */
  resize(block: Block, startTick: number, lenTick: number): void;
  /** Delete a block. */
  remove(block: Block): void;
  /** Audition a lane's pitch once (piano roll). Fired when a note is selected
   *  (grab confirmation) and for each new row crossed during a move drag — once
   *  per lane, never per pixel, so it does not machine-gun. */
  previewLane?(lane: number): void;
  /** Whether dragging can change the lane (piano roll: yes; timeline: no). */
  laneMovable: boolean;
  /** Whether the left edge resizes too (piano roll: yes; timeline clips: no,
   *  their start moves via a clip move, keeping `resize` length-only). */
  resizableLeft: boolean;
  /** Minimum length in ticks for a created or resized block. */
  minLen: number;
  /** Whether a persistent selection outline is shown after a click (default
   *  true). The timeline turns it off: a band click means "select the track",
   *  and its clip selection drives nothing (no timeline delete; move/resize
   *  work by direct drag). */
  showSelection?: boolean;
}

/** Live view state the machine reads on each gesture (viewport and data change). */
export interface InteractionDeps {
  viewport: () => Viewport;
  blocks: () => Block[];
  laneCount: () => number;
  /** Snap a tick to the nearest grid line, unless `free` (Alt held). */
  snap: (tick: number, free: boolean) => number;
  /** Current grid subdivision in ticks (adapts to zoom). All ops share it. */
  snapStep: () => number;
  adapter: EditAdapter;
}

export interface Modifiers {
  alt: boolean;
}

/** Max distance from a block edge, in CSS pixels, that starts a resize. */
const RESIZE_EDGE_PX = 6;
/** A resize zone never eats more than this fraction of the block's width, so a
 *  short note always keeps a draggable core (the remaining ~40%) at its centre. */
const MAX_EDGE_FRACTION = 0.3;
/** Below this on-screen width (px) a block is resize-disabled: move only. */
const MIN_RESIZABLE_PX = 12;
/** Pointer travel, in CSS pixels, before a press becomes a drag. */
const DRAG_THRESHOLD_PX = 3;

type Phase = "idle" | "pressed" | "dragging";

/**
 * The gesture machine. One instance per grid view; `GridView` forwards DOM
 * pointer/keyboard events to it and renders `ghost()`/`selected`.
 */
export class GridInteractions {
  private readonly deps: InteractionDeps;
  private phase: Phase = "idle";
  private kind: DragKind = "move";
  /** Which edge a resize grabbed (only meaningful while `kind === "resize"`). */
  private resizeEdge: ResizeEdge = "right";
  private origin = { x: 0, y: 0 };
  /** The block under the press (the edit target), or null. */
  private target: Block | null = null;
  /** Live preview of the target during a drag; what the renderer draws as ghost. */
  private draft: Block | null = null;
  /** Last lane auditioned during a move drag, so we echo only on a row change. */
  private previewedLane = NaN;
  /** Currently selected block (for keyboard delete). */
  selected: Block | null = null;

  constructor(deps: InteractionDeps) {
    this.deps = deps;
  }

  /** The ghost block to render during a drag, or null. */
  ghost(): Block | null {
    return this.draft;
  }

  /** CSS cursor for hovering at (x, y): resize on a right edge, move on a body. */
  cursorAt(x: number, y: number): string {
    const vp = this.deps.viewport();
    const b = hitTest(vp, this.deps.blocks(), x, y);
    if (!b) return "default";
    return this.edge(vp, b, x) ? "ew-resize" : "move";
  }

  onPointerDown(x: number, y: number): void {
    const vp = this.deps.viewport();
    const b = hitTest(vp, this.deps.blocks(), x, y);
    this.origin = { x, y };
    this.target = b;
    this.selected = b;
    if (b) {
      // Grabbing a note auditions its pitch once (a height confirmation), through
      // the same punctual preview path as the drag audition.
      this.deps.adapter.previewLane?.(b.lane);
      const edge = this.edge(vp, b, x);
      this.kind = edge ? "resize" : "move";
      if (edge) this.resizeEdge = edge;
      this.phase = "pressed";
    } else {
      this.phase = "idle";
    }
  }

  /** Returns true if the view should redraw. */
  onPointerMove(x: number, y: number, mods: Modifiers): boolean {
    if (this.phase === "pressed") {
      if (Math.hypot(x - this.origin.x, y - this.origin.y) < DRAG_THRESHOLD_PX) {
        return false;
      }
      this.phase = "dragging";
      this.draft = this.target ? { ...this.target } : null;
      // Seed with the start lane so the first echo fires only on a row change.
      this.previewedLane = this.target ? this.target.lane : NaN;
    }
    if (this.phase === "dragging" && this.draft && this.target) {
      this.updateDraft(x, y, mods);
      return true;
    }
    return false;
  }

  /** Commit the drag (if any). Returns true if the view should redraw. */
  onPointerUp(): boolean {
    const committed = this.phase === "dragging" && this.draft !== null && this.target !== null;
    if (committed && this.draft && this.target) {
      if (this.kind === "move") {
        this.deps.adapter.move(this.target, this.draft.startTick, this.draft.lane);
      } else {
        this.deps.adapter.resize(this.target, this.draft.startTick, this.draft.lenTick);
      }
    }
    this.reset();
    return committed;
  }

  onDoubleClick(x: number, y: number): void {
    const vp = this.deps.viewport();
    // Create only in empty space. On an existing block, double-click does nothing
    // (the selecting clicks already auditioned it) — otherwise a no-op create over
    // a duplicate note would sound a spurious extra echo.
    if (hitTest(vp, this.deps.blocks(), x, y)) return;
    // Floor to the start of the subdivision the click lands in, so a note begins
    // where you clicked rather than rounding into the next cell.
    const step = this.deps.snapStep();
    const tick = Math.max(0, Math.floor(xToTick(vp, x) / step) * step);
    const lane = yToLane(vp, y);
    if (lane < 0 || lane >= this.deps.laneCount()) return;
    this.deps.adapter.create(tick, lane);
  }

  /** Cancel an in-progress drag (Escape). Returns true if anything was cancelled. */
  cancel(): boolean {
    if (this.phase === "dragging") {
      this.reset();
      return true;
    }
    return false;
  }

  /** Delete the selected block. Returns true if one was deleted. */
  deleteSelected(): boolean {
    if (this.selected) {
      this.deps.adapter.remove(this.selected);
      this.selected = null;
      return true;
    }
    return false;
  }

  private updateDraft(x: number, y: number, mods: Modifiers): void {
    const vp = this.deps.viewport();
    if (!this.draft || !this.target) return;
    if (this.kind === "move") {
      const deltaTicks = xToTick(vp, x) - xToTick(vp, this.origin.x);
      this.draft.startTick = Math.max(0, this.deps.snap(this.target.startTick + deltaTicks, mods.alt));
      if (this.deps.adapter.laneMovable) {
        const deltaLane = yToLane(vp, y) - yToLane(vp, this.origin.y);
        const lane = this.target.lane + deltaLane;
        this.draft.lane = Math.min(Math.max(lane, 0), this.deps.laneCount() - 1);
        // Audition the new pitch only when the drag crosses into a new row.
        if (this.draft.lane !== this.previewedLane) {
          this.previewedLane = this.draft.lane;
          this.deps.adapter.previewLane?.(this.draft.lane);
        }
      }
    } else {
      // Snap the moving edge to the grid; the length floor is one subdivision
      // (so a resize lands on the displayed grid), or `minLen` when free (Alt).
      const minLen = mods.alt ? this.deps.adapter.minLen : this.deps.snapStep();
      if (this.resizeEdge === "right") {
        const end = this.deps.snap(xToTick(vp, x), mods.alt);
        this.draft.startTick = this.target.startTick;
        this.draft.lenTick = Math.max(minLen, end - this.target.startTick);
      } else {
        // Left edge: the end (start + length) stays put; the start moves.
        const end = this.target.startTick + this.target.lenTick;
        const start = Math.min(Math.max(0, this.deps.snap(xToTick(vp, x), mods.alt)), end - minLen);
        this.draft.startTick = start;
        this.draft.lenTick = end - start;
      }
    }
  }

  /** Which resize edge `x` is within, or null. Right wins ties (the common op);
   *  the left edge resizes only when the adapter allows it. The zone shrinks with
   *  the block so a short note keeps a draggable core, and vanishes entirely once
   *  the block is too narrow on screen (then the whole block moves). */
  private edge(vp: Viewport, b: Block, x: number): ResizeEdge | null {
    const widthPx = b.lenTick * vp.pxPerTick;
    if (widthPx < MIN_RESIZABLE_PX) return null; // too short: move only
    const zone = Math.min(RESIZE_EDGE_PX, widthPx * MAX_EDGE_FRACTION);
    const right = tickToX(vp, b.startTick + b.lenTick);
    if (Math.abs(x - right) <= zone) return "right";
    if (this.deps.adapter.resizableLeft) {
      const left = tickToX(vp, b.startTick);
      if (Math.abs(x - left) <= zone) return "left";
    }
    return null;
  }

  private reset(): void {
    this.phase = "idle";
    this.draft = null;
    this.target = null;
  }
}

/**
 * Finest grid step (in ticks) that is still at least `MIN_SNAP_PX` wide at the
 * current zoom, so snapping feels right from bar level (zoomed out) down to
 * sixteenths (zoomed in).
 */
export function snapStepTicks(ticksPerBeat: number, pxPerTick: number): number {
  const MIN_SNAP_PX = 12;
  const candidates = [
    ticksPerBeat / 4,
    ticksPerBeat / 2,
    ticksPerBeat,
    ticksPerBeat * 2,
    ticksPerBeat * 4,
  ];
  for (const step of candidates) {
    if (step * pxPerTick >= MIN_SNAP_PX) return step;
  }
  return candidates[candidates.length - 1];
}

/** Snap a tick to the adaptive grid, unless `free` (then round to a whole tick). */
export function adaptiveSnap(
  tick: number,
  ticksPerBeat: number,
  pxPerTick: number,
  free: boolean,
): number {
  if (free) return Math.round(tick);
  const step = snapStepTicks(ticksPerBeat, pxPerTick);
  return Math.round(tick / step) * step;
}
