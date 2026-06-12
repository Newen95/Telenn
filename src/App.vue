<script setup lang="ts">
import { onMounted, onUnmounted, reactive, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

// Dev UI for M2: a one-octave keyboard playing the subtractive synth (node 0)
// and a slider per declared parameter. Glyphs and raw technical labels on
// purpose — vue-i18n lands with the real UI at M5.

const SYNTH_NODE = 0;
const FIRST_PITCH = 60; // middle C

// Computer-keyboard layout, by physical key code (layout-independent, so it
// works on AZERTY too). Lets you hold a note on the keyboard while sweeping a
// slider with the mouse. White keys on the home row, black keys above.
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

interface EngineStatus {
  xruns: number;
  sampleRate: number;
}

// One octave of keys (C to C). `black` drives styling only.
const keys = Array.from({ length: 13 }, (_, i) => ({
  pitch: FIRST_PITCH + i,
  black: [1, 3, 6, 8, 10].includes(i % 12),
}));

const held = reactive(new Set<number>());
const params = ref<ParamInfo[]>([]);
const paramValues = reactive<Record<number, number>>({});

const xruns = ref(0);
const sampleRate = ref(0);
let statusTimer: number | undefined;

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
  // Snapshot: noteOff mutates `held` while we iterate.
  for (const pitch of [...held]) {
    void noteOff(pitch);
  }
}

// Press a key with the pointer. Release the implicit pointer capture so that
// dragging fires enter/leave on the *other* keys (glissando), one note at a time.
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

function onKeyDown(event: KeyboardEvent) {
  if (event.repeat) return;
  const offset = KEYMAP[event.code];
  if (offset !== undefined) void noteOn(FIRST_PITCH + offset);
}

function onKeyUp(event: KeyboardEvent) {
  const offset = KEYMAP[event.code];
  if (offset !== undefined) void noteOff(FIRST_PITCH + offset);
}

async function refreshStatus() {
  const status = await invoke<EngineStatus>("engine_status");
  xruns.value = status.xruns;
  sampleRate.value = status.sampleRate;
}

onMounted(async () => {
  params.value = await invoke<ParamInfo[]>("module_params", {
    moduleId: "subtractive",
  });
  for (const p of params.value) {
    paramValues[p.id] = p.default;
  }
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  void refreshStatus();
  statusTimer = window.setInterval(refreshStatus, 1000);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("keyup", onKeyUp);
  if (statusTimer !== undefined) {
    window.clearInterval(statusTimer);
  }
});
</script>

<template>
  <main class="app">
    <section class="params">
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
    </section>

    <section class="keyboard" @pointerleave="releaseAll">
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
    </section>

    <dl class="status">
      <div><dt>kHz</dt><dd>{{ (sampleRate / 1000).toFixed(1) }}</dd></div>
      <div><dt>xruns</dt><dd>{{ xruns }}</dd></div>
    </dl>
  </main>
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
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2rem;
  padding: 2rem;
  box-sizing: border-box;
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
  font-size: 0.8rem;
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
  height: 9rem;
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

.status {
  display: flex;
  gap: 2rem;
  margin: 0;
  font-variant-numeric: tabular-nums;
}

.status div {
  text-align: center;
}

.status dt {
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: #8e8e93;
}

.status dd {
  margin: 0.2rem 0 0;
  font-size: 1.3rem;
}
</style>
