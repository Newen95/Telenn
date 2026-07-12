//! Subtractive synthesiser: 8-voice polyphony, band-limited oscillators
//! (polyBLEP saw / pulse / triangle) + white noise + a detunable second
//! oscillator → resonant low-pass (RBJ biquad) → ADSR amplitude envelope.
//!
//! All DSP is hand-written. `process` is RT-SAFE: a fixed voice array, no
//! allocation, no locks. Voice stealing takes the oldest voice.

use crate::engine::dsp::{next_white, AdsrSettings, Biquad, Env, MIN_RELEASE_S, RNG_SEED};
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
// Appended (ids are frozen — presets address params by name, order by id).
const P_WAVE: ParamId = 7;
const P_WIDTH: ParamId = 8;
const P_NOISE: ParamId = 9;
const P_OSC2: ParamId = 10;
const P_DETUNE: ParamId = 11;

/// Waveform selector values (P_WAVE, rounded): saw, pulse, triangle.
const WAVE_SAW: usize = 0;
const WAVE_PULSE: usize = 1;
const WAVE_TRIANGLE: usize = 2;

const PARAMS: [ParamSpec; 12] = [
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
    ParamSpec {
        id: P_WAVE,
        name: "wave",
        unit: "",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_WIDTH,
        name: "width",
        unit: "",
        min: 0.05,
        max: 0.5,
        default: 0.5,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_NOISE,
        name: "noise",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_OSC2,
        name: "osc2",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_DETUNE,
        name: "detune",
        unit: "ct",
        min: 0.0,
        max: 50.0,
        default: 7.0,
        curve: ParamCurve::Linear,
    },
];

/// Per-block oscillator settings, shared by every voice (smoothed once per
/// block; `det` is the osc2 frequency ratio derived from the detune cents).
#[derive(Clone, Copy)]
struct OscParams {
    wave: usize,
    width: f32,
    noise: f32,
    osc2: f32,
    det: f32,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    pitch: u8,
    active: bool,
    age: u64,
    /// Oscillator phase in turns (0..1).
    phase: f32,
    /// Second oscillator phase (runs at `inc · det`).
    phase2: f32,
    /// Phase increment per sample (`freq / sr`).
    inc: f32,
    /// Triangle leaky-integrator states (one per oscillator).
    tri1: f32,
    tri2: f32,
    /// Velocity amplitude (0..1).
    amp: f32,
    filter: Biquad,
    env: Env,
}

impl Voice {
    // RT-SAFE
    fn render(&mut self, p: &OscParams, white: f32, adsr: &AdsrSettings, sr: f32) -> f32 {
        let o1 = osc_sample(p.wave, self.phase, self.inc, p.width, &mut self.tri1);
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // The second oscillator only runs once mixed in; its level fades the
        // normalized sum continuously, so osc2 = 0 is exactly the legacy voice.
        let osc = if p.osc2 > 0.0 {
            let inc2 = self.inc * p.det;
            let o2 = osc_sample(p.wave, self.phase2, inc2, p.width, &mut self.tri2);
            self.phase2 += inc2;
            if self.phase2 >= 1.0 {
                self.phase2 -= 1.0;
            }
            (o1 + p.osc2 * o2) / (1.0 + p.osc2)
        } else {
            o1
        };
        // Noise joins BEFORE the filter, so cutoff/resonance shape it (wind,
        // breath); the crossfade keeps noise = 0 bit-identical to the old path.
        let raw = if p.noise > 0.0 {
            osc * (1.0 - p.noise) + white * p.noise
        } else {
            osc
        };
        let filtered = self.filter.process(raw);
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
    wave_idx: usize,
    width: OnePole,
    noise: OnePole,
    osc2: OnePole,
    detune: OnePole,
    /// xorshift32 state — deterministic white noise, no crate, no alloc.
    rng: u32,
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
            wave_idx: WAVE_SAW,
            width: OnePole::new(0.5, 0.99),
            noise: OnePole::new(0.0, 0.99),
            osc2: OnePole::new(0.0, 0.99),
            detune: OnePole::new(7.0, 0.99),
            rng: RNG_SEED,
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
        // Oscillator settings once per block; det converts cents → freq ratio.
        let osc = OscParams {
            wave: self.wave_idx,
            width: self.width.advance(frames),
            noise: self.noise.advance(frames),
            osc2: self.osc2.advance(frames),
            det: (self.detune.advance(frames) / 1200.0).exp2(),
        };

        let out = io.output(0);
        for slot in out.iter_mut() {
            let level = self.level.tick();
            // One noise source per frame, enveloped per voice (a shared VCA'd
            // noise bed — cheaper than a generator per voice, same sound).
            let white = next_white(&mut self.rng);
            let mut sum = 0.0;
            for v in &mut self.voices {
                if v.active {
                    sum += v.render(&osc, white, &self.adsr, sr);
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
            P_WAVE => self.wave_idx = (real.round() as usize).min(WAVE_TRIANGLE),
            P_WIDTH => self.width.set_target(real),
            P_NOISE => self.noise.set_target(real),
            P_OSC2 => self.osc2.set_target(real),
            P_DETUNE => self.detune.set_target(real),
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
        self.wave_idx = default_real(P_WAVE).round() as usize;
        self.width = OnePole::new(default_real(P_WIDTH), coef);
        self.noise = OnePole::new(default_real(P_NOISE), coef);
        self.osc2 = OnePole::new(default_real(P_OSC2), coef);
        self.detune = OnePole::new(default_real(P_DETUNE), coef);
        self.rng = RNG_SEED; // fixed seed: offline renders stay deterministic
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

/// One band-limited oscillator sample. `tri` is the voice's leaky-integrator
/// state, used by the triangle only. // RT-SAFE
fn osc_sample(wave: usize, phase: f32, inc: f32, width: f32, tri: &mut f32) -> f32 {
    match wave {
        WAVE_PULSE => pulse_blep(phase, inc, width),
        WAVE_TRIANGLE => {
            // Leaky integral of the width-0.5 pulse: slope ±4·inc gives a ~±1
            // triangle; the 0.999 leak keeps DC from accumulating.
            let sq = pulse_blep(phase, inc, 0.5);
            *tri = *tri * 0.999 + sq * 4.0 * inc;
            *tri
        }
        _ => 2.0 * phase - 1.0 - poly_blep(phase, inc),
    }
}

/// Band-limited pulse: BLEP at BOTH edges (rise at 0, fall at `width`), plus
/// the DC the asymmetric duty cycle would otherwise leave. // RT-SAFE
fn pulse_blep(phase: f32, inc: f32, width: f32) -> f32 {
    let raw = if phase < width { 1.0 } else { -1.0 };
    raw + poly_blep(phase, inc)
        - poly_blep((phase - width).rem_euclid(1.0), inc)
        - (2.0 * width - 1.0)
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
            samples_per_beat: 0.0,
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

    /// Single-bin DFT magnitude (Goertzel-style), normalized by length.
    fn mag_at(buf: &[f32], freq: f32, sr: f32) -> f32 {
        let mut re = 0.0f32;
        let mut im = 0.0f32;
        for (n, &s) in buf.iter().enumerate() {
            let w = core::f32::consts::TAU * freq * n as f32 / sr;
            re += s * w.cos();
            im += s * w.sin();
        }
        (re * re + im * im).sqrt() / buf.len() as f32
    }

    /// Render a settled sustain of pitch 57 (220 Hz) with the filter wide open.
    fn sustain_at(s: &mut Subtractive, frames: usize) -> Vec<f32> {
        s.set_param(P_CUTOFF, 1.0); // 20 kHz: measure the oscillator, not the filter
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 57,
                vel: 110,
            },
        }];
        let mut buf = Vec::new();
        render_notes(s, &on, 4096, &mut buf); // settle smoothers + envelope
        let mut tail = Vec::new();
        render_notes(s, &[], frames, &mut tail);
        tail
    }

    fn set_real(s: &mut Subtractive, id: ParamId, real: f32) {
        s.set_param(id, PARAMS[id as usize].normalize(real));
    }

    #[test]
    fn square_spectrum_is_odd_only() {
        let mut s = synth();
        set_real(&mut s, P_WAVE, 1.0); // pulse, default width 0.5 = square
        let tail = sustain_at(&mut s, 8192);
        let h2 = mag_at(&tail, 440.0, 48_000.0);
        let h3 = mag_at(&tail, 660.0, 48_000.0);
        assert!(
            h3 > 10.0 * h2,
            "square should have no even harmonics: H2 {h2}, H3 {h3}"
        );
    }

    #[test]
    fn pulse_width_adds_even_harmonics() {
        let mut s = synth();
        set_real(&mut s, P_WAVE, 1.0);
        set_real(&mut s, P_WIDTH, 0.25);
        let tail = sustain_at(&mut s, 8192);
        let h1 = mag_at(&tail, 220.0, 48_000.0);
        let h2 = mag_at(&tail, 440.0, 48_000.0);
        // At 25% duty the 2nd harmonic is strong (comparable to the 1st).
        assert!(
            h2 > 0.3 * h1,
            "narrow pulse should grow H2: H1 {h1}, H2 {h2}"
        );
        // And the duty asymmetry must not leave DC behind.
        let dc = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(dc.abs() < 0.005, "pulse leaves DC {dc}");
    }

    #[test]
    fn triangle_is_darker_than_saw() {
        let mut saw = synth();
        let saw_tail = sustain_at(&mut saw, 8192);
        let mut tri = synth();
        set_real(&mut tri, P_WAVE, 2.0);
        let tri_tail = sustain_at(&mut tri, 8192);
        // Normalize by each wave's fundamental: triangle harmonics fall as
        // 1/n² (and evens vanish) vs the saw's 1/n.
        let rel = |t: &[f32], h: f32| mag_at(t, 220.0 * h, 48_000.0) / mag_at(t, 220.0, 48_000.0);
        assert!(
            rel(&saw_tail, 4.0) > 5.0 * rel(&tri_tail, 4.0),
            "triangle should lack H4"
        );
        assert!(
            rel(&saw_tail, 5.0) > 2.0 * rel(&tri_tail, 5.0),
            "triangle H5 should sit far below the saw's"
        );
    }

    #[test]
    fn full_noise_is_spectrally_flat() {
        use crate::analysis::{fft, spectral};
        let mut s = synth();
        set_real(&mut s, P_NOISE, 1.0);
        let tail = sustain_at(&mut s, 8192);
        let power = fft::power_spectrum(&tail).expect("power of two");
        let flatness = spectral::spectral_flatness_db(&power, tail.len(), 48_000);
        assert!(
            flatness > -20.0,
            "full noise should be broadband, flatness {flatness} dB"
        );
    }

    #[test]
    fn detuned_second_oscillator_beats() {
        let mut s = synth();
        set_real(&mut s, P_OSC2, 1.0);
        set_real(&mut s, P_DETUNE, 30.0); // ≈ 3.8 Hz beat at 220 Hz
                                          // Isolate the fundamental (cutoff just above 220 Hz): each harmonic
                                          // beats at its own rate, so the full spectrum never dips together —
                                          // the fundamental alone cancels nearly completely.
        set_real(&mut s, P_CUTOFF, 300.0);
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 57,
                vel: 110,
            },
        }];
        let mut buf = Vec::new();
        render_notes(&mut s, &on, 4096, &mut buf);
        let mut tail = Vec::new();
        render_notes(&mut s, &[], 48_000, &mut tail);
        // Windowed RMS over the sustain: the level must swell and dip.
        let rms: Vec<f32> = tail
            .chunks(4800) // 100 ms
            .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt())
            .collect();
        let hi = rms.iter().fold(0.0f32, |m, &v| m.max(v));
        let lo = rms.iter().fold(f32::MAX, |m, &v| m.min(v));
        assert!(hi > 2.0 * lo, "no beating heard: rms {lo}..{hi}");
    }

    #[test]
    fn smoothed_param_sweeps_are_click_free() {
        use crate::analysis::levels;
        let mut s = synth();
        set_real(&mut s, P_WAVE, 1.0);
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 57,
                vel: 110,
            },
        }];
        let mut buf = Vec::new();
        render_notes(&mut s, &on, 4096, &mut buf);
        // Sweep every glided oscillator param while the note holds.
        let mut whole = Vec::new();
        for k in 0..64 {
            let v = k as f32 / 63.0;
            s.set_param(P_WIDTH, 1.0 - v);
            s.set_param(P_NOISE, v);
            s.set_param(P_OSC2, v);
            s.set_param(P_DETUNE, v);
            render_notes(&mut s, &[], 256, &mut buf);
            whole.extend_from_slice(&buf);
        }
        assert!(whole.iter().all(|x| x.is_finite()));
        let clicks = levels::detect_clicks(&whole, 48_000);
        assert!(clicks.is_empty(), "sweep clicked: {clicks:?}");
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
