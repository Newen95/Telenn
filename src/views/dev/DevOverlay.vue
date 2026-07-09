<script setup lang="ts">
// Dev overlay (Ctrl+Alt+D): raw engine controls kept out of the main layout —
// demo reload / WAV export, the flat graph node list with quick edits, raw
// module sliders and a one-octave audition keyboard. Labels stay raw technical
// strings (not i18n), as before.

import { onMounted, onUnmounted, reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

import { useProjectStore } from "../../stores/project";
import Icon from "../../ui/Icon.vue";
import { DEMO_PATH, EXPORT_PATH } from "../../projectPaths";

const emit = defineEmits<{ close: [] }>();

const project = useProjectStore();

const SYNTH_NODE = 0;
const FIRST_PITCH = 60; // middle C

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
const actionMsg = ref("");
let fxCounter = 0;

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

// Computer-keyboard audition is live only while the overlay is open.
function onKeyDown(event: KeyboardEvent) {
  if (event.repeat || event.ctrlKey || event.altKey || event.metaKey) return;
  const offset = KEYMAP[event.code];
  if (offset !== undefined) void noteOn(FIRST_PITCH + offset);
}

function onKeyUp(event: KeyboardEvent) {
  const offset = KEYMAP[event.code];
  if (offset !== undefined) void noteOff(FIRST_PITCH + offset);
}

onMounted(async () => {
  params.value = await invoke<ParamInfo[]>("module_params", { moduleId: "plucked" });
  for (const p of params.value) paramValues[p.id] = p.default;
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  void refreshGraph();
});

onUnmounted(() => {
  releaseAll();
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("keyup", onKeyUp);
});
</script>

<template>
  <div class="dev-overlay">
    <header class="dev-head">
      <span class="dev-title">dev</span>
      <button type="button" class="dev-close" @click="emit('close')">
        <Icon name="close" :size="14" />
      </button>
    </header>

    <div class="dev-body">
      <div class="dev-actions">
        <button type="button" class="gbtn" @click="loadDemo">reload json</button>
        <button type="button" class="gbtn" @click="exportWav">export wav</button>
        <span class="action-msg">{{ actionMsg }}</span>
      </div>

      <div class="graph">
        <div v-for="n in graphNodes" :key="n.id" class="graph-row">
          <span class="gnode" :class="{ instrument: n.isInstrument }">
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
  </div>
</template>

<style scoped>
.dev-overlay {
  position: fixed;
  right: 12px;
  bottom: 12px;
  z-index: 200;
  width: min(680px, calc(100vw - 24px));
  max-height: min(70vh, 640px);
  overflow: auto;
  background: var(--panel);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-overlay);
  font-size: 0.8rem;
}

.dev-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.4rem 0.8rem;
  border-bottom: 1px solid var(--border);
}

.dev-title {
  color: var(--text-2);
  font-family: var(--font-mono);
}

.dev-close {
  border: none;
  background: none;
  padding: 0.2rem;
  color: var(--text-2);
  cursor: pointer;
}

.dev-close:hover {
  color: var(--text);
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
  color: var(--text-2);
}

.gbtn {
  background: var(--surface);
  color: var(--text);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  padding: 0.15rem 0.5rem;
  font-size: 0.75rem;
  cursor: pointer;
}

.gbtn:hover {
  background: var(--surface-2);
}

.graph {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}

.graph-row {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.gnode {
  flex: 1;
  color: var(--text-2);
}

.gnode.instrument {
  color: var(--text);
}

.gnode em {
  color: var(--text-3);
  font-style: normal;
}

.graph-actions {
  display: flex;
  gap: 0.4rem;
}

.params {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 0.5rem 1rem;
}

.param {
  display: flex;
  flex-direction: column;
  gap: 0.15rem;
}

.param-name {
  font-size: 0.7rem;
  color: var(--text-2);
}

.param-name em {
  color: var(--text-3);
  font-style: normal;
}

.keyboard {
  display: flex;
  height: 7rem;
}

.key {
  width: 2.4rem;
  border: 1px solid var(--bg);
  border-radius: 0 0 var(--radius-sm) var(--radius-sm);
  background: var(--text);
  cursor: pointer;
  padding: 0;
}

.key.black {
  background: var(--bg);
  width: 1.6rem;
  height: 60%;
  margin: 0 -0.8rem;
  z-index: 1;
}

.key.held {
  background: var(--accent);
}
</style>
