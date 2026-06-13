//! Canvas lifecycle for a grid view: sizes the backing store in physical pixels
//! (devicePixelRatio, no blur), observes resizes, and runs a requestAnimationFrame
//! loop that redraws only when dirty or while active (the playhead is moving).

import { onMounted, onUnmounted, type Ref } from "vue";

export interface GridCanvas {
  /** Force a redraw on the next frame (viewport or data changed). */
  markDirty(): void;
}

export function useGridCanvas(
  canvas: Ref<HTMLCanvasElement | null>,
  /** Draw one frame. `w`/`h` are CSS pixels; `ctx` is pre-scaled by the dpr. */
  drawFrame: (ctx: CanvasRenderingContext2D, w: number, h: number) => void,
  /** Redraw every frame while this is true (e.g. the transport is playing). */
  isActive: () => boolean,
): GridCanvas {
  let dirty = true;
  let raf = 0;
  let observer: ResizeObserver | null = null;

  function resize(): void {
    const el = canvas.value;
    if (!el) return;
    const dpr = window.devicePixelRatio || 1;
    const rect = el.getBoundingClientRect();
    el.width = Math.max(1, Math.round(rect.width * dpr));
    el.height = Math.max(1, Math.round(rect.height * dpr));
    dirty = true;
  }

  function loop(): void {
    const el = canvas.value;
    if (el && (dirty || isActive())) {
      const ctx = el.getContext("2d");
      if (ctx) {
        const dpr = window.devicePixelRatio || 1;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        drawFrame(ctx, el.width / dpr, el.height / dpr);
        dirty = false;
      }
    }
    raf = requestAnimationFrame(loop);
  }

  onMounted(() => {
    resize();
    if (canvas.value) {
      observer = new ResizeObserver(resize);
      observer.observe(canvas.value);
    }
    raf = requestAnimationFrame(loop);
  });

  onUnmounted(() => {
    cancelAnimationFrame(raf);
    observer?.disconnect();
  });

  return { markDirty: () => { dirty = true; } };
}
