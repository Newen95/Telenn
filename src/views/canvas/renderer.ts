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
}

export interface Grid {
  beatsPerBar: number;
  ticksPerBeat: number;
}

const BG = "#161618";
const LANE_LINE = "#232325";
const BEAT_LINE = "#2a2a2c";
const BAR_LINE = "#3c3c3f";
const PLAYHEAD = "#e0b050";

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

  // Playhead.
  const px = Math.round(tickToX(vp, playheadTick)) + 0.5;
  if (px >= 0 && px <= w) {
    ctx.strokeStyle = PLAYHEAD;
    ctx.beginPath();
    ctx.moveTo(px, 0);
    ctx.lineTo(px, h);
    ctx.stroke();
  }
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
