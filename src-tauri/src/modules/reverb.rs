//! Reverb effect: stereo Dattorro plate (Jon Dattorro, "Effect Design Part 1:
//! Reverberator and Other Filters", JAES 1997) — predelay, input diffusion,
//! and a modulated figure-of-eight tank. Denser and smoother than the old
//! Freeverb topology, with the tank allpasses gently modulated so the tail
//! never locks into metallic resonances. The tank is fed the input fold and
//! the paired L/R tap tables decorrelate the wet field; the mono fold of the
//! stereo wet is exactly the legacy mono wet (mono-compatible by design).
//!
//! `size` sets the tank decay (tail length), `damp` low-passes inside the
//! tank (darker tail), `predelay` separates the dry attack from the wet
//! field, `mix` blends dry/wet. All tunings are Dattorro's, given at his
//! 29.761 kHz reference rate and scaled to the engine sample rate in
//! `prepare`. Every feedback path flushes denormals (`DelayLine::write` and
//! the one-pole stores).

use crate::engine::dsp::{flush, DelayLine, EnvFollower};
use crate::engine::node::{
    AudioNode, NodeIo, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx, SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

/// Dattorro's reference sample rate: every tuning below is in samples at it.
const REF_SR: f32 = 29_761.0;

/// Input diffusion allpasses: (length, coefficient).
const INPUT_APS: [(usize, f32); 4] = [(142, 0.75), (107, 0.75), (379, 0.625), (277, 0.625)];
/// Input bandwidth one-pole (barely-there high shave, per the paper).
const BANDWIDTH: f32 = 0.9995;

/// Tank tunings per branch: modulated allpass, delay 1, allpass 2, delay 2.
const TANK_A: [usize; 4] = [672, 4453, 1800, 3720];
const TANK_B: [usize; 4] = [908, 4217, 2656, 3163];
/// Decay-diffusion-1 coefficient (the modulated tank allpasses).
const TANK_AP1_G: f32 = -0.70;
/// LFO rates of the two modulated allpasses, detuned so the branches never
/// phase-lock; excursion in samples at the reference rate.
const LFO_RATES_HZ: [f32; 2] = [0.60, 0.71];
const LFO_EXCURSION: f32 = 12.0;

/// Output tap offsets (paper, table 2), read positions per branch line, and
/// their signs. Order: D1b, D1b, AP2b, D2b, D1a, AP2a, D2a for the left ear,
/// mirrored for the right; mono output averages the two.
const TAPS_L: [usize; 7] = [266, 2974, 1913, 1996, 1990, 187, 1066];
const TAPS_R: [usize; 7] = [353, 3627, 1228, 2673, 2111, 335, 121];
const TAP_SIGNS: [f32; 7] = [1.0, 1.0, -1.0, 1.0, -1.0, -1.0, -1.0];
const TAP_GAIN: f32 = 0.6;

/// Wet make-up, calibrated so the default-mix level matches the old reverb.
const WET_GAIN: f32 = 1.0;

/// `size` 0..1 maps to the tank decay gain. The plan's 0.55 floor measured
/// out to a 2 s minimum RT60; 0.25..0.90 spans ~0.9 s (tight booth) to ~12 s
/// (grandiose) with the default 0.5 landing near 2 s.
const DECAY_BASE: f32 = 0.25;
const DECAY_SPAN: f32 = 0.65;
/// `damp` 0..1 scaled so full damping still lets the tail breathe.
const DAMP_SCALE: f32 = 0.7;

/// Smoothing: size/damp at block rate, mix per sample, predelay per sample
/// (τ 50 ms — moving the knob tape-bends instead of clicking).
const SMOOTH_S: f32 = 0.015;
const RAMP_MS: f32 = 10.0;
const PREDELAY_SMOOTH_S: f32 = 0.05;
const PREDELAY_MAX_MS: f32 = 100.0;

const P_SIZE: ParamId = 0;
const P_DAMP: ParamId = 1;
const P_MIX: ParamId = 2;
const P_PREDELAY: ParamId = 3;
const P_GATE: ParamId = 4;

/// Gate timing: how long the wet stays open after the input drops below the
/// threshold, and the open/close slews (the 80s gated-snare envelope).
const GATE_HOLD_S: f32 = 0.12;
const GATE_OPEN_S: f32 = 0.002;
const GATE_CLOSE_S: f32 = 0.008;
const GATE_DET_ATTACK_S: f32 = 0.001;
const GATE_DET_RELEASE_S: f32 = 0.03;

const PARAMS: [ParamSpec; 5] = [
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
    // Linear, not Log: Log cannot reach 0 ms (denormalize divides by min).
    ParamSpec {
        id: P_PREDELAY,
        name: "predelay",
        unit: "ms",
        min: 0.0,
        max: PREDELAY_MAX_MS,
        default: 10.0,
        curve: ParamCurve::Linear,
    },
    // Gate threshold on the dry input (0 = off): the wet opens with a hit
    // and slams shut 120 ms later — the gated-reverb snare.
    ParamSpec {
        id: P_GATE,
        name: "gate",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
];

/// Lattice allpass with a signed coefficient; output taps read its internal
/// line (the `v` history), as the plate's tap tables require.
#[derive(Default)]
struct PlateAllpass {
    line: DelayLine,
    len: usize,
    g: f32,
}

impl PlateAllpass {
    fn prepare(&mut self, len: usize, g: f32) {
        self.line.prepare(len);
        self.len = len;
        self.g = g;
    }

    // RT-SAFE
    fn tick(&mut self, x: f32) -> f32 {
        let d = self.line.read_at(self.len);
        let v = x + self.g * d;
        let y = d - self.g * v;
        self.line.write(v);
        y
    }

    // RT-SAFE
    fn tap(&self, offset: usize) -> f32 {
        self.line.read_at(offset)
    }
}

/// Tank allpass whose length breathes under a slow sine LFO — the plate's
/// anti-metallic trick: resonances never sit still long enough to ring.
#[derive(Default)]
struct ModAllpass {
    line: DelayLine,
    base_len: f32,
    g: f32,
    lfo_phase: f32,
    lfo_inc: f32,
    excursion: f32,
}

impl ModAllpass {
    fn prepare(&mut self, base_len: f32, excursion: f32, g: f32, phase: f32, inc: f32) {
        // +4: linear interpolation reads i and i+1 at the maximum excursion.
        self.line
            .prepare((base_len + excursion).ceil() as usize + 4);
        self.base_len = base_len;
        self.excursion = excursion;
        self.g = g;
        self.lfo_phase = phase;
        self.lfo_inc = inc;
    }

    // RT-SAFE
    fn tick(&mut self, x: f32) -> f32 {
        self.lfo_phase += self.lfo_inc;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }
        let offset = (std::f32::consts::TAU * self.lfo_phase).sin() * self.excursion;
        let d = self.line.read(self.base_len + offset);
        let v = x + self.g * d;
        let y = d - self.g * v;
        self.line.write(v);
        y
    }
}

/// One half of the figure-of-eight: modulated allpass → delay → damping
/// one-pole → decay → allpass → delay. `fb` holds the last delay-2 output,
/// the z⁻¹ the other branch reads (cross-coupling).
#[derive(Default)]
struct TankBranch {
    ap1: ModAllpass,
    delay1: DelayLine,
    d1_len: usize,
    damp_store: f32,
    ap2: PlateAllpass,
    delay2: DelayLine,
    d2_len: usize,
    fb: f32,
}

impl TankBranch {
    fn prepare(&mut self, tunings: &[usize; 4], scale: f32, lfo_phase: f32, lfo_inc: f32) {
        let len = |t: usize| ((t as f32 * scale).round() as usize).max(1);
        self.ap1.prepare(
            tunings[0] as f32 * scale,
            LFO_EXCURSION * scale,
            TANK_AP1_G,
            lfo_phase,
            lfo_inc,
        );
        self.d1_len = len(tunings[1]);
        self.delay1.prepare(self.d1_len);
        self.damp_store = 0.0;
        self.ap2.prepare(len(tunings[2]), 0.5);
        self.d2_len = len(tunings[3]);
        self.delay2.prepare(self.d2_len);
        self.fb = 0.0;
    }

    // RT-SAFE. `other_fb` is the opposite branch's previous-sample output.
    fn tick(&mut self, diffused: f32, other_fb: f32, decay: f32, damping: f32) {
        let x = self.ap1.tick(diffused + decay * other_fb);
        let d1 = self.delay1.read_at(self.d1_len);
        self.delay1.write(x);
        self.damp_store = flush(d1 * (1.0 - damping) + self.damp_store * damping);
        let y = self.ap2.tick(self.damp_store * decay);
        let d2 = self.delay2.read_at(self.d2_len);
        self.delay2.write(y);
        self.fb = d2;
    }
}

pub struct Reverb {
    sample_rate: f32,
    predelay_line: DelayLine,
    predelay_len: OnePole,
    bw_store: f32,
    input_aps: [PlateAllpass; 4],
    a: TankBranch,
    b: TankBranch,
    taps_l: [usize; 7],
    taps_r: [usize; 7],
    size: OnePole,
    damp: OnePole,
    mix: LinearRamp,
    /// Gate threshold (0 = off), detector, hold counter and slewed wet gain.
    gate_level: f32,
    gate_env: EnvFollower,
    gate_hold: u32,
    gate_hold_samples: u32,
    gate_gain: f32,
    gate_open_coef: f32,
    gate_close_coef: f32,
}

impl Default for Reverb {
    fn default() -> Self {
        // Real state is rebuilt in `prepare` once the sample rate is known.
        Self {
            sample_rate: 48_000.0,
            predelay_line: DelayLine::default(),
            predelay_len: OnePole::new(480.0, 0.99),
            bw_store: 0.0,
            input_aps: Default::default(),
            a: TankBranch::default(),
            b: TankBranch::default(),
            taps_l: [0; 7],
            taps_r: [0; 7],
            size: OnePole::new(default_real(P_SIZE), 0.99),
            damp: OnePole::new(default_real(P_DAMP), 0.99),
            mix: LinearRamp::new(default_real(P_MIX), RAMP_MS, 48_000.0),
            gate_level: 0.0,
            gate_env: EnvFollower::default(),
            gate_hold: 0,
            gate_hold_samples: 0,
            gate_gain: 1.0,
            gate_open_coef: 0.0,
            gate_close_coef: 0.0,
        }
    }
}

impl Reverb {
    /// Sum a branch's seven output taps with the plate's signs. // RT-SAFE
    fn read_taps(own: &TankBranch, other: &TankBranch, taps: &[usize; 7]) -> f32 {
        // Order: D1other ×2, AP2other, D2other, then D1own, AP2own, D2own.
        TAP_GAIN
            * (TAP_SIGNS[0] * other.delay1.read_at(taps[0])
                + TAP_SIGNS[1] * other.delay1.read_at(taps[1])
                + TAP_SIGNS[2] * other.ap2.tap(taps[2])
                + TAP_SIGNS[3] * other.delay2.read_at(taps[3])
                + TAP_SIGNS[4] * own.delay1.read_at(taps[4])
                + TAP_SIGNS[5] * own.ap2.tap(taps[5])
                + TAP_SIGNS[6] * own.delay2.read_at(taps[6]))
    }
}

impl AudioNode for Reverb {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        let frames = io.frames();
        let decay = DECAY_BASE + DECAY_SPAN * self.size.advance(frames);
        let damping = self.damp.advance(frames) * DAMP_SCALE;
        // Decay-diffusion-2 follows decay (paper's recommendation), capped.
        let g2 = (decay + 0.15).clamp(0.25, 0.5);
        self.a.ap2.g = g2;
        self.b.ap2.g = g2;

        // Stereo: the tank is fed the input fold and the L/R tap tables
        // finally part ways (they were built as pairs for this day). The mono
        // fold of the stereo wet is exactly the legacy wet 0.5·(yl+yr), so
        // mono compatibility is preserved by construction.
        let stereo = io.channels() > 1;
        for i in 0..frames {
            let l = io.input_ch(0, 0)[i];
            let r = if stereo { io.input_ch(0, 1)[i] } else { l };
            let x = 0.5 * (l + r);
            let mix = self.mix.tick();

            let pd = self.predelay_len.advance(1).max(1.0);
            let delayed = self.predelay_line.read(pd);
            self.predelay_line.write(x);

            self.bw_store = flush(delayed * BANDWIDTH + self.bw_store * (1.0 - BANDWIDTH));
            let mut diffused = self.bw_store;
            for ap in &mut self.input_aps {
                diffused = ap.tick(diffused);
            }

            // Cross-feed with the previous sample's branch outputs (z⁻¹).
            let (fb_a, fb_b) = (self.a.fb, self.b.fb);
            self.a.tick(diffused, fb_b, decay, damping);
            self.b.tick(diffused, fb_a, decay, damping);

            let yl = Self::read_taps(&self.a, &self.b, &self.taps_l);
            let yr = Self::read_taps(&self.b, &self.a, &self.taps_r);

            // Gate on the DRY input: the wet opens with a hit and slams shut
            // after the hold — dry hits keep triggering, tails never do. At
            // threshold 0 the gain is exactly 1.0 (bit-identical wet).
            let gate = if self.gate_level > 0.0 {
                let det = self.gate_env.tick(x);
                if det >= self.gate_level {
                    self.gate_hold = self.gate_hold_samples;
                } else if self.gate_hold > 0 {
                    self.gate_hold -= 1;
                }
                let target = if det >= self.gate_level || self.gate_hold > 0 {
                    1.0
                } else {
                    0.0
                };
                let coef = if target > self.gate_gain {
                    self.gate_open_coef
                } else {
                    self.gate_close_coef
                };
                self.gate_gain = flush(target + (self.gate_gain - target) * coef);
                self.gate_gain
            } else {
                1.0
            };

            if stereo {
                io.output_ch(0, 0)[i] = l * (1.0 - mix) + yl * WET_GAIN * mix * gate;
                io.output_ch(0, 1)[i] = r * (1.0 - mix) + yr * WET_GAIN * mix * gate;
            } else {
                let wet = 0.5 * (yl + yr) * WET_GAIN;
                io.output_ch(0, 0)[i] = l * (1.0 - mix) + wet * mix * gate;
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
            P_SIZE => self.size.set_target(real),
            P_DAMP => self.damp.set_target(real),
            P_MIX => self.mix.set_target(real),
            P_PREDELAY => self
                .predelay_len
                .set_target(real * 0.001 * self.sample_rate),
            P_GATE => self.gate_level = real,
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let scale = sample_rate / REF_SR;
        let len = |t: usize| ((t as f32 * scale).round() as usize).max(1);

        self.predelay_line
            .prepare((PREDELAY_MAX_MS * 0.001 * sample_rate).ceil() as usize + 2);
        self.bw_store = 0.0;
        for (ap, &(l, g)) in self.input_aps.iter_mut().zip(&INPUT_APS) {
            ap.prepare(len(l), g);
        }
        // Phases 0.0 / 0.5: decorrelated from the first sample, deterministic.
        self.a
            .prepare(&TANK_A, scale, 0.0, LFO_RATES_HZ[0] / sample_rate);
        self.b
            .prepare(&TANK_B, scale, 0.5, LFO_RATES_HZ[1] / sample_rate);
        for i in 0..7 {
            self.taps_l[i] = len(TAPS_L[i]);
            self.taps_r[i] = len(TAPS_R[i]);
        }

        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.size = OnePole::new(default_real(P_SIZE), coef);
        self.damp = OnePole::new(default_real(P_DAMP), coef);
        self.mix = LinearRamp::new(default_real(P_MIX), RAMP_MS, sample_rate);
        let pd_coef = (-1.0 / (PREDELAY_SMOOTH_S * sample_rate)).exp();
        self.predelay_len = OnePole::new(default_real(P_PREDELAY) * 0.001 * sample_rate, pd_coef);

        self.gate_level = default_real(P_GATE);
        self.gate_env = EnvFollower::default();
        self.gate_env
            .set_times(GATE_DET_ATTACK_S, GATE_DET_RELEASE_S, sample_rate);
        self.gate_hold = 0;
        self.gate_hold_samples = (GATE_HOLD_S * sample_rate) as u32;
        self.gate_gain = 1.0;
        self.gate_open_coef = (-1.0 / (GATE_OPEN_S * sample_rate)).exp();
        self.gate_close_coef = (-1.0 / (GATE_CLOSE_S * sample_rate)).exp();
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
    use crate::analysis::envelope::decay_rt60;
    use crate::analysis::fft::power_spectrum;
    use crate::analysis::levels::detect_clicks;
    use crate::analysis::spectral::spectral_flatness_db;

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
            channels: 1,
            samples_per_beat: 0.0,
            beat_phase: 0.0,
        };
        let mut io = NodeIo::new(&[], input, out, frames, 1);
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

    /// A 10 ms full-scale burst followed by silence (a snare-like hit).
    fn burst_input(secs: f32) -> Vec<f32> {
        let n = (secs * SR) as usize;
        let mut input = vec![0.0; n];
        let mut rng = crate::engine::dsp::RNG_SEED;
        for s in input.iter_mut().take(480) {
            *s = crate::engine::dsp::next_white(&mut rng) * 0.9;
        }
        input
    }

    #[test]
    fn gate_zero_is_bit_identical() {
        let input = burst_input(1.0);
        let mut a = fx();
        let mut ya = Vec::new();
        render(&mut a, &input, &mut ya);
        let mut b = fx();
        b.set_param(P_GATE, 0.0);
        let mut yb = Vec::new();
        render(&mut b, &input, &mut yb);
        assert_eq!(ya, yb, "gate 0 must leave the wet untouched");
    }

    #[test]
    fn gate_chops_the_tail() {
        let input = burst_input(1.2);
        let tail = |r: &mut Reverb| -> f32 {
            r.set_param(P_MIX, 1.0);
            r.set_param(P_SIZE, 0.9); // long tail: plenty to chop
            let mut y = Vec::new();
            render(r, &input, &mut y);
            energy(&y[(0.5 * SR) as usize..(1.0 * SR) as usize])
        };
        let open = tail(&mut fx());
        let mut gated_rv = fx();
        gated_rv.set_param(P_GATE, 0.2);
        let gated = tail(&mut gated_rv);
        assert!(
            gated < open * 0.05,
            "gate left the tail ringing: {gated} vs {open}"
        );
    }

    #[test]
    fn gate_close_is_click_free() {
        let mut r = fx();
        r.set_param(P_MIX, 1.0);
        r.set_param(P_GATE, 0.2);
        let input = burst_input(1.0);
        let mut y = Vec::new();
        render(&mut r, &input, &mut y);
        let clicks = detect_clicks(&y, 48_000);
        assert!(clicks.is_empty(), "gate close clicked: {clicks:?}");
    }

    #[test]
    fn stereo_wet_decorrelates_and_folds_to_mono() {
        // Same dual-mono impulse through a mono and a stereo run: yl/yr must
        // differ (the L/R tap tables finally part ways) while their fold
        // reproduces the legacy mono wet to f32 rounding.
        let secs = 2.0;
        let n = (secs * SR) as usize;
        let mut input = vec![0.0; n];
        input[0] = 1.0;

        let mut mono_fx = fx();
        mono_fx.set_param(P_MIX, 1.0);
        let mut mono_out = Vec::new();
        render(&mut mono_fx, &input, &mut mono_out);

        let mut st = fx();
        st.set_param(P_MIX, 1.0);
        let mut stereo_in = input.clone();
        stereo_in.extend_from_slice(&input);
        let mut stereo_out = vec![0.0f32; n * 2];
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames: n,
            channels: 2,
            samples_per_beat: 0.0,
            beat_phase: 0.0,
        };
        let mut io = NodeIo::new(&[], &stereo_in, &mut stereo_out, n, 2);
        st.process(&ctx, &mut io);

        let (l, r) = stereo_out.split_at(n);
        assert!(
            l.iter().zip(r).any(|(a, b)| (a - b).abs() > 1.0e-4),
            "stereo wet did not decorrelate"
        );
        let mut max_err = 0.0f32;
        for i in 0..n {
            max_err = max_err.max((0.5 * (l[i] + r[i]) - mono_out[i]).abs());
        }
        assert!(
            max_err < 1.0e-6,
            "stereo fold != legacy mono wet: {max_err}"
        );

        use crate::analysis::levels::stereo_correlation;
        let corr = stereo_correlation(l, r);
        assert!(corr < 0.999, "wet correlation still {corr}");
    }

    #[test]
    fn impulse_response_decays_to_silence() {
        // Default size (decay 0.575, RT60 ≈ 2 s): finite tail, silent by 3.5 s.
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

    #[test]
    fn predelay_shifts_the_onset() {
        // 10τ of silence lets the predelay glide settle before the impulse.
        let settle = 24_000usize;
        let onset = |predelay_norm: f32| -> usize {
            let mut r = fx();
            r.set_param(P_MIX, 1.0);
            r.set_param(P_PREDELAY, predelay_norm);
            let mut input = vec![0.0; settle + 48_000];
            input[settle] = 1.0;
            let mut out = Vec::new();
            render(&mut r, &input, &mut out);
            out[settle..]
                .iter()
                .position(|s| s.abs() > 1.0e-6)
                .expect("no output at all")
        };
        let base = onset(PARAMS[P_PREDELAY as usize].normalize(0.0));
        let shifted = onset(PARAMS[P_PREDELAY as usize].normalize(50.0));
        let diff = shifted as i64 - base as i64;
        assert!(
            (diff - 2_400).abs() <= 2,
            "predelay 50 ms shifted the onset by {diff} samples (expected 2400)"
        );
    }

    #[test]
    fn late_tail_is_spectrally_flat() {
        // The anti-metallic regression: a diffuse plate tail has no dominant
        // resonances. The old Freeverb topology measured far lower (comby).
        let mut r = fx();
        r.set_param(P_MIX, 1.0);
        r.set_param(P_SIZE, 0.8);
        let out = impulse_response(&mut r, 2.0);
        let slice = &out[48_000..72_000]; // 1.0–1.5 s into the tail
        let power = power_spectrum(slice).expect("spectrum");
        let flatness = spectral_flatness_db(&power, slice.len().next_power_of_two(), 48_000);
        assert!(
            flatness > -45.0,
            "tail is resonant/metallic: flatness {flatness} dB"
        );
    }

    #[test]
    fn param_sweep_produces_no_clicks() {
        // Sweep every knob while processing: smoothing must keep it click-free.
        let mut r = fx();
        r.set_param(P_MIX, 0.5);
        let total = 3 * 48_000usize;
        let block = 256usize;
        let mut out = Vec::with_capacity(total);
        let mut buf = Vec::new();
        let blocks = total / block;
        for i in 0..blocks {
            let t = i as f32 / blocks as f32;
            // size 0→1→0, damp 1→0, predelay 0→80 ms.
            let tri = 1.0 - (2.0 * t - 1.0).abs();
            r.set_param(P_SIZE, tri);
            r.set_param(P_DAMP, 1.0 - t);
            r.set_param(P_PREDELAY, PARAMS[P_PREDELAY as usize].normalize(80.0 * t));
            let input: Vec<f32> = (0..block)
                .map(|j| {
                    let n = i * block + j;
                    (std::f32::consts::TAU * 440.0 * n as f32 / SR).sin() * 0.5
                })
                .collect();
            render(&mut r, &input, &mut buf);
            out.extend_from_slice(&buf);
        }
        let clicks = detect_clicks(&out[4_800..], 48_000);
        assert!(clicks.is_empty(), "param sweep clicked: {clicks:?}");
    }

    #[test]
    fn rt60_grows_monotonically_with_size() {
        let rt = |size: f32| -> f32 {
            let mut r = fx();
            r.set_param(P_MIX, 1.0);
            r.set_param(P_SIZE, size);
            let out = impulse_response(&mut r, 6.0);
            decay_rt60(&out, 48_000, 0.0)
                .expect("measurable decay")
                .rt60_s
        };
        let (a, b, c) = (rt(0.2), rt(0.5), rt(0.9));
        assert!(
            a < b && b < c,
            "rt60 not monotonic in size: {a} / {b} / {c}"
        );
    }
}
