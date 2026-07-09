<script setup lang="ts">
// Two panes separated by a draggable handle. One side (`sized`) has an
// explicit size (v-model, px); the other flexes. Pointer-events grammar like
// the rest of the app; double-click restores the default size. The handle is
// a thin line that lights up copper on hover/drag — the "string" motif.

import { computed, ref } from "vue";

import { dragSize, type SplitSide } from "./split";

const props = withDefaults(
  defineProps<{
    /** Split axis: "horizontal" stacks a over b; "vertical" puts a beside b. */
    orientation?: "horizontal" | "vertical";
    /** Which pane the size applies to. */
    sized?: SplitSide;
    min?: number;
    max?: number;
    defaultSize?: number;
    /** A collapsed zone keeps its rail visible but its handle inert. */
    locked?: boolean;
  }>(),
  { orientation: "horizontal", sized: "a", min: 64, max: 1200, defaultSize: 200, locked: false },
);

const size = defineModel<number>({ required: true });

const dragging = ref(false);
let start = { pos: 0, size: 0 };

const horizontal = computed(() => props.orientation === "horizontal");

function axisPos(e: PointerEvent): number {
  return horizontal.value ? e.clientY : e.clientX;
}

function onHandleDown(e: PointerEvent): void {
  if (props.locked) return;
  e.preventDefault();
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  dragging.value = true;
  start = { pos: axisPos(e), size: size.value };
}

function onHandleMove(e: PointerEvent): void {
  if (!dragging.value) return;
  size.value = dragSize(start.size, axisPos(e) - start.pos, props.sized, props.min, props.max);
}

function onHandleUp(): void {
  dragging.value = false;
}

function onHandleDblClick(): void {
  if (props.locked) return;
  size.value = props.defaultSize;
}

const paneStyle = computed(() =>
  horizontal.value ? { height: `${size.value}px` } : { width: `${size.value}px` },
);
</script>

<template>
  <div class="split" :class="[orientation, { dragging }]">
    <div class="pane" :class="{ sized: sized === 'a' }" :style="sized === 'a' ? paneStyle : undefined">
      <slot name="a" />
    </div>
    <div
      class="handle"
      :class="{ locked }"
      role="separator"
      :aria-orientation="horizontal ? 'horizontal' : 'vertical'"
      @pointerdown="onHandleDown"
      @pointermove="onHandleMove"
      @pointerup="onHandleUp"
      @pointercancel="onHandleUp"
      @dblclick="onHandleDblClick"
    ></div>
    <div class="pane" :class="{ sized: sized === 'b' }" :style="sized === 'b' ? paneStyle : undefined">
      <slot name="b" />
    </div>
  </div>
</template>

<style scoped>
.split {
  display: flex;
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
}

.split.horizontal {
  flex-direction: column;
}

.split.vertical {
  flex-direction: row;
}

.pane {
  min-width: 0;
  min-height: 0;
  flex: 1;
  overflow: hidden;
}

.pane.sized {
  flex: none;
}

/* The handle: a hairline that widens its hit area with padding and lights up
   copper on hover/drag (the string). */
.handle {
  flex: none;
  position: relative;
  z-index: 5;
  background: var(--border);
  transition: background-color var(--t-fast);
}

.horizontal > .handle {
  height: 1px;
  cursor: row-resize;
}

.vertical > .handle {
  width: 1px;
  cursor: col-resize;
}

/* Forgiving hit zone without visual thickness. */
.handle::before {
  content: "";
  position: absolute;
  inset: -3px;
}

.handle:hover:not(.locked),
.dragging > .handle {
  background: var(--accent);
}

.handle.locked {
  cursor: default;
}

.dragging {
  user-select: none;
}
</style>
