//! Two-operator FM synthesiser: 8-voice polyphony, a sine modulator phase-modulates
//! a sine carrier, shaped by an ADSR amplitude envelope.
//!
//! A single modulator → carrier pair (the classic 2-op cell) already covers bells,
//! electric pianos and metallic basses: `ratio` sets the modulator frequency as a
//! multiple of the carrier, `index` the modulation depth (in radians of carrier
//! phase). All DSP is hand-written. `process` is RT-SAFE: a fixed voice array, no
//! allocation, no locks. Voice stealing takes the oldest voice.

use std::f32::consts::TAU;

use crate::engine::dsp::{AdsrSettings, Env, MIN_RELEASE_S};
use crate::engine::node::{
    AudioNode, NodeIo, NoteKind, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx,
    SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

const VOICES: usize = 8;
/// Ratio/index glide time constant (seconds) for the one-pole smoothers.
const SMOOTH_S: f32 = 0.015;

const P_RATIO: ParamId = 0;
const P_INDEX: ParamId = 1;
const P_ATTACK: ParamId = 2;
const P_DECAY: ParamId = 3;
const P_SUSTAIN: ParamId = 4;
const P_RELEASE: ParamId = 5;
const P_LEVEL: ParamId = 6;

const PARAMS: [ParamSpec; 7] = [
    ParamSpec {
        id: P_RATIO,
        name: "ratio",
        unit: "",
        min: 0.5,
        max: 12.0,
        default: 1.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_INDEX,
        name: "index",
        unit: "",
        min: 0.0,
        max: 12.0,
        default: 2.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_ATTACK,
        name: "attack",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 2.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_DECAY,
        name: "decay",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 300.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_SUSTAIN,
        name: "sustain",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.6,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_RELEASE,
        name: "release",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 200.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_LEVEL,
        name: "level",
        unit: "dB",
        min: -60.0,
        max: 0.0,
        default: -6.0,
        curve: ParamCurve::Db,
    },
];

#[derive(Clone, Copy, Default)]
struct Voice {
    pitch: u8,
    active: bool,
    age: u64,
    /// Carrier and modulator phases in turns (0..1).
    carrier_phase: f32,
    mod_phase: f32,
    /// Carrier phase increment per sample (`freq / sr`); the modulator runs at
    /// `inc * ratio`.
    inc: f32,
    /// Velocity amplitude (0..1).
    amp: f32,
    env: Env,
}

impl Voice {
    // RT-SAFE
    fn render(&mut self, ratio: f32, index: f32, adsr: &AdsrSettings, sr: f32) -> f32 {
        let mod_sig = (TAU * self.mod_phase).sin();
        let carrier = (TAU * self.carrier_phase + index * mod_sig).sin();

        self.carrier_phase += self.inc;
        self.carrier_phase -= self.carrier_phase.floor();
        self.mod_phase += self.inc * ratio;
        self.mod_phase -= self.mod_phase.floor();

        let env = self.env.tick(adsr, sr);
        if !self.env.is_active() {
            self.active = false;
        }
        carrier * env * self.amp
    }
}

pub struct Fm {
    sample_rate: f32,
    voices: [Voice; VOICES],
    ratio: OnePole,
    index: OnePole,
    level: LinearRamp,
    adsr: AdsrSettings,
    next_age: u64,
}

impl Default for Fm {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            voices: [Voice::default(); VOICES],
            ratio: OnePole::new(1.0, 0.99),
            index: OnePole::new(2.0, 0.99),
            level: LinearRamp::new(1.0, 10.0, 48_000.0),
            adsr: AdsrSettings::default(),
            next_age: 0,
        }
    }
}

impl Fm {
    fn note_on(&mut self, pitch: u8, vel: u8) {
        let sr = self.sample_rate;
        let idx = self.alloc_voice();
        let age = self.next_age;
        self.next_age += 1;
        let v = &mut self.voices[idx];
        v.pitch = pitch;
        v.inc = midi_to_freq(pitch) / sr;
        v.amp = vel as f32 / 127.0;
        v.active = true;
        v.age = age;
        v.carrier_phase = 0.0;
        v.mod_phase = 0.0;
        v.env = Env::default();
        v.env.note_on();
    }

    fn note_off(&mut self, pitch: u8) {
        for v in &mut self.voices {
            if v.active && v.pitch == pitch {
                v.env.note_off();
            }
        }
    }

    /// Pick a free voice, else steal the oldest. // RT-SAFE
    fn alloc_voice(&mut self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        let mut idx = 0;
        let mut oldest = u64::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if v.age < oldest {
                oldest = v.age;
                idx = i;
            }
        }
        idx
    }

    #[cfg(test)]
    fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    #[cfg(test)]
    fn active_pitches(&self) -> Vec<u8> {
        self.voices
            .iter()
            .filter(|v| v.active)
            .map(|v| v.pitch)
            .collect()
    }
}

impl AudioNode for Fm {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        for ev in io.notes() {
            match ev.kind {
                NoteKind::On { pitch, vel } => self.note_on(pitch, vel),
                NoteKind::Off { pitch } => self.note_off(pitch),
            }
        }

        let frames = io.frames();
        let sr = self.sample_rate;
        // Ratio and index glide once per block (anti-zipper), like the subtractive
        // synth's cutoff/resonance; voices read the settled values for this block.
        let ratio = self.ratio.advance(frames);
        let index = self.index.advance(frames);

        let out = io.output(0);
        for slot in out.iter_mut() {
            let level = self.level.tick();
            let mut sum = 0.0;
            for v in &mut self.voices {
                if v.active {
                    sum += v.render(ratio, index, &self.adsr, sr);
                }
            }
            *slot = sum * level;
        }
    }

    fn ports(&self) -> PortSpec {
        PortSpec {
            inputs: vec![Port {
                name: "notes",
                kind: SignalKind::Notes,
            }],
            outputs: vec![Port {
                name: "out",
                kind: SignalKind::Audio,
            }],
        }
    }

    fn params(&self) -> &[ParamSpec] {
        &PARAMS
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        let real = PARAMS[id as usize].denormalize(value);
        match id {
            P_RATIO => self.ratio.set_target(real),
            P_INDEX => self.index.set_target(real),
            P_ATTACK => self.adsr.attack_s = real / 1000.0,
            P_DECAY => self.adsr.decay_s = real / 1000.0,
            P_SUSTAIN => self.adsr.sustain = real,
            P_RELEASE => self.adsr.release_s = (real / 1000.0).max(MIN_RELEASE_S),
            P_LEVEL => self.level.set_target(real),
            _ => {}
        }
    }

    fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            if v.active {
                v.env.note_off();
            }
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.ratio = OnePole::new(default_real(P_RATIO), coef);
        self.index = OnePole::new(default_real(P_INDEX), coef);
        self.level = LinearRamp::new(default_real(P_LEVEL), 10.0, sample_rate);
        self.adsr = AdsrSettings {
            attack_s: default_real(P_ATTACK) / 1000.0,
            decay_s: default_real(P_DECAY) / 1000.0,
            sustain: default_real(P_SUSTAIN),
            release_s: (default_real(P_RELEASE) / 1000.0).max(MIN_RELEASE_S),
        };
        self.voices = [Voice::default(); VOICES];
        self.next_age = 0;
    }
}

/// Real default value for a parameter (round-trips through the curve).
fn default_real(id: ParamId) -> f32 {
    let spec = &PARAMS[id as usize];
    spec.denormalize(spec.normalize(spec.default))
}

/// MIDI note number → frequency in Hz (A4 = 69 = 440 Hz).
fn midi_to_freq(pitch: u8) -> f32 {
    440.0 * 2.0f32.powf((pitch as f32 - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node::NoteEvent;

    fn synth() -> Fm {
        let mut s = Fm::default();
        s.prepare(48_000.0, 256);
        s
    }

    fn render_notes(s: &mut Fm, notes: &[NoteEvent], frames: usize, out: &mut Vec<f32>) {
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: 48_000.0,
            frames,
            channels: 1,
        };
        let mut io = NodeIo::new(notes, &[], out, frames);
        s.process(&ctx, &mut io);
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
    }

    fn on(pitch: u8) -> NoteEvent {
        NoteEvent {
            frame: 0,
            kind: NoteKind::On { pitch, vel: 110 },
        }
    }

    /// Goertzel-style magnitude of a single bin, normalised by length.
    fn mag_at(buf: &[f32], freq: f32, sr: f32) -> f32 {
        let w = TAU * freq / sr;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, &x) in buf.iter().enumerate() {
            let p = w * n as f32;
            re += x * p.cos();
            im += x * p.sin();
        }
        (re * re + im * im).sqrt() / buf.len() as f32
    }

    #[test]
    fn fm_note_is_audible_and_clean() {
        let mut s = synth();
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(64)], 4096, &mut buf); // past the attack
        let p = peak(&buf);
        assert!(p > 0.05, "fm output is inaudible, peak {p}");
        assert!(p < 1.0, "fm output clips, peak {p}");
        assert!(buf.iter().all(|s| s.is_finite()), "fm produced NaN/inf");
    }

    #[test]
    fn index_creates_sidebands() {
        // With ratio = 2, the modulator sits an octave above the carrier, so the
        // first upper sideband lands at 3·carrier. Raising the index must put real
        // energy there; with no modulation it is essentially a pure carrier sine.
        let sr = 48_000.0;
        let pitch = 57u8; // A3, 220 Hz
        let carrier = midi_to_freq(pitch);
        let sideband = 3.0 * carrier;

        let mut quiet = synth();
        quiet.set_param(P_RATIO, PARAMS[P_RATIO as usize].normalize(2.0));
        quiet.set_param(P_INDEX, 0.0);
        let mut bq = Vec::new();
        render_notes(&mut quiet, &[on(pitch)], 8192, &mut bq);

        let mut bright = synth();
        bright.set_param(P_RATIO, PARAMS[P_RATIO as usize].normalize(2.0));
        bright.set_param(P_INDEX, PARAMS[P_INDEX as usize].normalize(4.0));
        let mut bb = Vec::new();
        render_notes(&mut bright, &[on(pitch)], 8192, &mut bb);

        // Measure on the settled tail (smoothers have reached their targets).
        let q = mag_at(&bq[4096..], sideband, sr);
        let b = mag_at(&bb[4096..], sideband, sr);
        assert!(b > 0.01, "bright sideband too weak: {b}");
        assert!(
            b > 10.0 * q,
            "index added no sidebands: quiet {q}, bright {b}"
        );
    }

    #[test]
    fn release_is_click_free() {
        let mut s = synth();
        s.set_param(P_RELEASE, 0.0); // floored to MIN_RELEASE_S, never instantaneous
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(64)], 2048, &mut buf); // reach sustain
        let off = [NoteEvent {
            frame: 0,
            kind: NoteKind::Off { pitch: 64 },
        }];
        let mut tail = Vec::new();
        render_notes(&mut s, &off, 2048, &mut tail);
        let max_delta = tail
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_delta < 0.2, "release click, max delta {max_delta}");
    }

    #[test]
    fn eight_voices_then_oldest_is_stolen() {
        let mut s = synth();
        let mut buf = Vec::new();
        let ons: Vec<NoteEvent> = (60u8..68).map(on).collect();
        render_notes(&mut s, &ons, 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8);

        render_notes(&mut s, &[on(72)], 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8);
        let pitches = s.active_pitches();
        assert!(pitches.contains(&72), "new note should be present");
        assert!(!pitches.contains(&60), "oldest note should be stolen");
    }

    #[test]
    fn all_notes_off_releases_every_voice() {
        let mut s = synth();
        let mut buf = Vec::new();
        let ons: Vec<NoteEvent> = (60u8..64).map(on).collect();
        render_notes(&mut s, &ons, 256, &mut buf);
        assert_eq!(s.active_voice_count(), 4);

        s.all_notes_off();
        render_notes(&mut s, &[], 48_000, &mut buf); // well past the release
        assert_eq!(
            s.active_voice_count(),
            0,
            "a voice rang on after all_notes_off"
        );
    }
}
