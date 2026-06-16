<script setup lang="ts">
// Timeline: lanes are tracks, blocks are clips. Thin mapping over GridView. Clip
// editing is light in 5b: a clip can be moved (it carries its notes) or resized
// (notes untouched); clips do not change track, and create/delete come later.

import { computed } from "vue";

import { useProjectStore } from "../../stores/project";
import GridView from "../canvas/GridView.vue";
import type { EditAdapter } from "../canvas/interactions";
import type { Block, Grid } from "../canvas/renderer";
import type { TimeAxis } from "../canvas/timeAxis";

const LANE_HEIGHT = 28;
const TRACK_COLORS = ["#3f7d5b", "#5b6f9e", "#9e6f5b", "#7d5b9e"];

defineProps<{ axis?: TimeAxis }>();
const project = useProjectStore();

const blocks = computed<Block[]>(() => {
  const out: Block[] = [];
  project.tracks.forEach((track, lane) => {
    for (const clip of track.clips) {
      out.push({
        startTick: clip.startTicks,
        lenTick: clip.lengthTicks,
        lane,
        color: TRACK_COLORS[lane % TRACK_COLORS.length],
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

const adapter = computed<EditAdapter>(() => ({
  laneMovable: false, // clips stay on their track in 5b (cross-track move is M6)
  resizableLeft: false, // clip start moves via a clip move; resize is length-only
  minLen: ticksPerBeat.value,
  // Creating/deleting clips is out of scope for 5b's light timeline editing.
  create() {},
  remove() {},
  move(block, startTick) {
    void project.moveClip(String(block.ref), startTick - block.startTick);
  },
  resize(block, _startTick, lenTick) {
    void project.resizeClip(String(block.ref), lenTick);
  },
}));
</script>

<template>
  <GridView
    :blocks="blocks"
    :lane-count="Math.max(project.tracks.length, 1)"
    :lane-height="LANE_HEIGHT"
    :grid="grid"
    :adapter="adapter"
    :axis="axis"
  />
</template>
