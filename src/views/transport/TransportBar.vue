<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";

import MasterMeter from "./MasterMeter.vue";

const { t } = useI18n();
const playing = ref(false);

async function play(): Promise<void> {
  await invoke("transport_play");
  playing.value = true;
}

async function stop(): Promise<void> {
  await invoke("transport_stop");
  playing.value = false;
}
</script>

<template>
  <header class="transport">
    <button type="button" class="tbtn" :class="{ active: playing }" @click="play">
      ▶ {{ t("transport.play") }}
    </button>
    <button type="button" class="tbtn" @click="stop">⏹ {{ t("transport.stop") }}</button>
    <MasterMeter />
  </header>
</template>

<style scoped>
.transport {
  display: flex;
  align-items: center;
  gap: 0.8rem;
  padding: 0.5rem 0.8rem;
  background: #1c1c1e;
  border-bottom: 1px solid #2c2c2e;
}

.tbtn {
  padding: 0.35rem 0.7rem;
  border: 1px solid #3a3a3c;
  border-radius: 6px;
  background: #2c2c2e;
  color: #f6f6f6;
  font-family: inherit;
  font-size: 0.9rem;
  cursor: pointer;
}

.tbtn.active {
  background: #2f7d5b;
  border-color: #2f7d5b;
}
</style>
