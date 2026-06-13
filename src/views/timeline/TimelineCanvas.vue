<script setup lang="ts">
// Timeline: lanes are tracks, blocks are clips. Thin mapping over GridView.

import { computed } from "vue";

import { useProjectStore } from "../../stores/project";
import GridView from "../canvas/GridView.vue";
import type { Block, Grid } from "../canvas/renderer";

const LANE_HEIGHT = 28;
const TRACK_COLORS = ["#3f7d5b", "#5b6f9e", "#9e6f5b", "#7d5b9e"];

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
      });
    }
  });
  return out;
});

const grid = computed<Grid>(() => ({
  beatsPerBar: project.timeSignature[0],
  ticksPerBeat: (project.ppq * 4) / project.timeSignature[1],
}));
</script>

<template>
  <GridView
    :blocks="blocks"
    :lane-count="Math.max(project.tracks.length, 1)"
    :lane-height="LANE_HEIGHT"
    :grid="grid"
  />
</template>
