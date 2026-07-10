<script setup lang="ts">
// Timeline: lanes are tracks, blocks are clips. Thin mapping over GridView.
// The left gutter shows the track names (hue chip + active highlight); a press
// there selects the track everywhere. The "+" under the names creates a track
// from an instrument picker. Clip editing stays light: move (carries notes)
// and length-resize; clips do not change track.

import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { useProjectStore, type ModuleDesc } from "../../stores/project";
import { useUiStore } from "../../stores/ui";
import { projectHues, theme } from "../../styles/theme";
import GridView, { type GutterSpec } from "../canvas/GridView.vue";
import type { EditAdapter } from "../canvas/interactions";
import type { Block, Grid } from "../canvas/renderer";
import { GUTTER_WIDTH, type TimeAxis } from "../canvas/timeAxis";
import Icon from "../../ui/Icon.vue";
import InstrumentMenu from "../../ui/InstrumentMenu.vue";

const LANE_HEIGHT = 28;

defineProps<{ axis?: TimeAxis }>();
const { t } = useI18n();
const project = useProjectStore();
const ui = useUiStore();

/** Collision-free display hue per track id (explicit colors first). */
const hues = computed(() => projectHues(project.tracks));

const blocks = computed<Block[]>(() => {
  const out: Block[] = [];
  project.tracks.forEach((track, lane) => {
    for (const clip of track.clips) {
      out.push({
        startTick: clip.startTicks,
        lenTick: clip.lengthTicks,
        lane,
        color: hues.value.get(track.id) ?? theme.color.panel,
        ref: clip.id,
      });
    }
  });
  return out;
});

const grid = computed<Grid>(() => ({
  beatsPerBar: project.timeSignature[0],
  ticksPerBeat: (project.ppq * 4) / project.timeSignature[1],
}));

const ticksPerBeat = computed(() => (project.ppq * 4) / project.timeSignature[1]);

/** Track-name gutter: a press selects the track across every view. */
const gutter = computed<GutterSpec>(() => ({
  kind: "labels",
  width: GUTTER_WIDTH,
  lane: (lane: number) => {
    const track = project.tracks[lane];
    return track
      ? {
          label: track.name || track.id,
          color: hues.value.get(track.id) ?? theme.color.panel,
          active: track.id === ui.activeTrackId,
        }
      : { label: "", color: theme.color.panel };
  },
  press: (lane: number) => {
    const track = project.tracks[lane];
    if (track) ui.activeTrackId = track.id;
  },
  release: () => {},
  // A vertical drag on a track name reorders the project's tracks.
  reorder: (from: number, to: number) => {
    const track = project.tracks[from];
    if (track) void project.moveTrack(track.id, to);
  },
}));

const adapter = computed<EditAdapter>(() => ({
  laneMovable: false, // clips stay on their track in 5b (cross-track move is M6)
  resizableLeft: false, // clip start moves via a clip move; resize is length-only
  showSelection: false, // a band click selects the TRACK; clips edit by direct drag
  minLen: ticksPerBeat.value,
  // Creating/deleting clips is out of scope for the light timeline editing.
  create() {},
  remove() {},
  move(block, startTick) {
    void project.moveClip(String(block.ref), startTick - block.startTick);
  },
  resize(block, _startTick, lenTick) {
    void project.resizeClip(String(block.ref), lenTick);
  },
}));

/** Any press on a track's band (clip or empty space) selects that track. */
function selectLane(lane: number): void {
  const track = project.tracks[lane];
  if (track) ui.activeTrackId = track.id;
}

/** The active track's lane, tinted across the whole band. */
const activeLane = computed(() => project.tracks.findIndex((tr) => tr.id === ui.activeTrackId));

// "+ track": picker anchored under the gutter, then select the new track.
const adding = ref(false);

async function addTrack(m: ModuleDesc): Promise<void> {
  adding.value = false;
  await project.addTrack(m.id, m.name);
  const last = project.tracks[project.tracks.length - 1];
  if (last) ui.activeTrackId = last.id;
}
</script>

<template>
  <div class="timeline">
    <GridView
      :blocks="blocks"
      :lane-count="Math.max(project.tracks.length, 1)"
      :lane-height="LANE_HEIGHT"
      :grid="grid"
      :adapter="adapter"
      :gutter="gutter"
      :axis="axis"
      :on-lane-press="selectLane"
      :active-lane="activeLane"
    />
    <div class="add-track" :style="{ top: `${project.tracks.length * LANE_HEIGHT + 4}px` }">
      <button type="button" class="add-btn" :title="t('timeline.addTrack')" @click="adding = !adding">
        <Icon name="plus" :size="12" />
      </button>
      <InstrumentMenu v-if="adding" class="menu" @pick="addTrack" @close="adding = false" />
    </div>
  </div>
</template>

<style scoped>
.timeline {
  position: relative;
  width: 100%;
  height: 100%;
}

/* Pinned under the last track row, inside the gutter column. */
.add-track {
  position: absolute;
  left: 6px;
}

.add-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  background: var(--surface);
  color: var(--text-2);
  cursor: pointer;
  transition: color var(--t-fast), border-color var(--t-fast);
}

.add-btn:hover {
  color: var(--accent-bright);
  border-color: var(--accent);
}

.menu {
  top: 24px;
  left: 0;
}
</style>
