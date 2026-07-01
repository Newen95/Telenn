import { describe, expect, it } from "vitest";

import type { ChainNodeView } from "../../stores/project";
import { formatDb, formatPan, tailIndex } from "./format";

const node = (id: string, flatIndex: number): ChainNodeView => ({
  id,
  module: "gain",
  isInstrument: false,
  flatIndex,
  params: {},
});

describe("mixer formatting", () => {
  it("formats gain in dB with an explicit sign above zero", () => {
    expect(formatDb(0)).toBe("0.0 dB");
    expect(formatDb(-6.02)).toBe("-6.0 dB");
    expect(formatDb(3.5)).toBe("+3.5 dB");
  });

  it("formats pan as L/C/R percentages", () => {
    expect(formatPan(0)).toBe("C");
    expect(formatPan(-1)).toBe("L100");
    expect(formatPan(0.5)).toBe("R50");
    expect(formatPan(-0.004)).toBe("C"); // rounds to centre
  });
});

describe("tailIndex", () => {
  it("returns the chain's last node flat index (the VU source)", () => {
    expect(tailIndex([node("a", 0), node("b", 4), node("c", 7)])).toBe(7);
    expect(tailIndex([node("solo", 2)])).toBe(2);
  });

  it("falls back to 0 on an empty chain", () => {
    expect(tailIndex([])).toBe(0);
  });
});
