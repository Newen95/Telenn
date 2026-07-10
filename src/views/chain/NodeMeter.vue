<script setup lang="ts">
// Per-node VU string, driven by the telemetry store's per-node meter (peak +
// RMS, indexed by flat graph index). Instant attack, ~300 ms linear release.
// The value comes from the audio thread — never a faked level.

import { onMounted, onUnmounted, ref } from "vue";

import { useTelemetryStore } from "../../stores/telemetry";
import { ballistic } from "../transport/meterBallistics";
import StringMeter from "../../ui/StringMeter.vue";

const props = withDefaults(defineProps<{ index: number; size?: number }>(), { size: 20 });

const telemetry = useTelemetryStore();
const rms = ref(0);
const peak = ref(0);
let raf = 0;
let last = performance.now();

function loop(): void {
  const now = performance.now();
  const dt = (now - last) / 1000;
  last = now;
  const m = telemetry.meterAt(props.index);
  rms.value = ballistic(rms.value, m.rms, dt);
  peak.value = ballistic(peak.value, m.peak, dt);
  raf = requestAnimationFrame(loop);
}

onMounted(() => {
  raf = requestAnimationFrame(loop);
});
onUnmounted(() => cancelAnimationFrame(raf));
</script>

<template>
  <StringMeter :rms="rms" :peak="peak" :size="size" />
</template>
