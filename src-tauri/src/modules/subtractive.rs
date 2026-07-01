//! Subtractive synthesiser: 8-voice polyphony, band-limited saw (polyBLEP) →
//! resonant low-pass (RBJ biquad) → ADSR amplitude envelope.
//!
//! All DSP is hand-written. `process` is RT-SAFE: a fixed voice array, no
//! allocation, no locks. Voice stealing takes the oldest voice.

use crate::engine::dsp::{AdsrSettings, Biquad, Env, MIN_RELEASE_S};
use crate::engine::node::{
    AudioNode, NodeIo, NoteKind, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx,
    SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

const VOICES: usize = 8;
/// Cutoff/resonance glide time constant (seconds) for the one-pole smoother.
const SMOOTH_S: f32 = 0.015;

const P_CUTOFF: ParamId = 0;
const P_RESONANCE: ParamId = 1;
const P_ATTACK: ParamId = 2;
const P_DECAY: ParamId = 3;
const P_SUSTAIN: ParamId = 4;
const P_RELEASE: ParamId = 5;
const P_LEVEL: ParamId = 6;

const PARAMS: [ParamSpec; 7] = [
    ParamSpec {
        id: P_CUTOFF,
        name: "cutoff",
        unit: "Hz",
        min: 20.0,
        max: 20_000.0,
        default: 2000.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_RESONANCE,
        name: "resonance",
        unit: "Q",
        min: 0.5,
        max: 20.0,
        default: 0.7,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_ATTACK,
        name: "attack",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 5.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_DECAY,
        name: "decay",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 200.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_SUSTAIN,
        name: "sustain",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.7,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_RELEASE,
        name: "release",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 150.0,
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
    /// Oscillator phase in turns (0..1).
    phase: f32,
    /// Phase increment per sample (`freq / sr`).
    inc: f32,
    /// Velocity amplitude (0..1).
    amp: f32,
    filter: Biquad,
    env: Env,
}

impl Voice {
    // RT-SAFE
    fn render(&mut self, adsr: &AdsrSettings, sr: f32) -> f32 {
        let saw = 2.0 * self.phase - 1.0 - poly_blep(self.phase, self.inc);
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let filtered = self.filter.process(saw);
        let env = self.env.tick(adsr, sr);
        if !self.env.is_active() {
            self.active = false;
        }
        filtered * env * self.amp
    }
}

pub struct Subtractive {
    sample_rate: f32,
    voices: [Voice; VOICES],
    cutoff: OnePole,
    resonance: OnePole,
    level: LinearRamp,
    adsr: AdsrSettings,
    next_age: u64,
}

impl Default for Subtractive {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            voices: [Voice::default(); VOICES],
            cutoff: OnePole::new(2000.0, 0.99),
            resonance: OnePole::new(0.7, 0.99),
            level: LinearRamp::new(1.0, 10.0, 48_000.0),
            adsr: AdsrSettings::default(),
            next_age: 0,
        }
    }
}

impl Subtractive {
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

impl AudioNode for Subtractive {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        for ev in io.notes() {
            match ev.kind {
                NoteKind::On { pitch, vel } => self.note_on(pitch, vel),
                NoteKind::Off { pitch } => self.note_off(pitch),
            }
        }

        let frames = io.frames();
        let sr = self.sample_rate;
        let fc = self.cutoff.advance(frames);
        let q = self.resonance.advance(frames);
        for v in &mut self.voices {
            if v.active {
                v.filter.set_lowpass(fc, q, sr);
            }
        }

        let out = io.output(0);
        for slot in out.iter_mut() {
            let level = self.level.tick();
            let mut sum = 0.0;
            for v in &mut self.voices {
                if v.active {
                    sum += v.render(&self.adsr, sr);
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
            P_CUTOFF => self.cutoff.set_target(real),
            P_RESONANCE => self.resonance.set_target(real),
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
        self.cutoff = OnePole::new(default_real(P_CUTOFF), coef);
        self.resonance = OnePole::new(default_real(P_RESONANCE), coef);
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

/// polyBLEP correction at a saw discontinuity. `t` is the phase (0..1), `dt` the
/// phase increment. Removes the worst of the aliasing of a naive saw. // RT-SAFE
fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node::NoteEvent;

    fn synth() -> Subtractive {
        let mut s = Subtractive::default();
        s.prepare(48_000.0, 256);
        s
    }

    fn render_notes(s: &mut Subtractive, notes: &[NoteEvent], frames: usize, out: &mut Vec<f32>) {
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: 48_000.0,
            frames,
            channels: 1,
        };
        let mut io = NodeIo::new(notes, &[], out, frames);
        s.process(&ctx, &mut io);
    }

    #[test]
    fn eight_voices_then_oldest_is_stolen() {
        let mut s = synth();
        let mut buf = Vec::new();

        let ons: Vec<NoteEvent> = (60u8..68)
            .map(|p| NoteEvent {
                frame: 0,
                kind: NoteKind::On { pitch: p, vel: 100 },
            })
            .collect();
        render_notes(&mut s, &ons, 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8);

        // A 9th note must steal the oldest (pitch 60), keeping 8 voices.
        let ninth = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 72,
                vel: 100,
            },
        }];
        render_notes(&mut s, &ninth, 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8);
        let pitches = s.active_pitches();
        assert!(pitches.contains(&72), "new note should be present");
        assert!(!pitches.contains(&60), "oldest note should be stolen");
    }

    #[test]
    fn all_notes_off_releases_every_voice() {
        // Regression: stopping the transport must not leave sequenced voices
        // ringing — every active voice goes to release and falls silent.
        let mut s = synth();
        let mut buf = Vec::new();
        let ons: Vec<NoteEvent> = (60u8..64)
            .map(|p| NoteEvent {
                frame: 0,
                kind: NoteKind::On { pitch: p, vel: 100 },
            })
            .collect();
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

    #[test]
    fn held_note_is_audible() {
        // Regression: a Db parameter (level) was initialised to ~-60 dB, making
        // the synth (and the gain effect) effectively silent.
        let mut s = synth();
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 64,
                vel: 110,
            },
        }];
        let mut buf = Vec::new();
        render_notes(&mut s, &on, 2048, &mut buf); // past the attack, into sustain
        let peak = buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.05, "synth output is inaudible, peak {peak}");
    }

    #[test]
    fn release_is_click_free() {
        let mut s = synth();
        // Shortest release: it is floored to MIN_RELEASE_S, never instantaneous.
        s.set_param(P_RELEASE, 0.0);
        let mut buf = Vec::new();

        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 64,
                vel: 110,
            },
        }];
        render_notes(&mut s, &on, 2048, &mut buf); // reach sustain
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
    fn cutoff_sweep_has_no_zipper() {
        let mut s = synth();
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 57,
                vel: 110,
            },
        }];
        let mut buf = Vec::new();
        render_notes(&mut s, &on, 256, &mut buf);

        // Sweep cutoff 200 Hz → 8 kHz over many blocks while the note is held.
        let mut whole = Vec::new();
        for k in 0..64 {
            let v = k as f32 / 63.0;
            s.set_param(P_CUTOFF, v);
            render_notes(&mut s, &[], 256, &mut buf);
            whole.extend_from_slice(&buf);
        }
        let max_delta = whole
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_delta < 0.5,
            "cutoff sweep zipper, max delta {max_delta}"
        );
    }
}
