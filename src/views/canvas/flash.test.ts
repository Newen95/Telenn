import { describe, expect, it } from "vitest";

import { flashIntensity, FLASH_MS } from "./flash";

describe("flash intensity", () => {
  it("is full at the moment the flash begins", () => {
    expect(flashIntensity(1000, 1000)).toBe(1);
  });

  it("decays linearly over the window", () => {
    expect(flashIntensity(0, FLASH_MS / 2)).toBeCloseTo(0.5, 6);
  });

  it("is zero once the window has elapsed", () => {
    expect(flashIntensity(0, FLASH_MS)).toBe(0);
    expect(flashIntensity(0, FLASH_MS + 500)).toBe(0);
  });

  it("is zero for a note that never flashed", () => {
    expect(flashIntensity(undefined, 1234)).toBe(0);
  });
});
