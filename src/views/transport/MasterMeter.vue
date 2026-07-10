<script setup lang="ts">
// Master VU string: instant attack, ~300 ms linear release, driven by the
// telemetry store's master peak (the max of both stereo channels).

import { onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import { useTelemetryStore } from "../../stores/telemetry";
import { ballistic } from "./meterBallistics";
import StringMeter from "../../ui/StringMeter.vue";

const { t } = useI18n();
const telemetry = useTelemetryStore();
const level = ref(0);
let raf = 0;
let last = performance.now();

function loop(): void {
  const now = performance.now();
  const dt = (now - last) / 1000;
  last = now;
  level.value = ballistic(level.value, telemetry.masterPeak, dt);
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
    <StringMeter :rms="level" :peak="level" :size="90" orientation="horizontal" />
  </div>
</template>

<style scoped>
.meter {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.72rem;
  color: var(--text-2);
}
</style>
