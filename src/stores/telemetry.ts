//! Pinia store for the audio→UI telemetry slice. Listens to the `telemetry`
//! event and interpolates the playhead between frames so the UI is smooth even
//! though frames arrive at ~60 Hz.

import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** One node's post-process level over a block (peak + RMS). */
export interface Meter {
  peak: number;
  rms: number;
}

interface TelemetryEvent {
  playheadSamples: number;
  playing: boolean;
  masterPeak: number;
  audioLoad: number;
  xruns: number;
  meterCount: number;
  /** Per-node meters, indexed by flat graph index (only the valid ones). */
  meters: Meter[];
}

interface EngineStatus {
  xruns: number;
  sampleRate: number;
}

export const useTelemetryStore = defineStore("telemetry", () => {
  const playing = ref(false);
  const masterPeak = ref(0);
  const sampleRate = ref(48000);
  // Streamed from the audio thread (replaces the old 1 Hz engine_status poll).
  const audioLoad = ref(0);
  const xruns = ref(0);
  const meters = ref<Meter[]>([]);
  let lastSamples = 0;
  let receivedAt = performance.now();

  async function init(): Promise<void> {
    const status = await invoke<EngineStatus>("engine_status");
    sampleRate.value = status.sampleRate;
    await listen<TelemetryEvent>("telemetry", (event) => {
      lastSamples = event.payload.playheadSamples;
      playing.value = event.payload.playing;
      masterPeak.value = event.payload.masterPeak;
      audioLoad.value = event.payload.audioLoad;
      xruns.value = event.payload.xruns;
      meters.value = event.payload.meters;
      receivedAt = performance.now();
    });
  }

  /** A node's latest meter by flat graph index (zeroed if out of range). */
  function meterAt(index: number): Meter {
    return meters.value[index] ?? { peak: 0, rms: 0 };
  }

  /** Interpolated playhead in samples at wall-clock time `now` (ms). */
  function currentSamples(now: number): number {
    const dt = playing.value ? (now - receivedAt) / 1000 : 0;
    return lastSamples + dt * sampleRate.value;
  }

  return {
    playing,
    masterPeak,
    sampleRate,
    audioLoad,
    xruns,
    meters,
    init,
    currentSamples,
    meterAt,
  };
});
