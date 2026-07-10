<script setup lang="ts">
// Shared grid view used by both the timeline and the piano roll. Only the data
// it is handed differs (clips-per-track vs notes-per-pitch); the rendering,
// viewport, wheel navigation and interpolated playhead are common here. The 5b
// gesture machine will attach to this same component (hitTest already exists).

import { onMounted, ref, watch } from "vue";

import { useProjectStore } from "../../stores/project";
import { useTelemetryStore } from "../../stores/telemetry";
import { draw, type Block, type Grid, type GutterStyle } from "./renderer";
import { adaptiveSnap, GridInteractions, snapStepTicks, type EditAdapter } from "./interactions";
import { useGridCanvas } from "./useGridCanvas";
import { applyAxis, type TimeAxis } from "./timeAxis";
import { samplesToTicks, yToLane, type Viewport } from "./viewport";
import {
  DEFAULT_PX_PER_TICK,
  fitView,
  keyZoomFactor,
  panBy,
  readWheel,
  zoomAt,
} from "./navigation";

/** A gutter plus its press behavior: the piano roll's keyboard holds a preview
 *  note (sliding rows = glissando); the timeline's track labels select, and a
 *  vertical drag past half a row becomes a reorder. */
export type GutterSpec = GutterStyle & {
  /** Press (and hold) the lane — a new lane while held presses again. */
  press(lane: number): void;
  /** Release the held lane (pointer up / leaving the view). */
  release(): void;
  /** Whether any preview voice is sounding (keeps the highlight animating). */
  sounding?(): boolean;
  /** Label gutters only: commit moving the lane `from` to position `to`. */
  reorder?(from: number, to: number): void;
};

const props = defineProps<{
  blocks: Block[];
  laneCount: number;
  laneHeight: number;
  grid: Grid;
  initialScrollLane?: number;
  initialPxPerTick?: number;
  /** When set, the view is editable: gestures route through this adapter. */
  adapter?: EditAdapter;
  /** When set, a left keyboard gutter is drawn and audible (piano roll). */
  gutter?: GutterSpec;
  /** Shared horizontal axis: when set, scroll/zoom/origin are common to all
   *  views using it, so the playhead lands at the same X everywhere. */
  axis?: TimeAxis;
  /** Called with the lane under any grid press (the timeline selects tracks). */
  onLanePress?: (lane: number) => void;
  /** Lane lifted with the active-track tint (timeline only). */
  activeLane?: number;
  /** View-level Delete handler, tried before block deletion (the timeline
   *  deletes the active track). Return true when the key was consumed. */
  onDeleteKey?: () => boolean;
}>();

const project = useProjectStore();
const telemetry = useTelemetryStore();
const canvas = ref<HTMLCanvasElement | null>(null);

// Plain (non-reactive) viewport: the renderer reads it directly and we redraw
// explicitly via markDirty, avoiding deep Pinia reactivity on the hot path.
const vp: Viewport = {
  scrollTicks: props.axis?.scrollTicks ?? 0,
  pxPerTick: props.axis?.pxPerTick ?? props.initialPxPerTick ?? DEFAULT_PX_PER_TICK,
  scrollLane: props.initialScrollLane ?? 0,
  laneHeight: props.laneHeight,
  originX: props.axis?.originX ?? props.gutter?.width ?? 0,
};

/** Pull the shared horizontal axis into the local viewport (no-op if unshared).
 *  Called before every read so pointer maths and drawing see the live axis. */
function syncAxis(): void {
  if (props.axis) applyAxis(vp, props.axis);
}

/** Push the local viewport's horizontal scroll/zoom back to the shared axis. */
function commitAxis(): void {
  if (!props.axis) return;
  props.axis.scrollTicks = vp.scrollTicks;
  props.axis.pxPerTick = vp.pxPerTick;
}

// --- Navigation state (mouse + trackpad + keyboard aliases) ---
// A pan drag in progress (middle button, or the Space "hand tool").
let panning = false;
let panLastX = 0;
let panLastY = 0;
// Space held → hand tool: a left-drag pans instead of editing (the trackpad's
// pan, since it has no middle button).
let spaceHeld = false;
// Last cursor X, so keyboard zoom (+/-) stays centred where the user is looking.
let lastPointerX = 0;
// A held gutter key (the keyboard preview): sounds while pressed, glissandos as
// the pointer slides to other rows, releases on pointer up.
let gutterPressing = false;
let gutterLane = NaN;
// Label-gutter reorder gesture: armed by the press, becomes a drag past half a
// row of vertical travel (below that, the press stays a plain click/select).
let gutterPressY = 0;
let gutterReorder: { from: number; to: number } | null = null;
// A press on the band region of a reorderable view (the timeline): watched
// until an axis wins — horizontal = the clip gesture proceeds, vertical =
// the whole track reorders (the same drag, another axis).
let bandWatch: { x: number; y: number; lane: number } | null = null;

const clampLane = (lane: number): number => Math.max(0, Math.min(props.laneCount - 1, lane));

const reorderable = (): boolean => props.gutter?.kind === "labels" && !!props.gutter.reorder;

/** Topmost lane the view may scroll to, so the last lane can't pass the top. */
function maxScrollLane(): number {
  const h = canvas.value?.clientHeight ?? 0;
  return Math.max(0, props.laneCount - h / vp.laneHeight);
}

// Editing gesture machine (shared with the timeline). Built only when editable;
// it reads the live viewport/data so it always sees the current zoom and blocks.
const fsm = props.adapter
  ? new GridInteractions({
      viewport: () => vp,
      blocks: () => props.blocks,
      laneCount: () => props.laneCount,
      snap: (tick, free) => adaptiveSnap(tick, props.grid.ticksPerBeat, vp.pxPerTick, free),
      snapStep: () => snapStepTicks(props.grid.ticksPerBeat, vp.pxPerTick),
      adapter: props.adapter,
    })
  : null;

function frame(ctx: CanvasRenderingContext2D, w: number, h: number): void {
  syncAxis();
  const samples = telemetry.currentSamples(performance.now());
  const tick = samplesToTicks(samples, project.bpm, telemetry.sampleRate, project.ppq);
  const overlay = fsm
    ? {
        ghost: fsm.ghost(),
        selected: props.adapter?.showSelection === false ? null : fsm.selected,
      }
    : undefined;
  const now = performance.now();
  const flash = (ref: unknown): number =>
    typeof ref === "string" ? project.noteFlash(ref, now) : 0;
  draw(
    ctx,
    w,
    h,
    vp,
    props.grid,
    props.blocks,
    props.laneCount,
    tick,
    overlay,
    props.gutter,
    flash,
    props.activeLane,
    gutterReorder,
  );
}

/** True if `x` falls in the keyboard gutter column. */
function inGutter(x: number): boolean {
  return !!props.gutter && x < (vp.originX ?? 0);
}

// Redraw every frame while the transport plays or a preview voice is flashing
// (so the gutter highlight fades on its own without explicit invalidation).
const { markDirty } = useGridCanvas(
  canvas,
  frame,
  () =>
    telemetry.playing ||
    (props.gutter?.sounding?.() ?? false) ||
    project.anyFlash(performance.now()),
);

function onPointerDown(e: PointerEvent): void {
  syncAxis();
  canvas.value?.focus();
  // Pan: middle button, or the Space hand tool on the primary button. Both give
  // the trackpad (no middle button) a drag-to-pan, and never start an edit.
  if (e.button === 1 || (spaceHeld && e.button === 0)) {
    e.preventDefault();
    panning = true;
    panLastX = e.offsetX;
    panLastY = e.offsetY;
    canvas.value?.setPointerCapture(e.pointerId);
    if (canvas.value) canvas.value.style.cursor = "grabbing";
    return;
  }
  // A press on the keyboard gutter plays the lane's pitch and holds it — never an
  // edit. Capture the pointer so a slide onto other rows (glissando) and the
  // release are tracked even if the cursor leaves the gutter column.
  if (inGutter(e.offsetX)) {
    const lane = yToLane(vp, e.offsetY);
    if (lane >= 0 && lane < props.laneCount) {
      gutterPressing = true;
      gutterLane = lane;
      gutterPressY = e.offsetY;
      canvas.value?.setPointerCapture(e.pointerId);
      props.gutter?.press(lane);
      markDirty();
    }
    return;
  }
  // A press anywhere in the grid names its lane first (the timeline selects
  // that track), then the edit gesture proceeds as usual.
  const pressedLane = yToLane(vp, e.offsetY);
  if (pressedLane >= 0 && pressedLane < props.laneCount) {
    props.onLanePress?.(pressedLane);
    // Watch the press until an axis wins (vertical → track reorder).
    if (reorderable()) bandWatch = { x: e.offsetX, y: e.offsetY, lane: pressedLane };
  }
  if (!fsm) return;
  canvas.value?.setPointerCapture(e.pointerId);
  fsm.onPointerDown(e.offsetX, e.offsetY);
  markDirty();
}

function onPointerMove(e: PointerEvent): void {
  lastPointerX = e.offsetX;
  syncAxis();
  // A pan drag: move the content with the cursor (drag right → reveal earlier
  // time / higher lanes), on both axes at once.
  if (panning) {
    panBy(vp, panLastX - e.offsetX, panLastY - e.offsetY, maxScrollLane());
    panLastX = e.offsetX;
    panLastY = e.offsetY;
    commitAxis();
    markDirty();
    return;
  }
  if (gutterPressing) {
    const lane = yToLane(vp, e.offsetY);
    // Label gutter: vertical travel past half a row turns the press into a
    // reorder drag (an insertion indicator tracks the target row).
    if (props.gutter?.kind === "labels" && props.gutter.reorder) {
      if (gutterReorder || Math.abs(e.offsetY - gutterPressY) > vp.laneHeight / 2) {
        gutterReorder = {
          from: gutterLane,
          to: Math.max(0, Math.min(props.laneCount - 1, lane)),
        };
        markDirty();
      }
      return;
    }
    // Keyboard gutter: glissando — switch to the row under the pointer.
    if (lane >= 0 && lane < props.laneCount && lane !== gutterLane) {
      gutterLane = lane;
      props.gutter?.press(lane);
      markDirty();
    }
    return;
  }
  if (canvas.value && spaceHeld) {
    canvas.value.style.cursor = "grab"; // hand tool armed, not yet dragging
    return;
  }
  // A watched band press: the first axis to clearly win decides the gesture.
  if (bandWatch) {
    const dx = Math.abs(e.offsetX - bandWatch.x);
    const dy = Math.abs(e.offsetY - bandWatch.y);
    if (dy > vp.laneHeight / 2 && dy > dx) {
      fsm?.cancel(); // abort the clip gesture: this drag reorders the track
      gutterReorder = { from: bandWatch.lane, to: clampLane(yToLane(vp, e.offsetY)) };
      bandWatch = null;
      markDirty();
    } else if (dx > 8) {
      bandWatch = null; // horizontal: the clip gesture owns the drag
    }
  }
  // A band-originated reorder in progress: track the target row.
  if (gutterReorder && !gutterPressing) {
    gutterReorder = { from: gutterReorder.from, to: clampLane(yToLane(vp, e.offsetY)) };
    markDirty();
    return;
  }
  if (!fsm) return;
  if (inGutter(e.offsetX)) {
    if (canvas.value) canvas.value.style.cursor = "pointer";
    return;
  }
  if (fsm.onPointerMove(e.offsetX, e.offsetY, { alt: e.altKey })) {
    markDirty();
  } else if (canvas.value) {
    canvas.value.style.cursor = fsm.cursorAt(e.offsetX, e.offsetY);
  }
}

function onPointerUp(e: PointerEvent): void {
  if (gutterPressing) {
    gutterPressing = false;
    gutterLane = NaN;
    canvas.value?.releasePointerCapture(e.pointerId);
    if (gutterReorder) {
      const { from, to } = gutterReorder;
      gutterReorder = null;
      if (from !== to) props.gutter?.reorder?.(from, to);
    }
    props.gutter?.release();
    markDirty();
    return;
  }
  if (panning) {
    panning = false;
    canvas.value?.releasePointerCapture(e.pointerId);
    if (canvas.value) canvas.value.style.cursor = spaceHeld ? "grab" : "default";
    return;
  }
  bandWatch = null;
  // A band-originated reorder commits here (the gutter one commits above).
  if (gutterReorder) {
    const { from, to } = gutterReorder;
    gutterReorder = null;
    canvas.value?.releasePointerCapture(e.pointerId);
    if (from !== to) props.gutter?.reorder?.(from, to);
    markDirty();
    return;
  }
  if (!fsm) return;
  canvas.value?.releasePointerCapture(e.pointerId);
  if (fsm.onPointerUp()) markDirty();
}

function onDoubleClick(e: MouseEvent): void {
  if (!fsm || inGutter(e.offsetX)) return;
  syncAxis();
  fsm.onDoubleClick(e.offsetX, e.offsetY);
  markDirty();
}

function onKeyDown(e: KeyboardEvent): void {
  // Navigation aliases — available in any grid view, edit adapter or not.
  // `stopPropagation` keeps these keys from also reaching the dev keyboard
  // listener on window (e.g. F is mapped there to a synth note).
  if (e.code === "Space") {
    if (!spaceHeld) {
      spaceHeld = true;
      if (canvas.value) canvas.value.style.cursor = "grab";
    }
    e.preventDefault(); // don't scroll the page or click a focused control
    e.stopPropagation();
    return;
  }
  if (e.key === "+" || e.key === "=") {
    e.preventDefault();
    e.stopPropagation();
    zoomKey(1);
    return;
  }
  if (e.key === "-" || e.key === "_") {
    e.preventDefault();
    e.stopPropagation();
    zoomKey(-1);
    return;
  }
  if (e.key === "f" || e.key === "F") {
    e.preventDefault();
    e.stopPropagation();
    fitToContent();
    return;
  }
  // Editing. The view gets Delete first (the timeline deletes the active
  // track — undoable); block deletion only runs if the view declined.
  if (e.key === "Delete" || e.key === "Backspace") {
    if (props.onDeleteKey?.()) {
      e.preventDefault();
      markDirty();
      return;
    }
    if (fsm?.deleteSelected()) {
      e.preventDefault();
      markDirty();
    }
    return;
  }
  if (!fsm) return;
  if (e.key === "Escape" && fsm.cancel()) {
    markDirty();
  }
}

function onKeyUp(e: KeyboardEvent): void {
  if (e.code === "Space") {
    spaceHeld = false;
    if (canvas.value && !panning) canvas.value.style.cursor = "default";
  }
}

/** Zoom by one keyboard step, centred where the cursor last was. */
function zoomKey(dir: number): void {
  syncAxis();
  zoomAt(vp, lastPointerX, keyZoomFactor(dir));
  commitAxis();
  markDirty();
}

/** Frame all of this view's content (the "tout cadrer" / F command). */
function fitToContent(): void {
  syncAxis();
  const el = canvas.value;
  fitView(vp, props.blocks, el?.clientWidth ?? 0, el?.clientHeight ?? 0, props.laneCount);
  commitAxis();
  markDirty();
}

function onWheel(e: WheelEvent): void {
  // Always prevent the browser default: a plain wheel would scroll the page and,
  // crucially, a trackpad pinch (a wheel event with ctrlKey) would zoom the whole
  // UI instead of the grid.
  e.preventDefault();
  syncAxis();
  const intent = readWheel(e);
  if (intent.kind === "zoom") {
    zoomAt(vp, e.offsetX, intent.factor); // centred on the cursor / pinch point
  } else {
    panBy(vp, intent.dxPx, intent.dyPx, maxScrollLane());
  }
  commitAxis();
  markDirty();
}

// Redraw on data changes and on transport start/stop transitions. After an edit
// reloads the blocks, re-bind the selection to the fresh block with the same ref
// (so its outline follows the edit), or drop it if the block is gone.
watch(
  () => props.blocks,
  (blocks) => {
    if (fsm && fsm.selected) {
      const ref = fsm.selected.ref;
      fsm.selected = blocks.find((b) => b.ref === ref) ?? null;
    }
    markDirty();
  },
  { deep: false },
);
watch(() => telemetry.playing, markDirty);

// Redraw when the gutter content changes identity (e.g. the active-track
// highlight moved) or when the active lane itself changes.
watch(() => props.gutter, markDirty);
watch(() => props.activeLane, markDirty);

// Redraw when another view changes the shared horizontal axis (so a scroll or
// zoom in one view keeps both in step even while the transport is stopped).
watch(
  () => props.axis && [props.axis.scrollTicks, props.axis.pxPerTick, props.axis.originX],
  markDirty,
  { deep: true },
);

// Centre vertically on the track's notes once, at mount. A track switch remounts
// this component (keyed by track in App.vue), so this re-runs with the new track's
// range; in-place note edits deliberately leave the scroll where the user put it.
onMounted(() => {
  vp.scrollLane = props.initialScrollLane ?? 0;
  markDirty();
});
</script>

<template>
  <canvas
    ref="canvas"
    class="grid-canvas"
    :class="{ editable: !!adapter }"
    tabindex="0"
    @wheel="onWheel"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @dblclick="onDoubleClick"
    @keydown="onKeyDown"
    @keyup="onKeyUp"
  ></canvas>
</template>

<style scoped>
.grid-canvas {
  width: 100%;
  height: 100%;
  display: block;
  touch-action: none;
}

/* Editable views are focusable for keyboard delete/undo; selection feedback is
   drawn on the canvas, so the default focus ring is redundant. */
.grid-canvas.editable:focus {
  outline: none;
}
</style>
