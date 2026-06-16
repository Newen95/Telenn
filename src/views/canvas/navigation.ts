//! Shared navigation grammar for the grid views (timeline, piano roll; the graph
//! follows in M6). One logic that serves mouse *and* trackpad: the wheel/scroll
//! pans, a modifier zooms (the DAW convention), so a trackpad's native two-finger
//! scroll means "pan" rather than fighting a wheel-zooms-by-default scheme.
//!
//! Pure maths over a `Viewport`; the Vue component wires DOM events to it. Keeping
//! it free of the DOM is what lets the device-handling rules — the trackpad pinch
//! trap in particular — be unit-tested.

import type { Block } from "./renderer";
import { xToTick, type Viewport } from "./viewport";

export const MIN_PX_PER_TICK = 0.004;
export const MAX_PX_PER_TICK = 0.5;
export const DEFAULT_PX_PER_TICK = 0.03;

// Wheel/pinch delta → zoom factor exponent. Tuned so one mouse notch (~100) is a
// gentle step while a fine trackpad pinch (~1-10) stays smooth, both via exp().
const ZOOM_WHEEL_RATE = 0.0015;
// Discrete zoom step for the +/- keys.
const ZOOM_KEY_STEP = 1.2;

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

/** Multiply horizontal zoom by `factor`, keeping the tick under `px` fixed. */
export function zoomAt(vp: Viewport, px: number, factor: number): void {
  const tick = xToTick(vp, px);
  vp.pxPerTick = clamp(vp.pxPerTick * factor, MIN_PX_PER_TICK, MAX_PX_PER_TICK);
  vp.scrollTicks = Math.max(0, tick - (px - (vp.originX ?? 0)) / vp.pxPerTick);
}

/** Pan by a pixel delta on each axis; `maxLane` bounds the vertical scroll. */
export function panBy(vp: Viewport, dxPx: number, dyPx: number, maxLane: number): void {
  if (dxPx) vp.scrollTicks = Math.max(0, vp.scrollTicks + dxPx / vp.pxPerTick);
  if (dyPx) vp.scrollLane = clamp(vp.scrollLane + dyPx / vp.laneHeight, 0, Math.max(0, maxLane));
}

/** Zoom factor for a wheel/pinch delta. An up-gesture zooms in (the dominant
 *  convention: maps, image editors, node editors). On the trackpad this app
 *  targets, an upward two-finger motion reports a *positive* deltaY, so a positive
 *  delta must zoom in (factor > 1). A mouse wheel reports the opposite sign and the
 *  OS "natural scrolling" setting can flip it too — making the direction
 *  configurable is a post-MVP refinement (see UX_SPEC). */
export function wheelZoomFactor(deltaY: number): number {
  return Math.exp(deltaY * ZOOM_WHEEL_RATE);
}

/** Zoom factor for a +/- keypress (`dir > 0` zooms in). */
export function keyZoomFactor(dir: number): number {
  return dir > 0 ? ZOOM_KEY_STEP : 1 / ZOOM_KEY_STEP;
}

/** What a wheel event means, once the device quirks are resolved. */
export type WheelIntent =
  | { kind: "zoom"; factor: number }
  | { kind: "pan"; dxPx: number; dyPx: number };

/** Minimal slice of a `WheelEvent` this module needs. */
export interface WheelLike {
  deltaX: number;
  deltaY: number;
  ctrlKey: boolean;
  shiftKey: boolean;
}

/**
 * Resolve a wheel event into a navigation intent.
 *
 * The OS reports a **trackpad pinch as a wheel event with `ctrlKey` set**, so
 * `ctrlKey` (real Ctrl on a mouse, or a pinch) always means zoom. Otherwise it is
 * a pan: a mouse wheel carries `deltaY` only — Shift redirects it to the
 * horizontal (time) axis — while a trackpad two-finger scroll carries both deltas
 * and pans on both axes at once. The caller must `preventDefault()` so the browser
 * does not zoom or scroll the whole page (the "pinch zooms the entire UI" trap).
 */
export function readWheel(e: WheelLike): WheelIntent {
  if (e.ctrlKey) return { kind: "zoom", factor: wheelZoomFactor(e.deltaY) };
  const dxPx = e.shiftKey ? e.deltaY : e.deltaX;
  const dyPx = e.shiftKey ? 0 : e.deltaY;
  return { kind: "pan", dxPx, dyPx };
}

/**
 * Frame every block in the view: fit the time span horizontally with a small
 * margin and centre the lane span vertically (lane height is fixed, so vertical
 * is scroll-only). With no blocks, return to the origin at the default zoom. `w`
 * and `h` are the canvas size in CSS pixels.
 */
export function fitView(
  vp: Viewport,
  blocks: readonly Block[],
  w: number,
  h: number,
  laneCount: number,
): void {
  if (blocks.length === 0) {
    vp.scrollTicks = 0;
    vp.pxPerTick = DEFAULT_PX_PER_TICK;
    vp.scrollLane = 0;
    return;
  }
  let minTick = Infinity;
  let maxTick = -Infinity;
  let minLane = Infinity;
  let maxLane = -Infinity;
  for (const b of blocks) {
    minTick = Math.min(minTick, b.startTick);
    maxTick = Math.max(maxTick, b.startTick + b.lenTick);
    minLane = Math.min(minLane, b.lane);
    maxLane = Math.max(maxLane, b.lane);
  }
  const gridW = Math.max(1, w - (vp.originX ?? 0));
  const span = Math.max(1, maxTick - minTick);
  vp.pxPerTick = clamp((gridW * 0.92) / span, MIN_PX_PER_TICK, MAX_PX_PER_TICK);
  vp.scrollTicks = Math.max(0, minTick - (gridW * 0.04) / vp.pxPerTick);
  const visible = h / vp.laneHeight;
  const mid = (minLane + maxLane + 1) / 2;
  vp.scrollLane = clamp(mid - visible / 2, 0, Math.max(0, laneCount - visible));
}
