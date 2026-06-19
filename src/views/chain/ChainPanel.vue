<script setup lang="ts">
// Effect chains of every track, rendered in the DOM (few elements). Each track is
// a row: the fixed instrument head, then its effects. The gaps between/around
// effects are the drop targets, so an effect can be dragged within a chain or to
// another track. Instruments are never drag-reordered (M6 = effects only).

import { computed, reactive, watchEffect } from "vue";
import { useI18n } from "vue-i18n";

import { useProjectStore, type ChainNodeView, type ParamInfo } from "../../stores/project";
import { useChainDrag, type DropTarget } from "./dragdrop";
import { ChainSelection } from "./selection";
import NodeMeter from "./NodeMeter.vue";

const { t } = useI18n();
const project = useProjectStore();
const drag = useChainDrag();

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

/** True while this gap is the one under the pointer (it spreads to show the drop). */
function isOver(target: DropTarget): boolean {
  const h = drag.state.hovered;
  return !!h && h.trackId === target.trackId && h.index === target.index;
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
    <div v-for="track in project.tracks" :key="track.id" class="chain-row">
      <span class="track-name">{{ track.name || track.id }}</span>
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
            :class="{ over: isOver({ trackId: track.id, index: i }) }"
            data-gap
            :data-track="track.id"
            :data-index="i"
          ></span>
          <div
            class="node"
            :class="{ instrument: node.isInstrument, selected: sel.selected === node.id }"
            :tabindex="node.isInstrument ? undefined : 0"
            @pointerdown="onNodePointerDown($event, track.id, node, i)"
            @keydown="onNodeKey($event, track.id, node, i)"
            @focusout="onNodeBlur($event, node.id)"
          >
            <span class="node-name">{{ node.module }}</span>
            <span class="node-role">{{ node.isInstrument ? t("chain.instrument") : t("chain.effect") }}</span>
            <!-- Bare slider (no wrapping label): the interactive zone is exactly
                 the slider, so its hover/cursor and a click never bleed onto the
                 card. Drag streams the value live; release persists it. The param
                 name lives in the tooltip rather than a redundant on-card label. -->
            <input
              v-for="p in (node.isInstrument ? [] : paramSpecs[node.module] ?? [])"
              :key="p.id"
              type="range"
              class="param-slider"
              min="0"
              max="1"
              step="0.001"
              :title="p.name"
              :value="paramValue(node, p)"
              @pointerdown.stop
              @input="project.setNodeParam(node.id, p.name, sliderValue($event))"
              @change="project.commitNodeParam(node.id, p.name, sliderValue($event))"
            />
            <!-- Live VU from the audio thread (peak + RMS), mapped by flat index. -->
            <NodeMeter :index="node.flatIndex" />
          </div>
        </div>
        <!-- Trailing gap = append to the end of the chain. -->
        <span
          key="trailing"
          class="gap"
          :class="{ over: isOver({ trackId: track.id, index: track.chain.length }) }"
          data-gap
          :data-track="track.id"
          :data-index="track.chain.length"
        ></span>
      </TransitionGroup>
    </div>

    <!-- Drag ghost: a ~50% copy of the real card. Cards are compact (params show
         only once selected), so the ghost is compact too — its footprint matches
         the card as it will land: unselected, header only. -->
    <div
      v-if="ghost"
      class="drag-ghost"
      :style="{ left: drag.state.x + 'px', top: drag.state.y + 'px' }"
    >
      <div class="node ghost">
        <span class="node-name">{{ ghost.module }}</span>
        <span class="node-role">{{ t("chain.effect") }}</span>
        <span v-if="drag.state.copy" class="ghost-copy">+</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chain-panel {
  height: 100%;
  overflow: auto;
  padding: 0.4rem 0.6rem;
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.chain-row {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}

.track-name {
  min-width: 4rem;
  font-size: 0.75rem;
  color: #8e8e93;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.chain {
  display: flex;
  align-items: center;
  flex: 1;
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
  align-items: center;
  gap: 0.4rem;
  padding: 0.3rem 0.5rem;
  border: 1px solid #3a3a3c;
  border-radius: 6px;
  background: #2c2c2e;
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
  border-color: #5fae7f;
  background: #26362b;
}

.node.instrument {
  background: #20303a;
  border-color: #2f5a6e;
}

.node-name {
  font-size: 0.85rem;
  color: #f6f6f6;
}

.node-role {
  font-size: 0.65rem;
  color: #8e8e93;
  text-transform: uppercase;
}

/* The cursor only changes over the slider itself (the grab cursor stays on the
   rest of the card), and it is the pointing hand — not a resize cursor. */
.param-slider {
  /* Hidden until the node is selected (the existing click selection); never on
     hover. A dropped or unselected node stays compact (name, role, VU only). */
  display: none;
  width: 70px;
  cursor: pointer;
}

/* Selecting a node reveals its params; focus-within keeps them shown while a
   slider inside is being dragged or focused. */
.node.selected .param-slider,
.node:focus-within .param-slider {
  display: inline-block;
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
  background: #3a3a3c;
}

.chain-panel.dragging .gap.over {
  width: 26px;
  background: #e0b050;
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
  color: #5fae7f;
}
</style>
