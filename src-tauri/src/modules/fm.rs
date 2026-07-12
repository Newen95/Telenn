//! Two-operator FM synthesiser: 8-voice polyphony, a sine modulator phase-modulates
//! a sine carrier, shaped by an ADSR amplitude envelope.
//!
//! A single modulator → carrier pair (the classic 2-op cell) already covers bells,
//! electric pianos and metallic basses: `ratio` sets the modulator frequency as a
//! multiple of the carrier, `index` the modulation depth (in radians of carrier
//! phase). `index_env` lets a per-voice envelope shape the index over the note
//! (bright pluck softening into a purer sustain — the living-spectrum FM sound),
//! and `feedback` feeds the modulator back into itself for saw-like density.
//! All DSP is hand-written. `process` is RT-SAFE: a fixed voice array, no
//! allocation, no locks. Voice stealing takes the oldest voice.

use std::f32::consts::{PI, TAU};

use crate::engine::dsp::{flush, AdsrSettings, Env, MIN_RELEASE_S};
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
// Appended (ids are frozen — presets address params by name, order by id).
const P_INDEX_ENV: ParamId = 7;
const P_INDEX_DECAY: ParamId = 8;
const P_FEEDBACK: ParamId = 9;

const PARAMS: [ParamSpec; 10] = [
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
    ParamSpec {
        id: P_INDEX_ENV,
        name: "index_env",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_INDEX_DECAY,
        name: "index_decay",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 200.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_FEEDBACK,
        name: "feedback",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
];

/// Per-block operator settings, shared by every voice.
#[derive(Clone, Copy)]
struct OpParams {
    ratio: f32,
    index: f32,
    /// How much of the index the modulator envelope shapes (0 = static).
    index_env: f32,
    /// Modulator self-feedback amount (0..1, scaled to 0..π radians).
    feedback: f32,
}

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
    /// Modulator-index envelope (attack/release follow the amp ADSR, decay is
    /// `index_decay`, sustain 0 — the spectrum settles toward a purer tone).
    mod_env: Env,
    /// Last two modulator samples, for the DX-style averaged self-feedback.
    fb1: f32,
    fb2: f32,
}

impl Voice {
    // RT-SAFE
    fn render(
        &mut self,
        p: &OpParams,
        mod_adsr: &AdsrSettings,
        adsr: &AdsrSettings,
        sr: f32,
    ) -> f32 {
        // Averaging the last two feedback samples tames the chaos a 1-sample
        // loop develops at high amounts (the classic DX trick).
        let fb = p.feedback * PI * (self.fb1 + self.fb2) * 0.5;
        let mod_sig = (TAU * self.mod_phase + fb).sin();
        self.fb2 = self.fb1;
        self.fb1 = flush(mod_sig);

        // Effective index: crossfade between the static knob and the same
        // knob shaped by the per-voice modulator envelope.
        let idx_env = self.mod_env.tick(mod_adsr, sr);
        let index_now = p.index * ((1.0 - p.index_env) + p.index_env * idx_env);
        let carrier = (TAU * self.carrier_phase + index_now * mod_sig).sin();

        self.carrier_phase += self.inc;
        self.carrier_phase -= self.carrier_phase.floor();
        self.mod_phase += self.inc * p.ratio;
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
    index_env: OnePole,
    /// Modulator envelope decay, seconds.
    index_decay_s: f32,
    feedback: OnePole,
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
            index_env: OnePole::new(0.0, 0.99),
            index_decay_s: 0.2,
            feedback: OnePole::new(0.0, 0.99),
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
        v.fb1 = 0.0;
        v.fb2 = 0.0;
        v.env = Env::default();
        v.env.note_on();
        v.mod_env = Env::default();
        v.mod_env.note_on();
    }

    fn note_off(&mut self, pitch: u8) {
        for v in &mut self.voices {
            if v.active && v.pitch == pitch {
                v.env.note_off();
                v.mod_env.note_off();
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
        // Operator settings glide once per block (anti-zipper), like the
        // subtractive synth's cutoff/resonance; voices read the settled values.
        let op = OpParams {
            ratio: self.ratio.advance(frames),
            index: self.index.advance(frames),
            index_env: self.index_env.advance(frames),
            feedback: self.feedback.advance(frames),
        };
        // The modulator envelope borrows the amp attack/release; its decay is
        // its own knob and it always settles to 0 (purer sustain by design).
        let mod_adsr = AdsrSettings {
            attack_s: self.adsr.attack_s,
            decay_s: self.index_decay_s,
            sustain: 0.0,
            release_s: self.adsr.release_s,
        };

        let out = io.output(0);
        for slot in out.iter_mut() {
            let level = self.level.tick();
            let mut sum = 0.0;
            for v in &mut self.voices {
                if v.active {
                    sum += v.render(&op, &mod_adsr, &self.adsr, sr);
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
            P_INDEX_ENV => self.index_env.set_target(real),
            P_INDEX_DECAY => self.index_decay_s = real / 1000.0,
            P_FEEDBACK => self.feedback.set_target(real),
            _ => {}
        }
    }

    fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            if v.active {
                v.env.note_off();
                v.mod_env.note_off();
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
        self.index_env = OnePole::new(default_real(P_INDEX_ENV), coef);
        self.index_decay_s = default_real(P_INDEX_DECAY) / 1000.0;
        self.feedback = OnePole::new(default_real(P_FEEDBACK), coef);
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
            samples_per_beat: 0.0,
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

    fn set_real(s: &mut Fm, id: ParamId, real: f32) {
        s.set_param(id, PARAMS[id as usize].normalize(real));
    }

    #[test]
    fn index_envelope_makes_the_spectrum_evolve() {
        // With the full index enveloped (sustain 0), the sideband must be loud
        // during the pluck and gone once the modulator envelope has decayed —
        // the living-spectrum behaviour a static index cannot produce.
        let sr = 48_000.0;
        let pitch = 57u8;
        let sideband = 3.0 * midi_to_freq(pitch);

        let mut s = synth();
        set_real(&mut s, P_RATIO, 2.0);
        set_real(&mut s, P_INDEX, 4.0);
        set_real(&mut s, P_INDEX_ENV, 1.0);
        set_real(&mut s, P_INDEX_DECAY, 100.0);
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(pitch)], 48_000, &mut buf);

        let early = mag_at(&buf[512..8192], sideband, sr); // after the 2 ms attack
        let late = mag_at(&buf[24_000..44_000], sideband, sr); // envelope long gone
        assert!(early > 0.01, "enveloped index never got bright: {early}");
        assert!(
            early > 5.0 * late,
            "spectrum did not settle: early {early}, late {late}"
        );
    }

    #[test]
    fn feedback_thickens_the_modulator() {
        // Modulator self-feedback enriches its waveform, which spreads carrier
        // energy into higher sidebands (ratio 1 → harmonics of the carrier).
        let sr = 48_000.0;
        let pitch = 57u8;
        let f0 = midi_to_freq(pitch);

        let render_with_fb = |fb: f32| {
            let mut s = synth();
            set_real(&mut s, P_RATIO, 1.0);
            set_real(&mut s, P_INDEX, 2.0);
            set_real(&mut s, P_FEEDBACK, fb);
            let mut buf = Vec::new();
            render_notes(&mut s, &[on(pitch)], 16_384, &mut buf);
            buf
        };
        let plain = render_with_fb(0.0);
        let thick = render_with_fb(0.8);
        let hi = |b: &[f32]| mag_at(&b[8192..], 6.0 * f0, sr) + mag_at(&b[8192..], 7.0 * f0, sr);
        let (p, t) = (hi(&plain), hi(&thick));
        assert!(
            t > 3.0 * p,
            "feedback added no high harmonics: plain {p}, fed back {t}"
        );
        assert!(thick.iter().all(|x| x.is_finite()), "feedback blew up");
    }

    #[test]
    fn new_param_sweeps_are_click_free() {
        // NOTE: not `detect_clicks` here — high-feedback FM is legitimately
        // edgy (near-saw waveform) and the detector flags its per-period
        // fronts. A parameter discontinuity would jump far beyond the
        // waveform's own slope, so an adjacent-delta bound is the right tool.
        let mut s = synth();
        set_real(&mut s, P_RATIO, 2.0);
        set_real(&mut s, P_INDEX, 4.0);
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(57)], 4096, &mut buf);
        let mut whole = Vec::new();
        for k in 0..64 {
            let v = k as f32 / 63.0;
            s.set_param(P_INDEX_ENV, v);
            s.set_param(P_FEEDBACK, v);
            render_notes(&mut s, &[], 256, &mut buf);
            whole.extend_from_slice(&buf);
        }
        assert!(whole.iter().all(|x| x.is_finite()));
        let max_delta = whole
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_delta < 0.6,
            "sweep discontinuity, max delta {max_delta}"
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
