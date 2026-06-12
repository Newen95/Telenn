<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

// Dev transport for M1 ("le bip"). Glyph-only controls and numeric readouts on
// purpose: no translatable copy is hard-coded here, vue-i18n lands with the real
// UI at M5.

interface EngineStatus {
  xruns: number;
  sampleRate: number;
}

const playing = ref(false);
const xruns = ref(0);
const sampleRate = ref(0);

let statusTimer: number | undefined;

async function toggle() {
  if (playing.value) {
    await invoke("transport_stop");
    playing.value = false;
  } else {
    await invoke("transport_play");
    playing.value = true;
  }
}

async function refreshStatus() {
  const status = await invoke<EngineStatus>("engine_status");
  xruns.value = status.xruns;
  sampleRate.value = status.sampleRate;
}

onMounted(() => {
  void refreshStatus();
  statusTimer = window.setInterval(refreshStatus, 1000);
});

onUnmounted(() => {
  if (statusTimer !== undefined) {
    window.clearInterval(statusTimer);
  }
});
</script>

<template>
  <main class="transport">
    <button
      class="play"
      :class="{ active: playing }"
      type="button"
      @click="toggle"
    >
      {{ playing ? "⏹" : "▶" }}
    </button>
    <dl class="status">
      <div>
        <dt>kHz</dt>
        <dd>{{ (sampleRate / 1000).toFixed(1) }}</dd>
      </div>
      <div>
        <dt>xruns</dt>
        <dd>{{ xruns }}</dd>
      </div>
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

.transport {
  height: 100vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 1.5rem;
}

.play {
  width: 5rem;
  height: 5rem;
  border-radius: 50%;
  border: none;
  font-size: 2rem;
  line-height: 1;
  color: #f6f6f6;
  background-color: #2c2c2e;
  cursor: pointer;
  transition: background-color 0.15s, transform 0.05s;
}

.play:hover {
  background-color: #3a3a3c;
}

.play:active {
  transform: scale(0.96);
}

.play.active {
  background-color: #2f7d5b;
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
