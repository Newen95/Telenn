//! Pinia store mirroring the project for rendering and editing.
//!
//! Populated from the `project_view` command (positions already in ticks). Edits
//! are *optimistic*: the local mirror is mutated immediately (so the view updates
//! on the same frame the gesture commits), the matching Tauri command is sent,
//! then `load()` reconciles against the authoritative project on disk.

import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

export interface NoteView {
  id: string;
  pitch: number;
  startTicks: number;
  lengthTicks: number;
  vel: number;
}

export interface ClipView {
  id: string;
  startTicks: number;
  lengthTicks: number;
  notes: NoteView[];
}

export interface TrackView {
  id: string;
  name: string;
  instrumentNode: number;
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

  /** Apply `fn` to the note with `id`, wherever it lives (optimistic mirror). */
  function patchNote(id: string, fn: (n: NoteView) => void): void {
    for (const track of tracks.value) {
      for (const clip of track.clips) {
        const note = clip.notes.find((n) => n.id === id);
        if (note) {
          fn(note);
          return;
        }
      }
    }
  }

  function findClip(id: string): ClipView | undefined {
    for (const track of tracks.value) {
      const clip = track.clips.find((c) => c.id === id);
      if (clip) return clip;
    }
    return undefined;
  }

  async function addNote(
    trackId: string,
    clipId: string,
    pitch: number,
    startTicks: number,
    lengthTicks: number,
    vel: number,
  ): Promise<void> {
    // No id yet (the backend assigns it); reload picks it up.
    await invoke("note_add", { trackId, clipId, pitch, startTicks, lengthTicks, vel });
    await load();
  }

  async function moveNote(id: string, startTicks: number, pitch: number): Promise<void> {
    patchNote(id, (n) => {
      n.startTicks = startTicks;
      n.pitch = pitch;
    });
    await invoke("note_move", { id, startTicks, pitch });
    await load();
  }

  async function resizeNote(id: string, startTicks: number, lengthTicks: number): Promise<void> {
    patchNote(id, (n) => {
      n.startTicks = startTicks;
      n.lengthTicks = lengthTicks;
    });
    await invoke("note_resize", { id, startTicks, lengthTicks });
    await load();
  }

  async function deleteNote(id: string): Promise<void> {
    for (const track of tracks.value) {
      for (const clip of track.clips) {
        const i = clip.notes.findIndex((n) => n.id === id);
        if (i !== -1) {
          clip.notes.splice(i, 1);
          break;
        }
      }
    }
    await invoke("note_delete", { id });
    await load();
  }

  async function moveClip(id: string, deltaTicks: number): Promise<void> {
    const clip = findClip(id);
    if (clip) {
      const applied = Math.max(deltaTicks, -clip.startTicks);
      clip.startTicks += applied;
      for (const n of clip.notes) n.startTicks = Math.max(0, n.startTicks + applied);
    }
    await invoke("clip_move", { id, deltaTicks });
    await load();
  }

  async function resizeClip(id: string, lengthTicks: number): Promise<void> {
    const clip = findClip(id);
    if (clip) clip.lengthTicks = lengthTicks;
    await invoke("clip_resize", { id, lengthTicks });
    await load();
  }

  return {
    bpm,
    ppq,
    timeSignature,
    tracks,
    load,
    addNote,
    moveNote,
    resizeNote,
    deleteNote,
    moveClip,
    resizeClip,
  };
});
