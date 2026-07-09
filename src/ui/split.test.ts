import { describe, expect, it } from "vitest";

import { dragSize } from "./split";

describe("dragSize", () => {
  it("grows pane a with a positive delta and shrinks it back", () => {
    expect(dragSize(200, 50, "a", 64, 480)).toBe(250);
    expect(dragSize(200, -50, "a", 64, 480)).toBe(150);
  });

  it("grows pane b when the handle moves toward a", () => {
    // Pane b sits after the handle: dragging up/left (negative) enlarges it.
    expect(dragSize(200, -50, "b", 64, 480)).toBe(250);
    expect(dragSize(200, 50, "b", 64, 480)).toBe(150);
  });

  it("clamps to the pane bounds", () => {
    expect(dragSize(200, 10_000, "a", 64, 480)).toBe(480);
    expect(dragSize(200, -10_000, "a", 64, 480)).toBe(64);
  });
});
