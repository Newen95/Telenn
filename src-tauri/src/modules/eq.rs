//! Three-band parametric EQ: a fixed 120 Hz low shelf, a sweepable mid
//! peaking bell, and a fixed 6 kHz high shelf in series (RBJ biquads).
//! All gains at 0 dB is a pass-through by construction.
//!
//! `process` is RT-SAFE: coefficients are recomputed once per block from the
//! glided values; per-channel biquad state, shared coefficients.

use crate::engine::dsp::Biquad;
use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::OnePole;

/// Gain/frequency glide time constant (seconds).
const SMOOTH_S: f32 = 0.015;
/// Fixed corner frequencies of the shelves and the mid bell width.
const LOW_SHELF_HZ: f32 = 120.0;
const HIGH_SHELF_HZ: f32 = 6000.0;
const MID_Q: f32 = 0.9;
/// Band indices in each channel's filter chain.
const BANDS: usize = 3;

const P_LOW: ParamId = 0;
const P_MID_FREQ: ParamId = 1;
const P_MID: ParamId = 2;
const P_HIGH: ParamId = 3;

const PARAMS: [ParamSpec; 4] = [
    ParamSpec {
        id: P_LOW,
        name: "low",
        unit: "dB",
        min: -12.0,
        max: 12.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_MID_FREQ,
        name: "mid_freq",
        unit: "Hz",
        min: 200.0,
        max: 5000.0,
        default: 900.0,
        curve: ParamCurve::Log,
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
        id: P_HIGH,
        name: "high",
        unit: "dB",
        min: -12.0,
        max: 12.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
];

pub struct Eq {
    sample_rate: f32,
    low: OnePole,
    mid_freq: OnePole,
    mid: OnePole,
    high: OnePole,
    /// Three bands per channel: shared coefficients, per-channel state.
    bands: [[Biquad; BANDS]; crate::engine::CHANNELS],
}

impl Default for Eq {
    fn default() -> Self {
        // The real smoothers are rebuilt in `prepare` with the sample rate.
        Self {
            sample_rate: 48_000.0,
            low: OnePole::new(0.0, 0.99),
            mid_freq: OnePole::new(900.0, 0.99),
            mid: OnePole::new(0.0, 0.99),
            high: OnePole::new(0.0, 0.99),
            bands: Default::default(),
        }
    }
}

impl AudioNode for Eq {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let sr = self.sample_rate;
        let low = self.low.advance(frames);
        let mid_freq = self.mid_freq.advance(frames);
        let mid = self.mid.advance(frames);
        let high = self.high.advance(frames);
        for chain in &mut self.bands {
            chain[0].set_lowshelf(LOW_SHELF_HZ, low, sr);
            chain[1].set_peaking(mid_freq, MID_Q, mid, sr);
            chain[2].set_highshelf(HIGH_SHELF_HZ, high, sr);
        }

        // RT-SAFE: no per-sample smoother in the loop, so channels can be
        // processed one after the other on their own biquad state.
        for ch in 0..io.channels().min(self.bands.len()) {
            let (input, output) = io.in_out_ch(0, 0, ch);
            let chain = &mut self.bands[ch];
            for (out, &x) in output.iter_mut().zip(input) {
                let low = chain[0].process(x);
                let mid = chain[1].process(low);
                *out = chain[2].process(mid);
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
            P_LOW => self.low.set_target(real),
            P_MID_FREQ => self.mid_freq.set_target(real),
            P_MID => self.mid.set_target(real),
            P_HIGH => self.high.set_target(real),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.low = OnePole::new(default_real(P_LOW), coef);
        self.mid_freq = OnePole::new(default_real(P_MID_FREQ), coef);
        self.mid = OnePole::new(default_real(P_MID), coef);
        self.high = OnePole::new(default_real(P_HIGH), coef);
        self.bands = Default::default();
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

    fn fx() -> Eq {
        let mut e = Eq::default();
        e.prepare(SR, 256);
        e
    }

    fn set_real(e: &mut Eq, id: ParamId, real: f32) {
        e.set_param(id, PARAMS[id as usize].normalize(real));
    }

    fn render(e: &mut Eq, input: &[f32], out: &mut Vec<f32>) {
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
        e.process(&ctx, &mut io);
    }

    fn sine(freq: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (TAU * freq * i as f32 / SR).sin()).collect()
    }

    /// Settled gain in dB of the EQ at `freq` (smoothers given time to land).
    fn gain_db_at(e: &mut Eq, freq: f32) -> f32 {
        let x = sine(freq, 19_200);
        let mut y = Vec::new();
        render(e, &x, &mut y);
        let tail = &y[9600..];
        let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
        20.0 * (rms / core::f32::consts::FRAC_1_SQRT_2).log10()
    }

    #[test]
    fn bands_boost_and_cut_where_they_should() {
        let mut e = fx();
        set_real(&mut e, P_LOW, 9.0);
        let g = gain_db_at(&mut e, 50.0);
        assert!((g - 9.0).abs() < 1.0, "low shelf at 50 Hz: {g} dB, want 9");

        let mut e = fx();
        set_real(&mut e, P_MID_FREQ, 1000.0);
        set_real(&mut e, P_MID, -9.0);
        let g = gain_db_at(&mut e, 1000.0);
        assert!((g + 9.0).abs() < 1.0, "mid cut at 1 kHz: {g} dB, want -9");
        let g = gain_db_at(&mut e, 60.0);
        assert!(g.abs() < 1.0, "mid cut leaked to 60 Hz: {g} dB");

        let mut e = fx();
        set_real(&mut e, P_HIGH, 6.0);
        let g = gain_db_at(&mut e, 14_000.0);
        assert!(
            (g - 6.0).abs() < 1.0,
            "high shelf at 14 kHz: {g} dB, want 6"
        );
    }

    #[test]
    fn neutral_settings_pass_the_signal_through() {
        let mut e = fx();
        for freq in [60.0, 900.0, 8000.0] {
            let g = gain_db_at(&mut e, freq);
            assert!(g.abs() < 0.2, "neutral EQ coloured {freq} Hz by {g} dB");
        }
    }

    #[test]
    fn gain_sweep_is_click_free() {
        let mut e = fx();
        // One continuous sine, rendered in blocks while the gains sweep.
        let x = sine(220.0, 80 * 256);
        let mut all = Vec::new();
        let mut chunk = Vec::new();
        for step in 0..80 {
            e.set_param(P_LOW, (step % 21) as f32 / 20.0);
            e.set_param(P_MID, ((step * 3) % 21) as f32 / 20.0);
            render(&mut e, &x[step * 256..(step + 1) * 256], &mut chunk);
            all.extend_from_slice(&chunk);
        }
        let clicks = crate::analysis::levels::detect_clicks(&all, 48_000);
        assert!(clicks.is_empty(), "eq sweep clicked: {clicks:?}");
    }
}
