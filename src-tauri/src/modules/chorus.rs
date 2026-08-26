//! Chorus effect: two modulated fractional-delay taps (decorrelated sine
//! LFOs) mixed with the dry signal — the "movement" of the palette.
//!
//! Stereo: one line fed the input fold, tap 0 routed left and tap 1 right —
//! the spread the two detuned taps were built for. The mono fold of the
//! stereo wet is exactly the legacy mono wet (mono-compatible by design).
//! All DSP is hand-written; `process` is RT-SAFE (the delay line is sized in
//! `prepare`, reads are clamped inside it).

use std::f32::consts::TAU;

use crate::engine::dsp::DelayLine;
use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

/// Depth glide time constant (seconds).
const SMOOTH_S: f32 = 0.05;
/// Mix ramp (ms), the shared gain idiom.
const RAMP_MS: f32 = 10.0;
/// Centre of the modulated delay, ms — short enough to blend, long enough to
/// detune (the classic chorus zone).
const BASE_MS: f32 = 15.0;
/// Start phases of the two LFOs (turns): decorrelated, deterministic.
const LFO_PHASES: [f32; 2] = [0.0, 0.35];

const P_RATE: ParamId = 0;
const P_DEPTH: ParamId = 1;
const P_MIX: ParamId = 2;

const PARAMS: [ParamSpec; 3] = [
    ParamSpec {
        id: P_RATE,
        name: "rate",
        unit: "Hz",
        min: 0.1,
        max: 5.0,
        default: 0.8,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_DEPTH,
        name: "depth",
        unit: "ms",
        min: 0.0,
        max: 10.0,
        default: 3.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_MIX,
        name: "mix",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.35,
        curve: ParamCurve::Linear,
    },
];

pub struct Chorus {
    sample_rate: f32,
    line: DelayLine,
    /// LFO phases in turns (0..1); the shared increment tracks `rate`.
    lfo_phase: [f32; 2],
    lfo_inc: f32,
    /// Depth in ms, glided (converted to samples per block).
    depth_ms: OnePole,
    mix: LinearRamp,
    base_samples: f32,
}

impl Default for Chorus {
    fn default() -> Self {
        // The real smoothers/line are rebuilt in `prepare` with the sample rate.
        Self {
            sample_rate: 48_000.0,
            line: DelayLine::default(),
            lfo_phase: LFO_PHASES,
            lfo_inc: 0.8 / 48_000.0,
            depth_ms: OnePole::new(3.0, 0.99),
            mix: LinearRamp::new(0.35, RAMP_MS, 48_000.0),
            base_samples: BASE_MS * 48.0,
        }
    }
}

impl AudioNode for Chorus {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let depth = self.depth_ms.advance(frames) * self.sample_rate / 1000.0;
        let base = self.base_samples;
        let inc = self.lfo_inc;

        // RT-SAFE: fractional reads are clamped inside the DelayLine; the
        // line was sized in `prepare` for base + max depth + margin.
        //
        // Stereo: ONE line fed the input fold, the two decorrelated taps
        // routed left/right. The mono fold of that stereo wet is exactly the
        // legacy mono wet (0.5·(t0+t1)), so mono compatibility is preserved
        // by construction.
        let stereo = io.channels() > 1;
        for i in 0..frames {
            let l = io.input_ch(0, 0)[i];
            let r = if stereo { io.input_ch(0, 1)[i] } else { l };
            let x = 0.5 * (l + r);
            let mut wet = [0.0f32; 2];
            for (w, phase) in wet.iter_mut().zip(&mut self.lfo_phase) {
                *phase += inc;
                if *phase >= 1.0 {
                    *phase -= 1.0;
                }
                *w = self.line.read(base + (TAU * *phase).sin() * depth);
            }
            self.line.write(x);
            let mix = self.mix.tick();
            if stereo {
                io.output_ch(0, 0)[i] = l * (1.0 - mix) + wet[0] * mix;
                io.output_ch(0, 1)[i] = r * (1.0 - mix) + wet[1] * mix;
            } else {
                io.output_ch(0, 0)[i] = l * (1.0 - mix) + 0.5 * (wet[0] + wet[1]) * mix;
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
            // Rate lands directly (a slow LFO speed change is inaudible).
            P_RATE => self.lfo_inc = real / self.sample_rate,
            P_DEPTH => self.depth_ms.set_target(real),
            P_MIX => self.mix.set_target(real),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.base_samples = BASE_MS * sample_rate / 1000.0;
        let max_depth = PARAMS[P_DEPTH as usize].max * sample_rate / 1000.0;
        // +4: interpolation margin, so the clamp inside `read` never engages.
        self.line
            .prepare((self.base_samples + max_depth) as usize + 4);
        self.lfo_phase = LFO_PHASES;
        self.lfo_inc = default_real(P_RATE) / sample_rate;
        self.depth_ms = OnePole::new(default_real(P_DEPTH), coef);
        self.mix = LinearRamp::new(default_real(P_MIX), RAMP_MS, sample_rate);
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

    fn fx() -> Chorus {
        let mut c = Chorus::default();
        c.prepare(SR, 256);
        c
    }

    fn set_real(c: &mut Chorus, id: ParamId, real: f32) {
        c.set_param(id, PARAMS[id as usize].normalize(real));
    }

    fn render(c: &mut Chorus, input: &[f32], out: &mut Vec<f32>) {
        out.resize(input.len(), 0.0);
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames: input.len(),
            channels: 1,
            samples_per_beat: 0.0,
            beat_phase: 0.0,
        };
        let mut io = NodeIo::new(&[], input, out, input.len(), 1);
        c.process(&ctx, &mut io);
    }

    fn sine(freq: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.5 * (TAU * freq * i as f32 / SR).sin())
            .collect()
    }

    #[test]
    fn zero_mix_passes_dry_signal() {
        let mut c = fx();
        c.set_param(P_MIX, 0.0);
        let input = sine(440.0, 9600);
        let mut out = Vec::new();
        render(&mut c, &input, &mut out); // let the mix ramp settle
        render(&mut c, &input, &mut out);
        for (o, x) in out.iter().zip(&input) {
            assert!((o - x).abs() < 1.0e-6, "mix 0 is not dry: {o} vs {x}");
        }
    }

    #[test]
    fn stereo_wet_decorrelates_and_folds_to_mono() {
        // Same dual-mono content through a mono and a stereo run: the stereo
        // wet must differ between channels (tap 0 → L, tap 1 → R) while its
        // fold reproduces the legacy mono wet to f32 rounding.
        let input = sine(220.0, 4800);
        let frames = input.len();

        let mut mono_fx = fx();
        set_real(&mut mono_fx, P_MIX, 1.0);
        let mut mono_out = Vec::new();
        render(&mut mono_fx, &input, &mut mono_out);

        let mut st = fx();
        set_real(&mut st, P_MIX, 1.0);
        let mut stereo_in = input.clone();
        stereo_in.extend_from_slice(&input);
        let mut stereo_out = vec![0.0f32; frames * 2];
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames,
            channels: 2,
            samples_per_beat: 0.0,
            beat_phase: 0.0,
        };
        let mut io = NodeIo::new(&[], &stereo_in, &mut stereo_out, frames, 2);
        st.process(&ctx, &mut io);

        let (l, r) = stereo_out.split_at(frames);
        assert!(
            l.iter().zip(r).any(|(a, b)| (a - b).abs() > 1.0e-4),
            "stereo wet did not decorrelate"
        );
        let mut max_err = 0.0f32;
        for i in 0..frames {
            max_err = max_err.max((0.5 * (l[i] + r[i]) - mono_out[i]).abs());
        }
        assert!(
            max_err < 1.0e-6,
            "stereo fold != legacy mono wet: {max_err}"
        );
    }

    #[test]
    fn modulation_moves_the_level() {
        // Full wet on a steady sine: the two moving taps comb-filter against
        // each other, so the level must swell and dip at the LFO rate.
        let mut c = fx();
        set_real(&mut c, P_MIX, 1.0);
        set_real(&mut c, P_RATE, 2.0);
        set_real(&mut c, P_DEPTH, 8.0);
        let input = sine(880.0, 4 * 48_000);
        let mut out = Vec::new();
        render(&mut c, &input, &mut out);
        let rms: Vec<f32> = out[48_000..]
            .chunks(2400) // 50 ms windows
            .map(|w| (w.iter().map(|x| x * x).sum::<f32>() / w.len() as f32).sqrt())
            .collect();
        let hi = rms.iter().fold(0.0f32, |m, &v| m.max(v));
        let lo = rms.iter().fold(f32::MAX, |m, &v| m.min(v));
        assert!(hi > 1.3 * lo, "no movement heard: rms {lo}..{hi}");
    }

    #[test]
    fn sweeps_are_click_free_and_finite() {
        use crate::analysis::levels;
        let mut c = fx();
        let input = sine(440.0, 256);
        let mut out = Vec::new();
        let mut whole = Vec::new();
        for k in 0..128 {
            let v = k as f32 / 127.0;
            c.set_param(P_DEPTH, v);
            c.set_param(P_RATE, 1.0 - v);
            render(&mut c, &input, &mut out);
            whole.extend_from_slice(&out);
        }
        assert!(whole.iter().all(|x| x.is_finite()));
        let clicks = levels::detect_clicks(&whole, 48_000);
        assert!(clicks.is_empty(), "sweep clicked: {clicks:?}");
    }
}
