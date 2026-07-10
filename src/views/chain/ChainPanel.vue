<script setup lang="ts">
// The ACTIVE track's chain, rendered in the DOM: the instrument head (click to
// replace it), then its effects. The gaps between/around effects are the drop
// targets, so an effect can be dragged within the chain (or to another track by
// selecting it first). The header navigates between tracks; the overview of all
// tracks is the mixer's job.

import { computed, reactive, ref, watchEffect } from "vue";
import { useI18n } from "vue-i18n";

import { useProjectStore, type ChainNodeView, type ModuleDesc, type ParamInfo } from "../../stores/project";
import { useUiStore } from "../../stores/ui";
import { projectHues } from "../../styles/theme";
import { useChainDrag, type DropTarget } from "./dragdrop";
import { ChainSelection } from "./selection";
import NodeMeter from "./NodeMeter.vue";
import Icon from "../../ui/Icon.vue";
import InstrumentMenu from "../../ui/InstrumentMenu.vue";

const { t } = useI18n();
const project = useProjectStore();
const ui = useUiStore();
const drag = useChainDrag();

// The displayed track: the active one, falling back to the first so the panel
// never goes blank when a selection points at a removed track.
const activeIndex = computed(() => {
  const i = project.tracks.findIndex((tr) => tr.id === ui.activeTrackId);
  return i >= 0 ? i : 0;
});
const track = computed(() => project.tracks[activeIndex.value]);

/** Collision-free display hue per track id (explicit colors first). */
const hues = computed(() => projectHues(project.tracks));

function step(dir: number): void {
  const n = project.tracks.length;
  if (n === 0) return;
  const next = (activeIndex.value + dir + n) % n;
  ui.activeTrackId = project.tracks[next].id;
}

// Click the instrument card → picker replacing the track's instrument.
const replacing = ref(false);

async function replaceInstrument(m: ModuleDesc): Promise<void> {
  replacing.value = false;
  if (track.value) await project.setTrackInstrument(track.value.id, m.id);
}

// True while an instrument module is being dragged from the browser: the drop
// targets are then the chain heads and the new-track zone, which light up.
const draggingInstrument = computed(() => {
  const s = drag.state.source;
  return s?.kind === "module" && s.category === "instrument";
});

// Node selection: a selected node carries the focus, so Delete/Ctrl+D act on it
// and only when the chain (not a text field or the piano roll) owns the focus.
const sel = reactive(new ChainSelection());

// Param specs (name, unit, default) per module id, fetched once and reused by
// every effect of that module. Ranges/curves stay declared in the module.
const paramSpecs = reactive<Record<string, ParamInfo[]>>({});

function ensureSpecs(module: string): void {
  if (!(module in paramSpecs)) {
    paramSpecs[module] = []; // mark in-flight so we fetch once
    void project.moduleParams(module).then((specs) => {
      paramSpecs[module] = specs;
    });
  }
}

// Fetch specs for every effect in a chain, plus whatever is being dragged (so the
// ghost can render its knobs at the right size before the node exists).
watchEffect(() => {
  for (const track of project.tracks) {
    for (const node of track.chain) {
      if (!node.isInstrument) ensureSpecs(node.module);
    }
  }
  if (drag.state.source) ensureSpecs(drag.state.source.module);
});

/** A param's value in a stored map (its value, else the module default). */
function specValue(params: Record<string, number>, spec: ParamInfo): number {
  const v = params[spec.name];
  return v === undefined ? spec.default : v;
}

const paramValue = (node: ChainNodeView, spec: ParamInfo): number => specValue(node.params, spec);

const sliderValue = (e: Event): number => Number((e.target as HTMLInputElement).value);

/** Real value behind a normalized 0..1 position, mirroring `ParamSpec::denormalize`.
 * For the `db` curve the *displayed* value is the dB position itself (the linear
 * gain conversion only matters to the engine). */
function realValue(p: ParamInfo, v: number): number {
  if (p.curve === "log") return p.min * Math.pow(p.max / p.min, v);
  return p.min + (p.max - p.min) * v;
}

/** "350 ms", "-6.0 dB", "0.35" — adaptive precision so short and huge ranges both read. */
function formatParam(node: ChainNodeView, p: ParamInfo): string {
  const r = realValue(p, paramValue(node, p));
  const digits = Math.abs(r) >= 100 ? 0 : Math.abs(r) >= 10 ? 1 : 2;
  const text = r.toFixed(digits);
  return p.unit ? `${text} ${p.unit}` : text;
}

/** The dragged item as a card preview: its module and the params to show. */
const ghost = computed<{ module: string; params: Record<string, number> } | null>(() => {
  const s = drag.state.source;
  if (!s) return null;
  if (s.kind === "node") {
    for (const track of project.tracks) {
      const node = track.chain.find((n) => n.id === s.nodeId);
      if (node) return { module: node.module, params: node.params };
    }
    return { module: s.module, params: {} };
  }
  if (s.kind === "preset") return { module: s.module, params: s.params };
  return { module: s.module, params: {} }; // a module from the browser
});

// Pin a leaving card at its current spot before it goes `position: absolute`,
// so it fades out in place instead of collapsing to the chain's left edge. Its
// neighbours then slide left into its place via the FLIP `chain-move` transition.
function onBeforeLeave(el: Element): void {
  const e = el as HTMLElement;
  e.style.left = `${e.offsetLeft}px`;
  e.style.top = `${e.offsetTop}px`;
  e.style.width = `${e.offsetWidth}px`;
}

/** True while this target is the one under the pointer (it lights up). */
function isOver(target: DropTarget): boolean {
  const h = drag.state.hovered;
  if (!h || h.kind !== target.kind) return false;
  if (h.kind === "gap" && target.kind === "gap")
    return h.trackId === target.trackId && h.index === target.index;
  if (h.kind === "instrument" && target.kind === "instrument")
    return h.trackId === target.trackId;
  return true; // newTrack: there is only one
}

// Pressing a node selects and focuses it right away (so Delete/Ctrl+D apply and the
// params unfold). The drag only arms here: it starts once the pointer travels past
// the threshold, so a plain click selects without moving the node.
function onNodePointerDown(e: PointerEvent, trackId: string, node: ChainNodeView, index: number): void {
  if (node.isInstrument) return;
  e.preventDefault();
  sel.select(node.id);
  (e.currentTarget as HTMLElement).focus({ preventScroll: true });
  drag.arm(
    { kind: "node", nodeId: node.id, module: node.module, fromTrack: trackId, fromIndex: index },
    e.clientX,
    e.clientY,
  );
}

// Selection follows the focus: when the card loses it (a click in the empty
// chain, another view…), drop the highlight so the green frame never outlives the
// "this node is selected and deletable" state. Focus moving to the card's own
// slider keeps the selection (the new target is still inside the card).
function onNodeBlur(e: FocusEvent, nodeId: string): void {
  const next = e.relatedTarget as Node | null;
  if (next && (e.currentTarget as HTMLElement).contains(next)) return;
  if (sel.selected === nodeId) sel.clear();
}

// The handler lives on the focused node, so `focused` is true by construction; a
// node in another context (text field, piano roll) never receives this event.
function onNodeKey(e: KeyboardEvent, trackId: string, node: ChainNodeView, index: number): void {
  const action = sel.action(e.key, { ctrl: e.ctrlKey, meta: e.metaKey }, true);
  if (action === "delete") {
    e.preventDefault();
    e.stopPropagation();
    sel.clear();
    void project.removeNode(node.id);
  } else if (action === "duplicate") {
    e.preventDefault();
    e.stopPropagation();
    void project.copyNode(node.id, trackId, index + 1);
  }
}
</script>

<template>
  <div class="chain-panel" :class="{ dragging: !!drag.state.source }">
    <template v-if="track">
      <header class="chain-head">
        <button type="button" class="nav" :title="t('chain.prevTrack')" @click="step(-1)">
          <Icon name="chevron-left" :size="13" />
        </button>
        <span class="chip" :style="{ background: hues.get(track.id) }"></span>
        <span class="chain-track-name">{{ track.name || track.id }}</span>
        <button type="button" class="nav" :title="t('chain.nextTrack')" @click="step(1)">
          <Icon name="chevron-right" :size="13" />
        </button>
        <span class="track-count num">{{ activeIndex + 1 }}/{{ project.tracks.length }}</span>
      </header>
      <div class="chain-row">
      <!-- TransitionGroup so a removed effect's neighbours slide into place. Each
           effect (gap + card) is one keyed item; the trailing gap appends. -->
      <TransitionGroup
        tag="div"
        name="chain"
        class="chain"
        data-chain
        :data-track="track.id"
        @before-leave="onBeforeLeave"
      >
        <div v-for="(node, i) in track.chain" :key="node.id" class="chain-item">
          <!-- A drop gap precedes every effect (none before the instrument). -->
          <span
            v-if="!node.isInstrument"
            class="gap"
            :class="{ over: isOver({ kind: 'gap', trackId: track.id, index: i }) }"
            data-gap
            :data-track="track.id"
            :data-index="i"
          ></span>
          <div
            class="node"
            :class="{
              instrument: node.isInstrument,
              selected: sel.selected === node.id,
              'swap-target': node.isInstrument && draggingInstrument,
              over: node.isInstrument && isOver({ kind: 'instrument', trackId: track.id }),
            }"
            :data-instrument="node.isInstrument ? '' : undefined"
            :tabindex="node.isInstrument ? undefined : 0"
            :title="node.isInstrument ? t('chain.replaceInstrument') : undefined"
            @pointerdown="onNodePointerDown($event, track.id, node, i)"
            @click="node.isInstrument && (replacing = !replacing)"
            @keydown="onNodeKey($event, track.id, node, i)"
            @focusout="onNodeBlur($event, node.id)"
          >
            <div class="node-head">
              <span class="node-name">{{ node.module }}</span>
              <!-- Live VU from the audio thread (peak + RMS), mapped by flat index. -->
              <NodeMeter :index="node.flatIndex" :size="22" />
            </div>
            <!-- Params unfold *below* the header on selection, one labelled row
                 each (name — slider — real value), so the card grows down and its
                 neighbours never move. `pointerdown.stop` on the whole block: a
                 press anywhere in the params area tweaks values, never arms a card
                 drag. Drag streams the value live; release persists it. -->
            <div
              v-if="!node.isInstrument && sel.selected === node.id"
              class="node-params"
              @pointerdown.stop
            >
              <label v-for="p in paramSpecs[node.module] ?? []" :key="p.id" class="param-row">
                <span class="param-name">{{ p.name }}</span>
                <input
                  type="range"
                  class="param-slider"
                  min="0"
                  max="1"
                  step="0.001"
                  :value="paramValue(node, p)"
                  @input="project.setNodeParam(node.id, p.name, sliderValue($event))"
                  @change="project.commitNodeParam(node.id, p.name, sliderValue($event))"
                />
                <span class="param-value num">{{ formatParam(node, p) }}</span>
              </label>
            </div>
          </div>
          <!-- Click the instrument card: pick its replacement. -->
          <InstrumentMenu
            v-if="node.isInstrument && replacing"
            class="replace-menu"
            @pick="replaceInstrument"
            @close="replacing = false"
          />
        </div>
        <!-- Trailing gap = append to the end of the chain. -->
        <span
          key="trailing"
          class="gap"
          :class="{ over: isOver({ kind: 'gap', trackId: track.id, index: track.chain.length }) }"
          data-gap
          :data-track="track.id"
          :data-index="track.chain.length"
        ></span>
      </TransitionGroup>
      </div>
    </template>

    <!-- Drag ghost: a ~50% copy of the real card. Cards are compact (params show
         only once selected), so the ghost is compact too — its footprint matches
         the card as it will land: unselected, header only. -->
    <div
      v-if="ghost"
      class="drag-ghost"
      :style="{ left: drag.state.x + 'px', top: drag.state.y + 'px' }"
    >
      <div class="node ghost">
        <div class="node-head">
          <span class="node-name">{{ ghost.module }}</span>
          <span v-if="drag.state.copy" class="ghost-copy">+</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chain-panel {
  height: 100%;
  /* Only the panel scrolls vertically (when there are many tracks); each track
     row scrolls its own effects horizontally, so the track name never leaves. */
  overflow-y: auto;
  overflow-x: hidden;
  padding: 0.4rem 0.6rem;
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.chain-row {
  display: flex;
  /* Top-aligned: a card unfolding its params grows downward without moving its
     neighbours or the track name. */
  align-items: flex-start;
  gap: 0.6rem;
}

/* Header: hue chip + track name + prev/next navigation + remove. */
.chain-head {
  display: flex;
  align-items: center;
  gap: 0.3rem;
}

.chip {
  width: 8px;
  height: 8px;
  border-radius: 2px;
  flex: none;
}

.chain-track-name {
  font-family: var(--font-display);
  font-size: 0.78rem;
  font-weight: 600;
  color: var(--text);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.nav {
  display: flex;
  align-items: center;
  border: none;
  background: none;
  padding: 0.15rem;
  color: var(--text-3);
  cursor: pointer;
  transition: color 120ms ease;
}

.nav:hover {
  color: var(--text);
}

.track-count {
  font-size: 0.65rem;
  color: var(--text-3);
}

/* Anchors the instrument-replace picker under the card. */
.chain-item {
  position: relative;
}

.replace-menu {
  top: calc(100% + 4px);
  left: 0;
}

.chain {
  display: flex;
  align-items: flex-start;
  flex: 1;
  /* A long chain scrolls within its row instead of being clipped or pushing the
     panel wider. `min-width: 0` lets this flex child shrink below its content so
     the overflow actually engages. */
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: thin;
  /* Offset parent for a leaving card pinned by `onBeforeLeave` (position: absolute). */
  position: relative;
}

/* One flex box per effect (its drop gap + its card), so a removal animates the
   neighbours sliding into place via the FLIP `chain-move` transition below. */
.chain-item {
  display: flex;
  align-items: center;
}

/* When a node is removed, the nodes to its right slide left into place (FLIP
   `move`). The removed node is pinned in place by `onBeforeLeave` and fades out
   there — it never slides before vanishing. */
.chain-move {
  transition: transform 120ms ease;
}

.chain-leave-active {
  position: absolute;
  transition: opacity 120ms ease;
}

.chain-leave-to {
  opacity: 0;
}

.node {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 0.3rem;
  padding: 0.3rem 0.5rem;
  border: 1px solid var(--border-strong);
  border-radius: 6px;
  background: var(--surface);
  /* Plain arrow on hover/click: a press selects (the drag only arms past the
     threshold), so the card should not advertise a grab affordance at rest. */
  cursor: default;
  user-select: none;
  white-space: nowrap;
}

/* Selection carries the focus; show our own accent outline, never the system
   focus ring (which would clash with the teal/anthracite palette). */
.node:focus,
.node:focus-visible {
  outline: none;
}

/* Selection uses the border (part of the box) plus a tinted fill — never an
   outset box-shadow, whose paint can be left orphaned as a thin green line when
   the card is removed during the leave/move transform (WebView2 compositor). */
.node.selected {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, var(--surface));
}

.node.instrument {
  background: color-mix(in srgb, var(--sig-notes) 12%, var(--surface));
  border-color: color-mix(in srgb, var(--sig-notes) 45%, var(--border-strong));
}

/* While an instrument is dragged, chain heads are the replace targets. */
.node.instrument.swap-target {
  border-style: dashed;
  border-color: var(--accent);
}

.node.instrument.over {
  background: color-mix(in srgb, var(--accent) 12%, var(--surface));
  border-style: solid;
  border-color: var(--accent);
}

.node-head {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.node-name {
  font-size: 0.85rem;
  color: var(--text);
  margin-right: auto;
}

/* Params of the selected node, one labelled row each, stacked under the header. */
.node-params {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  padding-top: 0.3rem;
  border-top: 1px solid var(--border-strong);
  cursor: default;
}

.param-row {
  display: grid;
  grid-template-columns: 4.6rem 6.5rem 4.2rem;
  align-items: center;
  gap: 0.4rem;
}

.param-name {
  font-size: 0.7rem;
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
}

.param-value {
  font-size: 0.7rem;
  color: var(--text-2);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

/* Palette-matched slider (WebView2 = Chromium, webkit pseudo-elements only):
   thin anthracite track, round accent thumb, tall transparent hit zone. */
.param-slider {
  appearance: none;
  -webkit-appearance: none;
  width: 100%;
  height: 14px;
  background: transparent;
  cursor: pointer;
}

.param-slider::-webkit-slider-runnable-track {
  height: 4px;
  border-radius: 2px;
  background: var(--border-strong);
}

.param-slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  width: 12px;
  height: 12px;
  /* Centre the thumb on the 4px track. */
  margin-top: -4px;
  border-radius: 50%;
  background: var(--accent);
}

/* Gaps are thin by default; they widen while a drag is active and the one under
   the pointer opens the drop slot (pushing its neighbours apart). */
.gap {
  width: 4px;
  align-self: stretch;
  min-height: 1.8rem;
  border-radius: 3px;
  transition: width 120ms ease, background-color 120ms ease;
}

.chain-panel.dragging .gap {
  width: 10px;
  background: var(--border-strong);
}

.chain-panel.dragging .gap.over {
  width: 26px;
  background: var(--accent-bright);
}

/* The ghost follows the pointer; ~50% opacity over the real card styling. */
.drag-ghost {
  position: fixed;
  z-index: 1000;
  transform: translate(10px, 10px);
  pointer-events: none;
  opacity: 0.5;
}

.drag-ghost .node {
  cursor: grabbing;
  box-shadow: 0 6px 16px rgba(0, 0, 0, 0.45);
}

.ghost-copy {
  font-weight: 700;
  color: var(--accent);
}
</style>
