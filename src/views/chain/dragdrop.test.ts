import { describe, expect, it } from "vitest";

import { resolveDrop, type DragSource, type DropTarget } from "./dragdrop";

// The drop decision is the testable core: a module/preset becomes an insert, an
// effect node a move (or a copy with Alt), and dropping a node onto its own place
// is a no-op (keeps the GraphDesc unchanged — the cancel acceptance criterion).
// Slots are typed: instruments only land on chain heads or the new-track zone.

const gap = (trackId: string, index: number): DropTarget => ({ kind: "gap", trackId, index });

describe("resolveDrop", () => {
  it("inserts a dropped effect module with empty params", () => {
    const source: DragSource = { kind: "module", module: "gain", label: "Gain", category: "effect" };
    expect(resolveDrop(source, gap("bass", 1), false)).toEqual({
      kind: "insert",
      trackId: "bass",
      index: 1,
      module: "gain",
      params: {},
    });
  });

  it("inserts a dropped preset with its params", () => {
    const source: DragSource = { kind: "preset", name: "warm", module: "gain", params: { gain: 0.3 } };
    expect(resolveDrop(source, gap("lead", 2), false)).toEqual({
      kind: "insert",
      trackId: "lead",
      index: 2,
      module: "gain",
      params: { gain: 0.3 },
    });
  });

  it("moves an effect node to another chain", () => {
    const source: DragSource = { kind: "node", nodeId: "verb", module: "gain", fromTrack: "lead", fromIndex: 1 };
    expect(resolveDrop(source, gap("bass", 1), false)).toEqual({
      kind: "move",
      nodeId: "verb",
      toTrack: "bass",
      index: 1,
    });
  });

  it("copies instead of moving when Alt is held", () => {
    const source: DragSource = { kind: "node", nodeId: "verb", module: "gain", fromTrack: "lead", fromIndex: 1 };
    expect(resolveDrop(source, gap("bass", 1), true)).toEqual({
      kind: "copy",
      nodeId: "verb",
      toTrack: "bass",
      index: 1,
    });
  });

  it("is a no-op when a node is dropped onto its own position", () => {
    const source: DragSource = { kind: "node", nodeId: "verb", module: "gain", fromTrack: "lead", fromIndex: 2 };
    // The gaps just before (2) and just after (3) itself, same track, change nothing.
    expect(resolveDrop(source, gap("lead", 2), false)).toBeNull();
    expect(resolveDrop(source, gap("lead", 3), false)).toBeNull();
    // But the same gap with Alt is a real copy.
    expect(resolveDrop(source, gap("lead", 2), true)?.kind).toBe("copy");
  });

  it("replaces a track's instrument when one is dropped on a chain head", () => {
    const source: DragSource = { kind: "module", module: "fm", label: "FM", category: "instrument" };
    expect(resolveDrop(source, { kind: "instrument", trackId: "lead" }, false)).toEqual({
      kind: "setInstrument",
      trackId: "lead",
      module: "fm",
    });
  });

  it("creates a track when an instrument lands on the new-track zone", () => {
    const source: DragSource = { kind: "module", module: "fm", label: "FM", category: "instrument" };
    expect(resolveDrop(source, { kind: "newTrack" }, false)).toEqual({
      kind: "addTrack",
      module: "fm",
      name: "FM",
    });
  });

  it("never lets an instrument land in the effects region", () => {
    const source: DragSource = { kind: "module", module: "fm", label: "FM", category: "instrument" };
    expect(resolveDrop(source, gap("lead", 1), false)).toBeNull();
  });

  it("only instruments land on instrument slots and the new-track zone", () => {
    const effect: DragSource = { kind: "module", module: "gain", label: "Gain", category: "effect" };
    const node: DragSource = { kind: "node", nodeId: "verb", module: "gain", fromTrack: "lead", fromIndex: 1 };
    const preset: DragSource = { kind: "preset", name: "warm", module: "gain", params: {} };
    for (const source of [effect, node, preset]) {
      expect(resolveDrop(source, { kind: "instrument", trackId: "lead" }, false)).toBeNull();
      expect(resolveDrop(source, { kind: "newTrack" }, false)).toBeNull();
    }
  });
});
