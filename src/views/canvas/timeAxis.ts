//! The horizontal time axis shared by the timeline and the piano roll.
//!
//! Both views map ticks to screen pixels with the exact same parameters (scroll,
//! zoom and the left gutter inset), so the same musical instant lands at the same
//! X in both — most visibly the playhead. The vertical axis (lanes) stays local to
//! each view, since a track row and a pitch row are unrelated.

import { reactive } from "vue";

import type { Viewport } from "./viewport";

/** Mutable horizontal mapping. The same object instance is handed to both views. */
export interface TimeAxis {
  /** Leftmost visible tick. */
  scrollTicks: number;
  /** Horizontal zoom, pixels per tick. */
  pxPerTick: number;
  /** Left inset (px): the keyboard gutter on the piano roll, reserved on both. */
  originX: number;
}

/** Width of the left gutter, in CSS pixels — wide enough for real-looking keys
 *  and octave labels (C4, C5…). It is the shared `originX` for both views. */
export const GUTTER_WIDTH = 56;

/** Create the shared axis. Reactive so a scroll/zoom on one view redraws both. */
export function useTimeAxis(init: Partial<TimeAxis> = {}): TimeAxis {
  return reactive<TimeAxis>({ scrollTicks: 0, pxPerTick: 0.03, originX: GUTTER_WIDTH, ...init });
}

/** Copy the shared horizontal fields into a view's local viewport before use. */
export function applyAxis(vp: Viewport, axis: TimeAxis): void {
  vp.scrollTicks = axis.scrollTicks;
  vp.pxPerTick = axis.pxPerTick;
  vp.originX = axis.originX;
}
