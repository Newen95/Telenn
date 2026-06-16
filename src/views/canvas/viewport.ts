//! Pure coordinate maths shared by every grid view (timeline, piano roll).
//!
//! The X axis is always musical time (ticks); the Y axis is "lanes" — tracks for
//! the timeline, pitches for the piano roll. Only the *meaning* of a lane differs
//! between views, so both reuse this module and the renderer; that is the shared
//! foundation the editing gestures (5b) will plug into.

export interface Viewport {
  /** Leftmost visible tick. */
  scrollTicks: number;
  /** Horizontal zoom, pixels per tick. */
  pxPerTick: number;
  /** Topmost visible lane. */
  scrollLane: number;
  /** Height of one lane in CSS pixels. */
  laneHeight: number;
  /**
   * Left inset, in CSS pixels, reserved for a lane gutter (the piano roll's
   * keyboard). Tick 0 sits at x = originX; the grid lives in [originX, w].
   * Defaults to 0 (timeline: no gutter).
   */
  originX?: number;
}

export function tickToX(vp: Viewport, tick: number): number {
  return (vp.originX ?? 0) + (tick - vp.scrollTicks) * vp.pxPerTick;
}

export function xToTick(vp: Viewport, x: number): number {
  return vp.scrollTicks + (x - (vp.originX ?? 0)) / vp.pxPerTick;
}

export function laneToY(vp: Viewport, lane: number): number {
  return (lane - vp.scrollLane) * vp.laneHeight;
}

export function yToLane(vp: Viewport, y: number): number {
  // Floor the whole sum, not just `y`: `scrollLane` may be fractional after a
  // pixel-precise vertical pan, and we still want the integer lane under `y`.
  return Math.floor(vp.scrollLane + y / vp.laneHeight);
}

/** Convert a sample count to ticks: `samples * bpm * ppq / (sr * 60)`. */
export function samplesToTicks(
  samples: number,
  bpm: number,
  sampleRate: number,
  ppq: number,
): number {
  return (samples * bpm * ppq) / (sampleRate * 60);
}
