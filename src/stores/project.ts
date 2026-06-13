//! Pinia store mirroring the project for rendering. Read-only in phase 5a:
//! populated from the `project_view` command (positions already in ticks).
//! Editing actions arrive in 5b.

import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface NoteView {
  pitch: number;
  startTicks: number;
  lengthTicks: number;
  vel: number;
}

export interface ClipView {
  startTicks: number;
  lengthTicks: number;
  notes: NoteView[];
}

export interface TrackView {
  id: string;
  name: string;
  clips: ClipView[];
}

export interface ProjectView {
  bpm: number;
  ppq: number;
  timeSignature: [number, number];
  tracks: TrackView[];
}

export const useProjectStore = defineStore("project", () => {
  const bpm = ref(120);
  const ppq = ref(960);
  const timeSignature = ref<[number, number]>([4, 4]);
  const tracks = ref<TrackView[]>([]);

  async function load(): Promise<void> {
    const view = await invoke<ProjectView>("project_view");
    bpm.value = view.bpm;
    ppq.value = view.ppq;
    timeSignature.value = view.timeSignature;
    tracks.value = view.tracks;
  }

  return { bpm, ppq, timeSignature, tracks, load };
});
