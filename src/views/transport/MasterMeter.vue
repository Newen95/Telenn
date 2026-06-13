<script setup lang="ts">
// Master VU: instant attack, ~300 ms linear release, driven by the telemetry
// store's master peak.

import { onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import { useTelemetryStore } from "../../stores/telemetry";

const RELEASE_S = 0.3;

const { t } = useI18n();
const telemetry = useTelemetryStore();
const level = ref(0);
let raf = 0;
let last = performance.now();

function loop(): void {
  const now = performance.now();
  const dt = (now - last) / 1000;
  last = now;
  const peak = telemetry.masterPeak;
  if (peak > level.value) {
    level.value = peak; // instant attack
  } else {
    level.value = Math.max(0, level.value - dt / RELEASE_S); // linear release
  }
  raf = requestAnimationFrame(loop);
}

onMounted(() => {
  raf = requestAnimationFrame(loop);
});
onUnmounted(() => cancelAnimationFrame(raf));
</script>

<template>
  <div class="meter">
    <span class="label">{{ t("meter.master") }}</span>
    <div class="bar">
      <div class="fill" :style="{ width: Math.min(100, level * 100) + '%' }"></div>
    </div>
  </div>
</template>

<style scoped>
.meter {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.75rem;
  color: #8e8e93;
}

.bar {
  width: 140px;
  height: 10px;
  background: #2c2c2e;
  border-radius: 3px;
  overflow: hidden;
}

.fill {
  height: 100%;
  background: linear-gradient(90deg, #2f7d5b, #e0b050);
  transition: none;
}
</style>
