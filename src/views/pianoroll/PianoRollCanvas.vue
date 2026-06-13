<script setup lang="ts">
// Piano roll: lanes are pitches (top = high), blocks are the selected track's
// notes. Thin mapping over GridView; only the lane semantics differ.

import { computed } from "vue";

import { useProjectStore } from "../../stores/project";
import GridView from "../canvas/GridView.vue";
import type { Block, Grid } from "../canvas/renderer";

const LANE_HEIGHT = 10;
const PITCHES = 128;
const NOTE_COLOR = "#5fae7f";

const props = defineProps<{ trackId: string }>();
const project = useProjectStore();

const track = computed(() => project.tracks.find((t) => t.id === props.trackId));

/** Lane = pitch flipped so high notes sit at the top. */
function pitchToLane(pitch: number): number {
  return PITCHES - 1 - pitch;
}

const blocks = computed<Block[]>(() => {
  const out: Block[] = [];
  for (const clip of track.value?.clips ?? []) {
    for (const note of clip.notes) {
      out.push({
        startTick: note.startTicks,
        lenTick: note.lengthTicks,
        lane: pitchToLane(note.pitch),
        color: NOTE_COLOR,
      });
    }
  }
  return out;
});

const grid = computed<Grid>(() => ({
  beatsPerBar: project.timeSignature[0],
  ticksPerBeat: (project.ppq * 4) / project.timeSignature[1],
}));

// Start scrolled just above the highest note of the track (default ~C6).
const initialScrollLane = computed(() => {
  let highest = 84;
  for (const clip of track.value?.clips ?? []) {
    for (const note of clip.notes) highest = Math.max(highest, note.pitch);
  }
  return Math.max(0, pitchToLane(highest) - 2);
});
</script>

<template>
  <GridView
    :blocks="blocks"
    :lane-count="PITCHES"
    :lane-height="LANE_HEIGHT"
    :grid="grid"
    :initial-scroll-lane="initialScrollLane"
  />
</template>
