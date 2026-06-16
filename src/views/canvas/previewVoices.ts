//! Monophonic preview voice for non-destructive audition (the gutter keyboard,
//! the create echo, the drag audition). It owns *both* facets of "one pitch is
//! sounding": the audio (note on/off, routed through the injected sink) and the
//! visual highlight (which pitch is lit). At most one pitch sounds at a time, so a
//! new strike releases the previous one first — and crucially it releases it on
//! the *same node* it was struck on, so switching the target instrument mid-flight
//! never leaves a note droning. Later the same single-voice state can be driven by
//! playback telemetry and agent-triggered notes.

import { ref } from "vue";

/** Where preview note on/off events are delivered (Tauri commands in the app). */
export interface PreviewSink {
  on(node: number, pitch: number): void;
  off(node: number, pitch: number): void;
}

export interface PreviewVoices {
  /** True while `pitch` is the one sounding. */
  has(pitch: number): boolean;
  /** Whether any pitch is sounding (keeps the canvas redrawing during a flash). */
  any(): boolean;
  /** Sound `pitch` on `node` as the sole voice for `ms`, releasing any previous.
   *  Punctual: a one-shot used for the create echo and the drag audition. */
  strike(node: number, pitch: number, ms: number): void;
  /** Sound `pitch` on `node` and hold it until `release` — no fixed duration.
   *  Used by a pressed gutter key; a new pitch (glissando) replaces the previous. */
  hold(node: number, pitch: number): void;
  /** Silence the current voice now (release a held key, unmount, …). */
  release(): void;
}

export function usePreviewVoices(sink: PreviewSink): PreviewVoices {
  const active = ref<number | null>(null);
  // The node the current voice was struck on: the release must target it, not
  // whatever node a later strike happens to use.
  let node: number | null = null;
  let timer: number | undefined;

  function release(): void {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    if (active.value !== null && node !== null) {
      sink.off(node, active.value);
    }
    active.value = null;
    node = null;
  }

  /** Make `pitch` on `n` the sole sounding voice, releasing any previous first. */
  function start(n: number, pitch: number): void {
    release();
    sink.on(n, pitch);
    active.value = pitch;
    node = n;
  }

  return {
    has: (pitch) => active.value === pitch,
    any: () => active.value !== null,
    strike(n, pitch, ms) {
      start(n, pitch);
      timer = setTimeout(release, ms);
    },
    hold(n, pitch) {
      // Already holding this exact key (pointer jitter within a row): no re-trigger.
      if (active.value === pitch && node === n) return;
      start(n, pitch);
    },
    release,
  };
}
