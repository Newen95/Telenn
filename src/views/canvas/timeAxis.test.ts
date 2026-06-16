import { describe, expect, it } from "vitest";

import { applyAxis, useTimeAxis } from "./timeAxis";
import { tickToX, type Viewport } from "./viewport";

// Regression for the desynced playheads: the timeline and the piano roll must map
// a tick to the *same* X. They share one horizontal axis; only their vertical lane
// height differs. So `tickToX` must agree between them at every scroll and zoom.

function viewWithLaneHeight(laneHeight: number): Viewport {
  return { scrollTicks: 0, pxPerTick: 0, scrollLane: 0, laneHeight };
}

describe("shared time axis", () => {
  it("places the same instant at the same X in both views, across zoom and scroll", () => {
    const axis = useTimeAxis();
    const timeline = viewWithLaneHeight(28); // tracks
    const pianoRoll = viewWithLaneHeight(10); // pitches

    const states = [
      { scrollTicks: 0, pxPerTick: 0.03 },
      { scrollTicks: 5000, pxPerTick: 0.1 },
      { scrollTicks: 123, pxPerTick: 0.004 },
    ];
    const ticks = [0, 960, 4000, 200_000];

    for (const s of states) {
      axis.scrollTicks = s.scrollTicks;
      axis.pxPerTick = s.pxPerTick;
      applyAxis(timeline, axis);
      applyAxis(pianoRoll, axis);
      for (const t of ticks) {
        expect(tickToX(pianoRoll, t)).toBe(tickToX(timeline, t));
      }
    }
  });

  it("reserves the gutter inset (originX) on the shared axis, so tick 0 is inset", () => {
    const axis = useTimeAxis();
    const vp = viewWithLaneHeight(10);
    applyAxis(vp, axis);
    expect(axis.originX).toBeGreaterThan(0);
    expect(tickToX(vp, axis.scrollTicks)).toBe(axis.originX);
  });
});
