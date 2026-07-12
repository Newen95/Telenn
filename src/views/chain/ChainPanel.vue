<script setup lang="ts">
// The ACTIVE track's chain, rendered in the DOM: the instrument head (click to
// edit its params, chevron to replace it), then its effects. The gaps between/
// around effects are the drop targets, so an effect can be dragged within the
// chain (or to another track by selecting it first). The header navigates
// between tracks; the overview of all tracks is the mixer's job.

import { computed, nextTick, reactive, ref, watch, watchEffect } from "vue";
import { useI18n } from "vue-i18n";

import { useProjectStore, type ChainNodeView, type ModuleDesc, type ParamInfo, type Preset } from "../../stores/project";
import { useTelemetryStore } from "../../stores/telemetry";
import { useUiStore } from "../../stores/ui";
import { projectHues } from "../../styles/theme";
import { useChainDrag, type DropTarget } from "./dragdrop";
import { ChainSelection } from "./selection";
import NodeMeter from "./NodeMeter.vue";
import Icon from "../../ui/Icon.vue";
import InstrumentMenu from "../../ui/InstrumentMenu.vue";

const { t } = useI18n();
const project = useProjectStore();
const telemetry = useTelemetryStore();
const ui = useUiStore();
const drag = useChainDrag();

/** Signal intensity (0..1) on the wire AFTER chain node `index`: the node's
 *  live output level, so each wire shows what actually flows at that point.
 *  Square root lifts quiet material into the visible range. */
function wireLevel(chain: ChainNodeView[], index: number): number {
  const node = chain[index];
  if (!node) return 0;
  return Math.min(1, Math.sqrt(telemetry.meterAt(node.flatIndex).peak));
}

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

// Click the instrument card's chevron → picker replacing the track's
// instrument. The menu is position: fixed, anchored under the card at click
// time: an absolute popover would be clipped by the chain's own scroll
// containers.
const replacing = ref(false);
const replaceAt = ref({ x: 0, y: 0 });

function toggleReplace(e: MouseEvent): void {
  const card = (e.currentTarget as HTMLElement).closest(".node") ?? (e.currentTarget as HTMLElement);
  const rect = card.getBoundingClientRect();
  replaceAt.value = { x: rect.left, y: rect.bottom + 4 };
  replacing.value = !replacing.value;
}

async function replaceInstrument(m: ModuleDesc): Promise<void> {
  replacing.value = false;
  if (track.value) await project.setTrackInstrument(track.value.id, m.id);
}

/** Picking a chain preset replaces the whole chain (its effects included):
 *  the preset IS the sound, keeping the old effects would betray it. */
async function replaceChain(_name: string, nodes: Preset[]): Promise<void> {
  replacing.value = false;
  if (track.value) await project.setTrackChain(track.value.id, nodes);
}

// True while an instrument module or a chain preset is being dragged from the
// browser: the drop targets are then the chain heads and the new-track zone,
// which light up.
const draggingInstrument = computed(() => {
  const s = drag.state.source;
  return (s?.kind === "module" && s.category === "instrument") || s?.kind === "chainPreset";
});

// Node selection: a selected node carries the focus, so Delete/Ctrl+D act on it
// and only when the chain (not a text field or the piano roll) owns the focus.
const sel = reactive(new ChainSelection());

// FLIP move transitions guide removals, but the end of a drag must land with
// zero motion: the in-place preview already showed the final layout, and the
// lifted card (display: none, so a 0×0 old rect) would otherwise FLIP in from
// the panel's corner. `flush: sync` so the flag flips before the drop's patch;
// a starting node drag also drops its selection (the held card travels folded).
const flip = ref(true);
watch(
  () => drag.state.source,
  (s, old) => {
    if (s?.kind === "node" && sel.selected === s.nodeId) sel.clear();
    if (old && !s) {
      flip.value = false;
      void nextTick(() => {
        flip.value = true;
      });
    }
  },
  { flush: "sync" },
);

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

// Fetch specs for every chain node (instrument included), plus whatever is
// being dragged (so the ghost can render its knobs at the right size before
// the node exists).
watchEffect(() => {
  for (const track of project.tracks) {
    for (const node of track.chain) ensureSpecs(node.module);
  }
  const dragged = drag.state.source;
  if (dragged && dragged.kind !== "chainPreset") ensureSpecs(dragged.module);
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
  if (s.kind === "chainPreset") return { module: s.name, params: {} };
  return { module: s.module, params: {} }; // a module from the browser, or a chain
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

/** True while `nodeId` is the node being dragged: its card leaves the row —
 *  what you hold is in your hand, and the preview shows where it lands. */
function lifting(nodeId: string): boolean {
  const s = drag.state.source;
  return s?.kind === "node" && s.nodeId === nodeId;
}

/** True while a chain preset hovers this track's head: the drop replaces the
 *  WHOLE chain, so the doomed effects dim to announce the gesture's scope. */
function chainReplaceHover(trackId: string): boolean {
  return drag.state.source?.kind === "chainPreset" && isOver({ kind: "instrument", trackId });
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
// the threshold, so a plain click selects without moving the node. The instrument
// card arms a whole-chain drag whose only destination is the browser's save zone.
function onNodePointerDown(e: PointerEvent, trackId: string, node: ChainNodeView, index: number): void {
  e.preventDefault();
  sel.select(node.id);
  (e.currentTarget as HTMLElement).focus({ preventScroll: true });
  drag.arm(
    node.isInstrument
      ? { kind: "chain", trackId, module: node.module }
      : { kind: "node", nodeId: node.id, module: node.module, fromTrack: trackId, fromIndex: index },
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
// The instrument stays inert: a chain head cannot be deleted or duplicated.
function onNodeKey(e: KeyboardEvent, trackId: string, node: ChainNodeView, index: number): void {
  if (node.isInstrument) return;
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
        <span class="chain-track-name" :style="{ borderColor: hues.get(track.id) }">
          {{ track.name || track.id }}
        </span>
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
        :class="{ 'chain-replacing': chainReplaceHover(track.id) }"
        data-chain
        :data-track="track.id"
        :move-class="flip ? 'chain-move' : 'chain-move-off'"
        @before-leave="onBeforeLeave"
      >
        <div
          v-for="(node, i) in track.chain"
          :key="node.id"
          class="chain-item"
          :class="{ lifted: lifting(node.id) }"
        >
          <!-- A drop gap precedes every effect (none before the instrument). -->
          <span
            v-if="!node.isInstrument"
            class="gap"
            :class="{ over: isOver({ kind: 'gap', trackId: track.id, index: i }) }"
            :style="{ '--sig': wireLevel(track.chain, i - 1) }"
            data-gap
            :data-track="track.id"
            :data-index="i"
          ></span>
          <!-- In-place drop preview: the dragged effect already sitting where it
               will land — release = exactly this. -->
          <div
            v-if="ghost && isOver({ kind: 'gap', trackId: track.id, index: i })"
            class="node preview"
          >
            <div class="node-head">
              <span class="node-name">{{ ghost.module }}</span>
              <!-- Silent meter: the preview is exactly as wide as the landed card. -->
              <NodeMeter :index="-1" :size="20" />
            </div>
          </div>
          <!-- The preview's outgoing wire: the ghost is plugged in, not floating. -->
          <span
            v-if="ghost && isOver({ kind: 'gap', trackId: track.id, index: i })"
            class="ghost-wire"
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
            tabindex="0"
            @pointerdown="onNodePointerDown($event, track.id, node, i)"
            @keydown="onNodeKey($event, track.id, node, i)"
            @focusout="onNodeBlur($event, node.id)"
          >
            <div class="node-head">
              <span class="node-name">{{ node.module }}</span>
              <!-- The instrument card selects like an effect; only its chevron
                   opens the replace picker (a select-like "click to change"). -->
              <button
                v-if="node.isInstrument"
                type="button"
                class="swap-hint"
                :title="t('chain.replaceInstrument')"
                @pointerdown.stop
                @click.stop="toggleReplace($event)"
              >
                <Icon name="chevron-down" :size="12" />
              </button>
              <!-- Live VU from the audio thread (peak + RMS), mapped by flat index. -->
              <NodeMeter :index="node.flatIndex" :size="20" />
            </div>
            <!-- Params unfold *below* the header on selection, one labelled row
                 each (name — slider — real value), so the card grows down and its
                 neighbours never move. `pointerdown.stop` on the whole block: a
                 press anywhere in the params area tweaks values, never arms a card
                 drag. Drag streams the value live; release persists it. -->
            <div
              v-if="sel.selected === node.id"
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
            :style="{ position: 'fixed', left: `${replaceAt.x}px`, top: `${replaceAt.y}px` }"
            @pick="replaceInstrument"
            @pick-chain="replaceChain"
            @close="replacing = false"
          />
        </div>
        <!-- Trailing gap = append to the end of the chain. -->
        <div key="trailing" class="chain-item">
          <span
            class="gap end"
            :class="{ over: isOver({ kind: 'gap', trackId: track.id, index: track.chain.length }) }"
            :style="{ '--sig': wireLevel(track.chain, track.chain.length - 1) }"
            data-gap
            :data-track="track.id"
            :data-index="track.chain.length"
          ></span>
          <div
            v-if="ghost && isOver({ kind: 'gap', trackId: track.id, index: track.chain.length })"
            class="node preview"
          >
            <div class="node-head">
              <span class="node-name">{{ ghost.module }}</span>
              <NodeMeter :index="-1" :size="20" />
            </div>
          </div>
        </div>
      </TransitionGroup>
      </div>
    </template>
    <div v-else class="empty-state">{{ t("empty.chain") }}</div>

    <!-- Drag ghost following the pointer — only away from the chain's gaps,
         where the in-place preview above takes over (one ghost at a time). -->
    <div
      v-if="ghost && drag.state.hovered?.kind !== 'gap'"
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
  gap: 0.65rem;
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

.chain-track-name {
  font-family: var(--font-display);
  font-size: 0.78rem;
  font-weight: 600;
  color: var(--text);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  border-bottom: 3px solid transparent; /* colored per track inline */
  border-radius: 0 0 3px 3px; /* rounds the underline's ends */
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
   neighbours sliding into place via the FLIP `chain-move` transition below.
   Top-aligned: an unfolded (tall) card must not push the preview downward. */
.chain-item {
  display: flex;
  align-items: flex-start;
}

/* When a node is removed, the nodes to its right slide left into place (FLIP
   `move`). The removed node is pinned in place by `onBeforeLeave` and fades out
   there — it never slides before vanishing. The end of a drag swaps this class
   for `chain-move-off` (no rule on purpose: without a transform transition Vue
   skips FLIP entirely), so a drop lands frozen where the preview stood. */
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
  padding: 0.45rem 0.65rem;
  border: 1px solid var(--border-strong);
  border-radius: 6px;
  background: var(--surface);
  /* Barely lifted off the panel: a graspable object, not part of the tray. */
  box-shadow: var(--shadow-raised);
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

/* The instrument: the chain's head, another kind of thing than an effect. A
   thick Notes-magenta spine and a rounder left flank give it its own silhouette;
   it selects like an effect, and only its chevron opens the replace picker. */
.node.instrument {
  background: color-mix(in srgb, var(--sig-notes) 12%, var(--surface));
  border-color: color-mix(in srgb, var(--sig-notes) 45%, var(--border-strong));
  border-left: 3px solid var(--sig-notes);
  border-radius: 12px 6px 6px 12px;
}

.swap-hint {
  display: inline-flex;
  align-items: center;
  border: none;
  background: none;
  padding: 0 0.1rem;
  color: var(--text-3);
  cursor: pointer;
  transition: color var(--t-fast);
}

.node.instrument:hover .swap-hint {
  color: var(--text);
}

/* While an instrument is dragged, chain heads are the replace targets. */
.node.instrument.swap-target {
  border-style: dashed;
  border-color: var(--accent);
}

/* A chain preset hovering the head replaces the WHOLE chain: the doomed
   effects (and their wires) dim so the gesture's scope reads before the drop. */
.chain-replacing .node:not(.instrument),
.chain-replacing .gap {
  opacity: 0.35;
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
  font-size: 0.9rem;
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

/* Gaps carry the signal: each one shows the audio wire running card to card,
   so the chain reads as a path, not a row of boxes — and a drop lands ON the
   wire. The gap itself stays invisible even during a drag: the in-place
   preview card is the one and only drop marker. */
.gap {
  width: 14px;
  align-self: stretch;
  min-height: 1.8rem;
  position: relative;
}

/* The wire: a teal (signal Audio) segment at the cards' header height. Its
   brightness and halo follow --sig, the node's live output level — the wire
   literally shows the signal flowing through it. */
.gap::before {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: 1rem;
  height: 3px;
  border-radius: 1.5px;
  /* At full level the wire overshoots the base teal toward white-hot. */
  background: color-mix(
    in srgb,
    color-mix(in srgb, var(--text) calc(var(--sig, 0) * 45%), var(--sig-audio)),
    transparent calc(70% - var(--sig, 0) * 70%)
  );
  box-shadow: 0 0 calc(var(--sig, 0) * 10px) var(--sig-audio);
  transition: background-color 90ms linear, box-shadow 90ms linear;
}

/* The trailing gap's wire only exists while it feeds a previewed card. */
.chain-panel.dragging .gap.end.over::before {
  display: block;
}

/* Wire out of the previewed card, at the same resting level as real wires. */
.ghost-wire {
  flex: none;
  align-self: flex-start;
  margin-top: 1rem;
  width: 14px;
  height: 3px;
  border-radius: 1.5px;
  background: color-mix(in srgb, var(--sig-audio), transparent 70%);
}

/* The in-place preview: the card as it will land — unmistakably the held one:
   copper-tinted fill, bright dashed outline and a soft copper glow. Inert to
   the pointer so hit-testing keeps seeing the row beneath. */
.node.preview {
  border: 1px dashed var(--accent-bright);
  background: color-mix(in srgb, var(--accent) 18%, var(--surface));
  pointer-events: none;
  animation: preview-in 120ms ease;
}

.node.preview .node-name {
  color: var(--accent-bright);
}

/* The held node leaves the row entirely (its gap too): the chain reads as
   "without it", the copper preview shows where it comes back. */
.chain-item.lifted {
  display: none;
}

@keyframes preview-in {
  from {
    opacity: 0;
  }
}


/* End of the chain: no wire after the last card — the path just ends. The
   trailing gap stays a drop target (it widens like any gap during a drag). */
.gap.end::before {
  display: none;
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
  box-shadow: var(--shadow-held);
}

.ghost-copy {
  font-weight: 700;
  color: var(--accent);
}
</style>
