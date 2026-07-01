//! Reverb effect: mono Freeverb topology — 8 parallel damped comb filters into
//! 4 series allpasses. Honest quality, not a Lexicon; mono by design for the MVP
//! (the mixer pans it with its track), stereo is the first post-MVP beneficiary
//! of the channel-agnostic seam.
//!
//! `size` sets the comb feedback (tail length), `damp` low-passes inside each
//! comb loop (darker tail), `mix` blends dry/wet. Comb/allpass delay lengths are
//! the classic Freeverb tunings, scaled to the engine sample rate. All feedback
//! paths flush denormals (inside `DelayLine::write` and the damping one-pole).

use crate::engine::dsp::{flush, DelayLine};
use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

/// Freeverb delay tunings, in samples at the reference rate.
const REF_SR: f32 = 44_100.0;
const COMB_TUNINGS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNINGS: [usize; 4] = [556, 441, 341, 225];

/// Input attenuation before the comb bank (8 combs sum back up), and the wet
/// make-up after it — both straight from Freeverb.
const INPUT_GAIN: f32 = 0.015;
const WET_GAIN: f32 = 3.0;
/// `size` 0..1 maps to comb feedback `0.7 + 0.28 * size` (Freeverb roomsize).
const FEEDBACK_BASE: f32 = 0.7;
const FEEDBACK_SPAN: f32 = 0.28;
/// `damp` 0..1 is scaled down so full damping still lets the tail breathe.
const DAMP_SCALE: f32 = 0.4;
/// Allpass diffusion feedback.
const ALLPASS_FEEDBACK: f32 = 0.5;

/// Smoothing for size/damp (block rate) and mix (per sample).
const SMOOTH_S: f32 = 0.015;
const RAMP_MS: f32 = 10.0;

const P_SIZE: ParamId = 0;
const P_DAMP: ParamId = 1;
const P_MIX: ParamId = 2;

const PARAMS: [ParamSpec; 3] = [
    ParamSpec {
        id: P_SIZE,
        name: "size",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_DAMP,
        name: "damp",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_MIX,
        name: "mix",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.3,
        curve: ParamCurve::Linear,
    },
];

/// Feedback comb with a one-pole low-pass in the loop (Freeverb's damped comb).
#[derive(Default)]
struct Comb {
    line: DelayLine,
    len: usize,
    /// Damping low-pass state.
    store: f32,
}

impl Comb {
    fn prepare(&mut self, len: usize) {
        self.line.prepare(len);
        self.len = len;
        self.store = 0.0;
    }

    // RT-SAFE
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.line.read_at(self.len);
        self.store = flush(out * (1.0 - damp) + self.store * damp);
        self.line.write(x + self.store * feedback);
        out
    }
}

/// Freeverb allpass diffuser.
#[derive(Default)]
struct Allpass {
    line: DelayLine,
    len: usize,
}

impl Allpass {
    fn prepare(&mut self, len: usize) {
        self.line.prepare(len);
        self.len = len;
    }

    // RT-SAFE
    fn tick(&mut self, x: f32) -> f32 {
        let bufout = self.line.read_at(self.len);
        self.line.write(x + bufout * ALLPASS_FEEDBACK);
        bufout - x
    }
}

pub struct Reverb {
    combs: [Comb; 8],
    allpasses: [Allpass; 4],
    size: OnePole,
    damp: OnePole,
    mix: LinearRamp,
}

impl Default for Reverb {
    fn default() -> Self {
        // Real state is rebuilt in `prepare` once the sample rate is known.
        Self {
            combs: Default::default(),
            allpasses: Default::default(),
            size: OnePole::new(default_real(P_SIZE), 0.99),
            damp: OnePole::new(default_real(P_DAMP), 0.99),
            mix: LinearRamp::new(default_real(P_MIX), RAMP_MS, 48_000.0),
        }
    }
}

impl AudioNode for Reverb {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let feedback = FEEDBACK_BASE + FEEDBACK_SPAN * self.size.advance(frames);
        let damp = self.damp.advance(frames) * DAMP_SCALE;

        let (input, output) = io.in_out(0, 0);
        for (out, &x) in output.iter_mut().zip(input) {
            let mix = self.mix.tick();
            let inject = x * INPUT_GAIN;
            let mut wet = 0.0;
            for comb in &mut self.combs {
                wet += comb.tick(inject, feedback, damp);
            }
            for ap in &mut self.allpasses {
                wet = ap.tick(wet);
            }
            *out = x * (1.0 - mix) + wet * WET_GAIN * mix;
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
            P_SIZE => self.size.set_target(real),
            P_DAMP => self.damp.set_target(real),
            P_MIX => self.mix.set_target(real),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        let scale = sample_rate / REF_SR;
        for (comb, &len) in self.combs.iter_mut().zip(&COMB_TUNINGS) {
            comb.prepare(((len as f32 * scale) as usize).max(1));
        }
        for (ap, &len) in self.allpasses.iter_mut().zip(&ALLPASS_TUNINGS) {
            ap.prepare(((len as f32 * scale) as usize).max(1));
        }
        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.size = OnePole::new(default_real(P_SIZE), coef);
        self.damp = OnePole::new(default_real(P_DAMP), coef);
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

    fn fx() -> Reverb {
        let mut r = Reverb::default();
        r.prepare(SR, 256);
        r
    }

    fn render(r: &mut Reverb, input: &[f32], out: &mut Vec<f32>) {
        let frames = input.len();
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames,
        };
        let mut io = NodeIo::new(&[], input, out, frames);
        r.process(&ctx, &mut io);
    }

    fn energy(buf: &[f32]) -> f32 {
        buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
    }

    fn impulse_response(r: &mut Reverb, secs: f32) -> Vec<f32> {
        let n = (secs * SR) as usize;
        let mut input = vec![0.0; n];
        input[0] = 1.0;
        let mut out = Vec::new();
        render(r, &input, &mut out);
        out
    }

    #[test]
    fn impulse_response_decays_to_silence() {
        // Default size (feedback 0.84): the tail must die out — finite RT60.
        let mut r = fx();
        r.set_param(P_MIX, 1.0);
        let out = impulse_response(&mut r, 4.0);

        assert!(out.iter().all(|s| s.is_finite()), "produced NaN/inf");
        assert!(
            peak(&out) <= 1.0,
            "impulse response clips, peak {}",
            peak(&out)
        );

        let early = energy(&out[..24_000]);
        let mid = energy(&out[96_000..120_000]);
        let tail = peak(&out[168_000..]);
        assert!(early > 0.0, "no reverb tail at all");
        assert!(mid < early, "tail does not decay: {early} then {mid}");
        assert!(tail < 1.0e-2, "tail still ringing after 3.5 s, peak {tail}");
    }

    #[test]
    fn bigger_size_rings_longer() {
        let mut small = fx();
        small.set_param(P_MIX, 1.0);
        small.set_param(P_SIZE, 0.0);
        let mut big = fx();
        big.set_param(P_MIX, 1.0);
        big.set_param(P_SIZE, 1.0);

        let a = impulse_response(&mut small, 2.0);
        let b = impulse_response(&mut big, 2.0);
        let late_small = energy(&a[72_000..]);
        let late_big = energy(&b[72_000..]);
        assert!(
            late_big > late_small * 10.0,
            "size has no effect on tail: {late_small} vs {late_big}"
        );
    }

    #[test]
    fn silence_in_is_silence_out() {
        let mut r = fx();
        r.set_param(P_MIX, 1.0);
        let input = vec![0.0; 24_000];
        let mut out = Vec::new();
        render(&mut r, &input, &mut out);
        assert!(peak(&out) == 0.0, "reverb makes noise on its own");
    }

    #[test]
    fn zero_mix_passes_dry_signal() {
        let mut r = fx();
        r.set_param(P_MIX, 0.0);
        let input: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let mut out = Vec::new();
        render(&mut r, &input, &mut out);
        // After the 10 ms mix ramp settles, output is exactly the input.
        for (i, (&y, &x)) in out.iter().zip(&input).enumerate().skip(1_000) {
            assert!((y - x).abs() < 1.0e-6, "wet leak at {i}: out {y} vs in {x}");
        }
    }
}
