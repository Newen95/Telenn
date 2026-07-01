//! Metronome: a generated click — a short enveloped sine burst — on every beat,
//! accented (higher pitch, louder) on beat 1. No sample involved.
//!
//! Monitoring only: `RtState` adds it to the real-time stereo mix while the
//! transport plays; the offline render never sees it, so an export can never
//! carry the click. The beat grid (`samples_per_beat`, `beats_per_bar`) comes
//! from the schedule, so a bpm change follows a project reload naturally.

use std::f32::consts::TAU;

use crate::engine::params::LinearRamp;

/// Click oscillator pitches: the accented downbeat rings a fifth-ish above.
const ACCENT_HZ: f32 = 1600.0;
const BEAT_HZ: f32 = 1050.0;
/// Exponential decay time constant of a click (seconds): a tick, not a tone.
const DECAY_S: f32 = 0.02;
/// Level of a regular beat relative to the accented downbeat.
const BEAT_LEVEL: f32 = 0.6;
/// Envelope floor below which a click is considered finished.
const ENV_FLOOR: f32 = 1.0e-4;
/// Volume ramp (anti-click on the volume knob itself).
const RAMP_MS: f32 = 10.0;

pub struct Metronome {
    sample_rate: f32,
    enabled: bool,
    volume: LinearRamp,
    /// Envelope of the click currently ringing (0 = silent).
    env: f32,
    /// Per-sample envelope decay factor.
    decay: f32,
    /// Oscillator phase in turns (0..1) and per-sample increment.
    phase: f32,
    inc: f32,
    /// Peak level of the current click (accent vs regular beat).
    amp: f32,
}

impl Metronome {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            enabled: false,
            volume: LinearRamp::new(0.7, RAMP_MS, sample_rate),
            env: 0.0,
            decay: (-1.0 / (DECAY_S * sample_rate)).exp(),
            phase: 0.0,
            inc: 0.0,
            amp: 0.0,
        }
    }

    /// Toggle and volume (0..1), from the transport bar. // RT-SAFE
    pub fn set(&mut self, enabled: bool, volume: f32) {
        self.enabled = enabled;
        self.volume.set_target(volume.clamp(0.0, 1.0));
        if !enabled {
            self.env = 0.0;
        }
    }

    /// Strike a click. // RT-SAFE
    fn trigger(&mut self, accent: bool) {
        self.env = 1.0;
        self.phase = 0.0;
        self.inc = if accent { ACCENT_HZ } else { BEAT_HZ } / self.sample_rate;
        self.amp = if accent { 1.0 } else { BEAT_LEVEL };
    }

    /// Add the clicks of a block starting at `playhead` (samples, wrapping in
    /// `loop_len`) onto the interleaved stereo buffer. A beat falls every
    /// `samples_per_beat`; the bar start (and the loop start) is accented.
    /// // RT-SAFE
    pub fn add_block(
        &mut self,
        playhead: u64,
        loop_len: u64,
        samples_per_beat: u64,
        beats_per_bar: u32,
        out: &mut [f32],
    ) {
        if !self.enabled || samples_per_beat == 0 || loop_len == 0 {
            return;
        }
        for (i, frame) in out.chunks_exact_mut(2).enumerate() {
            let pos = (playhead + i as u64) % loop_len;
            if pos.is_multiple_of(samples_per_beat) {
                let beat = pos / samples_per_beat;
                let accent = beats_per_bar > 0 && beat.is_multiple_of(beats_per_bar as u64);
                self.trigger(accent);
            }
            let volume = self.volume.tick();
            if self.env > ENV_FLOOR {
                let s = (TAU * self.phase).sin() * self.env * self.amp * volume;
                self.phase += self.inc;
                if self.phase >= 1.0 {
                    self.phase -= 1.0;
                }
                self.env *= self.decay;
                frame[0] += s;
                frame[1] += s;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;
    /// 120 bpm at 48 kHz.
    const SPB: u64 = 24_000;

    /// Render `frames` of metronome alone into an interleaved stereo buffer,
    /// walking a playhead in `block`-sized chunks like the audio thread does.
    fn render(m: &mut Metronome, frames: usize, block: usize, loop_len: u64) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        let mut playhead = 0u64;
        let mut done = 0usize;
        while done < frames {
            let n = (frames - done).min(block);
            m.add_block(
                playhead,
                loop_len,
                SPB,
                4,
                &mut out[done * 2..(done + n) * 2],
            );
            playhead = (playhead + n as u64) % loop_len;
            done += n;
        }
        out
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
    }

    /// Stereo window [from, to) in frames.
    fn window(out: &[f32], from: usize, to: usize) -> &[f32] {
        &out[from * 2..to * 2]
    }

    #[test]
    fn clicks_land_on_the_beats() {
        let mut m = Metronome::new(SR);
        m.set(true, 1.0);
        // One 4/4 bar; awkward block size so beats fall mid-block too.
        let out = render(&mut m, 96_000, 257, 96_000);

        for beat in 0..4usize {
            let at = beat * SPB as usize;
            let strike = peak(window(&out, at, at + 128));
            assert!(strike > 0.1, "no click at beat {beat}: {strike}");
            // The previous click has fully decayed just before each later beat.
            if beat > 0 {
                let quiet = peak(window(&out, at - 1_000, at - 10));
                assert!(quiet < 1.0e-3, "click bleeds into beat {beat}: {quiet}");
            }
        }
        assert!(out.iter().all(|s| s.is_finite()), "produced NaN/inf");
    }

    #[test]
    fn downbeat_is_accented() {
        let mut m = Metronome::new(SR);
        m.set(true, 1.0);
        let out = render(&mut m, 96_000, 256, 96_000);
        let down = peak(window(&out, 0, 512));
        let two = peak(window(&out, SPB as usize, SPB as usize + 512));
        assert!(
            down > two * 1.2,
            "downbeat not accented: {down} vs beat 2 {two}"
        );
    }

    #[test]
    fn disabled_metronome_is_silent() {
        let mut m = Metronome::new(SR);
        m.set(false, 1.0);
        let out = render(&mut m, 48_000, 256, 96_000);
        assert_eq!(peak(&out), 0.0, "disabled metronome made sound");
    }

    #[test]
    fn loop_wrap_restrikes_the_downbeat() {
        let mut m = Metronome::new(SR);
        m.set(true, 1.0);
        // Two loops of one bar: the wrap lands back on the accented downbeat.
        let out = render(&mut m, 192_000, 256, 96_000);
        let wrapped = peak(window(&out, 96_000, 96_000 + 128));
        assert!(wrapped > 0.1, "no click at the loop wrap: {wrapped}");
    }
}
