// Pure sizing logic of the SplitPane, DOM-free so it is unit-tested.

export type SplitSide = "a" | "b";

/**
 * New size of the controlled pane after a handle drag. `delta` is the pointer
 * travel along the split axis (down/right positive). Growing pane "a" means
 * dragging toward "b" (positive delta); growing "b" means the opposite.
 */
export function dragSize(
  startSize: number,
  delta: number,
  sized: SplitSide,
  min: number,
  max: number,
): number {
  const next = sized === "a" ? startSize + delta : startSize - delta;
  return Math.min(max, Math.max(min, next));
}
