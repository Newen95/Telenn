<script setup lang="ts">
// Mixer panel: one strip per track — vertical gain fader (linear in dB) with
// its VU string, pan, plus the master strip. A fader drag streams live values
// (retargeting the smoothed strips on the audio thread) and persists on
// release: one whole gesture = one undo step. Double-click resets a control.
// Clicking a strip makes its track the active one everywhere.

import { computed, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import { useProjectStore, type TrackView } from "../../stores/project";
import { useTelemetryStore } from "../../stores/telemetry";
import { useUiStore } from "../../stores/ui";
import { projectHues } from "../../styles/theme";
import { ballistic } from "../transport/meterBallistics";
import NodeMeter from "../chain/NodeMeter.vue";
import StringMeter from "../../ui/StringMeter.vue";
import { formatDb, formatPan, tailIndex } from "./format";

const { t } = useI18n();
const project = useProjectStore();
const telemetry = useTelemetryStore();
const ui = useUiStore();

const num = (e: Event): number => Number((e.target as HTMLInputElement).value);

/** Collision-free display hue per track id (explicit colors first). */
const hues = computed(() => projectHues(project.tracks));

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
    <div
      v-for="track in project.tracks"
      :key="track.id"
      class="strip"
      :class="{ active: track.id === ui.activeTrackId }"
      @pointerdown="ui.activeTrackId = track.id"
    >
      <!-- Underlined name: the one way a track is named everywhere (piano
           roll header, chain header, here). -->
      <span class="strip-name" :style="{ borderColor: hues.get(track.id) }">
        {{ track.name || track.id }}
      </span>
      <div class="fader-row">
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
        <NodeMeter :index="tailIndex(track.chain)" :size="96" />
      </div>
      <span class="value num">{{ formatDb(track.mixer.gainDb) }}</span>
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
      <span class="value num">{{ formatPan(track.mixer.pan) }}</span>
    </div>

    <div class="strip master">
      <span class="strip-name">{{ t("meter.master") }}</span>
      <div class="fader-row">
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
        <StringMeter :rms="masterLevel" :peak="masterLevel" :size="96" />
      </div>
      <span class="value num">{{ formatDb(project.masterGainDb) }}</span>
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
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--surface);
  cursor: pointer;
  /* Same elevation as the chain cards: a graspable object on the tray. */
  box-shadow: var(--shadow-raised);
  transition: border-color var(--t-fast);
}

/* The active track's strip (the one the chain and piano roll show). */
.strip.active {
  border-color: var(--accent);
}

.strip.master {
  background: var(--panel);
}

.strip-name {
  font-family: var(--font-display);
  font-size: 0.72rem;
  font-weight: 600;
  color: var(--text-2);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  border-bottom: 3px solid transparent; /* colored per track inline */
  border-radius: 0 0 3px 3px; /* rounds the underline's ends */
  max-width: 5.5rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.fader-row {
  display: flex;
  align-items: center;
  gap: 0.3rem;
}

/* Native appearance kept (accent-color themes it); vertical via writing-mode,
   rtl so up = louder. */
.fader {
  writing-mode: vertical-lr;
  direction: rtl;
  height: 96px;
  width: 18px;
  accent-color: var(--accent);
  cursor: pointer;
}

.pan {
  width: 62px;
  accent-color: var(--accent);
  cursor: pointer;
}

.value {
  font-size: 0.68rem;
  color: var(--text-2);
}
</style>
