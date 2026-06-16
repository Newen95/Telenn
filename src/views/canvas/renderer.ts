//! Immediate-mode renderer shared by the timeline and piano roll: it draws a
//! bar/beat grid, a set of generic blocks (clips or notes), and the playhead,
//! with viewport culling. Both views differ only in the blocks and lane mapping
//! they feed in. `hitTest` is exposed now so the 5b gesture machine reuses it.

import { laneToY, tickToX, xToTick, yToLane, type Viewport } from "./viewport";

/** A generic rectangle on the grid: a clip (timeline) or a note (piano roll). */
export interface Block {
  startTick: number;
  lenTick: number;
  lane: number;
  color: string;
  /** Opaque handle the view's edit adapter interprets (a note or clip id). */
  ref?: unknown;
}

export interface Grid {
  beatsPerBar: number;
  ticksPerBeat: number;
}

/** Transient editing overlay: the drag ghost and the current selection. */
export interface Overlay {
  ghost?: Block | null;
  selected?: Block | null;
}

/** Per-lane look of one gutter row (the piano roll's keyboard column). */
export interface LaneKey {
  /** A black key (drawn darker). */
  dark: boolean;
  /** Octave label, drawn on this row only when set (e.g. "C4"). */
  label?: string;
  /** This pitch is currently sounding (brief highlight). */
  lit?: boolean;
}

/** Left keyboard gutter for the piano roll. Width must equal `vp.originX`. */
export interface GutterStyle {
  width: number;
  lane(lane: number): LaneKey;
}

const BG = "#161618";
const LANE_LINE = "#232325";
const BEAT_LINE = "#2a2a2c";
const BAR_LINE = "#3c3c3f";
const PLAYHEAD = "#e0b050";
const SELECTED = "#f6f6f6";
const GHOST_FILL = "rgba(246, 246, 246, 0.35)";
const GUTTER_BG = "#161618";
const GUTTER_WHITE = "#e6e6e9"; // bright white keys, read at a glance
const GUTTER_BLACK = "#1b1b1d"; // near-black keys
const GUTTER_ROW = "#0d0d0e"; // crisp separation between key rows
const GUTTER_EDGE = "#000000"; // border between gutter and grid
const GUTTER_LABEL = "#4a4a4c"; // dark label, legible on a white key
const GUTTER_LIT = "#e0b050"; // a sounding key lights up (shared with playback)

/** Draw a frame. `w`/`h` are CSS pixels; `ctx` is already scaled by the dpr. */
export function draw(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  vp: Viewport,
  grid: Grid,
  blocks: Block[],
  laneCount: number,
  playheadTick: number,
  overlay?: Overlay,
  gutter?: GutterStyle,
): void {
  ctx.fillStyle = BG;
  ctx.fillRect(0, 0, w, h);

  // Horizontal lane separators (only the visible ones).
  ctx.strokeStyle = LANE_LINE;
  ctx.lineWidth = 1;
  for (let lane = 0; lane <= laneCount; lane += 1) {
    const y = Math.round(laneToY(vp, lane)) + 0.5;
    if (y < 0 || y > h) continue;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
  }

  // Vertical bar/beat grid across the visible tick range.
  const tpb = grid.ticksPerBeat;
  const tpbar = grid.beatsPerBar * tpb;
  const firstTick = xToTick(vp, 0);
  const lastTick = xToTick(vp, w);
  const firstBeat = Math.floor(firstTick / tpb);
  for (let t = firstBeat * tpb; t <= lastTick; t += tpb) {
    const x = Math.round(tickToX(vp, t)) + 0.5;
    ctx.strokeStyle = t % tpbar === 0 ? BAR_LINE : BEAT_LINE;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
    ctx.stroke();
  }

  // Blocks, culled to the viewport.
  for (const b of blocks) {
    const x = tickToX(vp, b.startTick);
    const bw = b.lenTick * vp.pxPerTick;
    if (x + bw < 0 || x > w) continue;
    const y = laneToY(vp, b.lane);
    if (y + vp.laneHeight < 0 || y > h) continue;
    ctx.fillStyle = b.color;
    ctx.fillRect(x, y + 1, Math.max(bw, 1.5), vp.laneHeight - 2);
  }

  // Selection outline and the drag ghost, on top of the blocks.
  if (overlay?.selected) outline(ctx, vp, overlay.selected, SELECTED);
  if (overlay?.ghost) {
    const g = overlay.ghost;
    const x = tickToX(vp, g.startTick);
    const y = laneToY(vp, g.lane);
    ctx.fillStyle = GHOST_FILL;
    ctx.fillRect(x, y + 1, Math.max(g.lenTick * vp.pxPerTick, 1.5), vp.laneHeight - 2);
    outline(ctx, vp, g, SELECTED);
  }

  // Playhead.
  const px = Math.round(tickToX(vp, playheadTick)) + 0.5;
  if (px >= 0 && px <= w) {
    ctx.strokeStyle = PLAYHEAD;
    ctx.beginPath();
    ctx.moveTo(px, 0);
    ctx.lineTo(px, h);
    ctx.stroke();
  }

  // Left inset, painted last and opaque so any grid/notes scrolled behind it are
  // covered; it owns the [0, originX) column. The piano roll fills it with its
  // keyboard; other views (timeline) reserve a plain strip so the time axes — and
  // thus the playheads — line up across both.
  const gw = vp.originX ?? 0;
  if (gutter) {
    drawGutter(ctx, h, vp, gutter, laneCount);
  } else if (gw > 0) {
    ctx.fillStyle = GUTTER_BG;
    ctx.fillRect(0, 0, gw, h);
    gutterEdge(ctx, h, gw);
  }
}

/** Draw the left keyboard gutter: one keycap per visible lane, octave labels. */
function drawGutter(
  ctx: CanvasRenderingContext2D,
  h: number,
  vp: Viewport,
  gutter: GutterStyle,
  laneCount: number,
): void {
  const gw = gutter.width;
  ctx.fillStyle = GUTTER_BG;
  ctx.fillRect(0, 0, gw, h);

  const first = Math.max(0, yToLane(vp, 0));
  const last = Math.min(laneCount - 1, yToLane(vp, h));
  ctx.textBaseline = "middle";
  ctx.font = "10px sans-serif";
  for (let lane = first; lane <= last; lane += 1) {
    const key = gutter.lane(lane);
    const y = laneToY(vp, lane);
    // Row separator first, then the keycap inset by 1px so rows read distinctly.
    ctx.fillStyle = GUTTER_ROW;
    ctx.fillRect(0, y, gw, vp.laneHeight);
    ctx.fillStyle = key.lit ? GUTTER_LIT : key.dark ? GUTTER_BLACK : GUTTER_WHITE;
    ctx.fillRect(0, y + 1, gw - 1, vp.laneHeight - 1);
    if (key.label) {
      ctx.fillStyle = GUTTER_LABEL;
      ctx.fillText(key.label, 4, y + vp.laneHeight / 2 + 0.5);
    }
  }

  gutterEdge(ctx, h, gw);
}

/** The 1px border separating the gutter/inset from the grid. */
function gutterEdge(ctx: CanvasRenderingContext2D, h: number, gw: number): void {
  ctx.strokeStyle = GUTTER_EDGE;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(gw - 0.5, 0);
  ctx.lineTo(gw - 0.5, h);
  ctx.stroke();
}

/** Stroke a 1px outline around a block (selection / ghost). */
function outline(ctx: CanvasRenderingContext2D, vp: Viewport, b: Block, color: string): void {
  const x = Math.round(tickToX(vp, b.startTick)) + 0.5;
  const y = Math.round(laneToY(vp, b.lane)) + 0.5;
  ctx.strokeStyle = color;
  ctx.lineWidth = 1;
  ctx.strokeRect(x, y + 1, Math.max(b.lenTick * vp.pxPerTick, 1.5), vp.laneHeight - 2);
}

/** Topmost block under a point, or `null`. Used by the 5b gesture machine. */
export function hitTest(vp: Viewport, blocks: Block[], x: number, y: number): Block | null {
  const tick = xToTick(vp, x);
  const lane = yToLane(vp, y);
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    const b = blocks[i];
    if (b.lane === lane && tick >= b.startTick && tick <= b.startTick + b.lenTick) {
      return b;
    }
  }
  return null;
}
