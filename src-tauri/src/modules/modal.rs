//! Modal percussion: 8-voice polyphony, each note a bank of damped sinusoid
//! modes (complex phasors) tuned by a material's physical frequency ratios —
//! marimba bar, vibraphone bar, church bell, glass.
//!
//! The complementary physics to `plucked`: a struck resonator instead of a
//! plucked string. High modes die faster than the fundamental, so the
//! spectrum LIVES during the note (bright strike settling into a purer hum).
//! All DSP is hand-written. `process` is RT-SAFE: a fixed voice/mode array,
//! no allocation, no locks. Voice stealing takes the oldest voice.

use std::f32::consts::TAU;

use crate::engine::dsp::{flush, next_white, RNG_SEED};
use crate::engine::node::{
    AudioNode, NodeIo, NoteKind, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx,
    SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

const VOICES: usize = 8;
const MODES: usize = 8;
/// Brightness/strike glide time constant (seconds).
const SMOOTH_S: f32 = 0.015;
/// Strike noise burst length (seconds) at strike = 1.
const STRIKE_S: f32 = 0.005;

const P_MATERIAL: ParamId = 0;
const P_DECAY: ParamId = 1;
const P_BRIGHTNESS: ParamId = 2;
const P_STRIKE: ParamId = 3;
const P_RELEASE: ParamId = 4;
const P_LEVEL: ParamId = 5;
const P_TUNE: ParamId = 6;

/// A material: mode frequency ratios (× the struck pitch), relative
/// amplitudes, and how steeply higher modes lose their ring
/// (`τ_k = τ · (f₁/f_k)^damp_slope`).
struct Material {
    ratios: [f32; MODES],
    amps: [f32; MODES],
    damp_slope: f32,
}

/// Material selector values (P_MATERIAL, rounded): wood, metal, bell, glass.
/// Ratios approximate the measured partials of the real instruments (a
/// marimba bar's deep-arch cut tunes H2 near 4:1; a church bell rings a hum
/// tone below its strike note; glass modes stretch quadratically).
const MATERIALS: [Material; 4] = [
    // wood (marimba): strong fundamental, fast-dying upper modes.
    Material {
        ratios: [1.0, 3.9, 9.2, 15.5, 22.9, 29.0, 35.5, 41.0],
        amps: [1.0, 0.55, 0.3, 0.16, 0.09, 0.05, 0.03, 0.02],
        damp_slope: 0.9,
    },
    // metal (vibraphone): near-harmonic tuned bar, long ring.
    Material {
        ratios: [1.0, 4.0, 10.0, 17.3, 26.5, 37.2, 49.0, 62.0],
        amps: [1.0, 0.4, 0.22, 0.12, 0.07, 0.04, 0.02, 0.01],
        damp_slope: 0.5,
    },
    // bell: hum tone below the strike note, dense minor-third cluster.
    Material {
        ratios: [0.56, 1.0, 1.19, 1.56, 2.0, 2.51, 2.66, 3.01],
        amps: [0.6, 1.0, 0.65, 0.5, 0.55, 0.35, 0.3, 0.25],
        damp_slope: 0.4,
    },
    // glass: sparse, stretched, crystalline.
    Material {
        ratios: [1.0, 2.32, 4.25, 6.63, 9.38, 12.5, 16.0, 19.9],
        amps: [1.0, 0.5, 0.35, 0.22, 0.14, 0.08, 0.05, 0.03],
        damp_slope: 0.6,
    },
];

const PARAMS: [ParamSpec; 7] = [
    ParamSpec {
        id: P_MATERIAL,
        name: "material",
        unit: "",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_DECAY,
        name: "decay",
        unit: "s",
        min: 0.1,
        max: 8.0,
        default: 1.5,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_BRIGHTNESS,
        name: "brightness",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_STRIKE,
        name: "strike",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.3,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_RELEASE,
        name: "release",
        unit: "ms",
        min: 1.0,
        max: 1000.0,
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
        id: P_TUNE,
        name: "tune",
        unit: "ct",
        min: -50.0,
        max: 50.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
];

/// One damped sinusoid: a complex phasor `z` rotated by `rot` each sample,
/// with the per-sample decay folded into the rotation's magnitude.
#[derive(Clone, Copy, Default)]
struct Mode {
    z_re: f32,
    z_im: f32,
    rot_re: f32,
    rot_im: f32,
}

impl Mode {
    // RT-SAFE
    fn tick(&mut self) -> f32 {
        let re = self.z_re * self.rot_re - self.z_im * self.rot_im;
        let im = self.z_re * self.rot_im + self.z_im * self.rot_re;
        self.z_re = flush(re);
        self.z_im = flush(im);
        im
    }

    /// Fold an extra per-sample decay into the rotation (note-off release).
    fn damp_by(&mut self, factor: f32) {
        self.rot_re *= factor;
        self.rot_im *= factor;
    }

    fn energy(&self) -> f32 {
        self.z_re * self.z_re + self.z_im * self.z_im
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    pitch: u8,
    active: bool,
    age: u64,
    modes: [Mode; MODES],
    /// Remaining strike-noise samples, its length, and its gain.
    burst: u32,
    burst_len: u32,
    burst_gain: f32,
}

impl Voice {
    // RT-SAFE
    fn render(&mut self, white: f32) -> f32 {
        let mut sum = 0.0;
        for m in &mut self.modes {
            sum += m.tick();
        }
        if self.burst > 0 {
            self.burst -= 1;
            // Linear fade over the burst keeps the mallet chiff click-free.
            sum += white * self.burst_gain * (self.burst as f32 / self.burst_len as f32);
        }
        sum
    }

    /// The bank has rung out when every phasor's energy is negligible.
    fn faded(&self) -> bool {
        self.burst == 0 && self.modes.iter().map(Mode::energy).sum::<f32>() < 1.0e-10
    }
}

pub struct Modal {
    sample_rate: f32,
    voices: [Voice; VOICES],
    material_idx: usize,
    /// τ of mode 1 in seconds (not smoothed: applied at note-on only).
    decay_s: f32,
    brightness: OnePole,
    strike: OnePole,
    /// Note-off accelerated damping, seconds.
    release_s: f32,
    level: LinearRamp,
    /// Global tune offset, cents (applied at note-on only).
    tune_ct: f32,
    rng: u32,
    next_age: u64,
}

impl Default for Modal {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            voices: [Voice::default(); VOICES],
            material_idx: 0,
            decay_s: 1.5,
            brightness: OnePole::new(0.5, 0.99),
            strike: OnePole::new(0.3, 0.99),
            release_s: 0.2,
            level: LinearRamp::new(1.0, 10.0, 48_000.0),
            tune_ct: 0.0,
            rng: RNG_SEED,
            next_age: 0,
        }
    }
}

impl Modal {
    fn note_on(&mut self, pitch: u8, vel: u8, brightness: f32, strike: f32) {
        let sr = self.sample_rate;
        let idx = self.alloc_voice();
        let age = self.next_age;
        self.next_age += 1;
        let mat = &MATERIALS[self.material_idx];
        let f0 = midi_to_freq(pitch) * (self.tune_ct / 1200.0).exp2();
        let amp = vel as f32 / 127.0;
        // Brightness tilts the bank: 0 keeps only the low modes, 1 opens the
        // whole ladder (amp_k scaled by tilt^k).
        let tilt = 0.35 + 0.65 * brightness;

        let v = &mut self.voices[idx];
        v.pitch = pitch;
        v.active = true;
        v.age = age;
        let mut tilt_k = 1.0;
        for (k, m) in v.modes.iter_mut().enumerate() {
            let f = f0 * mat.ratios[k];
            if f >= 0.45 * sr {
                *m = Mode::default(); // inaudible-legal: mode above the band is omitted
                tilt_k *= tilt;
                continue;
            }
            // Higher modes ring shorter: τ_k = τ · (f₁/f_k)^slope.
            let tau = self.decay_s * (mat.ratios[0].min(1.0) * f0 / f).powf(mat.damp_slope);
            let mag = (-1.0 / (tau * sr)).exp();
            let w = TAU * f / sr;
            m.rot_re = mag * w.cos();
            m.rot_im = mag * w.sin();
            // Strike at t=0: the phasor starts at full amplitude on the real
            // axis; `tick` reads the imaginary part, so the output rises from
            // zero like a struck sine — click-free by construction.
            m.z_re = mat.amps[k] * tilt_k * amp * 0.5;
            m.z_im = 0.0;
            tilt_k *= tilt;
        }
        // Mallet chiff: a few ms of enveloped noise, harder mallet = louder.
        v.burst_len = (STRIKE_S * sr) as u32;
        v.burst = v.burst_len;
        v.burst_gain = strike * amp * 0.6;
    }

    fn note_off(&mut self, pitch: u8) {
        let factor = (-1.0 / (self.release_s * self.sample_rate)).exp();
        for v in &mut self.voices {
            if v.active && v.pitch == pitch {
                for m in &mut v.modes {
                    m.damp_by(factor);
                }
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
}

impl AudioNode for Modal {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let brightness = self.brightness.advance(frames);
        let strike = self.strike.advance(frames);
        for i in 0..io.notes().len() {
            let ev = io.notes()[i];
            match ev.kind {
                NoteKind::On { pitch, vel } => self.note_on(pitch, vel, brightness, strike),
                NoteKind::Off { pitch } => self.note_off(pitch),
            }
        }

        let out = io.output(0);
        for slot in out.iter_mut() {
            let level = self.level.tick();
            let white = next_white(&mut self.rng);
            let mut sum = 0.0;
            for v in &mut self.voices {
                if v.active {
                    sum += v.render(white);
                }
            }
            *slot = sum * level;
        }
        // Free rung-out voices once per block (cheaper than per sample).
        for v in &mut self.voices {
            if v.active && v.faded() {
                v.active = false;
            }
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
            P_MATERIAL => self.material_idx = (real.round() as usize).min(MATERIALS.len() - 1),
            P_DECAY => self.decay_s = real,
            P_BRIGHTNESS => self.brightness.set_target(real),
            P_STRIKE => self.strike.set_target(real),
            P_RELEASE => self.release_s = (real / 1000.0).max(0.005),
            P_LEVEL => self.level.set_target(real),
            P_TUNE => self.tune_ct = real,
            _ => {}
        }
    }

    fn all_notes_off(&mut self) {
        let factor = (-1.0 / (self.release_s * self.sample_rate)).exp();
        for v in &mut self.voices {
            if v.active {
                for m in &mut v.modes {
                    m.damp_by(factor);
                }
            }
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.material_idx = default_real(P_MATERIAL).round() as usize;
        self.decay_s = default_real(P_DECAY);
        self.brightness = OnePole::new(default_real(P_BRIGHTNESS), coef);
        self.strike = OnePole::new(default_real(P_STRIKE), coef);
        self.release_s = default_real(P_RELEASE) / 1000.0;
        self.level = LinearRamp::new(default_real(P_LEVEL), 10.0, sample_rate);
        self.tune_ct = default_real(P_TUNE);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node::NoteEvent;

    const SR: f32 = 48_000.0;

    fn synth() -> Modal {
        let mut s = Modal::default();
        s.prepare(SR, 256);
        s
    }

    fn set_real(s: &mut Modal, id: ParamId, real: f32) {
        s.set_param(id, PARAMS[id as usize].normalize(real));
    }

    fn render_notes(s: &mut Modal, notes: &[NoteEvent], frames: usize, out: &mut Vec<f32>) {
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames,
            channels: 1,
            samples_per_beat: 0.0,
        };
        let mut io = NodeIo::new(notes, &[], out, frames);
        s.process(&ctx, &mut io);
    }

    fn on(pitch: u8) -> NoteEvent {
        NoteEvent {
            frame: 0,
            kind: NoteKind::On { pitch, vel: 110 },
        }
    }

    /// Goertzel-style magnitude of a single bin, normalised by length.
    fn mag_at(buf: &[f32], freq: f32) -> f32 {
        let w = TAU * freq / SR;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, &x) in buf.iter().enumerate() {
            let p = w * n as f32;
            re += x * p.cos();
            im += x * p.sin();
        }
        (re * re + im * im).sqrt() / buf.len() as f32
    }

    #[test]
    fn struck_note_is_audible_and_in_tune() {
        let mut s = synth();
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(69)], 24_000, &mut buf); // A4, 0.5 s
        let peak = buf.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak > 0.05, "modal strike inaudible, peak {peak}");
        assert!(buf.iter().all(|x| x.is_finite()));
        // Fundamental present and dominant over a detuned neighbour.
        let f1 = mag_at(&buf, 440.0);
        assert!(f1 > 5.0 * mag_at(&buf, 466.0), "fundamental off-pitch");
    }

    #[test]
    fn marimba_rings_its_four_to_one_mode() {
        let mut s = synth(); // default material = wood, ratio 3.9
        set_real(&mut s, P_BRIGHTNESS, 0.8);
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(69)], 24_000, &mut buf);
        let h39 = mag_at(&buf, 440.0 * 3.9);
        let h30 = mag_at(&buf, 440.0 * 3.0); // between modes: near-empty
        assert!(
            h39 > 5.0 * h30,
            "marimba 3.9 mode missing: {h39} vs gap {h30}"
        );
    }

    #[test]
    fn bell_hums_below_its_strike_note() {
        let mut s = synth();
        set_real(&mut s, P_MATERIAL, 2.0); // bell
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(69)], 48_000, &mut buf);
        let hum = mag_at(&buf, 440.0 * 0.56);
        let tierce = mag_at(&buf, 440.0 * 1.19);
        let gap = mag_at(&buf, 440.0 * 0.8);
        assert!(hum > 5.0 * gap, "bell hum tone missing");
        assert!(tierce > 5.0 * gap, "bell tierce missing");
    }

    #[test]
    fn high_modes_die_first() {
        use crate::analysis::{fft, spectral};
        let mut s = synth();
        set_real(&mut s, P_MATERIAL, 1.0); // vibraphone: long ring
        set_real(&mut s, P_DECAY, 3.0);
        set_real(&mut s, P_BRIGHTNESS, 1.0);
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(60)], 96_000, &mut buf); // 2 s
        let centroid = |slice: &[f32]| {
            let power = fft::power_spectrum(slice).expect("pow2");
            spectral::spectral_centroid_hz(&power, slice.len(), SR as u32)
        };
        let early = centroid(&buf[0..8192]);
        let late = centroid(&buf[88_000 - 8192..88_000]);
        assert!(
            early > 1.3 * late,
            "spectrum did not settle: early {early} Hz, late {late} Hz"
        );
    }

    #[test]
    fn decay_grows_the_ring_monotonically() {
        let ring_energy = |decay: f32| {
            let mut s = synth();
            set_real(&mut s, P_DECAY, decay);
            let mut buf = Vec::new();
            render_notes(&mut s, &[on(69)], 96_000, &mut buf);
            buf[48_000..].iter().map(|x| x * x).sum::<f32>()
        };
        let short = ring_energy(0.3);
        let mid = ring_energy(1.5);
        let long = ring_energy(6.0);
        assert!(
            short < mid && mid < long,
            "decay not monotonic: {short} {mid} {long}"
        );
    }

    #[test]
    fn release_gates_the_ring() {
        let mut s = synth();
        set_real(&mut s, P_DECAY, 6.0);
        set_real(&mut s, P_RELEASE, 30.0);
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(69)], 4800, &mut buf);
        let off = [NoteEvent {
            frame: 0,
            kind: NoteKind::Off { pitch: 69 },
        }];
        let mut tail = Vec::new();
        render_notes(&mut s, &off, 24_000, &mut tail);
        // Well past the 30 ms release the ring must be gone (vs 6 s decay).
        let end = &tail[12_000..];
        let rms = (end.iter().map(|x| x * x).sum::<f32>() / end.len() as f32).sqrt();
        assert!(rms < 1.0e-3, "release did not gate the ring, rms {rms}");
        // And the damped tail must stay click-free.
        let max_delta = tail
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_delta < 0.2, "release click, max delta {max_delta}");
    }

    #[test]
    fn voices_steal_and_all_notes_off_silences() {
        let mut s = synth();
        let mut buf = Vec::new();
        let ons: Vec<NoteEvent> = (60u8..68).map(on).collect();
        render_notes(&mut s, &ons, 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8);
        render_notes(&mut s, &[on(72)], 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8, "steal kept the count at 8");

        s.all_notes_off();
        render_notes(&mut s, &[], 96_000, &mut buf);
        assert_eq!(s.active_voice_count(), 0, "voices rang past all_notes_off");
    }
}
