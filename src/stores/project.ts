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
import { projectHues, unusedHue } from "../styles/theme";

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
  /** Persisted display hue (hex), when the track has one (see trackHue). */
  color?: string | null;
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

/** A saved chain preset — a named, reusable instrument: `nodes[0]` is the
 * instrument, the rest its effects in order. No ids: fresh ones are minted
 * when the chain is instantiated. */
export interface ChainPreset {
  nodes: Preset[];
}

export interface ProjectView {
  bpm: number;
  ppq: number;
  timeSignature: [number, number];
  masterGainDb: number;
  masterChain: ChainNodeView[];
  tracks: TrackView[];
}

/** Reserved chain target addressing the master bus (backend `graph_edit`). */
export const MASTER_ID = "_master";

export const useProjectStore = defineStore("project", () => {
  const bpm = ref(120);
  const ppq = ref(960);
  const timeSignature = ref<[number, number]>([4, 4]);
  const masterGainDb = ref(0);
  const masterChain = ref<ChainNodeView[]>([]);
  const tracks = ref<TrackView[]>([]);

  async function load(): Promise<void> {
    const view = await invoke<ProjectView>("project_view");
    bpm.value = view.bpm;
    ppq.value = view.ppq;
    timeSignature.value = view.timeSignature;
    masterGainDb.value = view.masterGainDb ?? 0;
    masterChain.value = view.masterChain ?? [];
    tracks.value = view.tracks;
  }

  // Notes flashing after an external reload: note id → flash start (ms). Plain
  // (non-reactive) — the canvas polls it each frame via `noteFlash`/`anyFlash`.
  const flashStart = new Map<string, number>();
  // Ghosts of notes an external reload REMOVED: their geometry is captured
  // BEFORE the store reloads (afterwards the notes no longer exist), then the
  // canvas draws a fading outline where each one was. Plain, like flashStart.
  const ghosts: {
    trackId: string;
    pitch: number;
    startTicks: number;
    lengthTicks: number;
    start: number;
  }[] = [];

  /** Capture the geometry of soon-to-vanish notes from the CURRENT view. */
  function captureGhosts(removedIds: string[], now: number): void {
    if (removedIds.length === 0) return;
    const wanted = new Set(removedIds);
    for (const track of tracks.value) {
      for (const clip of track.clips) {
        for (const note of clip.notes) {
          if (wanted.has(note.id)) {
            ghosts.push({
              trackId: track.id,
              pitch: note.pitch,
              startTicks: note.startTicks,
              lengthTicks: note.lengthTicks,
              start: now,
            });
          }
        }
      }
    }
  }
  /** Latest external-edit error, if the changed file failed to parse. `seq`
   *  bumps on every occurrence, so saving the same broken file twice still
   *  notifies watchers (the text alone would compare equal). */
  const lastError = ref<{ text: string; seq: number } | null>(null);

  /** Register the file-watcher listeners: external reloads refresh the view and
   *  flash the notes that changed; a parse error is surfaced, no reload. */
  async function init(): Promise<void> {
    await listen<ProjectDiff>("project_changed", async (e) => {
      const now = performance.now();
      // Before the reload: the removed notes still exist here, ghost them.
      captureGhosts(e.payload.removedNotes, now);
      await load();
      for (const id of e.payload.addedNotes) flashStart.set(id, now);
      for (const id of e.payload.modifiedNotes) flashStart.set(id, now);
      lastError.value = null;
    });
    await listen<string>("project_error", (e) => {
      lastError.value = { text: e.payload, seq: (lastError.value?.seq ?? 0) + 1 };
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
    return ghosts.some((g) => now - g.start < FLASH_MS);
  }

  /** Fading outlines of externally removed notes on `trackId` at time `now`
   *  (intensity 1 → 0); expired ghosts are purged as they are polled. */
  function noteGhosts(
    trackId: string,
    now: number,
  ): { pitch: number; startTicks: number; lengthTicks: number; intensity: number }[] {
    for (let i = ghosts.length - 1; i >= 0; i--) {
      if (flashIntensity(ghosts[i].start, now) === 0) ghosts.splice(i, 1);
    }
    return ghosts
      .filter((g) => g.trackId === trackId)
      .map((g) => ({
        pitch: g.pitch,
        startTicks: g.startTicks,
        lengthTicks: g.lengthTicks,
        intensity: flashIntensity(g.start, now),
      }));
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

  /** A fresh, collision-resistant id (the backend also rejects duplicates). */
  function freshId(prefix: string): string {
    return `${prefix}-${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6).toString(36)}`;
  }

  const freshNodeId = (): string => freshId("fx");

  // --- Track edits: create/remove a track, choose its instrument. Each is one
  // graph + schedule hot-swap (instrument indices can shift). ---

  /** Create a track playing a fresh instance of `module` (an instrument).
   *  Its display hue is drawn from the palette hues no track shows yet. */
  async function addTrack(module: string, name: string): Promise<void> {
    await invoke("track_add", {
      trackId: freshId("trk"),
      name,
      module,
      instrumentId: freshId("ins"),
      color: unusedHue([...projectHues(tracks.value).values()]),
    });
    await load();
  }

  /** Swap a track's instrument for a fresh `module` instance (notes stay). */
  async function setTrackInstrument(trackId: string, module: string): Promise<void> {
    await invoke("track_set_instrument", { trackId, module, nodeId: freshId("ins") });
    await load();
  }

  /** Remove a track entirely (instrument, effects, clips, mixer strip). */
  async function removeTrack(trackId: string): Promise<void> {
    await invoke("track_remove", { trackId });
    await load();
  }

  /** Move a track to `index` in the project order (timeline gutter drag). */
  async function moveTrack(trackId: string, index: number): Promise<void> {
    await invoke("track_move", { trackId, index });
    await load();
  }

  /** The chain a target id addresses: a track's, or the master bus's. */
  function chainOf(targetId: string): ChainNodeView[] | undefined {
    if (targetId === MASTER_ID) return masterChain.value;
    return tracks.value.find((t) => t.id === targetId)?.chain;
  }

  /** Optimistically place `node` in a chain at the backend's index rule
   *  (effects region only, clamped — the master chain has no instrument
   *  head), so a drop lands in its final layout on the very frame of the
   *  release — no post-reload catch-up movement. */
  function spliceIntoChain(targetId: string, index: number, node: ChainNodeView): void {
    const chain = chainOf(targetId);
    if (!chain) return;
    const floor = targetId === MASTER_ID ? 0 : 1;
    const pos = Math.min(Math.max(index, floor), chain.length);
    chain.splice(pos, 0, node);
  }

  /** Move an effect to a chain at `index`, keeping its id (its tail follows). */
  async function moveNode(id: string, toTrack: string, index: number): Promise<void> {
    // Mirror of the backend's move: remove first, shift the target left when
    // the removal happened before it in the same chain.
    const srcId =
      tracks.value.find((t) => t.chain.some((n) => n.id === id))?.id ??
      (masterChain.value.some((n) => n.id === id) ? MASTER_ID : undefined);
    const srcChain = srcId !== undefined ? chainOf(srcId) : undefined;
    if (srcId !== undefined && srcChain) {
      const from = srcChain.findIndex((n) => n.id === id);
      // Never lift a track's instrument head (the backend refuses it too).
      if (from >= (srcId === MASTER_ID ? 0 : 1)) {
        const [node] = srcChain.splice(from, 1);
        spliceIntoChain(toTrack, srcId === toTrack && from < index ? index - 1 : index, node);
      }
    }
    await invoke("node_move_to", { id, toTrack, index });
    await load();
  }

  /** Copy an effect into a chain at `index` under a fresh id. */
  async function copyNode(id: string, toTrack: string, index: number): Promise<void> {
    const newId = freshNodeId();
    const original = [...tracks.value.flatMap((t) => t.chain), ...masterChain.value].find(
      (n) => n.id === id,
    );
    if (original) {
      spliceIntoChain(toTrack, index, { ...original, id: newId, flatIndex: -1 });
    }
    await invoke("node_copy", { id, newId, toTrack, index });
    await load();
  }

  /** Insert a dropped module/preset as a new effect at `index`. */
  async function insertNode(
    trackId: string,
    index: number,
    module: string,
    params: Record<string, number> = {},
  ): Promise<void> {
    const id = freshNodeId();
    // flatIndex -1 = no meter yet; the reload assigns the real one.
    spliceIntoChain(trackId, index, { id, module, isInstrument: false, flatIndex: -1, params });
    await invoke("node_insert", { trackId, index, module, id, params });
    await load();
  }

  async function removeNode(id: string): Promise<void> {
    await invoke("graph_remove_node", { id });
    await load();
  }

  /** Optimistically write a node's param in the local mirror (keeps knobs live). */
  function patchNodeParam(nodeId: string, param: string, value: number): void {
    for (const chain of [...tracks.value.map((t) => t.chain), masterChain.value]) {
      const node = chain.find((n) => n.id === nodeId);
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

  const listChainPresets = (): Promise<string[]> => invoke<string[]>("list_chain_presets");
  const readChainPreset = (name: string): Promise<ChainPreset> =>
    invoke<ChainPreset>("read_chain_preset", { name });

  /** Save a track's whole chain (instrument + effects) as a named chain preset. */
  async function saveChainPreset(name: string, trackId: string): Promise<void> {
    await invoke("save_chain_preset", { name, trackId });
    presetsVersion.value += 1;
  }

  /** Fresh-id node specs for a chain preset's nodes (the head is the instrument). */
  function mintChainNodes(nodes: Preset[]): { id: string; module: string; params: Record<string, number> }[] {
    return nodes.map((n, i) => ({
      id: i === 0 ? freshId("ins") : freshNodeId(),
      module: n.module,
      params: n.params,
    }));
  }

  /** Create a track whose whole chain comes from a chain preset (one atomic
   *  edit: a single hot-swap and a single undo step). */
  async function addTrackFromChain(name: string, nodes: Preset[]): Promise<void> {
    await invoke("track_add_chain", {
      trackId: freshId("trk"),
      name,
      color: unusedHue([...projectHues(tracks.value).values()]),
      nodes: mintChainNodes(nodes),
    });
    await load();
  }

  /** Replace a track's whole chain from a chain preset (clips and mixer stay). */
  async function setTrackChain(trackId: string, nodes: Preset[]): Promise<void> {
    await invoke("track_set_chain", { trackId, nodes: mintChainNodes(nodes) });
    await load();
  }

  return {
    bpm,
    ppq,
    timeSignature,
    masterGainDb,
    masterChain,
    tracks,
    presetsVersion,
    lastError,
    init,
    noteFlash,
    anyFlash,
    noteGhosts,
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
    addTrack,
    setTrackInstrument,
    removeTrack,
    moveTrack,
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
    listChainPresets,
    readChainPreset,
    saveChainPreset,
    addTrackFromChain,
    setTrackChain,
  };
});
