//! Fundamental frequency estimation — YIN-lite (cumulative-mean-normalized
//! difference function with parabolic refinement).

use serde::Serialize;

/// Analysis window, in samples (85 ms at 48 kHz — enough for 30 Hz).
const WINDOW: usize = 4096;
/// CMNDF dip threshold: below this the lag is accepted as a period.
const THRESHOLD: f32 = 0.15;
/// Lowest detectable fundamental, Hz.
const F0_MIN: f32 = 30.0;
/// Highest detectable fundamental, Hz.
const F0_MAX: f32 = 4000.0;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct F0 {
    pub hz: f32,
    /// `1 − CMNDF(τ)` at the accepted dip: near 1 for a clean pitch, low for
    /// noisy or ambiguous material.
    pub clarity: f32,
}

/// Estimate the fundamental of (a slice of) a steady tone. Returns `None`
/// when no periodic dip is found — expected for drums and noise, not an
/// error. Pass the sustain segment, not the attack.
pub fn estimate_f0(x: &[f32], sample_rate: u32) -> Option<F0> {
    estimate_f0_in(x, sample_rate, F0_MIN, F0_MAX)
}

/// [`estimate_f0`] with an explicit search range. A narrow range (e.g.
/// 30–200 Hz for a bass line) keeps the first-dip rule from locking onto a
/// harmonic of another voice.
pub fn estimate_f0_in(x: &[f32], sample_rate: u32, f_min: f32, f_max: f32) -> Option<F0> {
    if sample_rate == 0 || f_min <= 0.0 || f_max <= f_min {
        return None;
    }
    let window = WINDOW.min(x.len());
    let tau_max = ((sample_rate as f32 / f_min) as usize).min(window / 2);
    let tau_min = ((sample_rate as f32 / f_max) as usize).max(2);
    if tau_max <= tau_min + 2 {
        return None; // slice too short to hold a full period
    }
    let x = &x[..window];

    // Difference function d(τ), then cumulative-mean-normalized d'(τ).
    let mut diff = vec![0.0f32; tau_max + 1];
    for (tau, d) in diff.iter_mut().enumerate().skip(1) {
        let mut acc = 0.0f64;
        for n in 0..window - tau {
            let delta = x[n] - x[n + tau];
            acc += (delta * delta) as f64;
        }
        *d = acc as f32;
    }
    let mut cmndf = vec![1.0f32; tau_max + 1];
    let mut running = 0.0f64;
    for tau in 1..=tau_max {
        running += diff[tau] as f64;
        if running > 0.0 {
            cmndf[tau] = ((diff[tau] as f64) * tau as f64 / running) as f32;
        }
    }

    // First dip under the threshold, walked down to its local minimum.
    let mut tau = (tau_min..=tau_max).find(|&t| cmndf[t] < THRESHOLD)?;
    while tau < tau_max && cmndf[tau + 1] < cmndf[tau] {
        tau += 1;
    }

    // Parabolic interpolation around the minimum for sub-sample precision.
    let refined = if tau > tau_min && tau < tau_max {
        let (a, b, c) = (cmndf[tau - 1], cmndf[tau], cmndf[tau + 1]);
        let denom = a - 2.0 * b + c;
        if denom.abs() > 1e-12 {
            tau as f32 + 0.5 * (a - c) / denom
        } else {
            tau as f32
        }
    } else {
        tau as f32
    };

    Some(F0 {
        hz: sample_rate as f32 / refined,
        clarity: (1.0 - cmndf[tau]).clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_tone_with_harmonic() {
        // 440 Hz + 20% third harmonic: f0 must stay at 440, not jump to 1320.
        let sr = 48_000u32;
        let x: Vec<f32> = (0..8192)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (std::f32::consts::TAU * 440.0 * t).sin()
                    + 0.2 * (std::f32::consts::TAU * 1320.0 * t).sin()
            })
            .collect();
        let f0 = estimate_f0(&x, sr).unwrap();
        assert!((f0.hz - 440.0).abs() < 0.5, "f0 {}", f0.hz);
        assert!(f0.clarity > 0.9, "clarity {}", f0.clarity);
    }

    #[test]
    fn low_fundamental() {
        let sr = 48_000u32;
        let x: Vec<f32> = (0..8192)
            .map(|i| (std::f32::consts::TAU * 55.0 * i as f32 / sr as f32).sin())
            .collect();
        let f0 = estimate_f0(&x, sr).unwrap();
        assert!((f0.hz - 55.0).abs() < 0.5, "f0 {}", f0.hz);
    }

    #[test]
    fn bass_range_resists_a_strong_second_harmonic() {
        // 55 Hz with a second harmonic almost as loud: the bass search range
        // must still report the fundamental, not the octave.
        let sr = 48_000u32;
        let x: Vec<f32> = (0..8192)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (std::f32::consts::TAU * 55.0 * t).sin()
                    + 0.9 * (std::f32::consts::TAU * 110.0 * t).sin()
            })
            .collect();
        let f0 = estimate_f0_in(&x, sr, 30.0, 200.0).unwrap();
        assert!((f0.hz - 55.0).abs() < 1.0, "f0 {}", f0.hz);
    }

    #[test]
    fn degenerate_ranges_yield_none() {
        let sr = 48_000u32;
        let x: Vec<f32> = (0..8192)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        assert!(estimate_f0_in(&x, sr, 0.0, 200.0).is_none());
        assert!(estimate_f0_in(&x, sr, 200.0, 100.0).is_none());
    }

    #[test]
    fn noise_yields_none() {
        // Deterministic LCG white noise: no periodic dip below the threshold.
        let mut state = 0x2545_f491u32;
        let x: Vec<f32> = (0..8192)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
            })
            .collect();
        assert!(estimate_f0(&x, 48_000).is_none());
    }
}
