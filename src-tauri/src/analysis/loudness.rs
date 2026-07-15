//! Integrated loudness per ITU-R BS.1770-4.
//!
//! K-weighting (a high shelf modelling the head, then the RLB high-pass) with
//! coefficients DESIGNED AT THE FILE'S SAMPLE RATE from the analog prototypes
//! — the spec's published tables are 48 kHz only, and references are often
//! 44.1 kHz. Energy is measured over 400 ms blocks with 75 % overlap, then
//! gated: an absolute -70 LUFS gate, and a relative gate 10 LU under the
//! ungated mean. Analysis-side twin of the spec, off the audio thread.

use serde::Serialize;

/// Gating block length and hop (seconds): 400 ms with 75 % overlap.
const BLOCK_S: f64 = 0.400;
const HOP_S: f64 = 0.100;
/// The spec's channel-sum offset.
const OFFSET_LU: f64 = -0.691;
/// Absolute gate (LUFS).
const ABS_GATE: f64 = -70.0;
/// Relative gate (LU under the ungated mean).
const REL_GATE: f64 = -10.0;

/// Direct-form-II-transposed biquad, local to the analysis side (the
/// `engine` DSP is deliberately not imported here).
#[derive(Default, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Stage 1: the spectral-weighting high shelf (+~4 dB above ~1.68 kHz),
/// designed from the BS.1770 analog prototype at `fs`.
fn shelf(fs: f64) -> Biquad {
    let f0 = 1_681.974_450_955_533;
    let gain_db = 3.999_843_853_973_347;
    let q = 0.707_175_236_955_419_3;

    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10.0f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_155);
    let a0 = 1.0 + k / q + k * k;
    Biquad {
        b0: (vh + vb * k / q + k * k) / a0,
        b1: 2.0 * (k * k - vh) / a0,
        b2: (vh - vb * k / q + k * k) / a0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
        ..Default::default()
    }
}

/// Stage 2: the RLB revised low-frequency B-weighting high-pass.
fn highpass(fs: f64) -> Biquad {
    let f0 = 38.135_470_876_139_82;
    let q = 0.500_327_037_325_395_3;

    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    Biquad {
        // The spec keeps the numerator un-normalized (1, -2, 1) — reproduced
        // so a 48 kHz design matches its published table exactly.
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
        ..Default::default()
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Loudness {
    /// Gated integrated loudness (LUFS). `None` when everything gated out.
    pub integrated_lufs: Option<f32>,
    /// Loudest 3 s short-term value (LUFS), ungated — the "peak loudness".
    pub short_term_max_lufs: Option<f32>,
}

/// Mean square of one K-weighted channel over `[start, start+len)`.
fn block_power(x: &[f64], start: usize, len: usize) -> f64 {
    let end = (start + len).min(x.len());
    if end <= start {
        return 0.0;
    }
    x[start..end].iter().map(|s| s * s).sum::<f64>() / (end - start) as f64
}

/// Integrated (gated) and max short-term loudness of a stereo pair.
pub fn measure(left: &[f32], right: &[f32], sample_rate: u32) -> Loudness {
    let none = Loudness {
        integrated_lufs: None,
        short_term_max_lufs: None,
    };
    if sample_rate == 0 || left.is_empty() {
        return none;
    }
    let fs = sample_rate as f64;

    // K-weight both channels once.
    let weighted = |x: &[f32]| -> Vec<f64> {
        let mut s1 = shelf(fs);
        let mut s2 = highpass(fs);
        x.iter()
            .map(|&v| s2.process(s1.process(v as f64)))
            .collect()
    };
    let l = weighted(left);
    let r = weighted(right);

    let block = (BLOCK_S * fs) as usize;
    let hop = (HOP_S * fs) as usize;
    if block == 0 || hop == 0 || l.len() < block {
        return none;
    }

    // Per-block loudness (both channels at unit weight, per the spec).
    let mut blocks: Vec<f64> = Vec::new();
    let mut start = 0usize;
    while start + block <= l.len() {
        let power = block_power(&l, start, block) + block_power(&r, start, block);
        blocks.push(OFFSET_LU + 10.0 * power.max(1e-15).log10());
        start += hop;
    }

    // Two-stage gate: absolute, then relative to the mean of what survived.
    let gated_mean = |min_lufs: f64| -> Option<f64> {
        let survivors: Vec<f64> = blocks.iter().copied().filter(|&b| b > min_lufs).collect();
        if survivors.is_empty() {
            return None;
        }
        let mean_power = survivors
            .iter()
            .map(|b| 10.0f64.powf((b - OFFSET_LU) / 10.0))
            .sum::<f64>()
            / survivors.len() as f64;
        Some(OFFSET_LU + 10.0 * mean_power.log10())
    };
    let integrated = gated_mean(ABS_GATE)
        .and_then(|ungated| gated_mean((ungated + REL_GATE).max(ABS_GATE)))
        .map(|v| v as f32);

    // Short-term max: 3 s windows on the same hop, ungated.
    let st_block = (3.0 * fs) as usize;
    let mut st_max: Option<f64> = None;
    let mut start = 0usize;
    while start + st_block <= l.len() {
        let power = block_power(&l, start, st_block) + block_power(&r, start, st_block);
        let v = OFFSET_LU + 10.0 * power.max(1e-15).log10();
        st_max = Some(st_max.map_or(v, |m: f64| m.max(v)));
        start += hop;
    }

    Loudness {
        integrated_lufs: integrated,
        short_term_max_lufs: st_max.map(|v| v as f32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(freq: f32, amp: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (secs * sr as f32) as usize;
        (0..n)
            .map(|i| amp * (TAU * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn a_997hz_sine_reads_its_dbfs_as_lufs() {
        // BS.1770 calibration: a 997 Hz full-scale sine on BOTH channels reads
        // ~-3.01 LUFS; at -18 dBFS per channel it reads ~-18 - 3.01 + 3.01 =
        // -18 LUFS... more simply, a MONO -18 dBFS sine duplicated on L and R
        // measures ≈ -15 LUFS (two unit-weight channels) and the same sine on
        // one channel with silence on the other ≈ -18 LUFS. Assert the
        // dual-channel case within ±0.5 LU.
        let sr = 48_000;
        let amp = 10.0f32.powf(-18.0 / 20.0);
        let x = sine(997.0, amp, 5.0, sr);
        let out = measure(&x, &x, sr);
        let lufs = out.integrated_lufs.expect("gated out");
        let expected = -18.0 + 10.0 * (2.0f32).log10() - 3.01; // ≈ -17.99
        assert!(
            (lufs - expected).abs() < 0.5,
            "got {lufs} LUFS, expected ≈ {expected}"
        );
    }

    #[test]
    fn design_is_sample_rate_adaptive() {
        // The same tone at 44.1 k and 48 k must read within 0.2 LU.
        let amp = 10.0f32.powf(-20.0 / 20.0);
        let a = measure(
            &sine(997.0, amp, 5.0, 48_000),
            &sine(997.0, amp, 5.0, 48_000),
            48_000,
        );
        let b = measure(
            &sine(997.0, amp, 5.0, 44_100),
            &sine(997.0, amp, 5.0, 44_100),
            44_100,
        );
        let (a, b) = (a.integrated_lufs.unwrap(), b.integrated_lufs.unwrap());
        assert!((a - b).abs() < 0.2, "48k {a} vs 44.1k {b}");
    }

    #[test]
    fn silence_gates_out_without_nan() {
        let x = vec![0.0f32; 48_000 * 2];
        let out = measure(&x, &x, 48_000);
        assert!(out.integrated_lufs.is_none());
    }

    #[test]
    fn low_frequencies_are_attenuated_by_the_k_curve() {
        // The RLB roll-off puts 50 Hz ≈ 4.6 LU under 997 Hz at equal
        // amplitude (the spec's curve, not a brickwall) — assert the
        // attenuation sits in that region.
        let sr = 48_000;
        let amp = 0.25f32;
        let hi = measure(&sine(997.0, amp, 5.0, sr), &sine(997.0, amp, 5.0, sr), sr);
        let lo = measure(&sine(50.0, amp, 5.0, sr), &sine(50.0, amp, 5.0, sr), sr);
        let (hi, lo) = (hi.integrated_lufs.unwrap(), lo.integrated_lufs.unwrap());
        let att = hi - lo;
        assert!(
            (3.0..7.0).contains(&att),
            "K-curve off: 997 Hz {hi}, 50 Hz {lo} (att {att})"
        );
    }
}
