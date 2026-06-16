import { describe, expect, it } from "vitest";

import { draw, type Block, type Grid } from "./renderer";
import type { Viewport } from "./viewport";

// Performance gate (MVP_PLAN M5): a 5000-note scene scrolled and zoomed must keep
// the renderer's own frame time well under one 60 fps frame. We measure `draw`
// against a no-op 2D context, which isolates the renderer's JS cost (culling,
// iteration) from real rasterisation — the part this codebase owns.

/** Minimal 2D context stub: records nothing, just satisfies the calls `draw` makes. */
class FakeCtx {
  fillStyle = "";
  strokeStyle = "";
  lineWidth = 1;
  fillRect(): void {}
  strokeRect(): void {}
  beginPath(): void {}
  moveTo(): void {}
  lineTo(): void {}
  stroke(): void {}
}

function scene(count: number): Block[] {
  const blocks: Block[] = [];
  const PPQ = 960;
  for (let i = 0; i < count; i += 1) {
    blocks.push({
      startTick: (i % 512) * (PPQ / 4),
      lenTick: PPQ / 2,
      lane: 24 + (i % 60),
      color: "#5fae7f",
      ref: `n${i}`,
    });
  }
  return blocks;
}

function p95(samples: number[]): number {
  const sorted = [...samples].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.95))];
}

describe("renderer performance", () => {
  it("renders 5000 notes under 16 ms p95 while scrolling and zooming", () => {
    const ctx = new FakeCtx() as unknown as CanvasRenderingContext2D;
    const w = 1200;
    const h = 700;
    const grid: Grid = { beatsPerBar: 4, ticksPerBeat: 960 };
    const blocks = scene(5000);

    const FRAMES = 120;
    const times: number[] = [];
    for (let f = 0; f < FRAMES; f += 1) {
      // Scripted scroll (time) and zoom (pxPerTick) sweep across the run.
      const phase = f / FRAMES;
      const vp: Viewport = {
        scrollTicks: phase * 200_000,
        pxPerTick: 0.004 + phase * 0.1,
        scrollLane: Math.floor(phase * 40),
        laneHeight: 10,
      };
      const playhead = phase * 200_000;
      const start = performance.now();
      draw(ctx, w, h, vp, grid, blocks, 128, playhead);
      times.push(performance.now() - start);
    }

    expect(p95(times)).toBeLessThan(16);
  });
});
