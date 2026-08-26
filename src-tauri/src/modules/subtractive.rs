//! Subtractive synthesiser: 8-voice polyphony, band-limited oscillators
//! (polyBLEP saw / pulse / triangle) + white noise + a detunable second
//! oscillator → resonant low-pass (RBJ biquad) → ADSR amplitude envelope.
//! An optional filter envelope opens the cutoff at note-on and closes it
//! exponentially — the bright-attack / dark-sustain shape of classic synth
//! basses and plucks.
//!
//! All DSP is hand-written. `process` is RT-SAFE: a fixed voice array, no
//! allocation, no locks. Voice stealing takes the oldest voice.

use crate::engine::dsp::{flush, next_white, AdsrSettings, Biquad, Env, MIN_RELEASE_S, RNG_SEED};
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
const P_FENV: ParamId = 12;
const P_FENV_DECAY: ParamId = 13;
const P_VIB_RATE: ParamId = 14;
const P_VIB_DEPTH: ParamId = 15;
const P_UNISON: ParamId = 16;
const P_SPREAD: ParamId = 17;

/// Hard cap on unison sub-oscillators per voice (7 × 8 voices = 56 osc).
const MAX_UNISON: usize = 7;

/// Waveform selector values (P_WAVE, rounded): saw, pulse, triangle.
const WAVE_SAW: usize = 0;
const WAVE_PULSE: usize = 1;
const WAVE_TRIANGLE: usize = 2;

const PARAMS: [ParamSpec; 18] = [
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
    // Filter envelope: octaves added to the cutoff at note-on (0 = off, the
    // legacy static filter), closing exponentially over `fenv_decay`.
    ParamSpec {
        id: P_FENV,
        name: "fenv",
        unit: "oct",
        min: -4.0,
        max: 4.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_FENV_DECAY,
        name: "fenv_decay",
        unit: "ms",
        min: 5.0,
        max: 2000.0,
        default: 200.0,
        curve: ParamCurve::Log,
    },
    // Module-wide vibrato LFO (block-rate pitch wobble; depth 0 = off).
    ParamSpec {
        id: P_VIB_RATE,
        name: "vib_rate",
        unit: "Hz",
        min: 0.1,
        max: 12.0,
        default: 5.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_VIB_DEPTH,
        name: "vib_depth",
        unit: "ct",
        min: 0.0,
        max: 100.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    // Unison: N detuned sub-oscillators fanned across the stereo field
    // (1 = off, the legacy voice; > 1 replaces the osc2 pair).
    ParamSpec {
        id: P_UNISON,
        name: "unison",
        unit: "",
        min: 1.0,
        max: 7.0,
        default: 1.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_SPREAD,
        name: "spread",
        unit: "ct",
        min: 0.0,
        max: 50.0,
        default: 12.0,
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
    /// Vibrato frequency multiplier for this block (1.0 = none).
    vib: f32,
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
    /// Filter-envelope level: 1.0 at note-on, exponential decay per block
    /// (a one-field AD envelope — the attack is the note-on itself).
    fenv_level: f32,
    /// Unison sub-oscillator phases and triangle states (used when
    /// `unison > 1`; the legacy fields above cover the single-osc path).
    phases: [f32; MAX_UNISON],
    tris: [f32; MAX_UNISON],
    /// Right-channel filter for the stereo unison path (same coefficients as
    /// `filter`, separate state).
    filter2: Biquad,
}

/// Per-block unison settings: detune ratio and constant-power pan gains for
/// each sub-oscillator, plus the 1/√N loudness normalization.
#[derive(Clone, Copy)]
struct UnisonParams {
    n: usize,
    ratios: [f32; MAX_UNISON],
    gl: [f32; MAX_UNISON],
    gr: [f32; MAX_UNISON],
    norm: f32,
}

impl UnisonParams {
    /// Sub-oscillator k sits at `pos` in −1..1: detuned by `spread · pos`
    /// cents and panned there (constant-power). Mono collapses the pan.
    fn build(n: usize, spread_ct: f32, stereo: bool) -> Self {
        let mut u = Self {
            n,
            ratios: [1.0; MAX_UNISON],
            gl: [1.0; MAX_UNISON],
            gr: [0.0; MAX_UNISON],
            norm: 1.0 / (n as f32).sqrt(),
        };
        for k in 0..n {
            let pos = if n > 1 {
                2.0 * k as f32 / (n - 1) as f32 - 1.0
            } else {
                0.0
            };
            u.ratios[k] = (spread_ct * pos / 1200.0).exp2();
            if stereo {
                let theta = (pos + 1.0) * core::f32::consts::FRAC_PI_4;
                u.gl[k] = theta.cos();
                u.gr[k] = theta.sin();
            }
        }
        u
    }
}

impl Voice {
    // RT-SAFE
    fn render(&mut self, p: &OscParams, white: f32, adsr: &AdsrSettings, sr: f32) -> f32 {
        let inc = self.inc * p.vib;
        let o1 = osc_sample(p.wave, self.phase, inc, p.width, &mut self.tri1);
        self.phase += inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // The second oscillator only runs once mixed in; its level fades the
        // normalized sum continuously, so osc2 = 0 is exactly the legacy voice.
        let osc = if p.osc2 > 0.0 {
            let inc2 = inc * p.det;
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

    /// Unison render: N detuned sub-oscillators panned into an L/R pair,
    /// each side through its own filter (same coefficients). Mono callers
    /// read only `.0`. Replaces (does not stack with) the osc2 pair. // RT-SAFE
    fn render_unison(
        &mut self,
        p: &OscParams,
        u: &UnisonParams,
        white: f32,
        adsr: &AdsrSettings,
        sr: f32,
    ) -> (f32, f32) {
        let mut l = 0.0;
        let mut r = 0.0;
        for k in 0..u.n {
            let inc = self.inc * p.vib * u.ratios[k];
            let s = osc_sample(p.wave, self.phases[k], inc, p.width, &mut self.tris[k]);
            self.phases[k] += inc;
            if self.phases[k] >= 1.0 {
                self.phases[k] -= 1.0;
            }
            l += s * u.gl[k];
            r += s * u.gr[k];
        }
        l *= u.norm;
        r *= u.norm;
        if p.noise > 0.0 {
            l = l * (1.0 - p.noise) + white * p.noise;
            r = r * (1.0 - p.noise) + white * p.noise;
        }
        let fl = self.filter.process(l);
        let fr = self.filter2.process(r);
        let env = self.env.tick(adsr, sr);
        if !self.env.is_active() {
            self.active = false;
        }
        (fl * env * self.amp, fr * env * self.amp)
    }
}

pub struct Subtractive {
    sample_rate: f32,
    voices: [Voice; VOICES],
    cutoff: OnePole,
    resonance: OnePole,
    fenv_oct: OnePole,
    fenv_decay_s: f32,
    vib_rate: OnePole,
    vib_depth: OnePole,
    /// Vibrato LFO phase in turns (0..1), advanced per block.
    vib_phase: f32,
    /// Unison sub-oscillator count (1 = off, legacy voice path).
    unison_n: usize,
    spread: OnePole,
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
            fenv_oct: OnePole::new(0.0, 0.99),
            fenv_decay_s: 0.2,
            vib_rate: OnePole::new(5.0, 0.99),
            vib_depth: OnePole::new(0.0, 0.99),
            vib_phase: 0.0,
            unison_n: 1,
            spread: OnePole::new(12.0, 0.99),
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
        v.fenv_level = 1.0;
        // Deterministically decorrelated unison phases: an aligned start
        // would sum to a single loud saw for the first few cycles.
        for (k, ph) in v.phases.iter_mut().enumerate() {
            *ph = (k as f32 * 0.618).fract();
        }
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
        let unison = self.unison_n.clamp(1, MAX_UNISON);
        let stereo_unison = unison > 1 && io.channels() > 1;
        let fenv_oct = self.fenv_oct.advance(frames);
        // Per-voice cutoff, block-rate like the cutoff glide: the filter
        // envelope opens `fenv_oct` octaves at note-on and closes
        // exponentially; at 0 this is the legacy static filter exactly.
        let close = (-(frames as f32) / (self.fenv_decay_s * sr)).exp();
        for v in &mut self.voices {
            if !v.active {
                continue;
            }
            let fc_v = if fenv_oct != 0.0 {
                let f = (fc * (fenv_oct * v.fenv_level).exp2()).clamp(20.0, 20_000.0);
                v.fenv_level = flush(v.fenv_level * close);
                f
            } else {
                fc
            };
            v.filter.set_lowpass(fc_v, q, sr);
            if stereo_unison {
                v.filter2.set_lowpass(fc_v, q, sr);
            }
        }
        // Oscillator settings once per block; det converts cents → freq ratio.
        // The vibrato multiplier is sampled once per block (5.3 ms at 256):
        // a staircase inaudible at ≤ 12 Hz / 100 ct.
        let vib_depth = self.vib_depth.advance(frames);
        let vib_rate = self.vib_rate.advance(frames);
        let vib = if vib_depth > 0.0 {
            let m = (vib_depth * (core::f32::consts::TAU * self.vib_phase).sin() / 1200.0).exp2();
            self.vib_phase = (self.vib_phase + vib_rate * frames as f32 / sr).fract();
            m
        } else {
            1.0
        };
        let osc = OscParams {
            wave: self.wave_idx,
            width: self.width.advance(frames),
            noise: self.noise.advance(frames),
            osc2: self.osc2.advance(frames),
            det: (self.detune.advance(frames) / 1200.0).exp2(),
            vib,
        };

        if unison > 1 {
            let uni = UnisonParams::build(unison, self.spread.advance(frames), stereo_unison);
            if stereo_unison {
                for i in 0..frames {
                    let level = self.level.tick();
                    let white = next_white(&mut self.rng);
                    let mut sum_l = 0.0;
                    let mut sum_r = 0.0;
                    for v in &mut self.voices {
                        if v.active {
                            let (l, r) = v.render_unison(&osc, &uni, white, &self.adsr, sr);
                            sum_l += l;
                            sum_r += r;
                        }
                    }
                    io.output_ch(0, 0)[i] = sum_l * level;
                    io.output_ch(0, 1)[i] = sum_r * level;
                    for ch in 2..io.channels() {
                        let left = io.output_ch(0, 0)[i];
                        io.output_ch(0, ch)[i] = left;
                    }
                }
            } else {
                let out = io.output(0);
                for slot in out.iter_mut() {
                    let level = self.level.tick();
                    let white = next_white(&mut self.rng);
                    let mut sum = 0.0;
                    for v in &mut self.voices {
                        if v.active {
                            sum += v.render_unison(&osc, &uni, white, &self.adsr, sr).0;
                        }
                    }
                    *slot = sum * level;
                }
                io.spread_output(0);
            }
        } else {
            let out = io.output(0);
            for slot in out.iter_mut() {
                let level = self.level.tick();
                // One noise source per frame, enveloped per voice (a shared
                // VCA'd noise bed — cheaper than one generator per voice).
                let white = next_white(&mut self.rng);
                let mut sum = 0.0;
                for v in &mut self.voices {
                    if v.active {
                        sum += v.render(&osc, white, &self.adsr, sr);
                    }
                }
                *slot = sum * level;
            }
            io.spread_output(0);
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
            P_FENV => self.fenv_oct.set_target(real),
            P_FENV_DECAY => self.fenv_decay_s = real / 1000.0,
            P_VIB_RATE => self.vib_rate.set_target(real),
            P_VIB_DEPTH => self.vib_depth.set_target(real),
            P_UNISON => self.unison_n = (real.round() as usize).clamp(1, MAX_UNISON),
            P_SPREAD => self.spread.set_target(real),
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
        self.fenv_oct = OnePole::new(default_real(P_FENV), coef);
        self.fenv_decay_s = default_real(P_FENV_DECAY) / 1000.0;
        self.vib_rate = OnePole::new(default_real(P_VIB_RATE), coef);
        self.vib_depth = OnePole::new(default_real(P_VIB_DEPTH), coef);
        self.vib_phase = 0.0;
        self.unison_n = default_real(P_UNISON).round() as usize;
        self.spread = OnePole::new(default_real(P_SPREAD), coef);
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
            beat_phase: 0.0,
        };
        let mut io = NodeIo::new(notes, &[], out, frames, 1);
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
    fn filter_env_zero_ignores_the_decay_setting() {
        // fenv = 0 must stay the exact legacy static-filter path: the decay
        // knob alone changes nothing.
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 36,
                vel: 110,
            },
        }];
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut s = synth();
        s.set_param(P_FENV_DECAY, 0.0);
        render_notes(&mut s, &on, 8192, &mut a);
        let mut s = synth();
        s.set_param(P_FENV_DECAY, 1.0);
        render_notes(&mut s, &on, 8192, &mut b);
        assert_eq!(a, b, "fenv 0 must ignore fenv_decay entirely");
    }

    /// Goertzel magnitude over a window of the buffer.
    fn mag_win(buf: &[f32], from: usize, len: usize, freq: f32, sr: f32) -> f32 {
        mag_at(&buf[from..from + len], freq, sr)
    }

    /// Render in engine-sized blocks (256): block-rate behaviour (filter
    /// envelope, glides) advances as it does live — a single giant block
    /// would freeze the per-block state at its first value.
    fn render_chunked(s: &mut Subtractive, notes: &[NoteEvent], frames: usize, out: &mut Vec<f32>) {
        out.clear();
        let mut chunk = Vec::new();
        let mut done = 0;
        while done < frames {
            let n = 256.min(frames - done);
            render_notes(s, if done == 0 { notes } else { &[] }, n, &mut chunk);
            out.extend_from_slice(&chunk);
            done += n;
        }
    }

    #[test]
    fn filter_env_brightens_the_attack_then_closes() {
        // Low saw, filter at 150 Hz, envelope +3 octaves closing over 100 ms:
        // a high harmonic must be loud in the attack window and gone from the
        // sustain — the bright-attack / dark-sustain plume.
        let mut s = synth();
        set_real(&mut s, P_CUTOFF, 150.0);
        set_real(&mut s, P_FENV, 3.0);
        set_real(&mut s, P_FENV_DECAY, 100.0);
        s.set_param(P_ATTACK, 0.0);
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 36, // C2, 65.4 Hz
                vel: 110,
            },
        }];
        let mut buf = Vec::new();
        render_chunked(&mut s, &on, 48_000, &mut buf);
        let f0 = midi_to_freq(36);
        let h8 = f0 * 8.0; // 523 Hz: above the 150 Hz base, under the opened peak
        let attack = mag_win(&buf, 0, 2400, h8, 48_000.0);
        let sustain = mag_win(&buf, 24_000, 2400, h8, 48_000.0);
        assert!(
            attack > sustain * 4.0,
            "h8 attack {attack} vs sustain {sustain}: the envelope did not close"
        );
        let h1_sustain = mag_win(&buf, 24_000, 2400, f0, 48_000.0);
        assert!(h1_sustain > 0.01, "fundamental vanished from the sustain");
    }

    #[test]
    fn vibrato_zero_ignores_the_rate_setting() {
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 69,
                vel: 110,
            },
        }];
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut s = synth();
        s.set_param(P_VIB_RATE, 0.0);
        render_chunked(&mut s, &on, 8192, &mut a);
        let mut s = synth();
        s.set_param(P_VIB_RATE, 1.0);
        render_chunked(&mut s, &on, 8192, &mut b);
        assert_eq!(a, b, "vib_depth 0 must ignore vib_rate entirely");
    }

    #[test]
    fn vibrato_wobbles_the_pitch() {
        let mut s = synth();
        s.set_param(P_CUTOFF, 1.0); // wide open: give YIN a clean waveform
        set_real(&mut s, P_VIB_RATE, 2.0);
        set_real(&mut s, P_VIB_DEPTH, 50.0);
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 69, // A4
                vel: 110,
            },
        }];
        let mut buf = Vec::new();
        render_chunked(&mut s, &on, 48_000, &mut buf);
        // f0 over sliding 50 ms windows: ±50 ct peak depth reads as ≥ 3 %
        // total excursion even with window averaging.
        let (mut lo, mut hi) = (f32::MAX, 0.0f32);
        for start in (4800..43_200).step_by(2400) {
            if let Some(f0) = crate::analysis::pitch::estimate_f0(&buf[start..start + 2400], 48_000)
            {
                lo = lo.min(f0.hz);
                hi = hi.max(f0.hz);
            }
        }
        assert!(hi > 0.0 && lo < f32::MAX, "no pitch detected");
        let span = (hi - lo) / 440.0;
        assert!(span > 0.03, "vibrato span {span} too small ({lo}..{hi} Hz)");
    }

    /// Stereo chunked render: `out` holds [ch0.., ch1..] per 256-frame block,
    /// re-flattened to two contiguous channels on return.
    fn render_stereo_chunked(
        s: &mut Subtractive,
        notes: &[NoteEvent],
        frames: usize,
    ) -> (Vec<f32>, Vec<f32>) {
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        let mut out = vec![0.0f32; 2 * 256];
        let mut done = 0;
        while done < frames {
            let n = 256.min(frames - done);
            out.resize(2 * n, 0.0);
            let ctx = ProcessCtx {
                sample_rate: 48_000.0,
                frames: n,
                channels: 2,
                samples_per_beat: 0.0,
                beat_phase: 0.0,
            };
            let evs = if done == 0 { notes } else { &[] };
            let mut io = NodeIo::new(evs, &[], &mut out, n, 2);
            s.process(&ctx, &mut io);
            left.extend_from_slice(&out[..n]);
            right.extend_from_slice(&out[n..2 * n]);
            done += n;
        }
        (left, right)
    }

    #[test]
    fn unison_one_ignores_the_spread_setting() {
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 57,
                vel: 110,
            },
        }];
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut s = synth();
        s.set_param(P_SPREAD, 0.0);
        render_chunked(&mut s, &on, 8192, &mut a);
        let mut s = synth();
        s.set_param(P_SPREAD, 1.0);
        render_chunked(&mut s, &on, 8192, &mut b);
        assert_eq!(a, b, "unison 1 must ignore spread entirely");
    }

    #[test]
    fn unison_widens_and_thickens() {
        let mut s = synth();
        s.set_param(P_CUTOFF, 1.0);
        set_real(&mut s, P_UNISON, 5.0);
        set_real(&mut s, P_SPREAD, 30.0);
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 57, // A3
                vel: 110,
            },
        }];
        let (l, r) = render_stereo_chunked(&mut s, &on, 24_000);
        let corr = crate::analysis::levels::stereo_correlation(&l, &r);
        assert!(
            corr < 0.95,
            "unison spread left the image mono (correlation {corr})"
        );
        assert!(corr > -0.5, "channels out of phase (correlation {corr})");
        // Both sides carry signal, and no clicks slip through the detune sum.
        let peak_r = r.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        assert!(peak_r > 0.02, "right channel silent, peak {peak_r}");
        let clicks = crate::analysis::levels::detect_clicks(&l, 48_000);
        assert!(clicks.is_empty(), "unison clicked: {clicks:?}");
    }

    #[test]
    fn filter_env_sweep_is_click_free() {
        let mut s = synth();
        set_real(&mut s, P_CUTOFF, 200.0);
        set_real(&mut s, P_FENV, 3.5);
        set_real(&mut s, P_FENV_DECAY, 80.0);
        let mut out = Vec::new();
        let mut chunk = Vec::new();
        for i in 0..8 {
            let on = [NoteEvent {
                frame: 0,
                kind: NoteKind::On {
                    pitch: 36 + (i % 3) * 12,
                    vel: 110,
                },
            }];
            render_chunked(&mut s, &on, 6000, &mut chunk);
            out.extend_from_slice(&chunk);
        }
        let clicks = crate::analysis::levels::detect_clicks(&out, 48_000);
        assert!(clicks.is_empty(), "filter env clicked: {clicks:?}");
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
