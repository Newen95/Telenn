//! Flash decay for notes changed by an external file reload: a note lights up on
//! reload and fades to nothing over a fixed window. Pure so it is unit-tested
//! without a render loop or a store.

/** How long a changed note stays highlighted after a reload. */
export const FLASH_MS = 1500;

/**
 * Flash intensity (1 → 0) for a note whose flash began at `start`, at time
 * `now`. Returns 0 once the window has elapsed (or when `start` is undefined),
 * so an expired or never-flashed note draws normally.
 */
export function flashIntensity(start: number | undefined, now: number, durationMs = FLASH_MS): number {
  if (start === undefined) return 0;
  const a = 1 - (now - start) / durationMs;
  return a > 0 ? a : 0;
}
