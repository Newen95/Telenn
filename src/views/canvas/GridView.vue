<script setup lang="ts">
// Shared grid view used by both the timeline and the piano roll. Only the data
// it is handed differs (clips-per-track vs notes-per-pitch); the rendering,
// viewport, wheel navigation and interpolated playhead are common here. The 5b
// gesture machine will attach to this same component (hitTest already exists).

import { onMounted, ref, watch } from "vue";

import { useProjectStore } from "../../stores/project";
import { useTelemetryStore } from "../../stores/telemetry";
import { draw, type Block, type Grid } from "./renderer";
import { useGridCanvas } from "./useGridCanvas";
import { samplesToTicks, xToTick, type Viewport } from "./viewport";

const props = defineProps<{
  blocks: Block[];
  laneCount: number;
  laneHeight: number;
  grid: Grid;
  initialScrollLane?: number;
  initialPxPerTick?: number;
}>();

const project = useProjectStore();
const telemetry = useTelemetryStore();
const canvas = ref<HTMLCanvasElement | null>(null);

// Plain (non-reactive) viewport: the renderer reads it directly and we redraw
// explicitly via markDirty, avoiding deep Pinia reactivity on the hot path.
const vp: Viewport = {
  scrollTicks: 0,
  pxPerTick: props.initialPxPerTick ?? 0.03,
  scrollLane: props.initialScrollLane ?? 0,
  laneHeight: props.laneHeight,
};

const MIN_PX_PER_TICK = 0.004;
const MAX_PX_PER_TICK = 0.5;

function frame(ctx: CanvasRenderingContext2D, w: number, h: number): void {
  const samples = telemetry.currentSamples(performance.now());
  const tick = samplesToTicks(samples, project.bpm, telemetry.sampleRate, project.ppq);
  draw(ctx, w, h, vp, props.grid, props.blocks, props.laneCount, tick);
}

const { markDirty } = useGridCanvas(canvas, frame, () => telemetry.playing);

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

function onWheel(e: WheelEvent): void {
  e.preventDefault();
  if (e.ctrlKey) {
    // Zoom horizontally around the cursor.
    const cursorTick = xToTick(vp, e.offsetX);
    const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
    vp.pxPerTick = clamp(vp.pxPerTick * factor, MIN_PX_PER_TICK, MAX_PX_PER_TICK);
    vp.scrollTicks = Math.max(0, cursorTick - e.offsetX / vp.pxPerTick);
  } else if (e.shiftKey) {
    // Scroll lanes (vertical).
    const maxLane = Math.max(0, props.laneCount - 1);
    vp.scrollLane = clamp(vp.scrollLane + Math.sign(e.deltaY), 0, maxLane);
  } else {
    // Scroll time (horizontal).
    vp.scrollTicks = Math.max(0, vp.scrollTicks + e.deltaY / vp.pxPerTick);
  }
  markDirty();
}

// Redraw on data changes and on transport start/stop transitions.
watch(() => props.blocks, markDirty, { deep: false });
watch(() => telemetry.playing, markDirty);

// Re-center vertically when the data-derived initial lane changes (the project
// loads asynchronously, after this component has mounted).
watch(
  () => props.initialScrollLane,
  (lane) => {
    if (lane !== undefined) {
      vp.scrollLane = lane;
      markDirty();
    }
  },
);

onMounted(() => {
  vp.scrollLane = props.initialScrollLane ?? 0;
  markDirty();
});
</script>

<template>
  <canvas ref="canvas" class="grid-canvas" @wheel="onWheel"></canvas>
</template>

<style scoped>
.grid-canvas {
  width: 100%;
  height: 100%;
  display: block;
  touch-action: none;
}
</style>
