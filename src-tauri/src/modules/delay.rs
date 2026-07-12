//! Delay effect: a fractional delay line with a filtered feedback loop.
//!
//! `time` is in milliseconds; `sync` overrides it with a musical division of
//! the host tempo (off, 1/16 … 1/1) whenever the context carries one, falling
//! back to the ms knob otherwise (old projects, probes without a tempo). The
//! active time glides sample-per-sample through a one-pole, so knob moves AND
//! tempo changes bend pitch like tape instead of clicking. The feedback path
//! runs through a low-pass (`tone`), so repeats darken naturally; the delay
//! line flushes denormals, keeping the decaying loop RT-cheap.

use crate::engine::dsp::{Biquad, DelayLine};
use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

/// Longest settable delay; sizes the line allocation in `prepare`.
const MAX_TIME_MS: f32 = 2000.0;
/// Glide time constant for `time` changes (seconds). Long enough that a knob
/// sweep sounds like a gentle tape bend, short enough to feel responsive.
const TIME_SMOOTH_S: f32 = 0.05;
/// Glide time constant for the feedback low-pass cutoff (seconds).
const TONE_SMOOTH_S: f32 = 0.015;
/// Mix/feedback ramp (anti-click), same spirit as the gain template.
const RAMP_MS: f32 = 10.0;

/// Musical divisions for `sync`, in beats (a beat is a quarter note, matching
/// the schedule's `samples_per_beat`). Index 0 = off (the ms knob rules).
const SYNC_BEATS: [f32; 8] = [0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 4.0];

const P_TIME: ParamId = 0;
const P_FEEDBACK: ParamId = 1;
const P_TONE: ParamId = 2;
const P_MIX: ParamId = 3;
const P_SYNC: ParamId = 4;

const PARAMS: [ParamSpec; 5] = [
    ParamSpec {
        id: P_TIME,
        name: "time",
        unit: "ms",
        min: 10.0,
        max: MAX_TIME_MS,
        default: 350.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_FEEDBACK,
        name: "feedback",
        unit: "",
        min: 0.0,
        // Strictly below 1: the loop always decays, whatever the knob does.
        max: 0.9,
        default: 0.35,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_TONE,
        name: "tone",
        unit: "Hz",
        min: 200.0,
        max: 18_000.0,
        default: 4500.0,
        curve: ParamCurve::Log,
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
    // Stepped: 0 = off, then 1/16, 1/8, 1/8., 1/4, 1/4., 1/2, 1/1.
    ParamSpec {
        id: P_SYNC,
        name: "sync",
        unit: "div",
        min: 0.0,
        max: 7.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
];

pub struct Delay {
    sample_rate: f32,
    line: DelayLine,
    /// Low-pass inside the feedback loop (repeats get darker each pass).
    filter: Biquad,
    /// Active delay time in samples, glided per sample.
    time: OnePole,
    /// The ms knob's target in samples, kept apart so ms and sync never fight.
    time_ms_samples: f32,
    /// Index into [`SYNC_BEATS`]; 0 = follow the ms knob.
    sync_idx: usize,
    /// Longest reachable delay (line length minus the interpolation margin).
    max_delay_samples: f32,
    tone: OnePole,
    feedback: LinearRamp,
    mix: LinearRamp,
}

impl Default for Delay {
    fn default() -> Self {
        // Real state is rebuilt in `prepare` once the sample rate is known.
        Self {
            sample_rate: 48_000.0,
            line: DelayLine::default(),
            filter: Biquad::default(),
            time: OnePole::new(default_real(P_TIME) / 1000.0 * 48_000.0, 0.999),
            time_ms_samples: default_real(P_TIME) / 1000.0 * 48_000.0,
            sync_idx: 0,
            max_delay_samples: MAX_TIME_MS / 1000.0 * 48_000.0,
            tone: OnePole::new(default_real(P_TONE), 0.99),
            feedback: LinearRamp::new(default_real(P_FEEDBACK), RAMP_MS, 48_000.0),
            mix: LinearRamp::new(default_real(P_MIX), RAMP_MS, 48_000.0),
        }
    }
}

impl AudioNode for Delay {
    fn process(&mut self, ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let cutoff = self.tone.advance(frames);
        self.filter.set_lowpass(cutoff, 0.707, self.sample_rate);

        // RT-SAFE: pick the time target once per block — synced when a
        // division is set and the host has a tempo, ms knob otherwise (old
        // projects, live play before a load, offline tail). Divisions longer
        // than the line clamp to it (1/1 at 60 bpm on a 2 s line).
        let target = if self.sync_idx > 0 && ctx.samples_per_beat > 0.0 {
            (SYNC_BEATS[self.sync_idx] * ctx.samples_per_beat).clamp(1.0, self.max_delay_samples)
        } else {
            self.time_ms_samples
        };
        self.time.set_target(target);

        let (input, output) = io.in_out(0, 0);
        for (out, &x) in output.iter_mut().zip(input) {
            let t = self.time.advance(1);
            let fb = self.feedback.tick();
            let mix = self.mix.tick();
            let wet = self.line.read(t);
            self.line.write(x + self.filter.process(wet) * fb);
            *out = x * (1.0 - mix) + wet * mix;
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
            // The block-rate selector in `process` posts the actual target.
            P_TIME => self.time_ms_samples = real / 1000.0 * self.sample_rate,
            P_FEEDBACK => self.feedback.set_target(real),
            P_TONE => self.tone.set_target(real),
            P_MIX => self.mix.set_target(real),
            P_SYNC => self.sync_idx = (real.round() as usize).min(SYNC_BEATS.len() - 1),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let line_len = (MAX_TIME_MS / 1000.0 * sample_rate).ceil() as usize + 2;
        self.line.prepare(line_len);
        self.max_delay_samples = (line_len - 2) as f32;
        self.filter = Biquad::default();
        let time_coef = (-1.0 / (TIME_SMOOTH_S * sample_rate)).exp();
        let tone_coef = (-1.0 / (TONE_SMOOTH_S * sample_rate)).exp();
        self.time = OnePole::new(default_real(P_TIME) / 1000.0 * sample_rate, time_coef);
        self.time_ms_samples = default_real(P_TIME) / 1000.0 * sample_rate;
        self.sync_idx = 0;
        self.tone = OnePole::new(default_real(P_TONE), tone_coef);
        self.feedback = LinearRamp::new(default_real(P_FEEDBACK), RAMP_MS, sample_rate);
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

    fn fx() -> Delay {
        let mut d = Delay::default();
        d.prepare(SR, 256);
        d
    }

    fn render(d: &mut Delay, input: &[f32], out: &mut Vec<f32>) {
        render_with_tempo(d, input, out, 0.0);
    }

    fn render_with_tempo(d: &mut Delay, input: &[f32], out: &mut Vec<f32>, samples_per_beat: f32) {
        let frames = input.len();
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames,
            channels: 1,
            samples_per_beat,
        };
        let mut io = NodeIo::new(&[], input, out, frames);
        d.process(&ctx, &mut io);
    }

    /// Echo position of a settled synced delay: silence long enough for the
    /// glide from the 350 ms default to converge, then an impulse.
    fn synced_echo_pos(sync_idx: f32, samples_per_beat: f32) -> usize {
        let mut d = fx();
        d.set_param(P_MIX, 1.0);
        d.set_param(P_FEEDBACK, 0.0);
        d.set_param(P_SYNC, PARAMS[P_SYNC as usize].normalize(sync_idx));

        let settle = 48_000usize; // 20τ of the 50 ms glide
        let mut input = vec![0.0; settle + 60_000];
        input[settle] = 1.0;
        let mut out = Vec::new();
        render_with_tempo(&mut d, &input, &mut out, samples_per_beat);

        let (arg, _) = out
            .iter()
            .enumerate()
            .skip(settle + 8)
            .fold((0, 0.0f32), |acc, (i, &s)| {
                if s.abs() > acc.1 {
                    (i, s.abs())
                } else {
                    acc
                }
            });
        arg - settle
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
    }

    #[test]
    fn echo_lands_at_the_set_time() {
        // Default time is 350 ms = 16 800 samples at 48 kHz (no glide involved).
        let mut d = fx();
        d.set_param(P_MIX, 1.0);
        d.set_param(P_FEEDBACK, 0.0);

        let mut input = vec![0.0; 20_000];
        input[0] = 1.0;
        let mut out = Vec::new();
        render(&mut d, &input, &mut out);

        // Skip the dry leak while the mix ramp settles, then locate the echo.
        let (arg, _) = out
            .iter()
            .enumerate()
            .skip(1_000)
            .fold((0, 0.0f32), |acc, (i, &s)| {
                if s.abs() > acc.1 {
                    (i, s.abs())
                } else {
                    acc
                }
            });
        let expected = (0.350 * SR) as usize;
        assert!(
            arg.abs_diff(expected) <= 2,
            "echo at {arg}, expected {expected}"
        );
        assert!(out.iter().all(|s| s.is_finite()), "produced NaN/inf");
    }

    #[test]
    fn feedback_makes_decaying_repeats() {
        let mut d = fx();
        d.set_param(P_MIX, 1.0);
        d.set_param(P_FEEDBACK, 1.0); // real 0.9, the loop maximum

        let mut input = vec![0.0; 60_000];
        input[0] = 1.0;
        let mut out = Vec::new();
        render(&mut d, &input, &mut out);

        let t = (0.350 * SR) as usize;
        let first = peak(&out[t - 100..t + 100]);
        let second = peak(&out[2 * t - 100..2 * t + 100]);
        assert!(first > 0.5, "first echo missing, peak {first}");
        assert!(second > 0.1, "second echo missing, peak {second}");
        assert!(
            second < first,
            "repeats do not decay: {first} then {second}"
        );
    }

    #[test]
    fn max_feedback_stays_stable() {
        // A dense input burst through the hottest loop must still die out.
        let mut d = fx();
        d.set_param(P_MIX, 1.0);
        d.set_param(P_FEEDBACK, 1.0);

        let mut input = vec![0.0; 48_000 * 8];
        for (i, s) in input.iter_mut().take(4_800).enumerate() {
            *s = if i % 7 == 0 { 0.8 } else { -0.3 };
        }
        let mut out = Vec::new();
        render(&mut d, &input, &mut out);

        assert!(out.iter().all(|s| s.is_finite()), "produced NaN/inf");
        let early = peak(&out[..48_000]);
        let late = peak(&out[48_000 * 7..]);
        assert!(late < early * 0.5, "loop does not decay: {early} → {late}");
    }

    // Synced-echo tolerance: the time OnePole glides in f32 and stalls once
    // its per-step increment (delta/τ) drops under the float quantum at the
    // target's magnitude — ≈ τ·ulp(target)/2 ≈ 5 samples at 33 k. That is
    // 0.014 % of the period (inaudible); the tests allow ±8.

    #[test]
    fn synced_echo_lands_on_the_beat() {
        // 120 bpm at 48 kHz: one beat = 24 000 samples; sync = 1/4.
        let pos = synced_echo_pos(4.0, 24_000.0);
        assert!(
            pos.abs_diff(24_000) <= 8,
            "synced echo at {pos}, expected 24000"
        );
    }

    #[test]
    fn synced_echo_follows_odd_tempo() {
        // 87 bpm: samples_per_beat = 60/87*48000 = 33103.45 (fractional read).
        let pos = synced_echo_pos(4.0, 33_103.45);
        assert!(
            pos.abs_diff(33_103) <= 8,
            "synced echo at {pos}, expected 33103"
        );
    }

    #[test]
    fn sync_falls_back_to_ms_without_tempo() {
        // sync set but no tempo in the context: the ms knob (350 ms) rules.
        let pos = synced_echo_pos(4.0, 0.0);
        assert!(
            pos.abs_diff(16_800) <= 2,
            "fallback echo at {pos}, expected 16800"
        );
    }

    #[test]
    fn zero_mix_passes_dry_signal() {
        let mut d = fx();
        d.set_param(P_MIX, 0.0);

        let input: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let mut out = Vec::new();
        render(&mut d, &input, &mut out);

        // After the 10 ms mix ramp settles, output is exactly the input.
        for (i, (&y, &x)) in out.iter().zip(&input).enumerate().skip(1_000) {
            assert!((y - x).abs() < 1.0e-6, "wet leak at {i}: out {y} vs in {x}");
        }
    }
}
