// Pure formatting/selection helpers of the mixer panel, extracted for tests.

import type { ChainNodeView } from "../../stores/project";

/** Gain readout: "-6.0 dB", "+3.5 dB", "0.0 dB". */
export const formatDb = (db: number): string => `${db > 0 ? "+" : ""}${db.toFixed(1)} dB`;

/** Pan readout in the usual DAW notation: L100 … C … R100. */
export const formatPan = (pan: number): string => {
  const pct = Math.round(Math.abs(pan) * 100);
  if (pct === 0) return "C";
  return pan < 0 ? `L${pct}` : `R${pct}`;
};

/** Flat index of a chain's tail (its last node) → the strip's telemetry meter. */
export const tailIndex = (chain: ChainNodeView[]): number =>
  chain[chain.length - 1]?.flatIndex ?? 0;
