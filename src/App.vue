<script setup lang="ts">
import { onMounted, onUnmounted, reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";

import TransportBar from "./views/transport/TransportBar.vue";
import TimelineCanvas from "./views/timeline/TimelineCanvas.vue";
import PianoRollCanvas from "./views/pianoroll/PianoRollCanvas.vue";
import { useTimeAxis } from "./views/canvas/timeAxis";
import { useProjectStore } from "./stores/project";
import { useTelemetryStore } from "./stores/telemetry";

const { t } = useI18n();
const project = useProjectStore();
const telemetry = useTelemetryStore();
const selectedTrackId = ref("");
// One horizontal time axis for the timeline and the piano roll, so the same
// instant — the playhead above all — sits at the same X in both.
const timeAxis = useTimeAxis();

// --- Dev panel (raw technical labels, not i18n): keyboard, graph edit, sliders.

const SYNTH_NODE = 0;
const FIRST_PITCH = 60; // middle C
const DEMO_PATH = "../projects/demo/song.json";
const EXPORT_PATH = "../projects/demo/export.wav";

// Computer-keyboard layout by physical key code (works on AZERTY too).
const KEYMAP: Record<string, number> = {
  KeyA: 0, KeyW: 1, KeyS: 2, KeyE: 3, KeyD: 4, KeyF: 5,
  KeyT: 6, KeyG: 7, KeyY: 8, KeyH: 9, KeyU: 10, KeyJ: 11, KeyK: 12,
};

interface ParamInfo {
  id: number;
  name: string;
  unit: string;
  default: number;
}

interface GraphNode {
  trackId: string;
  id: string;
  module: string;
  isInstrument: boolean;
}

const keys = Array.from({ length: 13 }, (_, i) => ({
  pitch: FIRST_PITCH + i,
  black: [1, 3, 6, 8, 10].includes(i % 12),
}));

const held = reactive(new Set<number>());
const params = ref<ParamInfo[]>([]);
const paramValues = reactive<Record<number, number>>({});
const graphNodes = ref<GraphNode[]>([]);
const xruns = ref(0);
const actionMsg = ref("");
let fxCounter = 0;
let statusTimer: number | undefined;

async function loadDemo() {
  try {
    await invoke("load_project", { path: DEMO_PATH });
    actionMsg.value = "loaded";
  } catch (e) {
    actionMsg.value = String(e);
  }
}

async function exportWav() {
  try {
    await invoke("export_wav", { projectPath: DEMO_PATH, outPath: EXPORT_PATH });
    actionMsg.value = "exported";
  } catch (e) {
    actionMsg.value = String(e);
  }
}

async function refreshGraph() {
  graphNodes.value = await invoke<GraphNode[]>("graph_nodes");
}

async function runEdit(command: string, args: Record<string, unknown>, label: string) {
  try {
    await invoke(command, args);
    actionMsg.value = label;
  } catch (e) {
    actionMsg.value = String(e);
  }
  await refreshGraph();
  await project.load(); // reflect the edit in the views
}

function insertEffect(trackId: string) {
  fxCounter += 1;
  void runEdit(
    "graph_insert_effect",
    { trackId, module: "gain", id: `${trackId}_fx${fxCounter}`, params: { gain: 0.6 } },
    "inserted (gain -17 dB)",
  );
}

const removeNode = (id: string) => runEdit("graph_remove_node", { id }, "removed");
const moveNode = (id: string, dir: number) => runEdit("graph_move_node", { id, dir }, "moved");
const graphUndo = () => runEdit("edit_undo", {}, "undo");
const graphRedo = () => runEdit("edit_redo", {}, "redo");

async function noteOn(pitch: number) {
  if (held.has(pitch)) return;
  held.add(pitch);
  await invoke("note_on", { node: SYNTH_NODE, pitch, vel: 100 });
}

async function noteOff(pitch: number) {
  if (!held.has(pitch)) return;
  held.delete(pitch);
  await invoke("note_off", { node: SYNTH_NODE, pitch });
}

function releaseAll() {
  for (const pitch of [...held]) void noteOff(pitch);
}

function pressPointer(event: PointerEvent, pitch: number) {
  const target = event.target as Element;
  if (target.hasPointerCapture?.(event.pointerId)) {
    target.releasePointerCapture(event.pointerId);
  }
  void noteOn(pitch);
}

async function onParam(id: number, value: number) {
  paramValues[id] = value;
  await invoke("set_node_param", { node: SYNTH_NODE, param: id, value });
}

async function undoRedo(redo: boolean) {
  await invoke(redo ? "edit_redo" : "edit_undo");
  await project.load();
  await refreshGraph();
}

function onKeyDown(event: KeyboardEvent) {
  // Universal undo/redo (semantic key, so it works on AZERTY too). Ctrl+Z /
  // Ctrl+Shift+Z covers both notes and graph edits via the unified history.
  if (event.ctrlKey && event.key.toLowerCase() === "z") {
    event.preventDefault();
    void undoRedo(event.shiftKey);
    return;
  }
  if (event.repeat) return;
  const offset = KEYMAP[event.code];
  if (offset !== undefined) void noteOn(FIRST_PITCH + offset);
}

function onKeyUp(event: KeyboardEvent) {
  const offset = KEYMAP[event.code];
  if (offset !== undefined) void noteOff(FIRST_PITCH + offset);
}

async function refreshStatus() {
  const status = await invoke<{ xruns: number; sampleRate: number }>("engine_status");
  xruns.value = status.xruns;
}

onMounted(async () => {
  await telemetry.init();
  await project.load();
  selectedTrackId.value = project.tracks[0]?.id ?? "";
  params.value = await invoke<ParamInfo[]>("module_params", { moduleId: "plucked" });
  for (const p of params.value) paramValues[p.id] = p.default;
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  void refreshGraph();
  void refreshStatus();
  statusTimer = window.setInterval(refreshStatus, 1000);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("keyup", onKeyUp);
  if (statusTimer !== undefined) window.clearInterval(statusTimer);
});
</script>

<template>
  <div class="app">
    <TransportBar />

    <section class="view">
      <h2 class="view-title">{{ t("view.timeline") }}</h2>
      <div class="timeline-pane"><TimelineCanvas :axis="timeAxis" /></div>
    </section>

    <section class="view grow">
      <header class="view-head">
        <h2 class="view-title">{{ t("view.pianoRoll") }}</h2>
        <select v-model="selectedTrackId" class="track-select">
          <option v-for="t in project.tracks" :key="t.id" :value="t.id">{{ t.name || t.id }}</option>
        </select>
        <span class="hint">{{ t("edit.hints") }}</span>
      </header>
      <div class="pianoroll-pane">
        <PianoRollCanvas
          v-if="selectedTrackId"
          :key="selectedTrackId"
          :track-id="selectedTrackId"
          :axis="timeAxis"
        />
      </div>
    </section>

    <details class="dev">
      <summary>dev</summary>
      <div class="dev-body">
        <div class="dev-actions">
          <button type="button" class="gbtn" @click="loadDemo">reload json</button>
          <button type="button" class="gbtn" @click="exportWav">export wav</button>
          <span class="action-msg">{{ actionMsg }}</span>
          <span class="action-msg">xruns {{ xruns }}</span>
        </div>

        <div class="graph">
          <div v-for="n in graphNodes" :key="n.id" class="graph-row">
            <span class="node" :class="{ instrument: n.isInstrument }">
              {{ n.trackId }} / {{ n.id }} <em>({{ n.module }})</em>
            </span>
            <template v-if="n.isInstrument">
              <button type="button" class="gbtn" @click="insertEffect(n.trackId)">+ gain</button>
            </template>
            <template v-else>
              <button type="button" class="gbtn" @click="moveNode(n.id, -1)">◀</button>
              <button type="button" class="gbtn" @click="moveNode(n.id, 1)">▶</button>
              <button type="button" class="gbtn" @click="removeNode(n.id)">✕</button>
            </template>
          </div>
          <div class="graph-actions">
            <button type="button" class="gbtn" @click="graphUndo">undo</button>
            <button type="button" class="gbtn" @click="graphRedo">redo</button>
          </div>
        </div>

        <div class="params">
          <label v-for="p in params" :key="p.id" class="param">
            <span class="param-name">{{ p.name }}<em v-if="p.unit"> ({{ p.unit }})</em></span>
            <input
              type="range"
              min="0"
              max="1"
              step="0.001"
              :value="paramValues[p.id]"
              @input="onParam(p.id, ($event.target as HTMLInputElement).valueAsNumber)"
            />
          </label>
        </div>

        <div class="keyboard" @pointerleave="releaseAll">
          <button
            v-for="k in keys"
            :key="k.pitch"
            class="key"
            :class="{ black: k.black, held: held.has(k.pitch) }"
            type="button"
            @pointerdown="pressPointer($event, k.pitch)"
            @pointerenter="$event.buttons === 1 && noteOn(k.pitch)"
            @pointerleave="noteOff(k.pitch)"
            @pointerup="noteOff(k.pitch)"
            @pointercancel="noteOff(k.pitch)"
          ></button>
        </div>
      </div>
    </details>
  </div>
</template>

<style>
:root {
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
  color: #f6f6f6;
  background-color: #1c1c1e;
}

body {
  margin: 0;
}

.app {
  height: 100vh;
  display: flex;
  flex-direction: column;
  box-sizing: border-box;
}

.view {
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.view.grow {
  flex: 1;
}

.view-head {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}

.view-title {
  margin: 0;
  padding: 0.3rem 0.8rem;
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: #8e8e93;
}

.hint {
  font-size: 0.7rem;
  color: #6e6e73;
  margin-left: auto;
  padding-right: 0.8rem;
}

.track-select {
  background: #2c2c2e;
  color: #f6f6f6;
  border: 1px solid #3a3a3c;
  border-radius: 4px;
  font-family: inherit;
  font-size: 0.8rem;
  padding: 0.1rem 0.3rem;
}

.timeline-pane {
  height: 110px;
}

.pianoroll-pane {
  flex: 1;
  min-height: 0;
}

/* --- dev panel --- */
.dev {
  border-top: 1px solid #2c2c2e;
  background: #161618;
  font-size: 0.8rem;
}

.dev > summary {
  cursor: pointer;
  padding: 0.3rem 0.8rem;
  color: #8e8e93;
  user-select: none;
}

.dev-body {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  padding: 0.8rem;
}

.dev-actions {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}

.action-msg {
  font-size: 0.8rem;
  color: #8e8e93;
}

.graph {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
  width: min(640px, 100%);
}

.graph-row {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.graph-row .node {
  flex: 1;
  color: #c7c7cc;
}

.graph-row .node.instrument {
  color: #f6f6f6;
  font-weight: 600;
}

.graph-row em {
  color: #8e8e93;
  font-style: normal;
}

.gbtn {
  padding: 0.15rem 0.5rem;
  border: 1px solid #3a3a3c;
  border-radius: 5px;
  background: #2c2c2e;
  color: #f6f6f6;
  font-family: inherit;
  font-size: 0.8rem;
  cursor: pointer;
}

.graph-actions {
  display: flex;
  gap: 0.4rem;
  margin-top: 0.3rem;
}

.params {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 1rem 2rem;
  width: min(640px, 100%);
}

.param {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}

.param-name {
  color: #c7c7cc;
}

.param-name em {
  color: #8e8e93;
  font-style: normal;
}

.param input {
  width: 100%;
}

.keyboard {
  display: flex;
  height: 7rem;
  touch-action: none;
}

.key {
  width: 2.4rem;
  border: 1px solid #1c1c1e;
  border-radius: 0 0 5px 5px;
  background: #f6f6f6;
  cursor: pointer;
}

.key.black {
  background: #2c2c2e;
}

.key.held {
  background: #2f7d5b;
}
</style>
