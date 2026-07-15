//! Saturator effect: input drive into a tanh waveshaper, a post low-pass to
//! tame the added highs, wet/dry mix and an output trim.
//!
//! The "dirt" of the palette — warmth at low drive, aggression at high drive.
//! The shaper is bounded (±1 before trim) so it can never clip downstream by
//! itself. All DSP is hand-written; `process` is RT-SAFE.

use crate::engine::dsp::Biquad;
use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

/// Drive/tone glide time constant (seconds).
const SMOOTH_S: f32 = 0.015;
/// Mix/level ramp (ms), the shared gain idiom.
const RAMP_MS: f32 = 10.0;

const P_DRIVE: ParamId = 0;
const P_TONE: ParamId = 1;
const P_MIX: ParamId = 2;
const P_LEVEL: ParamId = 3;

const PARAMS: [ParamSpec; 4] = [
    ParamSpec {
        id: P_DRIVE,
        name: "drive",
        unit: "dB",
        min: 0.0,
        max: 36.0,
        default: 12.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_TONE,
        name: "tone",
        unit: "Hz",
        min: 500.0,
        max: 18_000.0,
        default: 8000.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_MIX,
        name: "mix",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 1.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_LEVEL,
        name: "level",
        unit: "dB",
        min: -24.0,
        max: 12.0,
        default: 0.0,
        curve: ParamCurve::Db,
    },
];

pub struct Saturator {
    sample_rate: f32,
    /// Drive in dB, glided (converted to linear gain per block).
    drive_db: OnePole,
    tone: OnePole,
    mix: LinearRamp,
    level: LinearRamp,
    /// One post-shaper low-pass per channel: shared coefficients, own state.
    lps: [Biquad; crate::engine::CHANNELS],
}

impl Default for Saturator {
    fn default() -> Self {
        // The real smoothers are rebuilt in `prepare` with the sample rate.
        Self {
            sample_rate: 48_000.0,
            drive_db: OnePole::new(12.0, 0.99),
            tone: OnePole::new(8000.0, 0.99),
            mix: LinearRamp::new(1.0, RAMP_MS, 48_000.0),
            level: LinearRamp::new(1.0, RAMP_MS, 48_000.0),
            lps: Default::default(),
        }
    }
}

impl AudioNode for Saturator {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let sr = self.sample_rate;
        let gain = 10.0f32.powf(self.drive_db.advance(frames) / 20.0);
        let tone = self.tone.advance(frames);
        for lp in &mut self.lps {
            lp.set_lowpass(tone, 0.707, sr);
        }

        // RT-SAFE: the mix/level ramps are snapshotted and replayed so every
        // channel sees the same per-frame trajectory (never tick a shared ramp
        // once per channel-sample).
        let mix_start = self.mix;
        let level_start = self.level;
        for ch in 0..io.channels().min(self.lps.len()) {
            if ch > 0 {
                self.mix = mix_start;
                self.level = level_start;
            }
            let (input, output) = io.in_out_ch(0, 0, ch);
            for (out, &x) in output.iter_mut().zip(input) {
                let wet = self.lps[ch].process((x * gain).tanh());
                let mix = self.mix.tick();
                *out = (x * (1.0 - mix) + wet * mix) * self.level.tick();
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
            P_TONE => self.tone.set_target(real),
            P_MIX => self.mix.set_target(real),
            P_LEVEL => self.level.set_target(real),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.drive_db = OnePole::new(default_real(P_DRIVE), coef);
        self.tone = OnePole::new(default_real(P_TONE), coef);
        self.mix = LinearRamp::new(default_real(P_MIX), RAMP_MS, sample_rate);
        self.level = LinearRamp::new(default_real(P_LEVEL), RAMP_MS, sample_rate);
        self.lps = Default::default();
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

    fn fx() -> Saturator {
        let mut s = Saturator::default();
        s.prepare(SR, 256);
        s
    }

    fn set_real(s: &mut Saturator, id: ParamId, real: f32) {
        s.set_param(id, PARAMS[id as usize].normalize(real));
    }

    fn render(s: &mut Saturator, input: &[f32], out: &mut Vec<f32>) {
        out.resize(input.len(), 0.0);
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames: input.len(),
            channels: 1,
            samples_per_beat: 0.0,
        };
        let mut io = NodeIo::new(&[], input, out, input.len(), 1);
        s.process(&ctx, &mut io);
    }

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (TAU * freq * i as f32 / SR).sin())
            .collect()
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
    fn drive_adds_odd_harmonics_and_flattens_the_crest() {
        let input = sine(220.0, 0.5, 19_200);
        let saturate = |drive: f32| {
            let mut s = fx();
            set_real(&mut s, P_DRIVE, drive);
            set_real(&mut s, P_TONE, 18_000.0); // measure the shaper, not the LP
            let mut out = Vec::new();
            render(&mut s, &input, &mut out);
            out.split_off(9600) // settled tail
        };
        let clean = saturate(0.0);
        let hot = saturate(24.0);
        let h3 = |b: &[f32]| mag_at(b, 660.0);
        assert!(
            h3(&hot) > 10.0 * h3(&clean).max(1.0e-6),
            "drive added no distortion: clean {}, hot {}",
            h3(&clean),
            h3(&hot)
        );
        // A saturated sine squares up: its crest factor drops toward 0 dB.
        let crest = |b: &[f32]| {
            let peak = b.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
            let rms = (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt();
            20.0 * (peak / rms).log10()
        };
        assert!(
            crest(&hot) < crest(&clean) - 0.5,
            "crest did not flatten: clean {} dB, hot {} dB",
            crest(&clean),
            crest(&hot)
        );
        // Bounded before trim: tanh never exceeds ±1.
        assert!(hot.iter().all(|x| x.abs() <= 1.0));
    }

    #[test]
    fn zero_mix_passes_dry_signal() {
        let mut s = fx();
        s.set_param(P_MIX, 0.0);
        let input = sine(220.0, 0.5, 9600);
        let mut out = Vec::new();
        render(&mut s, &input, &mut out); // let the mix ramp settle
        render(&mut s, &input, &mut out);
        for (o, x) in out.iter().zip(&input) {
            assert!((o - x).abs() < 1.0e-6, "mix 0 is not dry: {o} vs {x}");
        }
    }

    #[test]
    fn drive_sweep_is_click_free() {
        use crate::analysis::levels;
        let mut s = fx();
        let input = sine(220.0, 0.5, 256);
        let mut out = Vec::new();
        let mut whole = Vec::new();
        for k in 0..128 {
            s.set_param(P_DRIVE, k as f32 / 127.0);
            render(&mut s, &input, &mut out);
            whole.extend_from_slice(&out);
        }
        assert!(whole.iter().all(|x| x.is_finite()));
        let clicks = levels::detect_clicks(&whole, 48_000);
        assert!(clicks.is_empty(), "drive sweep clicked: {clicks:?}");
    }
}
