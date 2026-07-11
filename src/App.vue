<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";

import TransportBar from "./views/transport/TransportBar.vue";
import TimelineCanvas from "./views/timeline/TimelineCanvas.vue";
import PianoRollCanvas from "./views/pianoroll/PianoRollCanvas.vue";
import ChainPanel from "./views/chain/ChainPanel.vue";
import PresetBrowser from "./views/browser/PresetBrowser.vue";
import MixerPanel from "./views/mixer/MixerPanel.vue";
import DevOverlay from "./views/dev/DevOverlay.vue";
import SplitPane from "./ui/SplitPane.vue";
import Icon from "./ui/Icon.vue";
import ToastHost from "./ui/ToastHost.vue";
import { GUTTER_WIDTH, useTimeAxis } from "./views/canvas/timeAxis";
import { useChainDrag, type DropIntent, type DropTarget } from "./views/chain/dragdrop";
import { useProjectStore } from "./stores/project";
import { useTelemetryStore } from "./stores/telemetry";
import { useToastStore } from "./stores/toasts";
import { useUiStore } from "./stores/ui";
import { BOUNDS } from "./stores/uiLayout";
import { projectHues } from "./styles/theme";

const { t } = useI18n();
const project = useProjectStore();
const telemetry = useTelemetryStore();
const toasts = useToastStore();
const ui = useUiStore();
ui.init(); // restore the saved zone layout before first paint

// One horizontal time axis for the timeline and the piano roll, so the same
// instant — the playhead above all — sits at the same X in both.
const timeAxis = useTimeAxis();

const activeTrackName = computed(() => {
  const track = project.tracks.find((tr) => tr.id === ui.activeTrackId);
  return track ? track.name || track.id : "";
});

/** The active track's hue: shown next to its name so the piano roll header
 *  reads as "this track", the same color language as timeline/chain/mixer. */
const activeTrackHue = computed(() => projectHues(project.tracks).get(ui.activeTrackId));

// The active track always points at a real track: default to the first, and
// fall back when the selected one disappears (removal, external reload).
watch(
  () => project.tracks,
  (tracks) => {
    if (!tracks.some((tr) => tr.id === ui.activeTrackId)) {
      ui.activeTrackId = tracks[0]?.id ?? "";
    }
  },
  { deep: false },
);

// An external edit that failed to parse would otherwise be invisible: the file
// changed but the app kept playing the previous version. Surface it.
watch(
  () => project.lastError,
  (err) => {
    if (err) toasts.push("error", t("toast.projectError"), err.text);
  },
);

/** Width of a collapsed zone's rail (chevron + vertical label). */
const RAIL = 28;
/** Height of the bottom zone's tab bar (visible even collapsed). */
const TAB_BAR = 33;

// The splitters size the zones; a collapsed zone is pinned to its rail/bar and
// its handle goes inert (`locked`). Writes only land while the zone is open.
const browserSize = computed({
  get: () => (ui.layout.browser.open ? ui.layout.browser.width : RAIL),
  set: (v: number) => {
    if (ui.layout.browser.open) ui.layout.browser.width = v;
  },
});

const bottomSize = computed({
  get: () => (ui.layout.bottom.open ? ui.layout.bottom.height : TAB_BAR),
  set: (v: number) => {
    if (ui.layout.bottom.open) ui.layout.bottom.height = v;
  },
});

// --- Effect-chain drag & drop controller (M6) ---
// The pure decision lives in dragdrop.ts; here we own the pointer plumbing: a
// floating ghost, the gap under the pointer, and running the resolved edit.
const drag = useChainDrag();

// The drop target under the pointer. An instrument drag only sees instrument
// slots and the new-track zone (its valid targets); anything else sees the
// chain gaps — the whole row is a target and the insert position is the gap
// nearest the pointer in X, far more forgiving than hitting the thin gap itself.
function targetUnder(x: number, y: number): DropTarget | null {
  const el = document.elementFromPoint(x, y);
  const source = drag.state.source;
  if (source?.kind === "module" && source.category === "instrument") {
    if (el?.closest("[data-new-track]")) return { kind: "newTrack" };
    const head = el?.closest<HTMLElement>("[data-instrument]");
    const row = head?.closest<HTMLElement>("[data-chain]");
    if (head && row) return { kind: "instrument", trackId: row.dataset.track ?? "" };
    return null;
  }
  const row = el?.closest<HTMLElement>("[data-chain]");
  if (!row) return null;
  const gaps = row.querySelectorAll<HTMLElement>("[data-gap]");
  let best: HTMLElement | null = null;
  let bestDist = Infinity;
  for (const gap of gaps) {
    const rect = gap.getBoundingClientRect();
    const dist = Math.abs((rect.left + rect.right) / 2 - x);
    if (dist < bestDist) {
      bestDist = dist;
      best = gap;
    }
  }
  if (!best) return null;
  return { kind: "gap", trackId: row.dataset.track ?? "", index: Number(best.dataset.index) };
}

function executeIntent(intent: DropIntent): void {
  if (intent.kind === "move") void project.moveNode(intent.nodeId, intent.toTrack, intent.index);
  else if (intent.kind === "copy") void project.copyNode(intent.nodeId, intent.toTrack, intent.index);
  else if (intent.kind === "setInstrument") void project.setTrackInstrument(intent.trackId, intent.module);
  else if (intent.kind === "addTrack") {
    void project.addTrack(intent.module, intent.name).then(() => {
      // The freshly created track (appended last) becomes the active one.
      ui.activeTrackId = project.tracks[project.tracks.length - 1]?.id ?? ui.activeTrackId;
    });
  } else void project.insertNode(intent.trackId, intent.index, intent.module, intent.params);
}

function onDragMove(e: PointerEvent): void {
  // Pump moves while a press is merely armed too, so it can cross the threshold
  // and promote itself to a real drag (see useChainDrag.move).
  if (!drag.state.source && !drag.state.pressing) return;
  drag.move(e.clientX, e.clientY);
  drag.setCopy(e.altKey);
  drag.hover(targetUnder(e.clientX, e.clientY));
}

function onDragUp(e: PointerEvent): void {
  const source = drag.state.source;
  if (!source) {
    drag.clearPress(); // released before crossing the threshold: it was a click
    return;
  }
  drag.setCopy(e.altKey);
  // Dropping an effect node on the browser saves it as a preset.
  const overBrowser = document.elementFromPoint(e.clientX, e.clientY)?.closest("[data-save-preset]");
  if (overBrowser && source.kind === "node") {
    const name = `${source.module}-${Date.now().toString(36).slice(-4)}`;
    void project.savePreset(name, source.nodeId);
    drag.cancel();
    return;
  }
  const intent = drag.drop(); // resolves on the hovered gap, or null outside
  if (intent) executeIntent(intent);
}

function onDragKey(e: KeyboardEvent): void {
  if (drag.state.source && e.key === "Escape") drag.cancel();
}

// --- Global keyboard: undo/redo + zone shortcuts ---

async function undoRedo(redo: boolean) {
  await invoke(redo ? "edit_redo" : "edit_undo");
  await project.load();
}

/** True when the key lands in a typing/adjusting context the shortcut must not steal. */
function inInputContext(e: KeyboardEvent): boolean {
  return !!(e.target as HTMLElement | null)?.closest("input, select, textarea");
}

function onKeyDown(event: KeyboardEvent) {
  // Universal undo/redo (semantic key, so it works on AZERTY too). Ctrl+Z /
  // Ctrl+Shift+Z covers both notes and graph edits via the unified history.
  if (event.ctrlKey && !event.altKey && event.key.toLowerCase() === "z") {
    event.preventDefault();
    void undoRedo(event.shiftKey);
    return;
  }
  if (event.repeat || inInputContext(event)) return;
  if (event.ctrlKey && event.altKey && event.code === "KeyD") {
    event.preventDefault();
    ui.toggleDev();
    return;
  }
  if (!event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  if (event.code === "KeyB") {
    event.preventDefault();
    ui.toggleBrowser();
  } else if (event.code === "KeyE") {
    event.preventDefault();
    ui.toggleBottom("chain");
  } else if (event.code === "KeyM") {
    event.preventDefault();
    ui.toggleBottom("mixer");
  }
}

onMounted(async () => {
  await telemetry.init();
  await project.init(); // file-watcher listeners (external reload + note flash)
  await project.load();
  if (!ui.activeTrackId) ui.activeTrackId = project.tracks[0]?.id ?? "";
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("pointermove", onDragMove);
  window.addEventListener("pointerup", onDragUp);
  window.addEventListener("keydown", onDragKey);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("pointermove", onDragMove);
  window.removeEventListener("pointerup", onDragUp);
  window.removeEventListener("keydown", onDragKey);
});
</script>

<template>
  <div class="app" :class="{ 'chain-dragging': !!drag.state.source, 'chain-pressing': drag.state.pressing }">
    <TransportBar />

    <SplitPane
      class="shell"
      orientation="vertical"
      sized="a"
      :min="BOUNDS.browserWidth.min"
      :max="BOUNDS.browserWidth.max"
      :default-size="BOUNDS.browserWidth.default"
      :locked="!ui.layout.browser.open"
      v-model="browserSize"
    >
      <!-- Left zone: the browser (instruments / effects / presets). Collapses
           to a slim rail whose label reopens it. -->
      <template #a>
        <aside class="zone browser-zone">
          <header class="zone-head">
            <button
              type="button"
              class="zone-toggle"
              :title="ui.layout.browser.open ? t('ui.collapse') : t('ui.expand')"
              @click="ui.toggleBrowser()"
            >
              <Icon :name="ui.layout.browser.open ? 'chevron-left' : 'chevron-right'" :size="14" />
            </button>
            <h2 v-if="ui.layout.browser.open" class="zone-title">{{ t("view.browser") }}</h2>
          </header>
          <div v-if="ui.layout.browser.open" class="zone-body">
            <PresetBrowser />
          </div>
          <button v-else type="button" class="rail-label" @click="ui.toggleBrowser()">
            {{ t("view.browser") }}
          </button>
        </aside>
      </template>

      <template #b>
        <SplitPane
          orientation="horizontal"
          sized="b"
          :min="BOUNDS.bottomHeight.min"
          :max="BOUNDS.bottomHeight.max"
          :default-size="BOUNDS.bottomHeight.default"
          :locked="!ui.layout.bottom.open"
          v-model="bottomSize"
        >
          <!-- Central zone: timeline over piano roll, sharing the time axis. -->
          <template #a>
            <SplitPane
              orientation="horizontal"
              sized="a"
              :min="BOUNDS.timelineHeight.min"
              :max="BOUNDS.timelineHeight.max"
              :default-size="BOUNDS.timelineHeight.default"
              v-model="ui.layout.timelineHeight"
            >
              <template #a>
                <section class="zone">
                  <h2 class="zone-title padded">{{ t("view.timeline") }}</h2>
                  <div class="zone-fill well" :style="{ '--well-left': `${GUTTER_WIDTH}px` }">
                    <TimelineCanvas :axis="timeAxis" />
                  </div>
                </section>
              </template>
              <template #b>
                <section class="zone">
                  <header class="zone-head row">
                    <h2 class="zone-title">{{ t("view.pianoRoll") }}</h2>
                    <span v-if="activeTrackName" class="track-tag">
                      <span class="active-track" :style="{ borderColor: activeTrackHue }">
                        {{ activeTrackName }}
                      </span>
                    </span>
                    <span class="hint">{{ t("edit.hints") }}</span>
                  </header>
                  <div class="zone-fill well" :style="{ '--well-left': `${GUTTER_WIDTH}px` }">
                    <PianoRollCanvas
                      v-if="ui.activeTrackId"
                      :key="ui.activeTrackId"
                      :track-id="ui.activeTrackId"
                      :axis="timeAxis"
                    />
                    <div v-else class="empty-state">{{ t("empty.pianoRoll") }}</div>
                  </div>
                </section>
              </template>
            </SplitPane>
          </template>

          <!-- Bottom zone: chain / mixer tabs. The tab bar stays visible when
               collapsed; clicking a tab reopens on it. -->
          <template #b>
            <section class="zone bottom-zone">
              <header class="zone-head tabs">
                <button
                  type="button"
                  class="tab"
                  :class="{ active: ui.layout.bottom.open && ui.layout.bottom.tab === 'chain' }"
                  @click="ui.toggleBottom('chain')"
                >
                  {{ t("view.chain") }}
                </button>
                <button
                  type="button"
                  class="tab"
                  :class="{ active: ui.layout.bottom.open && ui.layout.bottom.tab === 'mixer' }"
                  @click="ui.toggleBottom('mixer')"
                >
                  {{ t("view.mixer") }}
                </button>
                <button
                  type="button"
                  class="zone-toggle end"
                  :title="ui.layout.bottom.open ? t('ui.collapse') : t('ui.expand')"
                  @click="ui.toggleBottom()"
                >
                  <Icon name="chevron-down" :size="14" :class="{ flip: !ui.layout.bottom.open }" />
                </button>
              </header>
              <div v-show="ui.layout.bottom.open" class="bottom-body">
                <div v-show="ui.layout.bottom.tab === 'chain'" class="bottom-page">
                  <ChainPanel />
                </div>
                <div v-show="ui.layout.bottom.tab === 'mixer'" class="bottom-page">
                  <MixerPanel />
                </div>
              </div>
            </section>
          </template>
        </SplitPane>
      </template>
    </SplitPane>

    <DevOverlay v-if="ui.layout.devOpen" @close="ui.toggleDev()" />
    <ToastHost />
  </div>
</template>

<style scoped>
.app {
  height: 100vh;
  display: flex;
  flex-direction: column;
}

.shell {
  flex: 1;
  min-height: 0;
}

.zone {
  display: flex;
  flex-direction: column;
  min-height: 0;
  min-width: 0;
  height: 100%;
  background: var(--panel);
}

.zone-head {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex: none;
}

.zone-head.row {
  gap: 0.6rem;
}

.zone-title {
  margin: 0;
  padding: 0.3rem 0.4rem;
  font-family: var(--font-display);
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-2);
}

.zone-title.padded {
  padding-left: 0.8rem;
}

/* The row right under the top bar (browser head, timeline title) keeps a
   little air between the bar's shadow and its text — padded on the titles
   themselves so both texts sit at exactly the same height. */
.browser-zone .zone-title,
.zone-title.padded {
  padding-top: 0.55rem;
}

.zone-fill {
  flex: 1;
  min-height: 0;
}

/* Depth: the two note canvases (timeline + piano roll) read as ONE recessed
   plane behind the tooling — same deepest background, same inset shadow cast
   by the panels "in front". The left gutters (piano keys, track names) are
   tooling too: the well starts at their right edge, so its shaded left flank
   makes them overhang the canvas. Painted by an overlay pseudo-element
   because the canvas child would cover a shadow set on the container itself. */
.zone-fill.well {
  position: relative;
}

.zone-fill.well::after {
  content: "";
  position: absolute;
  inset: 0 0 0 var(--well-left, 0px);
  pointer-events: none;
  box-shadow: var(--shadow-well);
}

.zone-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.zone-toggle {
  border: none;
  background: none;
  padding: 0.35rem 0.3rem;
  color: var(--text-2);
  cursor: pointer;
  display: flex;
  align-items: center;
  transition: color var(--t-fast);
}

.zone-toggle:hover {
  color: var(--text);
}

.zone-toggle.end {
  margin-left: auto;
  margin-right: 0.4rem;
}

/* The fold chevron turns as the zone folds/unfolds, in step with the pane. */
.zone-toggle svg {
  transition: transform var(--t-med);
}

.flip {
  transform: rotate(180deg);
}

/* Collapsed browser: a slim rail with a vertical label that reopens it. Same
   type treatment as every zone title (.zone-title), only rotated. */
.rail-label {
  flex: 1;
  border: none;
  background: none;
  padding: 0.6rem 0;
  color: var(--text-2);
  font-family: var(--font-display);
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  writing-mode: vertical-rl;
  cursor: pointer;
  transition: color var(--t-fast);
}

.rail-label:hover {
  color: var(--text);
}

.browser-zone {
  border-right: none;
}

.hint {
  font-size: 0.7rem;
  color: var(--text-3);
  margin-left: auto;
  padding-right: 0.8rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* The active track's tag: hue chip + name, the same language as the chain
   header — visually distinct from the muted zone title beside it. */
.track-tag {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  padding-left: 0.4rem;
  border-left: 1px solid var(--border);
}

.active-track {
  font-family: var(--font-display);
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  /* Rounded underline: a border can't round alone, so the color rides the
     bottom 3px of the box as a pill-shaped stripe. Colored per track inline. */
  border-bottom: 3px solid transparent;
  border-radius: 0 0 3px 3px;
}

/* Bottom zone tabs: the bar itself stays when the zone is collapsed. */
.zone-head.tabs {
  gap: 0;
  border-top: 1px solid var(--border);
}

.tab {
  border: none;
  background: none;
  padding: 0.45rem 0.9rem;
  color: var(--text-2);
  font-family: var(--font-display);
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  cursor: pointer;
  border-bottom: 2px solid transparent;
  transition: color var(--t-fast), border-color var(--t-fast);
}

.tab:hover {
  color: var(--text);
}

.tab.active {
  color: var(--text);
  border-bottom-color: var(--accent);
}

.bottom-body {
  flex: 1;
  min-height: 0;
}

.bottom-page {
  height: 100%;
  min-height: 0;
  overflow: auto;
}
</style>
