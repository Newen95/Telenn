import { describe, expect, it } from "vitest";

import { ballistic, RELEASE_S } from "./meterBallistics";

describe("meter ballistics", () => {
  it("attacks instantly to a louder target", () => {
    expect(ballistic(0.2, 0.9, 0.016)).toBe(0.9);
  });

  it("releases linearly toward zero", () => {
    // Half the release window elapsed: drop by half of the full scale.
    const dt = RELEASE_S / 2;
    expect(ballistic(1, 0, dt)).toBeCloseTo(0.5, 6);
  });

  it("never falls below zero", () => {
    expect(ballistic(0.1, 0, RELEASE_S)).toBe(0);
  });

  it("holds when the target equals the level", () => {
    expect(ballistic(0.5, 0.5, 0.016)).toBe(0.5);
  });
});
