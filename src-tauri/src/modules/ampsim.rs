//! Guitar-amp style distortion: input high-pass → preamp waveshaper → 3-band
//! tone stack → asymmetric power-amp waveshaper → fixed cabinet filter.
//!
//! Two distinct nonlinear stages are what separate an amp from the plain
//! saturator: the preamp tanh adds odd harmonics, the biased power stage adds
//! the even ones (the "warmth"), and the cabinet rolls the fizz off. All DSP
//! is hand-written; `process` is RT-SAFE.

use crate::engine::dsp::Biquad;
use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

/// Drive/tone glide time constant (seconds).
const SMOOTH_S: f32 = 0.015;
/// Level ramp (ms), the shared gain idiom.
const RAMP_MS: f32 = 10.0;
/// Power-stage bias: the asymmetry that creates even harmonics.
const BIAS: f32 = 0.25;
/// Fixed corners: rumble cut, tone-stack centers, cabinet voicing.
const IN_HP_HZ: f32 = 30.0;
const BASS_HZ: f32 = 250.0;
const MID_HZ: f32 = 800.0;
const MID_Q: f32 = 0.8;
const TREBLE_HZ: f32 = 3000.0;
const CAB_HP_HZ: f32 = 90.0;
const CAB_LP_HZ: f32 = 5500.0;

/// Filter indices in each channel's chain.
const F_IN_HP: usize = 0;
const F_BASS: usize = 1;
const F_MID: usize = 2;
const F_TREBLE: usize = 3;
const F_CAB_HP: usize = 4;
const F_CAB_LP: usize = 5;
const FILTERS: usize = 6;

const P_DRIVE: ParamId = 0;
const P_BASS: ParamId = 1;
const P_MID: ParamId = 2;
const P_TREBLE: ParamId = 3;
const P_LEVEL: ParamId = 4;

const PARAMS: [ParamSpec; 5] = [
    ParamSpec {
        id: P_DRIVE,
        name: "drive",
        unit: "dB",
        min: 0.0,
        max: 48.0,
        default: 18.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_BASS,
        name: "bass",
        unit: "dB",
        min: -12.0,
        max: 12.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_MID,
        name: "mid",
        unit: "dB",
        min: -12.0,
        max: 12.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_TREBLE,
        name: "treble",
        unit: "dB",
        min: -12.0,
        max: 12.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_LEVEL,
        name: "level",
        unit: "dB",
        min: -24.0,
        max: 12.0,
        default: -6.0,
        curve: ParamCurve::Db,
    },
];

pub struct AmpSim {
    sample_rate: f32,
    drive_db: OnePole,
    bass: OnePole,
    mid: OnePole,
    treble: OnePole,
    level: LinearRamp,
    /// Six filters per channel: shared coefficients, per-channel state.
    chains: [[Biquad; FILTERS]; crate::engine::CHANNELS],
}

impl Default for AmpSim {
    fn default() -> Self {
        // The real smoothers are rebuilt in `prepare` with the sample rate.
        Self {
            sample_rate: 48_000.0,
            drive_db: OnePole::new(18.0, 0.99),
            bass: OnePole::new(0.0, 0.99),
            mid: OnePole::new(0.0, 0.99),
            treble: OnePole::new(0.0, 0.99),
            level: LinearRamp::new(0.5, RAMP_MS, 48_000.0),
            chains: Default::default(),
        }
    }
}

impl AudioNode for AmpSim {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let sr = self.sample_rate;
        let g1 = 10.0f32.powf(self.drive_db.advance(frames) / 20.0);
        // The power stage drives softer than the preamp; its output is
        // recentered so silence in = silence out despite the bias.
        let g2 = g1.sqrt().max(1.5);
        let bias_out = (g2 * BIAS).tanh();
        let bass = self.bass.advance(frames);
        let mid = self.mid.advance(frames);
        let treble = self.treble.advance(frames);
        for chain in &mut self.chains {
            chain[F_BASS].set_lowshelf(BASS_HZ, bass, sr);
            chain[F_MID].set_peaking(MID_HZ, MID_Q, mid, sr);
            chain[F_TREBLE].set_highshelf(TREBLE_HZ, treble, sr);
        }

        // RT-SAFE: the level ramp is snapshotted and replayed per channel.
        let level_start = self.level;
        for ch in 0..io.channels().min(self.chains.len()) {
            if ch > 0 {
                self.level = level_start;
            }
            let (input, output) = io.in_out_ch(0, 0, ch);
            let chain = &mut self.chains[ch];
            for (out, &x) in output.iter_mut().zip(input) {
                let hp = chain[F_IN_HP].process(x);
                let pre = (hp * g1).tanh();
                let low = chain[F_BASS].process(pre);
                let mid_s = chain[F_MID].process(low);
                let toned = chain[F_TREBLE].process(mid_s);
                // Recentered and renormalized: silence in = silence out, and
                // the stage output stays within ±1 whatever the drive.
                let power = (((toned + BIAS) * g2).tanh() - bias_out) / (1.0 + bias_out);
                let cab_hp = chain[F_CAB_HP].process(power);
                let cab = chain[F_CAB_LP].process(cab_hp);
                *out = cab * self.level.tick();
            }
        }
    }

    fn ports(&self) -> PortSpec {
        PortSpec {
            inputs: vec![Port {
                name: "in",
                kind: SignalKind::Audio,
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
            P_DRIVE => self.drive_db.set_target(real),
            P_BASS => self.bass.set_target(real),
            P_MID => self.mid.set_target(real),
            P_TREBLE => self.treble.set_target(real),
            P_LEVEL => self.level.set_target(real),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.drive_db = OnePole::new(default_real(P_DRIVE), coef);
        self.bass = OnePole::new(default_real(P_BASS), coef);
        self.mid = OnePole::new(default_real(P_MID), coef);
        self.treble = OnePole::new(default_real(P_TREBLE), coef);
        self.level = LinearRamp::new(default_real(P_LEVEL), RAMP_MS, sample_rate);
        self.chains = Default::default();
        for chain in &mut self.chains {
            chain[F_IN_HP].set_highpass(IN_HP_HZ, 0.707, sample_rate);
            chain[F_CAB_HP].set_highpass(CAB_HP_HZ, 0.707, sample_rate);
            chain[F_CAB_LP].set_lowpass(CAB_LP_HZ, 0.707, sample_rate);
        }
    }
}

/// Real default value for a parameter (round-trips through the curve).
fn default_real(id: ParamId) -> f32 {
    let spec = &PARAMS[id as usize];
    spec.denormalize(spec.normalize(spec.default))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const SR: f32 = 48_000.0;

    fn fx() -> AmpSim {
        let mut a = AmpSim::default();
        a.prepare(SR, 256);
        a
    }

    fn set_real(a: &mut AmpSim, id: ParamId, real: f32) {
        a.set_param(id, PARAMS[id as usize].normalize(real));
    }

    fn render(a: &mut AmpSim, input: &[f32], out: &mut Vec<f32>) {
        out.resize(input.len(), 0.0);
        let frames = input.len();
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames,
            channels: 1,
            samples_per_beat: 0.0,
            beat_phase: 0.0,
        };
        let mut io = NodeIo::new(&[], input, out, frames, 1);
        a.process(&ctx, &mut io);
    }

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (TAU * freq * i as f32 / SR).sin())
            .collect()
    }

    /// Single-bin DFT magnitude, normalized by length.
    fn mag_at(buf: &[f32], freq: f32) -> f32 {
        let mut re = 0.0f32;
        let mut im = 0.0f32;
        for (n, &s) in buf.iter().enumerate() {
            let w = TAU * freq * n as f32 / SR;
            re += s * w.cos();
            im += s * w.sin();
        }
        (re * re + im * im).sqrt() / buf.len() as f32
    }

    #[test]
    fn adds_even_and_odd_harmonics() {
        // The biased power stage separates the amp from the pure-tanh
        // saturator: a driven sine must grow BOTH h2 and h3.
        let mut a = fx();
        set_real(&mut a, P_DRIVE, 24.0);
        set_real(&mut a, P_LEVEL, 0.0);
        let x = sine(220.0, 0.5, 19_200);
        let mut y = Vec::new();
        render(&mut a, &x, &mut y);
        let tail = &y[4800..];
        let h1 = mag_at(tail, 220.0);
        let h2 = mag_at(tail, 440.0);
        let h3 = mag_at(tail, 660.0);
        assert!(h1 > 0.05, "fundamental vanished: {h1}");
        assert!(h2 > 0.02 * h1, "no even harmonics: h2 {h2} vs h1 {h1}");
        assert!(h3 > 0.02 * h1, "no odd harmonics: h3 {h3} vs h1 {h1}");
    }

    #[test]
    fn tone_stack_shapes_the_bands() {
        // Low drive + small input keeps the stages near-linear so the tone
        // stack response is measurable.
        let gain_at = |bass_db: f32, freq: f32| -> f32 {
            let mut a = fx();
            set_real(&mut a, P_DRIVE, 0.0);
            set_real(&mut a, P_BASS, bass_db);
            set_real(&mut a, P_LEVEL, 0.0);
            let x = sine(freq, 0.05, 19_200);
            let mut y = Vec::new();
            render(&mut a, &x, &mut y);
            let tail = &y[9600..];
            mag_at(tail, freq)
        };
        let boosted = gain_at(9.0, 150.0);
        let cut = gain_at(-9.0, 150.0);
        let ratio_db = 20.0 * (boosted / cut).log10();
        assert!(
            (ratio_db - 18.0).abs() < 3.0,
            "bass knob span {ratio_db} dB, want ~18"
        );
    }

    #[test]
    fn output_stays_bounded_at_full_drive() {
        let mut a = fx();
        set_real(&mut a, P_DRIVE, 48.0);
        set_real(&mut a, P_LEVEL, 12.0);
        let x = sine(110.0, 1.0, 9600);
        let mut y = Vec::new();
        render(&mut a, &x, &mut y);
        let peak = y.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        assert!(peak.is_finite() && peak < 5.0, "amp blew up: peak {peak}");
    }

    #[test]
    fn drive_sweep_is_click_free() {
        let mut a = fx();
        let x = sine(220.0, 0.5, 80 * 256);
        let mut all = Vec::new();
        let mut chunk = Vec::new();
        for step in 0..80 {
            a.set_param(P_DRIVE, (step % 17) as f32 / 16.0);
            render(&mut a, &x[step * 256..(step + 1) * 256], &mut chunk);
            all.extend_from_slice(&chunk);
        }
        let clicks = crate::analysis::levels::detect_clicks(&all, 48_000);
        assert!(clicks.is_empty(), "amp drive sweep clicked: {clicks:?}");
    }
}
