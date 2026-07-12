import { describe, expect, it } from "vitest";

import { adaptiveSnap, GridInteractions, snapStepTicks, type EditAdapter } from "./interactions";
import type { Block } from "./renderer";
import type { Viewport } from "./viewport";

const PPQ = 960;
const TICKS_PER_BEAT = PPQ; // 4/4

describe("adaptive snap", () => {
  it("picks a finer grid step the more we zoom in", () => {
    // Three zoom levels → three distinct steps (32nd, eighth, bar).
    const zoomedIn = snapStepTicks(TICKS_PER_BEAT, 0.1); // 120 ticks (32nd) is 12px
    const medium = snapStepTicks(TICKS_PER_BEAT, 0.02); // eighth (480) is 9.6px
    const zoomedOut = snapStepTicks(TICKS_PER_BEAT, 0.004); // bar (3840) is 15px

    expect(zoomedIn).toBe(TICKS_PER_BEAT / 8);
    expect(medium).toBe(TICKS_PER_BEAT / 2);
    expect(zoomedOut).toBe(TICKS_PER_BEAT * 4);
    expect(zoomedIn).toBeLessThan(medium);
    expect(medium).toBeLessThan(zoomedOut);
  });

  it("reaches sixteenth notes at a drum-grid zoom (a beat ≈ 36px)", () => {
    // The regression the drum patterns hit: 16ths (240 ticks) are 9px here —
    // under the old 12px floor the grid stopped at eighths.
    expect(snapStepTicks(TICKS_PER_BEAT, 0.0375)).toBe(TICKS_PER_BEAT / 4);
  });

  it("snaps to the nearest step, and to whole ticks when free (Alt)", () => {
    expect(adaptiveSnap(130, TICKS_PER_BEAT, 0.1, false)).toBe(120); // nearest 32nd
    expect(adaptiveSnap(130.6, TICKS_PER_BEAT, 0.1, true)).toBe(131); // free → rounded
  });
});

function viewport(): Viewport {
  return { scrollTicks: 0, pxPerTick: 0.1, scrollLane: 0, laneHeight: 10 };
}

/** Records adapter calls for assertions. */
function recordingAdapter(overrides: Partial<EditAdapter> = {}) {
  const calls: string[] = [];
  const adapter: EditAdapter = {
    create: (tick, lane) => calls.push(`create ${tick} ${lane}`),
    move: (b, tick, lane) => calls.push(`move ${String(b.ref)} ${tick} ${lane}`),
    resize: (b, start, len) => calls.push(`resize ${String(b.ref)} ${start} ${len}`),
    remove: (b) => calls.push(`remove ${String(b.ref)}`),
    laneMovable: true,
    resizableLeft: true,
    minLen: 60,
    ...overrides,
  };
  return { adapter, calls };
}

function makeFsm(blocks: Block[], adapter: EditAdapter, vp: Viewport = viewport()) {
  return new GridInteractions({
    viewport: () => vp,
    blocks: () => blocks,
    laneCount: () => 128,
    snap: (tick, free) => adaptiveSnap(tick, TICKS_PER_BEAT, vp.pxPerTick, free),
    snapStep: () => snapStepTicks(TICKS_PER_BEAT, vp.pxPerTick),
    adapter,
  });
}

describe("gesture state machine", () => {
  // A note: lane 0, start 0, length one beat. At pxPerTick 0.1 it spans 0..96px.
  const block: Block = { startTick: 0, lenTick: TICKS_PER_BEAT, lane: 0, color: "#000", ref: "n1" };

  it("commits a move on drag release, snapped", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(20, 5); // inside the body
    // Drag right ~240 ticks (24px) and down one lane (10px).
    expect(fsm.onPointerMove(44, 15, { alt: false })).toBe(true);
    expect(fsm.ghost()).not.toBeNull();
    fsm.onPointerUp();
    expect(calls).toEqual(["move n1 240 1"]);
    expect(fsm.ghost()).toBeNull();
  });

  it("resizes from the right edge, start unchanged", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(96, 5); // right edge (96px = end)
    fsm.onPointerMove(140, 5, { alt: false }); // drag to ~1400 ticks → snaps
    fsm.onPointerUp();
    expect(calls.length).toBe(1);
    expect(calls[0]).toMatch(/^resize n1 0 \d+$/); // start stays 0
  });

  it("resizes from the left edge, keeping the end fixed", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(2, 5); // left edge (~0px)
    fsm.onPointerMove(44, 5, { alt: false }); // drag the start right to ~480 ticks
    fsm.onPointerUp();
    // End was 960 (tick 0 + one beat); start snaps to 480, so length = 480.
    expect(calls).toEqual(["resize n1 480 480"]);
  });

  it("does not resize from the left edge when the adapter forbids it (timeline)", () => {
    const { adapter, calls } = recordingAdapter({ resizableLeft: false, laneMovable: false });
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(2, 5); // near the left edge, but left resize is off
    fsm.onPointerMove(26, 5, { alt: false }); // +24px ≈ +240 ticks from origin
    fsm.onPointerUp();
    expect(calls).toEqual(["move n1 240 0"]); // treated as a move, not a resize
  });

  it("keeps a thin right-edge handle on short notes", () => {
    // 100 ticks at 0.1 px/tick = 10px on screen: above the 6px move-only
    // floor, so the right edge still resizes through its (thin, 3px) zone
    // while the centre stays a move grab.
    const short: Block = { startTick: 0, lenTick: 100, lane: 0, color: "#000", ref: "n2" };
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([short], adapter);
    expect(fsm.cursorAt(9, 5)).toBe("ew-resize"); // 1px inside the right edge
    expect(fsm.cursorAt(5, 5)).toBe("move"); // the centre stays a move grab
    fsm.onPointerDown(9, 5);
    fsm.onPointerMove(44, 5, { alt: false }); // drag the end to ~440 ticks
    fsm.onPointerUp();
    expect(calls.length).toBe(1);
    expect(calls[0]).toMatch(/^resize n2 0 \d+$/);
  });

  it("does not commit if the pointer never crossed the drag threshold", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(20, 5);
    fsm.onPointerMove(21, 5, { alt: false }); // < 3px
    fsm.onPointerUp();
    expect(calls).toEqual([]);
    expect(fsm.selected?.ref).toBe("n1"); // but it did select
  });

  it("cancels an in-progress drag on Escape", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(20, 5);
    fsm.onPointerMove(60, 5, { alt: false });
    expect(fsm.cancel()).toBe(true);
    fsm.onPointerUp();
    expect(calls).toEqual([]);
    expect(fsm.ghost()).toBeNull();
  });

  it("creates on double-click, floored to the start of the clicked subdivision", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([block], adapter);
    // x=260 → tick 2600. Step is a 32nd (120). Floor → 2520, not the nearest 2640.
    fsm.onDoubleClick(260, 35); // lane 3
    expect(calls).toEqual(["create 2520 3"]);
  });

  it("does not create when double-clicking on an existing block", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([block], adapter); // block at lane 0, ticks 0..960
    fsm.onDoubleClick(20, 5); // over the note → no create (avoids a spurious echo)
    expect(calls).toEqual([]);
  });

  it("deletes the selected block", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(20, 5);
    fsm.onPointerUp();
    expect(fsm.deleteSelected()).toBe(true);
    expect(calls).toEqual(["remove n1"]);
  });

  it("keeps the lane fixed when the adapter is not lane-movable (timeline)", () => {
    const { adapter, calls } = recordingAdapter({ laneMovable: false });
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(20, 5);
    fsm.onPointerMove(44, 55, { alt: false }); // big vertical move
    fsm.onPointerUp();
    expect(calls).toEqual(["move n1 240 0"]); // lane stays 0
  });

  it("echoes a note's pitch once when it is selected (grab confirmation)", () => {
    const previews: number[] = [];
    const { adapter } = recordingAdapter({ previewLane: (lane) => previews.push(lane) });
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(20, 5); // press on the note (lane 0) — selects it
    fsm.onPointerUp(); // no drag
    expect(previews).toEqual([0]); // its pitch sounded once
  });

  it("auditions on grab, then once per new lane crossed during a vertical drag", () => {
    const previews: number[] = [];
    const { adapter } = recordingAdapter({ previewLane: (lane) => previews.push(lane) });
    const fsm = makeFsm([block], adapter);
    fsm.onPointerDown(20, 5); // grab on lane 0 → echo lane 0
    fsm.onPointerMove(44, 8, { alt: false }); // still lane 0 → no echo
    fsm.onPointerMove(44, 18, { alt: false }); // down one lane → echo lane 1
    fsm.onPointerMove(46, 19, { alt: false }); // same lane 1 → no echo
    fsm.onPointerMove(46, 28, { alt: false }); // down again → echo lane 2
    fsm.onPointerUp();
    expect(previews).toEqual([0, 1, 2]); // grab + one per crossed row, never per pixel
  });

  // One subdivision (a 16th, 240 ticks). At 0.1 px/tick it is 24px: edge zones
  // are bounded so a draggable core always remains at the centre.
  const sub: Block = { startTick: 0, lenTick: 240, lane: 0, color: "#000", ref: "s" };

  it("keeps a one-subdivision note draggable by its centre", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([sub], adapter);
    fsm.onPointerDown(12, 5); // centre of the 24px note
    fsm.onPointerMove(40, 5, { alt: false });
    fsm.onPointerUp();
    expect(calls[0]).toMatch(/^move s /); // centre press is a move, not a resize
  });

  it("still resizes a one-subdivision note from its edge", () => {
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([sub], adapter);
    fsm.onPointerDown(24, 5); // right edge of the 24px note
    fsm.onPointerMove(60, 5, { alt: false });
    fsm.onPointerUp();
    expect(calls[0]).toMatch(/^resize s /);
  });

  it("disables resize on a note too narrow on screen (move only)", () => {
    // 240 ticks at 0.02 px/tick = 4.8px: below the resize floor, so any press
    // moves the whole note even right at its edge.
    const lowZoom: Viewport = { scrollTicks: 0, pxPerTick: 0.02, scrollLane: 0, laneHeight: 10 };
    const { adapter, calls } = recordingAdapter();
    const fsm = makeFsm([sub], adapter, lowZoom);
    fsm.onPointerDown(4, 5); // at the (tiny) right edge
    fsm.onPointerMove(40, 5, { alt: false });
    fsm.onPointerUp();
    expect(calls[0]).toMatch(/^move s /);
  });
});
