<script setup lang="ts">
// Per-node VU, driven by the telemetry store's per-node meter (peak + RMS,
// indexed by flat graph index). Same ballistics as the master meter: instant
// attack, ~300 ms linear release. The RMS fills the bar; the peak rides as a
// thin tick so transients stay visible. The value comes from the audio thread —
// never a faked level.

import { onMounted, onUnmounted, ref } from "vue";

import { useTelemetryStore } from "../../stores/telemetry";
import { ballistic } from "../transport/meterBallistics";

const props = defineProps<{ index: number }>();

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
  <div class="node-meter" :class="{ clip: peak >= 1 }">
    <div class="rms" :style="{ width: Math.min(100, rms * 100) + '%' }"></div>
    <div class="peak" :style="{ left: Math.min(100, peak * 100) + '%' }"></div>
  </div>
</template>

<style scoped>
.node-meter {
  position: relative;
  width: 44px;
  height: 6px;
  background: #1c1c1e;
  border-radius: 2px;
  overflow: hidden;
}

.rms {
  position: absolute;
  inset: 0 auto 0 0;
  background: linear-gradient(90deg, #2f7d5b, #e0b050);
}

.peak {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  background: #f6f6f6;
  transform: translateX(-1px);
}

/* The peak reached full scale this release window: flag a clip. */
.node-meter.clip .peak {
  background: #e0564f;
}
</style>
