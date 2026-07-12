// Ghosts of externally removed notes: the store captures their geometry
// BEFORE the reload wipes them, and the renderer draws a fading silhouette
// where each one was. Both halves are covered here — the renderer through a
// stub 2D context (jsdom has no canvas), the store through mocked Tauri APIs.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

import { draw, type GhostMark } from "./renderer";
import type { Viewport } from "./viewport";

// --- Tauri API mocks: capture the project_changed handler, serve a canned view.
let changedHandler: ((e: { payload: unknown }) => Promise<void>) | undefined;
const view = {
  bpm: 100,
  ppq: 960,
  timeSignature: [4, 4] as [number, number],
  masterGainDb: -6,
  tracks: [
    {
      id: "lead",
      name: "Lead",
      instrumentNode: 0,
      chain: [],
      mixer: { gainDb: 0, pan: 0 },
      clips: [
        {
          id: "c1",
          startTicks: 0,
          lengthTicks: 3840,
          notes: [{ id: "li1", pitch: 69, startTicks: 0, lengthTicks: 960, vel: 72 }],
        },
      ],
    },
  ],
};

vi.mock("@tauri-apps/api/core", () => ({
  // Deep copy: the real IPC deserializes fresh objects on every call.
  invoke: vi.fn(async (cmd: string) => (cmd === "project_view" ? structuredClone(view) : undefined)),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (e: { payload: unknown }) => Promise<void>) => {
    if (name === "project_changed") changedHandler = handler;
    return () => undefined;
  }),
}));

import { useProjectStore } from "../../stores/project";

describe("store ghost capture", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    changedHandler = undefined;
    view.tracks[0].clips[0].notes = [
      { id: "li1", pitch: 69, startTicks: 0, lengthTicks: 960, vel: 72 },
    ];
  });

  it("captures removed notes and fades them out", async () => {
    const project = useProjectStore();
    await project.init();
    await project.load();
    expect(changedHandler).toBeDefined();

    // External edit removes li1: the diff arrives, the store reloads.
    view.tracks[0].clips[0].notes = [];
    await changedHandler!({
      payload: { addedNotes: [], removedNotes: ["li1"], modifiedNotes: [] },
    });

    const now = performance.now();
    const ghosts = project.noteGhosts("lead", now);
    expect(ghosts).toHaveLength(1);
    expect(ghosts[0].pitch).toBe(69);
    expect(ghosts[0].intensity).toBeGreaterThan(0.9);
    expect(project.anyFlash(now)).toBe(true);
    // Other tracks see nothing; after the window everything is purged.
    expect(project.noteGhosts("pad", now)).toHaveLength(0);
    expect(project.noteGhosts("lead", now + 5000)).toHaveLength(0);
    expect(project.anyFlash(now + 5000)).toBe(false);
  });
});

function stubCtx(): CanvasRenderingContext2D {
  const target: Record<string, unknown> = {
    getTransform: () => ({ a: 1 }),
    measureText: () => ({ width: 10 }),
  };
  return new Proxy(target, {
    get(t, prop) {
      if (!(prop in t)) t[prop as string] = (..._args: unknown[]) => undefined;
      return t[prop as string];
    },
    set(t, prop, value) {
      t[prop as string] = value;
      return true;
    },
  }) as unknown as CanvasRenderingContext2D;
}

describe("renderer with ghosts", () => {
  it("draws ghost marks without throwing", () => {
    const vp: Viewport = {
      scrollTicks: 0,
      pxPerTick: 0.05,
      scrollLane: 40,
      laneHeight: 14,
      originX: 56,
    };
    const ghosts: GhostMark[] = [
      { startTick: 0, lenTick: 960, lane: 58, intensity: 0.9 },
      { startTick: 1920, lenTick: 480, lane: 55, intensity: 0.4 },
    ];
    expect(() =>
      draw(
        stubCtx(),
        800,
        600,
        vp,
        { beatsPerBar: 4, ticksPerBeat: 960 },
        [{ startTick: 0, lenTick: 960, lane: 60, color: "#c87b4a", ref: "n1" }],
        128,
        0,
        undefined,
        undefined,
        () => 0,
        undefined,
        null,
        false,
        ghosts,
      ),
    ).not.toThrow();
  });
});
