// Shared VU ballistics for every meter (master and per-node): instant attack,
// linear release. Pure so it is unit-tested without a render loop.

/** Default release time: the displayed level falls to zero in 300 ms. */
export const RELEASE_S = 0.3;

/**
 * Next displayed level given the current level, the incoming target and the
 * elapsed time `dt` (seconds). The attack is instant (jump up to a louder or
 * equal target, so a steady level holds); the release is linear over `releaseS`.
 */
export function ballistic(level: number, target: number, dt: number, releaseS = RELEASE_S): number {
  if (target >= level) return target; // instant attack (and hold at steady state)
  return Math.max(0, level - dt / releaseS); // linear release
}
