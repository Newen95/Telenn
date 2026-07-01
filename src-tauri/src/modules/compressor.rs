//! Compressor effect: feed-forward, hard knee. A peak envelope follower tracks
//! the input level; whatever exceeds `threshold` is reduced by `ratio`, then
//! `makeup` restores the lost level. The follower's attack/release one-poles
//! are the gain smoothing — no extra ramp is needed on the gain itself.

use crate::engine::dsp::EnvFollower;
use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::LinearRamp;

/// Makeup ramp (anti-click), same spirit as the gain template.
const RAMP_MS: f32 = 10.0;
/// Floor for the level→dB conversion (-120 dB), keeps log10 finite on silence.
const LEVEL_FLOOR: f32 = 1.0e-6;

const P_THRESHOLD: ParamId = 0;
const P_RATIO: ParamId = 1;
const P_ATTACK: ParamId = 2;
const P_RELEASE: ParamId = 3;
const P_MAKEUP: ParamId = 4;

const PARAMS: [ParamSpec; 5] = [
    ParamSpec {
        id: P_THRESHOLD,
        // Linear (not Db): the gain computer wants the dB value itself,
        // not the equivalent linear gain.
        name: "threshold",
        unit: "dB",
        min: -60.0,
        max: 0.0,
        default: -18.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_RATIO,
        name: "ratio",
        unit: ":1",
        min: 1.0,
        max: 20.0,
        default: 4.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_ATTACK,
        name: "attack",
        unit: "ms",
        min: 0.1,
        max: 100.0,
        default: 10.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_RELEASE,
        name: "release",
        unit: "ms",
        min: 10.0,
        max: 1000.0,
        default: 120.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_MAKEUP,
        name: "makeup",
        unit: "dB",
        min: 0.0,
        max: 24.0,
        default: 0.0,
        curve: ParamCurve::Db,
    },
];

pub struct Compressor {
    sample_rate: f32,
    follower: EnvFollower,
    threshold_db: f32,
    ratio: f32,
    attack_s: f32,
    release_s: f32,
    /// Smoothed linear makeup gain.
    makeup: LinearRamp,
}

impl Default for Compressor {
    fn default() -> Self {
        // Real state is rebuilt in `prepare` once the sample rate is known.
        let mut c = Self {
            sample_rate: 48_000.0,
            follower: EnvFollower::default(),
            threshold_db: default_real(P_THRESHOLD),
            ratio: default_real(P_RATIO),
            attack_s: default_real(P_ATTACK) / 1000.0,
            release_s: default_real(P_RELEASE) / 1000.0,
            makeup: LinearRamp::new(default_real(P_MAKEUP), RAMP_MS, 48_000.0),
        };
        c.follower.set_times(c.attack_s, c.release_s, c.sample_rate);
        c
    }
}

impl AudioNode for Compressor {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let (input, output) = io.in_out(0, 0);
        for (out, &x) in output.iter_mut().zip(input) {
            let env = self.follower.tick(x);
            let level_db = 20.0 * env.max(LEVEL_FLOOR).log10();
            let over = level_db - self.threshold_db;
            // Hard knee: above threshold the level slope drops to 1/ratio,
            // i.e. a gain reduction of `over * (1/ratio - 1)` dB.
            let reduction_db = if over > 0.0 {
                over * (1.0 / self.ratio - 1.0)
            } else {
                0.0
            };
            let gain = 10.0f32.powf(reduction_db / 20.0) * self.makeup.tick();
            *out = x * gain;
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
            P_THRESHOLD => self.threshold_db = real,
            P_RATIO => self.ratio = real.max(1.0),
            P_ATTACK => {
                self.attack_s = real / 1000.0;
                self.follower
                    .set_times(self.attack_s, self.release_s, self.sample_rate);
            }
            P_RELEASE => {
                self.release_s = real / 1000.0;
                self.follower
                    .set_times(self.attack_s, self.release_s, self.sample_rate);
            }
            P_MAKEUP => self.makeup.set_target(real),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        self.threshold_db = default_real(P_THRESHOLD);
        self.ratio = default_real(P_RATIO);
        self.attack_s = default_real(P_ATTACK) / 1000.0;
        self.release_s = default_real(P_RELEASE) / 1000.0;
        self.follower = EnvFollower::default();
        self.follower
            .set_times(self.attack_s, self.release_s, sample_rate);
        self.makeup = LinearRamp::new(default_real(P_MAKEUP), RAMP_MS, sample_rate);
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

    const SR: f32 = 48_000.0;

    fn fx() -> Compressor {
        let mut c = Compressor::default();
        c.prepare(SR, 256);
        c
    }

    fn render(c: &mut Compressor, input: &[f32], out: &mut Vec<f32>) {
        let frames = input.len();
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames,
            channels: 1,
        };
        let mut io = NodeIo::new(&[], input, out, frames);
        c.process(&ctx, &mut io);
    }

    fn sine(amp: f32, freq: f32, secs: f32) -> Vec<f32> {
        let n = (secs * SR) as usize;
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin() * amp)
            .collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
    }

    fn db(x: f32) -> f32 {
        20.0 * x.log10()
    }

    #[test]
    fn steady_tone_is_reduced_by_the_ratio() {
        // -6 dB sine into threshold -18 dB, ratio 4:1 (defaults): 12 dB over
        // compresses to 3 dB over, so the output settles at -15 dB.
        let mut c = fx();
        let input = sine(0.5, 1000.0, 1.0);
        let mut out = Vec::new();
        render(&mut c, &input, &mut out);

        let settled = db(peak(&out[40_000..]));
        assert!(
            (settled - (-15.0)).abs() < 1.5,
            "expected ~-15 dBFS in steady state, got {settled:.1}"
        );
        assert!(out.iter().all(|s| s.is_finite()), "produced NaN/inf");
    }

    #[test]
    fn below_threshold_passes_unchanged() {
        // -26 dB sine stays under the -18 dB threshold: unity gain.
        let mut c = fx();
        let input = sine(0.05, 1000.0, 0.5);
        let mut out = Vec::new();
        render(&mut c, &input, &mut out);

        let delta = (peak(&out[12_000..]) - 0.05).abs();
        assert!(delta < 1.0e-3, "quiet signal was altered by {delta}");
    }

    #[test]
    fn makeup_boosts_the_output() {
        // +12 dB makeup on a below-threshold signal ≈ ×3.98.
        let mut c = fx();
        c.set_param(P_MAKEUP, PARAMS[P_MAKEUP as usize].normalize(12.0));
        let input = sine(0.05, 1000.0, 0.5);
        let mut out = Vec::new();
        render(&mut c, &input, &mut out);

        let settled = peak(&out[12_000..]);
        assert!(
            (settled - 0.199).abs() < 0.01,
            "expected ~0.199 after +12 dB makeup, got {settled}"
        );
    }

    #[test]
    fn higher_ratio_squashes_harder() {
        let mut soft = fx();
        soft.set_param(P_RATIO, PARAMS[P_RATIO as usize].normalize(2.0));
        let mut hard = fx();
        hard.set_param(P_RATIO, PARAMS[P_RATIO as usize].normalize(20.0));

        let input = sine(0.5, 1000.0, 1.0);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        render(&mut soft, &input, &mut a);
        render(&mut hard, &input, &mut b);

        let soft_peak = peak(&a[40_000..]);
        let hard_peak = peak(&b[40_000..]);
        assert!(
            hard_peak < soft_peak,
            "20:1 not squashing more than 2:1: {soft_peak} vs {hard_peak}"
        );
    }
}
