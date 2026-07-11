//! Immediate-mode renderer shared by the timeline and piano roll: it draws a
//! bar/beat grid, a set of generic blocks (clips or notes), and the playhead,
//! with viewport culling. Both views differ only in the blocks and lane mapping
//! they feed in. `hitTest` is exposed now so the 5b gesture machine reuses it.

import { theme, withAlpha } from "../../styles/theme";
import { laneToY, tickToX, xToTick, yToLane, type Viewport } from "./viewport";

/** A generic rectangle on the grid: a clip (timeline) or a note (piano roll). */
export interface Block {
  startTick: number;
  lenTick: number;
  lane: number;
  color: string;
  /** Opaque handle the view's edit adapter interprets (a note or clip id). */
  ref?: unknown;
  /** Miniature content preview (a clip's notes), normalized inside the block:
   *  `t`/`len` as fractions of the block length, `y` 0 = top of the row. */
  marks?: { t: number; len: number; y: number }[];
  /** How the body renders: `solid` (a note) or `glass` (a clip: a faint tint
   *  of the hue, a crisp hue border, and the marks in the full hue). */
  style?: "solid" | "glass";
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

/** Per-lane content of a label gutter row (the timeline's track column). */
export interface LaneLabel {
  label: string;
  /** The track's hue chip. */
  color: string;
  /** The active track row is highlighted. */
  active?: boolean;
}

/** The piano roll's keyboard gutter (default kind). */
export interface KeyGutter {
  kind?: "keys";
  width: number;
  lane(lane: number): LaneKey;
}

/** The timeline's clickable track-name gutter. */
export interface LabelGutter {
  kind: "labels";
  width: number;
  lane(lane: number): LaneLabel;
}

/** Left gutter. Width must equal `vp.originX`. */
export type GutterStyle = KeyGutter | LabelGutter;

// Every shade comes from the theme (single source of truth). The drag grammar
// is one language everywhere: what you hold turns into a light ghost, where it
// lands lights up copper.
const BG = theme.color.bg;
// The active track's band: a faint warm wash held between two copper
// hairlines. An edge reads as a marker where a neutral veil could pass for
// the track's own color — and it stays in the copper language of "active".
const ACTIVE_WASH = withAlpha(theme.color.accent, 0.08);
const ACTIVE_EDGE = withAlpha(theme.color.accentBright, 0.6);
const LANE_LINE = theme.canvas.laneLine;
const BEAT_LINE = theme.canvas.beatLine;
const BAR_LINE = theme.canvas.barLine;
const PLAYHEAD = theme.color.accentBright; // the taut copper string
const PLAYHEAD_HALO = "rgba(232, 163, 76, 0.25)";
const SELECTED = theme.color.text;
const LANDING = theme.color.accentBright; // where a dragged thing will land
const FLASH = theme.color.sigMod; // externally-edited notes pulse in signal amber
const GHOST_FILL = "rgba(232, 228, 220, 0.35)";
const GHOST_ROW = "rgba(232, 228, 220, 0.14)"; // a whole row held during a reorder
// A block under the running playhead reads as lit by warm yellow light — the
// playhead's warmth spreads to what it is sounding, not a neutral white flash.
const SOUNDING = withAlpha(theme.color.glow, 0.7);
const SOUNDING_GLOW = theme.color.glow;
const GUTTER_BG = theme.color.bg;
const GUTTER_WHITE = theme.canvas.keyWhite;
const GUTTER_BLACK = theme.canvas.keyBlack;
const GUTTER_ROW = theme.canvas.keyRow;
const GUTTER_EDGE = theme.canvas.gutterEdge;
const GUTTER_LABEL = theme.canvas.keyLabel;
const GUTTER_LIT = theme.color.accentBright; // a sounding key lights up

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
  /** Flash intensity (1 → 0) for a block by its `ref`; 0 (or omitted) = none. */
  flash?: (ref: unknown) => number,
  /** Lane to lift with the active-track tint (the timeline's selected row). */
  activeLane?: number,
  /** In-progress gutter reorder: dims `from` and marks the insertion at `to`. */
  gutterDrag?: { from: number; to: number } | null,
  /** Transport running: blocks under the playhead light up as they sound. */
  playing?: boolean,
): void {
  ctx.fillStyle = BG;
  ctx.fillRect(0, 0, w, h);

  // Horizontal lane separators (only the visible ones). In the timeline the
  // clips fill their whole row, so the separators are redrawn OVER the blocks
  // further down; here they only serve the empty rows.
  drawLaneLines(ctx, w, h, vp, laneCount);

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
    const rw = Math.max(bw, 1.5);
    if (b.style === "glass") {
      // Tinted glass: a faint wash of the hue with the notes as the brightest
      // element — the grid stays visible through it, no border. Full row
      // height: the hairline separators are redrawn over the blocks below.
      ctx.fillStyle = withAlpha(b.color, 0.24);
      ctx.fillRect(x, y, rw, vp.laneHeight);
      if (b.marks && vp.laneHeight >= 14) {
        const innerH = vp.laneHeight - 8;
        for (const m of b.marks) {
          const mx = x + m.t * bw;
          const mw = Math.max(m.len * bw - 1, 1.5);
          const my = y + 4 + m.y * (innerH - 2);
          ctx.fillStyle = b.color;
          ctx.fillRect(mx, my, mw, 2);
          // A mark the running playhead is crossing lights up (state-driven:
          // it is sounding right now). Same geometry with a tight halo — at
          // this scale a size change would read as a deformation.
          if (playing) {
            const ms = b.startTick + m.t * b.lenTick;
            if (playheadTick >= ms && playheadTick < ms + m.len * b.lenTick) {
              ctx.save();
              ctx.shadowColor = SOUNDING_GLOW;
              ctx.shadowBlur = 3;
              ctx.fillStyle = SOUNDING;
              ctx.fillRect(mx, my, mw, 2);
              ctx.restore();
            }
          }
        }
      }
    } else {
      ctx.fillStyle = b.color;
      ctx.fillRect(x, y + 1, rw, vp.laneHeight - 2);
      // A sounding note lights up: a yellow-lit core and a soft halo around it
      // (the glow pass fills with the light color so the halo is truly bright).
      if (playing && playheadTick >= b.startTick && playheadTick < b.startTick + b.lenTick) {
        ctx.save();
        ctx.shadowColor = SOUNDING_GLOW;
        ctx.shadowBlur = 6;
        ctx.fillStyle = SOUNDING;
        ctx.fillRect(x, y + 1, rw, vp.laneHeight - 2);
        ctx.restore();
      }
    }
    // A note changed by an external reload pulses, fading over its flash window.
    const a = flash ? flash(b.ref) : 0;
    if (a > 0) {
      ctx.save();
      ctx.globalAlpha = a;
      ctx.fillStyle = FLASH;
      ctx.fillRect(x, y + 1, rw, vp.laneHeight - 2);
      ctx.restore();
    }
  }

  // Timeline (label-gutter) rows: redraw the separators OVER the full-height
  // clips, so two adjacent filled tracks part with the same hairline as two
  // empty ones.
  if (gutter?.kind === "labels") {
    drawLaneLines(ctx, w, h, vp, laneCount);
  }

  // The active track's whole band reads as selected, whatever selected it.
  // Drawn OVER the blocks: a lane fully covered by clips still visibly marks.
  if (activeLane !== undefined && activeLane >= 0) {
    const y = laneToY(vp, activeLane);
    if (y + vp.laneHeight >= 0 && y <= h) {
      ctx.fillStyle = ACTIVE_WASH;
      ctx.fillRect(0, y, w, vp.laneHeight);
      ctx.strokeStyle = ACTIVE_EDGE;
      ctx.lineWidth = 1.5;
      const yt = Math.round(y) + 0.75;
      const yb = Math.round(y + vp.laneHeight) - 0.75;
      ctx.beginPath();
      ctx.moveTo(0, yt);
      ctx.lineTo(w, yt);
      ctx.moveTo(0, yb);
      ctx.lineTo(w, yb);
      ctx.stroke();
      ctx.lineWidth = 1;
    }
  }

  // Selection outline and the drag ghost, on top of the blocks. The ghost's
  // outline is copper: it marks where the held block will land.
  if (overlay?.selected) outline(ctx, vp, overlay.selected, SELECTED);
  if (overlay?.ghost) {
    const g = overlay.ghost;
    const x = tickToX(vp, g.startTick);
    const y = laneToY(vp, g.lane);
    ctx.fillStyle = GHOST_FILL;
    ctx.fillRect(x, y + 1, Math.max(g.lenTick * vp.pxPerTick, 1.5), vp.laneHeight - 2);
    outline(ctx, vp, g, LANDING);
  }

  // Playhead: the taut copper string — a bright core with a faint halo.
  const px = Math.round(tickToX(vp, playheadTick)) + 0.5;
  if (px >= 0 && px <= w) {
    ctx.strokeStyle = PLAYHEAD_HALO;
    ctx.lineWidth = 3;
    ctx.beginPath();
    ctx.moveTo(px, 0);
    ctx.lineTo(px, h);
    ctx.stroke();
    ctx.strokeStyle = PLAYHEAD;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(px, 0);
    ctx.lineTo(px, h);
    ctx.stroke();
  }

  // Left inset, painted last and opaque so any grid/notes scrolled behind it are
  // covered; it owns the [0, originX) column. The piano roll fills it with its
  // keyboard; other views (timeline) reserve a plain strip so the time axes — and
  // thus the playheads — line up across both.
  // Track-reorder drag, same grammar as every drag: the held row turns into a
  // light ghost and the landing boundary lights up copper, full width (the
  // gutter repeats both over its own column).
  if (gutterDrag) {
    const fy = laneToY(vp, gutterDrag.from);
    ctx.fillStyle = GHOST_ROW;
    ctx.fillRect(0, fy, w, vp.laneHeight);
    if (gutterDrag.to !== gutterDrag.from) {
      const edgeLane = gutterDrag.to <= gutterDrag.from ? gutterDrag.to : gutterDrag.to + 1;
      const y = Math.round(laneToY(vp, edgeLane)) + 0.5;
      ctx.strokeStyle = LANDING;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
      ctx.lineWidth = 1;
    }
  }

  const gw = vp.originX ?? 0;
  if (gutter?.kind === "labels") {
    drawLabelGutter(ctx, h, vp, gutter, laneCount, gutterDrag);
  } else if (gutter) {
    drawGutter(ctx, h, vp, gutter, laneCount);
  } else if (gw > 0) {
    ctx.fillStyle = GUTTER_BG;
    ctx.fillRect(0, 0, gw, h);
    gutterEdge(ctx, h, gw);
  }
}

/** Draw the label gutter: one clickable track row per lane, hue chip + name. */
function drawLabelGutter(
  ctx: CanvasRenderingContext2D,
  h: number,
  vp: Viewport,
  gutter: LabelGutter,
  laneCount: number,
  drag?: { from: number; to: number } | null,
): void {
  const gw = gutter.width;
  ctx.fillStyle = theme.color.panel;
  ctx.fillRect(0, 0, gw, h);

  const first = Math.max(0, yToLane(vp, 0));
  const last = Math.min(laneCount - 1, yToLane(vp, h));
  ctx.textBaseline = "middle";
  ctx.font = `600 10px ${theme.font.display}`;
  for (let lane = first; lane <= last; lane += 1) {
    const row = gutter.lane(lane);
    const y = laneToY(vp, lane);
    if (row.active) {
      ctx.fillStyle = theme.color.surface2;
      ctx.fillRect(0, y, gw, vp.laneHeight);
    }
    ctx.fillStyle = row.color;
    ctx.fillRect(0, y + 1, row.active ? 4 : 2, vp.laneHeight - 2);
    ctx.fillStyle = row.active ? theme.color.text : theme.color.text2;
    ctx.fillText(fitText(ctx, row.label.toUpperCase(), gw - 14), 9, y + vp.laneHeight / 2 + 0.5);
    // The held row of a reorder reads as a light ghost (same drag grammar).
    if (drag && lane === drag.from) {
      ctx.fillStyle = GHOST_ROW;
      ctx.fillRect(0, y, gw, vp.laneHeight);
    }
    ctx.strokeStyle = LANE_LINE;
    ctx.beginPath();
    const by = Math.round(y + vp.laneHeight) + 0.5;
    ctx.moveTo(0, by);
    ctx.lineTo(gw, by);
    ctx.stroke();
  }

  // Reorder insertion mark: a copper line at the row boundary the dragged
  // track will land on (above `to` when moving up, below it when moving down).
  if (drag && drag.to !== drag.from) {
    const edgeLane = drag.to <= drag.from ? drag.to : drag.to + 1;
    const y = Math.round(laneToY(vp, edgeLane)) + 0.5;
    ctx.strokeStyle = theme.color.accentBright;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(gw, y);
    ctx.stroke();
    ctx.lineWidth = 1;
  }

  gutterEdge(ctx, h, gw);
}

/** Horizontal lane separators as true device hairlines: exactly one physical
 *  pixel whatever the display scale, aligned to the device grid so every row
 *  parts with the same thickness. */
function drawLaneLines(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  vp: Viewport,
  laneCount: number,
): void {
  // The context is pre-scaled by the dpr (useGridCanvas); read it back to plant
  // device hairlines. Mock contexts (perf test) may lack getTransform.
  const dpr = (typeof ctx.getTransform === "function" ? ctx.getTransform().a : 1) || 1;
  const hairline = 1 / dpr;
  ctx.strokeStyle = LANE_LINE;
  ctx.lineWidth = hairline;
  for (let lane = 0; lane <= laneCount; lane += 1) {
    const y = Math.round(laneToY(vp, lane) * dpr) / dpr + hairline / 2;
    if (y < 0 || y > h) continue;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
  }
  ctx.lineWidth = 1;
}

/** Truncate `text` with an ellipsis so it fits in `maxWidth` pixels. */
function fitText(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string {
  if (ctx.measureText(text).width <= maxWidth) return text;
  let end = text.length;
  while (end > 1 && ctx.measureText(`${text.slice(0, end)}…`).width > maxWidth) end -= 1;
  return `${text.slice(0, end)}…`;
}

/** Draw the left keyboard gutter: one keycap per visible lane, octave labels. */
function drawGutter(
  ctx: CanvasRenderingContext2D,
  h: number,
  vp: Viewport,
  gutter: KeyGutter,
  laneCount: number,
): void {
  const gw = gutter.width;
  ctx.fillStyle = GUTTER_BG;
  ctx.fillRect(0, 0, gw, h);

  const first = Math.max(0, yToLane(vp, 0));
  const last = Math.min(laneCount - 1, yToLane(vp, h));
  for (let lane = first; lane <= last; lane += 1) {
    const key = gutter.lane(lane);
    const y = laneToY(vp, lane);
    // Row separator first, then the keycap inset by 1px so rows read distinctly.
    ctx.fillStyle = GUTTER_ROW;
    ctx.fillRect(0, y, gw, vp.laneHeight);
    ctx.fillStyle = key.lit ? GUTTER_LIT : key.dark ? GUTTER_BLACK : GUTTER_WHITE;
    ctx.fillRect(0, y + 1, gw - 1, vp.laneHeight - 1);
  }

  // Octave labels in a second pass: a keycap painted after its neighbour would
  // otherwise clip the label's glyphs (rows are barely taller than the font).
  ctx.textBaseline = "middle";
  ctx.textAlign = "right";
  ctx.font = `600 9px ${theme.font.mono}`;
  ctx.fillStyle = GUTTER_LABEL;
  for (let lane = first; lane <= last; lane += 1) {
    const key = gutter.lane(lane);
    if (!key.label) continue;
    const y = laneToY(vp, lane);
    ctx.fillText(key.label, gw - 5, y + vp.laneHeight / 2 + 0.5);
  }
  ctx.textAlign = "left";

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
