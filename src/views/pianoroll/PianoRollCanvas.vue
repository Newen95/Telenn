<script setup lang="ts">
// Piano roll: lanes are pitches (top = high), blocks are the selected track's
// notes. Thin mapping over GridView; only the lane semantics differ. The editing
// gestures are the shared machine in GridView; here we only translate generic
// (tick, lane) edits into note commands and play an audio echo on creation.

import { computed, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";

import { useProjectStore } from "../../stores/project";
import GridView, { type GutterSpec } from "../canvas/GridView.vue";
import type { EditAdapter } from "../canvas/interactions";
import type { Block, Grid } from "../canvas/renderer";
import { GUTTER_WIDTH, type TimeAxis } from "../canvas/timeAxis";
import { usePreviewVoices } from "../canvas/previewVoices";
import { projectHues, theme } from "../../styles/theme";

const LANE_HEIGHT = 12;
const PITCHES = 128;
/** Audio echo length when a note is created or auditioned, in milliseconds. */
const ECHO_MS = 150;
/** Semitone offsets that are black keys within an octave. */
const BLACK_KEYS = new Set([1, 3, 6, 8, 10]);

const props = defineProps<{ trackId: string; axis?: TimeAxis }>();
const project = useProjectStore();
// The single preview voice: owns both the audio (note on/off via Tauri) and the
// gutter highlight. Monophonic and node-aware, so it never leaves a note ringing.
const voices = usePreviewVoices({
  on: (node, pitch) => void invoke("note_on", { node, pitch, vel: 100 }),
  off: (node, pitch) => void invoke("note_off", { node, pitch }),
});

const track = computed(() => project.tracks.find((t) => t.id === props.trackId));

/** Lane = pitch flipped so high notes sit at the top (and its inverse). */
function pitchToLane(pitch: number): number {
  return PITCHES - 1 - pitch;
}
function laneToPitch(lane: number): number {
  return PITCHES - 1 - lane;
}

// Notes wear the track's hue: the same color links this track in the
// timeline, the chain header, the mixer strip and here.
const noteColor = computed(
  () => projectHues(project.tracks).get(props.trackId) ?? theme.color.accent,
);

const blocks = computed<Block[]>(() => {
  const out: Block[] = [];
  for (const clip of track.value?.clips ?? []) {
    for (const note of clip.notes) {
      out.push({
        startTick: note.startTicks,
        lenTick: note.lengthTicks,
        lane: pitchToLane(note.pitch),
        color: noteColor.value,
        ref: note.id,
      });
    }
  }
  return out;
});

const grid = computed<Grid>(() => ({
  beatsPerBar: project.timeSignature[0],
  ticksPerBeat: (project.ppq * 4) / project.timeSignature[1],
}));

const ticksPerBeat = computed(() => (project.ppq * 4) / project.timeSignature[1]);

/** Clip that contains `tick`, else the track's first clip (notes land there). */
function clipAt(tick: number) {
  const clips = track.value?.clips ?? [];
  return clips.find((c) => tick >= c.startTicks && tick < c.startTicks + c.lengthTicks) ?? clips[0];
}

/** Briefly sound a pitch on the track's instrument: the punctual preview used by
 *  note creation, note selection and the vertical drag audition (never touches
 *  song.json). Monophonic — replaces any pitch currently sounding. */
function echo(pitch: number): void {
  const node = track.value?.instrumentNode;
  if (node === undefined) return;
  voices.strike(node, pitch, ECHO_MS);
}

// Leaving the view (track switch, close) silences any voice still sounding.
onUnmounted(() => voices.release());

/** Note name for the C of a pitch's octave (MIDI 60 = C4). */
function octaveLabel(pitch: number): string {
  return `C${Math.floor(pitch / 12) - 1}`;
}

/** Keyboard gutter: visual pitch reference + non-destructive audition on click. */
const gutter = computed<GutterSpec>(() => ({
  width: GUTTER_WIDTH,
  lane(lane) {
    const pitch = laneToPitch(lane);
    const semitone = ((pitch % 12) + 12) % 12;
    return {
      dark: BLACK_KEYS.has(semitone),
      label: semitone === 0 ? octaveLabel(pitch) : undefined,
      lit: voices.has(pitch),
    };
  },
  press(lane) {
    const node = track.value?.instrumentNode;
    if (node !== undefined) voices.hold(node, laneToPitch(lane)); // held while pressed
  },
  release() {
    voices.release();
  },
  sounding: () => voices.any(),
}));

const adapter = computed<EditAdapter>(() => ({
  laneMovable: true,
  resizableLeft: true,
  minLen: Math.round(ticksPerBeat.value / 8),
  create(tick, lane) {
    const clip = clipAt(tick);
    const t = track.value;
    if (!clip || !t) return;
    const pitch = laneToPitch(lane);
    void project.addNote(t.id, clip.id, pitch, tick, Math.round(ticksPerBeat.value / 2), 100);
    echo(pitch);
  },
  move(block, startTick, lane) {
    void project.moveNote(String(block.ref), startTick, laneToPitch(lane));
  },
  previewLane(lane) {
    echo(laneToPitch(lane)); // audition the pitch crossed during a vertical drag
  },
  resize(block, startTick, lenTick) {
    void project.resizeNote(String(block.ref), startTick, lenTick);
  },
  remove(block) {
    void project.deleteNote(String(block.ref));
  },
}));

// Start scrolled just above the track's highest note (a couple of lanes of
// headroom), so a low track like a bass lands on its notes rather than on C6.
// Falls back to ~C6 only when the track has no notes at all.
const initialScrollLane = computed(() => {
  let highest: number | null = null;
  for (const clip of track.value?.clips ?? []) {
    for (const note of clip.notes) {
      highest = highest === null ? note.pitch : Math.max(highest, note.pitch);
    }
  }
  return Math.max(0, pitchToLane(highest ?? 84) - 2);
});
</script>

<template>
  <GridView
    :blocks="blocks"
    :lane-count="PITCHES"
    :lane-height="LANE_HEIGHT"
    :grid="grid"
    :initial-scroll-lane="initialScrollLane"
    :adapter="adapter"
    :gutter="gutter"
    :axis="axis"
  />
</template>
