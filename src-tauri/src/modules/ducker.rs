//! Tempo-synced ducker: a gain dip at every beat division that recovers
//! exponentially — the sidechain-pump sound without any routing (the classic
//! synthwave "breathing" bass/pad). Grid position comes from
//! [`ProcessCtx::beat_phase`]; without a tempo the signal passes dry.
//!
//! All DSP is hand-written; `process` is RT-SAFE (the gain is a pure function
//! of the grid phase — no state beyond the smoothed depth).

use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::OnePole;

/// Depth glide time constant (seconds).
const SMOOTH_S: f32 = 0.015;
/// The dip itself: a 5 ms linear drop (fast but never a discontinuity).
const DIP_S: f32 = 0.005;
/// Beat divisions per rate step: every beat, every eighth, every half note.
const DUCK_BEATS: [f32; 3] = [1.0, 0.5, 2.0];

const P_DEPTH: ParamId = 0;
const P_RATE: ParamId = 1;
const P_RELEASE: ParamId = 2;

const PARAMS: [ParamSpec; 3] = [
    ParamSpec {
        id: P_DEPTH,
        name: "depth",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        curve: ParamCurve::Linear,
    },
    // Rounded selector — 0: every beat, 1: every eighth, 2: every half note.
    ParamSpec {
        id: P_RATE,
        name: "rate",
        unit: "",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_RELEASE,
        name: "release",
        unit: "ms",
        min: 20.0,
        max: 500.0,
        default: 180.0,
        curve: ParamCurve::Log,
    },
];

pub struct Ducker {
    sample_rate: f32,
    depth: OnePole,
    rate_idx: usize,
    release_s: f32,
}

impl Default for Ducker {
    fn default() -> Self {
        // The real smoother is rebuilt in `prepare` with the sample rate.
        Self {
            sample_rate: 48_000.0,
            depth: OnePole::new(0.5, 0.99),
            rate_idx: 0,
            release_s: 0.18,
        }
    }
}

impl AudioNode for Ducker {
    fn process(&mut self, ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let depth = self.depth.advance(frames);
        let spb = ctx.samples_per_beat;
        if spb <= 0.0 || depth <= 0.0 {
            // No grid (or no depth): pass dry on every channel.
            for ch in 0..io.channels() {
                let (input, output) = io.in_out_ch(0, 0, ch);
                output.copy_from_slice(input);
            }
            return;
        }

        let div_samples = DUCK_BEATS[self.rate_idx] * spb;
        let dip_samples = (DIP_S * self.sample_rate).max(1.0);
        let release_samples = (self.release_s * self.sample_rate).max(1.0);
        // Grid offset of the block start, in samples past the last division.
        let block_offset = (ctx.beat_phase * spb).rem_euclid(div_samples);
        for ch in 0..io.channels() {
            let (input, output) = io.in_out_ch(0, 0, ch);
            for (i, (out, &x)) in output.iter_mut().zip(input).enumerate() {
                let t = (block_offset + i as f32).rem_euclid(div_samples);
                let gain = if t < dip_samples {
                    1.0 - depth * (t / dip_samples)
                } else {
                    1.0 - depth * (-(t - dip_samples) / release_samples).exp()
                };
                *out = x * gain;
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
            P_DEPTH => self.depth.set_target(real),
            P_RATE => self.rate_idx = (real.round() as usize).min(DUCK_BEATS.len() - 1),
            P_RELEASE => self.release_s = real / 1000.0,
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.depth = OnePole::new(default_real(P_DEPTH), coef);
        self.rate_idx = default_real(P_RATE).round() as usize;
        self.release_s = default_real(P_RELEASE) / 1000.0;
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

    fn fx() -> Ducker {
        let mut d = Ducker::default();
        d.prepare(SR, 256);
        d
    }

    fn set_real(d: &mut Ducker, id: ParamId, real: f32) {
        d.set_param(id, PARAMS[id as usize].normalize(real));
    }

    fn render_with_grid(
        d: &mut Ducker,
        input: &[f32],
        out: &mut Vec<f32>,
        samples_per_beat: f32,
        beat_phase: f32,
    ) {
        let frames = input.len();
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames,
            channels: 1,
            samples_per_beat,
            beat_phase,
        };
        let mut io = NodeIo::new(&[], input, out, frames, 1);
        d.process(&ctx, &mut io);
    }

    #[test]
    fn no_tempo_passes_dry() {
        let mut d = fx();
        let input: Vec<f32> = (0..2048)
            .map(|i| (TAU * 220.0 * i as f32 / SR).sin())
            .collect();
        let mut out = Vec::new();
        render_with_grid(&mut d, &input, &mut out, 0.0, 0.0);
        assert_eq!(out, input, "no tempo must be a pass-through");
    }

    #[test]
    fn dips_on_every_beat_and_recovers() {
        let mut d = fx();
        set_real(&mut d, P_DEPTH, 0.8);
        set_real(&mut d, P_RELEASE, 60.0);
        let spb = 4800.0; // fast grid: two beats in one render
        let input = vec![1.0f32; 9600];
        let mut out = Vec::new();
        render_with_grid(&mut d, &input, &mut out, spb, 0.0);
        // Right after the dip ramp: gain ≈ 1 - depth.
        let dip = (DIP_S * SR) as usize;
        assert!(
            (out[dip] - 0.2).abs() < 0.1,
            "dip floor {} at the beat",
            out[dip]
        );
        // Just before the next beat (~1.5 τ later): recovery well under way
        // (theory: 1 - 0.8·e^(-4460/2880) ≈ 0.83).
        assert!(out[4700] > 0.8, "no recovery before beat 2: {}", out[4700]);
        // Second beat dips again.
        assert!(
            (out[4800 + dip] - 0.2).abs() < 0.1,
            "no dip on beat 2: {}",
            out[4800 + dip]
        );
    }

    #[test]
    fn grid_phase_offsets_the_dip() {
        // Block starting half a beat in: the dip lands half a beat later.
        let mut d = fx();
        set_real(&mut d, P_DEPTH, 0.8);
        set_real(&mut d, P_RELEASE, 60.0);
        let spb = 4800.0;
        let input = vec![1.0f32; 4800];
        let mut out = Vec::new();
        render_with_grid(&mut d, &input, &mut out, spb, 0.5);
        let dip = (DIP_S * SR) as usize;
        assert!(
            (out[2400 + dip] - 0.2).abs() < 0.1,
            "dip missed the offset grid: {}",
            out[2400 + dip]
        );
        assert!(out[2300] > 0.5, "early dip: {}", out[2300]);
    }

    #[test]
    fn pumping_is_click_free() {
        let mut d = fx();
        set_real(&mut d, P_DEPTH, 0.9);
        let input: Vec<f32> = (0..48_000)
            .map(|i| (TAU * 110.0 * i as f32 / SR).sin())
            .collect();
        let mut out = Vec::new();
        render_with_grid(&mut d, &input, &mut out, 12_000.0, 0.0);
        let clicks = crate::analysis::levels::detect_clicks(&out, 48_000);
        assert!(clicks.is_empty(), "ducker clicked: {clicks:?}");
    }
}
