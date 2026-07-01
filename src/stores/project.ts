//! Pinia store mirroring the project for rendering and editing.
//!
//! Populated from the `project_view` command (positions already in ticks). Edits
//! are *optimistic*: the local mirror is mutated immediately (so the view updates
//! on the same frame the gesture commits), the matching Tauri command is sent,
//! then `load()` reconciles against the authoritative project on disk.

import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { flashIntensity, FLASH_MS } from "../views/canvas/flash";

/** Id-keyed diff emitted by the backend after an external `song.json` reload. */
interface ProjectDiff {
  addedNotes: string[];
  removedNotes: string[];
  modifiedNotes: string[];
}

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

export interface ChainNodeView {
  id: string;
  module: string;
  isInstrument: boolean;
  /** Flat graph index (instruments-first layout) → its telemetry meter. */
  flatIndex: number;
  /** Stored parameter values (normalized 0..1) by name. */
  params: Record<string, number>;
}

/** A module parameter as the UI needs it. `default` is normalized 0..1;
 * `min`/`max`/`curve` are the module-declared range so the UI can display the
 * real value (Hz, dB, ms…) behind a normalized slider. */
export interface ParamInfo {
  id: number;
  name: string;
  unit: string;
  default: number;
  min: number;
  max: number;
  curve: "linear" | "log" | "db";
}

/** A track's mixer strip settings (gain in dB, constant-power pan -1..1). */
export interface TrackMixView {
  gainDb: number;
  pan: number;
}

export interface TrackView {
  id: string;
  name: string;
  instrumentNode: number;
  chain: ChainNodeView[];
  mixer: TrackMixView;
  clips: ClipView[];
}

/** A registered module, as the browser lists it. */
export interface ModuleDesc {
  id: string;
  name: string;
  category: "instrument" | "effect";
}

/** A saved effect preset (module + normalized params), without a node id. */
export interface Preset {
  module: string;
  params: Record<string, number>;
}

export interface ProjectView {
  bpm: number;
  ppq: number;
  timeSignature: [number, number];
  masterGainDb: number;
  tracks: TrackView[];
}

export const useProjectStore = defineStore("project", () => {
  const bpm = ref(120);
  const ppq = ref(960);
  const timeSignature = ref<[number, number]>([4, 4]);
  const masterGainDb = ref(0);
  const tracks = ref<TrackView[]>([]);

  async function load(): Promise<void> {
    const view = await invoke<ProjectView>("project_view");
    bpm.value = view.bpm;
    ppq.value = view.ppq;
    timeSignature.value = view.timeSignature;
    masterGainDb.value = view.masterGainDb ?? 0;
    tracks.value = view.tracks;
  }

  // Notes flashing after an external reload: note id → flash start (ms). Plain
  // (non-reactive) — the canvas polls it each frame via `noteFlash`/`anyFlash`.
  const flashStart = new Map<string, number>();
  /** Latest external-edit error, if the changed file failed to parse. */
  const lastError = ref<string | null>(null);

  /** Register the file-watcher listeners: external reloads refresh the view and
   *  flash the notes that changed; a parse error is surfaced, no reload. */
  async function init(): Promise<void> {
    await listen<ProjectDiff>("project_changed", async (e) => {
      await load();
      const now = performance.now();
      for (const id of e.payload.addedNotes) flashStart.set(id, now);
      for (const id of e.payload.modifiedNotes) flashStart.set(id, now);
      lastError.value = null;
    });
    await listen<string>("project_error", (e) => {
      lastError.value = e.payload;
    });
  }

  /** Flash intensity (1 → 0) of a note at time `now`; 0 if not flashing. */
  function noteFlash(id: string, now: number): number {
    const a = flashIntensity(flashStart.get(id), now);
    if (a === 0) flashStart.delete(id); // purge once faded
    return a;
  }

  /** Whether any note is still within its flash window (drives the redraw). */
  function anyFlash(now: number): boolean {
    for (const start of flashStart.values()) {
      if (now - start < FLASH_MS) return true;
    }
    return false;
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

  // --- Effect chain edits (M6). Each is one graph hot-swap; reload reconciles. ---

  /** A fresh, collision-resistant node id (the backend also rejects duplicates). */
  function freshNodeId(): string {
    return `fx-${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6).toString(36)}`;
  }

  /** Move an effect to a chain at `index`, keeping its id (its tail follows). */
  async function moveNode(id: string, toTrack: string, index: number): Promise<void> {
    await invoke("node_move_to", { id, toTrack, index });
    await load();
  }

  /** Copy an effect into a chain at `index` under a fresh id. */
  async function copyNode(id: string, toTrack: string, index: number): Promise<void> {
    await invoke("node_copy", { id, newId: freshNodeId(), toTrack, index });
    await load();
  }

  /** Insert a dropped module/preset as a new effect at `index`. */
  async function insertNode(
    trackId: string,
    index: number,
    module: string,
    params: Record<string, number> = {},
  ): Promise<void> {
    await invoke("node_insert", { trackId, index, module, id: freshNodeId(), params });
    await load();
  }

  async function removeNode(id: string): Promise<void> {
    await invoke("graph_remove_node", { id });
    await load();
  }

  /** Optimistically write a node's param in the local mirror (keeps knobs live). */
  function patchNodeParam(nodeId: string, param: string, value: number): void {
    for (const track of tracks.value) {
      const node = track.chain.find((n) => n.id === nodeId);
      if (node) {
        node.params = { ...node.params, [param]: value };
        return;
      }
    }
  }

  /** Module param specs (name, unit, default), for the chain panel's knobs. */
  const moduleParams = (moduleId: string): Promise<ParamInfo[]> =>
    invoke<ParamInfo[]>("module_params", { moduleId });

  /** Live param change streamed while a knob is dragged (real-time, no persist). */
  async function setNodeParam(nodeId: string, param: string, value: number): Promise<void> {
    patchNodeParam(nodeId, param, value);
    await invoke("node_set_param", { nodeId, param, value });
  }

  /** Persist a settled param value (one undo step); reload reconciles. */
  async function commitNodeParam(nodeId: string, param: string, value: number): Promise<void> {
    patchNodeParam(nodeId, param, value);
    await invoke("node_commit_param", { nodeId, param, value });
    await load();
  }

  /** Live mixer change streamed while a fader/pan is dragged (no persist). */
  async function setTrackMix(trackId: string, gainDb: number, pan: number): Promise<void> {
    const track = tracks.value.find((t) => t.id === trackId);
    if (track) track.mixer = { gainDb, pan };
    await invoke("mixer_set_track", { trackId, gainDb, pan });
  }

  /** Persist a settled mixer gesture (one undo step); reload reconciles. */
  async function commitTrackMix(trackId: string, gainDb: number, pan: number): Promise<void> {
    const track = tracks.value.find((t) => t.id === trackId);
    if (track) track.mixer = { gainDb, pan };
    await invoke("mixer_commit_track", { trackId, gainDb, pan });
    await load();
  }

  /** Live master gain change (dB) streamed while the master fader is dragged. */
  async function setMasterGain(gainDb: number): Promise<void> {
    masterGainDb.value = gainDb;
    await invoke("mixer_set_master", { gainDb });
  }

  /** Persist the settled master gain (one undo step). */
  async function commitMasterGain(gainDb: number): Promise<void> {
    masterGainDb.value = gainDb;
    await invoke("mixer_commit_master", { gainDb });
    await load();
  }

  // Bumped after a preset is saved, so the browser reloads its list.
  const presetsVersion = ref(0);

  const listModules = (): Promise<ModuleDesc[]> => invoke<ModuleDesc[]>("list_modules");
  const listPresets = (): Promise<string[]> => invoke<string[]>("list_presets");
  const readPreset = (name: string): Promise<Preset> => invoke<Preset>("read_preset", { name });
  async function savePreset(name: string, nodeId: string): Promise<void> {
    await invoke("save_preset", { name, nodeId });
    presetsVersion.value += 1;
  }

  return {
    bpm,
    ppq,
    timeSignature,
    masterGainDb,
    tracks,
    presetsVersion,
    lastError,
    init,
    noteFlash,
    anyFlash,
    load,
    addNote,
    moveNote,
    resizeNote,
    deleteNote,
    moveClip,
    resizeClip,
    moveNode,
    copyNode,
    insertNode,
    removeNode,
    moduleParams,
    setNodeParam,
    commitNodeParam,
    setTrackMix,
    commitTrackMix,
    setMasterGain,
    commitMasterGain,
    listModules,
    listPresets,
    readPreset,
    savePreset,
  };
});
