import { describe, expect, it } from "vitest";

import {
  DEFAULT_PX_PER_TICK,
  fitView,
  keyZoomFactor,
  panBy,
  readWheel,
  wheelZoomFactor,
  zoomAt,
} from "./navigation";
import type { Block } from "./renderer";
import { tickToX, type Viewport } from "./viewport";

function vp(over: Partial<Viewport> = {}): Viewport {
  return { scrollTicks: 0, pxPerTick: 0.1, scrollLane: 0, laneHeight: 10, originX: 0, ...over };
}

describe("wheel interpretation (the DAW convention, mouse + trackpad)", () => {
  it("treats ctrlKey as zoom — this is how the OS reports a trackpad pinch", () => {
    const intent = readWheel({ deltaX: 0, deltaY: -50, ctrlKey: true, shiftKey: false });
    expect(intent.kind).toBe("zoom");
  });

  it("pans vertically on a plain mouse wheel (deltaY only)", () => {
    const intent = readWheel({ deltaX: 0, deltaY: 30, ctrlKey: false, shiftKey: false });
    expect(intent).toEqual({ kind: "pan", dxPx: 0, dyPx: 30 });
  });

  it("redirects the wheel to the horizontal axis with Shift (mouse)", () => {
    const intent = readWheel({ deltaX: 0, deltaY: 30, ctrlKey: false, shiftKey: true });
    expect(intent).toEqual({ kind: "pan", dxPx: 30, dyPx: 0 });
  });

  it("pans both axes from a trackpad two-finger scroll (deltaX + deltaY)", () => {
    const intent = readWheel({ deltaX: 12, deltaY: -8, ctrlKey: false, shiftKey: false });
    expect(intent).toEqual({ kind: "pan", dxPx: 12, dyPx: -8 });
  });

  it("zooms in on an up-gesture (positive deltaY here) and out on a down-gesture", () => {
    // Convention: up = zoom in. On the target trackpad an up motion is +deltaY.
    expect(wheelZoomFactor(100)).toBeGreaterThan(1);
    expect(wheelZoomFactor(-100)).toBeLessThan(1);
    expect(keyZoomFactor(1)).toBeGreaterThan(1);
    expect(keyZoomFactor(-1)).toBeLessThan(1);
  });
});

describe("zoomAt", () => {
  it("keeps the tick under the cursor fixed (cursor-centred zoom)", () => {
    const v = vp({ pxPerTick: 0.1 });
    const px = 100;
    const before = tickToX(v, 1000); // the tick at px 100
    expect(before).toBeCloseTo(px);
    zoomAt(v, px, 2);
    expect(v.pxPerTick).toBeCloseTo(0.2);
    expect(tickToX(v, 1000)).toBeCloseTo(px); // same instant, same X
  });

  it("centres on the cursor accounting for the gutter inset", () => {
    const v = vp({ pxPerTick: 0.1, originX: 56 });
    const px = 120;
    const tick = (px - 56) / 0.1;
    zoomAt(v, px, 2); // zoom in, so the scroll stays positive (no clamp at 0)
    expect(tickToX(v, tick)).toBeCloseTo(px);
  });

  it("clamps the zoom and never scrolls before tick 0", () => {
    const v = vp({ pxPerTick: 0.49 });
    zoomAt(v, 0, 100); // way past the max
    expect(v.pxPerTick).toBeLessThanOrEqual(0.5);
    expect(v.scrollTicks).toBeGreaterThanOrEqual(0);
  });
});

describe("panBy", () => {
  it("scrolls horizontally in ticks scaled by the zoom", () => {
    const v = vp({ pxPerTick: 0.1 });
    panBy(v, 50, 0, 100);
    expect(v.scrollTicks).toBeCloseTo(500); // 50px / 0.1 px-per-tick
  });

  it("never scrolls before tick 0", () => {
    const v = vp({ scrollTicks: 100, pxPerTick: 0.1 });
    panBy(v, -100000, 0, 100);
    expect(v.scrollTicks).toBe(0);
  });

  it("clamps the vertical scroll between 0 and maxLane", () => {
    const v = vp({ laneHeight: 10 });
    panBy(v, 0, 100000, 5); // far past the bottom
    expect(v.scrollLane).toBe(5);
    panBy(v, 0, -100000, 5); // far past the top
    expect(v.scrollLane).toBe(0);
  });
});

describe("fitView", () => {
  it("frames the content: span fits the width, lanes centre vertically", () => {
    const blocks: Block[] = [
      { startTick: 1000, lenTick: 500, lane: 40, color: "#000", ref: "a" },
      { startTick: 4000, lenTick: 1000, lane: 60, color: "#000", ref: "b" },
    ];
    const v = vp({ originX: 0, laneHeight: 10 });
    fitView(v, blocks, 1000, 400, 128);
    // The whole span (1000..5000) is inside [0, width] after fitting.
    expect(tickToX(v, 1000)).toBeGreaterThanOrEqual(0);
    expect(tickToX(v, 5000)).toBeLessThanOrEqual(1000);
    // Lane span 40..60 (centre ~50) sits around the middle of the 40 visible lanes.
    expect(v.scrollLane).toBeCloseTo(50.5 - 20, 0);
  });

  it("returns to the origin at the default zoom when there is nothing to frame", () => {
    const v = vp({ scrollTicks: 999, pxPerTick: 0.3, scrollLane: 70 });
    fitView(v, [], 1000, 400, 128);
    expect(v.scrollTicks).toBe(0);
    expect(v.pxPerTick).toBe(DEFAULT_PX_PER_TICK);
    expect(v.scrollLane).toBe(0);
  });
});
