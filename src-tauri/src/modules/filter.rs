//! Multimode filter effect: one RBJ biquad switched between low-pass,
//! high-pass, band-pass and notch, with glided cutoff and resonance.
//!
//! The first pure sound-sculpting effect: it shapes ANY source in the chain
//! (thin a harmonic, isolate a band, carve a resonant sweep). All DSP is
//! hand-written; `process` is RT-SAFE (no allocation, no locks).

use crate::engine::dsp::Biquad;
use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::OnePole;

/// Cutoff/resonance glide time constant (seconds), same feel as the synths.
const SMOOTH_S: f32 = 0.015;

const P_TYPE: ParamId = 0;
const P_CUTOFF: ParamId = 1;
const P_RESONANCE: ParamId = 2;

/// Filter-type selector values (P_TYPE, rounded).
const TYPE_LOWPASS: usize = 0;
const TYPE_HIGHPASS: usize = 1;
const TYPE_BANDPASS: usize = 2;
const TYPE_NOTCH: usize = 3;

const PARAMS: [ParamSpec; 3] = [
    ParamSpec {
        id: P_TYPE,
        name: "type",
        unit: "",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_CUTOFF,
        name: "cutoff",
        unit: "Hz",
        min: 20.0,
        max: 20_000.0,
        default: 1000.0,
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
];

pub struct Filter {
    sample_rate: f32,
    type_idx: usize,
    cutoff: OnePole,
    resonance: OnePole,
    /// One biquad per channel: shared coefficients, per-channel state.
    biquads: [Biquad; crate::engine::CHANNELS],
}

impl Default for Filter {
    fn default() -> Self {
        // The real smoothers are rebuilt in `prepare` with the sample rate.
        Self {
            sample_rate: 48_000.0,
            type_idx: TYPE_LOWPASS,
            cutoff: OnePole::new(1000.0, 0.99),
            resonance: OnePole::new(0.7, 0.99),
            biquads: Default::default(),
        }
    }
}

impl AudioNode for Filter {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let sr = self.sample_rate;
        // Coefficients once per block from the glided values (anti-zipper);
        // a type change swaps coefficient sets — see the module_specs quirk.
        let fc = self.cutoff.advance(frames);
        let q = self.resonance.advance(frames);
        for biquad in &mut self.biquads {
            match self.type_idx {
                TYPE_HIGHPASS => biquad.set_highpass(fc, q, sr),
                TYPE_BANDPASS => biquad.set_bandpass(fc, q, sr),
                TYPE_NOTCH => biquad.set_notch(fc, q, sr),
                _ => biquad.set_lowpass(fc, q, sr),
            }
        }

        // RT-SAFE: no per-sample smoother in the loop, so channels can be
        // processed one after the other on their own biquad state.
        for ch in 0..io.channels().min(self.biquads.len()) {
            let (input, output) = io.in_out_ch(0, 0, ch);
            for (out, &x) in output.iter_mut().zip(input) {
                *out = self.biquads[ch].process(x);
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
            P_TYPE => self.type_idx = (real.round() as usize).min(TYPE_NOTCH),
            P_CUTOFF => self.cutoff.set_target(real),
            P_RESONANCE => self.resonance.set_target(real),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.type_idx = default_real(P_TYPE).round() as usize;
        self.cutoff = OnePole::new(default_real(P_CUTOFF), coef);
        self.resonance = OnePole::new(default_real(P_RESONANCE), coef);
        self.biquads = Default::default();
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

    fn fx() -> Filter {
        let mut f = Filter::default();
        f.prepare(SR, 256);
        f
    }

    fn set_real(f: &mut Filter, id: ParamId, real: f32) {
        f.set_param(id, PARAMS[id as usize].normalize(real));
    }

    fn render(f: &mut Filter, input: &[f32], out: &mut Vec<f32>) {
        out.resize(input.len(), 0.0);
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames: input.len(),
            channels: 1,
            samples_per_beat: 0.0,
            beat_phase: 0.0,
        };
        let mut io = NodeIo::new(&[], input, out, input.len(), 1);
        f.process(&ctx, &mut io);
    }

    fn sine(freq: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (TAU * freq * i as f32 / SR).sin()).collect()
    }

    fn tail_rms(buf: &[f32]) -> f32 {
        let tail = &buf[buf.len() / 2..];
        (tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32).sqrt()
    }

    /// Settled RMS of a unit sine at `freq` through a configured filter.
    fn response(type_real: f32, cutoff: f32, q: f32, freq: f32) -> f32 {
        let mut f = fx();
        set_real(&mut f, P_TYPE, type_real);
        set_real(&mut f, P_CUTOFF, cutoff);
        set_real(&mut f, P_RESONANCE, q);
        // Two renders: the first lets the cutoff/resonance glide settle.
        let mut out = Vec::new();
        render(&mut f, &sine(freq, 9600), &mut out);
        render(&mut f, &sine(freq, 9600), &mut out);
        tail_rms(&out)
    }

    #[test]
    fn lowpass_darkens_and_highpass_brightens() {
        const UNIT: f32 = 0.707; // RMS of a unit sine
        assert!(
            response(0.0, 500.0, 0.7, 4000.0) < 0.05,
            "lp passed the top"
        );
        assert!(
            response(0.0, 500.0, 0.7, 100.0) > 0.8 * UNIT,
            "lp lost the bottom"
        );
        assert!(
            response(1.0, 2000.0, 0.7, 100.0) < 0.05,
            "hp passed the bottom"
        );
        assert!(
            response(1.0, 2000.0, 0.7, 10_000.0) > 0.8 * UNIT,
            "hp lost the top"
        );
    }

    #[test]
    fn bandpass_and_notch_work_the_centre() {
        let centre = response(2.0, 1000.0, 2.0, 1000.0);
        assert!(centre > 0.3, "bp lost its centre: {centre}");
        assert!(
            response(2.0, 1000.0, 2.0, 60.0) < 0.3 * centre,
            "bp passed the skirt"
        );
        assert!(
            response(3.0, 1000.0, 4.0, 1000.0) < 0.05,
            "notch passed its centre"
        );
        assert!(
            response(3.0, 1000.0, 4.0, 100.0) > 0.5,
            "notch lost the low end"
        );
    }

    #[test]
    fn max_cutoff_lowpass_is_near_transparent() {
        let rms = response(0.0, 20_000.0, 0.7, 1000.0);
        let db = 20.0 * (rms / 0.707).log10();
        assert!(db.abs() < 1.0, "lp at 20 kHz coloured the signal: {db} dB");
    }

    #[test]
    fn cutoff_sweep_is_click_free() {
        use crate::analysis::levels;
        let mut f = fx();
        let input = sine(220.0, 256);
        let mut out = Vec::new();
        let mut whole = Vec::new();
        for k in 0..128 {
            let v = k as f32 / 127.0;
            f.set_param(P_CUTOFF, 1.0 - v); // 20 kHz → 20 Hz
            render(&mut f, &input, &mut out);
            whole.extend_from_slice(&out);
        }
        assert!(whole.iter().all(|x| x.is_finite()));
        let clicks = levels::detect_clicks(&whole, 48_000);
        assert!(clicks.is_empty(), "cutoff sweep clicked: {clicks:?}");
    }

    #[test]
    fn silence_in_silence_out() {
        let mut f = fx();
        set_real(&mut f, P_RESONANCE, 10.0);
        let mut out = Vec::new();
        render(&mut f, &vec![0.0; 4096], &mut out);
        assert!(out.iter().all(|&x| x == 0.0));
    }
}
