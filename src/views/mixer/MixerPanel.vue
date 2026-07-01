<script setup lang="ts">
// Mixer panel: one strip per track — vertical gain fader (linear in dB), pan,
// and a VU on the track's chain tail — plus the master strip. A fader drag
// streams live values (retargeting the smoothed strips on the audio thread)
// and persists on release: one whole gesture = one undo step. Double-click
// resets a control to neutral.

import { onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import { useProjectStore, type TrackView } from "../../stores/project";
import { useTelemetryStore } from "../../stores/telemetry";
import { ballistic } from "../transport/meterBallistics";
import NodeMeter from "../chain/NodeMeter.vue";
import { formatDb, formatPan, tailIndex } from "./format";

const { t } = useI18n();
const project = useProjectStore();
const telemetry = useTelemetryStore();

const num = (e: Event): number => Number((e.target as HTMLInputElement).value);

function onGain(track: TrackView, e: Event): void {
  void project.setTrackMix(track.id, num(e), track.mixer.pan);
}
function onGainCommit(track: TrackView, e: Event): void {
  void project.commitTrackMix(track.id, num(e), track.mixer.pan);
}
function onPan(track: TrackView, e: Event): void {
  void project.setTrackMix(track.id, track.mixer.gainDb, num(e));
}
function onPanCommit(track: TrackView, e: Event): void {
  void project.commitTrackMix(track.id, track.mixer.gainDb, num(e));
}
function resetGain(track: TrackView): void {
  void project.commitTrackMix(track.id, 0, track.mixer.pan);
}
function resetPan(track: TrackView): void {
  void project.commitTrackMix(track.id, track.mixer.gainDb, 0);
}

function onMaster(e: Event): void {
  void project.setMasterGain(num(e));
}
function onMasterCommit(e: Event): void {
  void project.commitMasterGain(num(e));
}
function resetMaster(): void {
  void project.commitMasterGain(0);
}

// Master VU (peak of the stereo mix), same ballistics as every other meter.
const masterLevel = ref(0);
let raf = 0;
let last = performance.now();
function loop(): void {
  const now = performance.now();
  masterLevel.value = ballistic(masterLevel.value, telemetry.masterPeak, (now - last) / 1000);
  last = now;
  raf = requestAnimationFrame(loop);
}
onMounted(() => {
  raf = requestAnimationFrame(loop);
});
onUnmounted(() => cancelAnimationFrame(raf));
</script>

<template>
  <div class="mixer-panel">
    <div v-for="track in project.tracks" :key="track.id" class="strip">
      <span class="strip-name">{{ track.name || track.id }}</span>
      <input
        class="fader"
        type="range"
        min="-60"
        max="6"
        step="0.1"
        :value="track.mixer.gainDb"
        @input="onGain(track, $event)"
        @change="onGainCommit(track, $event)"
        @dblclick="resetGain(track)"
      />
      <span class="value">{{ formatDb(track.mixer.gainDb) }}</span>
      <input
        class="pan"
        type="range"
        min="-1"
        max="1"
        step="0.01"
        :value="track.mixer.pan"
        @input="onPan(track, $event)"
        @change="onPanCommit(track, $event)"
        @dblclick="resetPan(track)"
      />
      <span class="value">{{ formatPan(track.mixer.pan) }}</span>
      <NodeMeter :index="tailIndex(track.chain)" />
    </div>

    <div class="strip master">
      <span class="strip-name">{{ t("meter.master") }}</span>
      <input
        class="fader"
        type="range"
        min="-60"
        max="6"
        step="0.1"
        :value="project.masterGainDb"
        @input="onMaster($event)"
        @change="onMasterCommit($event)"
        @dblclick="resetMaster()"
      />
      <span class="value">{{ formatDb(project.masterGainDb) }}</span>
      <div class="vu" :class="{ clip: masterLevel >= 1 }">
        <div class="fill" :style="{ width: Math.min(100, masterLevel * 100) + '%' }"></div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.mixer-panel {
  display: flex;
  align-items: stretch;
  gap: 0.5rem;
  padding: 0.4rem 0.6rem;
  overflow-x: auto;
  scrollbar-width: thin;
}

.strip {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.35rem;
  padding: 0.45rem 0.6rem;
  min-width: 4.6rem;
  border: 1px solid #3a3a3c;
  border-radius: 6px;
  background: #2c2c2e;
}

.strip.master {
  background: #20303a;
  border-color: #2f5a6e;
}

.strip-name {
  font-size: 0.72rem;
  color: #c7c7cc;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  max-width: 5.5rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Native appearance kept (accent-color themes it); vertical via writing-mode,
   rtl so up = louder. */
.fader {
  writing-mode: vertical-lr;
  direction: rtl;
  height: 110px;
  width: 18px;
  accent-color: #5fae7f;
  cursor: pointer;
}

.pan {
  width: 62px;
  accent-color: #5fae7f;
  cursor: pointer;
}

.value {
  font-size: 0.68rem;
  color: #8e8e93;
  font-variant-numeric: tabular-nums;
}

.vu {
  position: relative;
  width: 44px;
  height: 6px;
  background: #1c1c1e;
  border-radius: 2px;
  overflow: hidden;
}

.vu .fill {
  height: 100%;
  background: linear-gradient(90deg, #2f7d5b, #e0b050);
}

.vu.clip .fill {
  background: #e0564f;
}
</style>
