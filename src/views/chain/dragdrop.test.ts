import { describe, expect, it } from "vitest";

import { resolveDrop, type DragSource } from "./dragdrop";

// The drop decision is the testable core: a module/preset becomes an insert, an
// effect node a move (or a copy with Alt), and dropping a node onto its own place
// is a no-op (keeps the GraphDesc unchanged — the cancel acceptance criterion).

describe("resolveDrop", () => {
  it("inserts a dropped module with empty params", () => {
    const source: DragSource = { kind: "module", module: "gain", label: "Gain" };
    expect(resolveDrop(source, { trackId: "bass", index: 1 }, false)).toEqual({
      kind: "insert",
      trackId: "bass",
      index: 1,
      module: "gain",
      params: {},
    });
  });

  it("inserts a dropped preset with its params", () => {
    const source: DragSource = { kind: "preset", name: "warm", module: "gain", params: { gain: 0.3 } };
    expect(resolveDrop(source, { trackId: "lead", index: 2 }, false)).toEqual({
      kind: "insert",
      trackId: "lead",
      index: 2,
      module: "gain",
      params: { gain: 0.3 },
    });
  });

  it("moves an effect node to another chain", () => {
    const source: DragSource = { kind: "node", nodeId: "verb", module: "gain", fromTrack: "lead", fromIndex: 1 };
    expect(resolveDrop(source, { trackId: "bass", index: 1 }, false)).toEqual({
      kind: "move",
      nodeId: "verb",
      toTrack: "bass",
      index: 1,
    });
  });

  it("copies instead of moving when Alt is held", () => {
    const source: DragSource = { kind: "node", nodeId: "verb", module: "gain", fromTrack: "lead", fromIndex: 1 };
    expect(resolveDrop(source, { trackId: "bass", index: 1 }, true)).toEqual({
      kind: "copy",
      nodeId: "verb",
      toTrack: "bass",
      index: 1,
    });
  });

  it("is a no-op when a node is dropped onto its own position", () => {
    const source: DragSource = { kind: "node", nodeId: "verb", module: "gain", fromTrack: "lead", fromIndex: 2 };
    // The gaps just before (2) and just after (3) itself, same track, change nothing.
    expect(resolveDrop(source, { trackId: "lead", index: 2 }, false)).toBeNull();
    expect(resolveDrop(source, { trackId: "lead", index: 3 }, false)).toBeNull();
    // But the same gap with Alt is a real copy.
    expect(resolveDrop(source, { trackId: "lead", index: 2 }, true)?.kind).toBe("copy");
  });
});
