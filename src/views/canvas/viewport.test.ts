import { describe, expect, it } from "vitest";

import { samplesToTicks, tickToX, xToTick, type Viewport } from "./viewport";

const vp: Viewport = {
  scrollTicks: 480,
  pxPerTick: 0.05,
  scrollLane: 0,
  laneHeight: 12,
};

describe("viewport", () => {
  it("tickToX and xToTick are inverses", () => {
    const tick = 2000;
    expect(xToTick(vp, tickToX(vp, tick))).toBeCloseTo(tick, 6);
  });

  it("samplesToTicks: one quarter note at 120 bpm / 48 kHz = 960 ticks", () => {
    // 24000 samples = 0.5 s = one quarter note at 120 bpm.
    expect(samplesToTicks(24000, 120, 48000, 960)).toBeCloseTo(960, 6);
  });
});
