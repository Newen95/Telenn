//! Pointer-based drag & drop for the effect chain (M6).
//!
//! Native HTML5 drag & drop is unusable here (ghost image, inconsistent events),
//! so we drive it with pointer events and a small shared state. The *decision* —
//! what a drop means — is the pure `resolveDrop`, kept free of the DOM so it is
//! unit-tested; the Vue components own the pointer plumbing and the ghost.

import { reactive } from "vue";

/** What is being dragged: an existing effect, or a browser item (module/preset). */
export type DragSource =
  | { kind: "node"; nodeId: string; module: string; fromTrack: string; fromIndex: number }
  | { kind: "module"; module: string; label: string }
  | { kind: "preset"; name: string; module: string; params: Record<string, number> };

/** A gap in a chain: insert before chain `index` (always in the effects region). */
export interface DropTarget {
  trackId: string;
  index: number;
}

/** The edit a drop resolves to (executed by the view through the store). */
export type DropIntent =
  | { kind: "move"; nodeId: string; toTrack: string; index: number }
  | { kind: "copy"; nodeId: string; toTrack: string; index: number }
  | { kind: "insert"; trackId: string; index: number; module: string; params: Record<string, number> };

/**
 * What dropping `source` on `target` means, or `null` for a no-op. A node dropped
 * in the gap just before or after itself (same track) does not move — returning
 * `null` keeps the graph (and its `GraphDesc`) strictly unchanged.
 */
export function resolveDrop(
  source: DragSource,
  target: DropTarget,
  copy: boolean,
): DropIntent | null {
  switch (source.kind) {
    case "module":
      return { kind: "insert", trackId: target.trackId, index: target.index, module: source.module, params: {} };
    case "preset":
      return {
        kind: "insert",
        trackId: target.trackId,
        index: target.index,
        module: source.module,
        params: source.params,
      };
    case "node": {
      const sameTrack = target.trackId === source.fromTrack;
      const samePlace = sameTrack && (target.index === source.fromIndex || target.index === source.fromIndex + 1);
      if (!copy && samePlace) return null;
      return { kind: copy ? "copy" : "move", nodeId: source.nodeId, toTrack: target.trackId, index: target.index };
    }
  }
}

interface DragState {
  source: DragSource | null;
  hovered: DropTarget | null;
  copy: boolean;
  /** Pointer position, for the floating ghost. */
  x: number;
  y: number;
}

// One shared drag across the chain panel and the browser (singleton).
const state = reactive<DragState>({ source: null, hovered: null, copy: false, x: 0, y: 0 });

export function useChainDrag() {
  return {
    state,
    begin(source: DragSource, x: number, y: number): void {
      state.source = source;
      state.hovered = null;
      state.x = x;
      state.y = y;
    },
    move(x: number, y: number): void {
      state.x = x;
      state.y = y;
    },
    hover(target: DropTarget | null): void {
      state.hovered = target;
    },
    setCopy(copy: boolean): void {
      state.copy = copy;
    },
    cancel(): void {
      state.source = null;
      state.hovered = null;
    },
    /** Resolve the drop on the hovered target and clear the drag. */
    drop(): DropIntent | null {
      const { source, hovered, copy } = state;
      state.source = null;
      state.hovered = null;
      if (!source || !hovered) return null;
      return resolveDrop(source, hovered, copy);
    },
  };
}
