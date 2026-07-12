//! Tonal features: harmonic content, brightness, noisiness.

use serde::Serialize;

use super::fft::bin_hz;

/// Single-bin DFT magnitude at `freq`, scaled so a full-scale sine reads ~1.0.
/// (Promoted from the ad-hoc test helper in `modules/fm.rs`.)
pub fn goertzel(x: &[f32], sample_rate: f32, freq: f32) -> f32 {
    if x.is_empty() || sample_rate <= 0.0 {
        return 0.0;
    }
    let w = std::f32::consts::TAU * freq / sample_rate;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (n, &s) in x.iter().enumerate() {
        let phase = w * n as f32;
        re += (s * phase.cos()) as f64;
        im += (s * phase.sin()) as f64;
    }
    2.0 * ((re * re + im * im).sqrt() as f32) / x.len() as f32
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Harmonic {
    /// Harmonic number (1 = fundamental).
    pub k: u32,
    /// Frequency where the partial was actually found.
    pub freq_hz: f32,
    /// Magnitude relative to the fundamental, dB.
    pub db_rel_f1: f32,
    /// Deviation from the exact `k·f0` grid, cents. Large values = stretched
    /// or bell-like partials.
    pub cents_dev: f32,
}

/// Measure partials k = 1..=16 (below 0.45·fs). Each partial is peak-searched
/// within ±3% of `k·f0` (coarse Goertzel scan, then a fine pass around the
/// best coarse hit — one pass alone quantizes deviations to ~13 cents),
/// and its deviation from the harmonic grid is reported.
pub fn harmonics(x: &[f32], sample_rate: u32, f0: f32) -> Vec<Harmonic> {
    if f0 <= 0.0 || x.is_empty() {
        return Vec::new();
    }
    let sr = sample_rate as f32;
    let scan = |centre: f32, span: f32, steps: i32| -> (f32, f32) {
        let (mut best_freq, mut best_mag) = (centre, 0.0f32);
        for step in -steps..=steps {
            let freq = centre * (1.0 + span * step as f32 / steps as f32);
            let mag = goertzel(x, sr, freq);
            if mag > best_mag {
                best_mag = mag;
                best_freq = freq;
            }
        }
        (best_freq, best_mag)
    };
    let mut out = Vec::new();
    let mut fundamental_mag = 0.0f32;
    for k in 1..=16u32 {
        let target = k as f32 * f0;
        if target >= 0.45 * sr {
            break;
        }
        // Coarse ±3% (13-cent grid), then fine ±0.375% (1.6-cent grid).
        let (coarse_freq, _) = scan(target, 0.03, 4);
        let (best_freq, best_mag) = scan(coarse_freq, 0.00375, 4);
        if k == 1 {
            fundamental_mag = best_mag;
            if fundamental_mag <= 0.0 {
                return Vec::new();
            }
        }
        out.push(Harmonic {
            k,
            freq_hz: best_freq,
            db_rel_f1: 20.0 * (best_mag / fundamental_mag).max(1e-9).log10(),
            cents_dev: 1200.0 * (best_freq / target).log2(),
        });
    }
    out
}

/// Energy-weighted mean |cents deviation| of partials k ≥ 2.
pub fn inharmonicity_cents(harmonics: &[Harmonic]) -> f32 {
    let mut weighted = 0.0f32;
    let mut weight_sum = 0.0f32;
    for h in harmonics.iter().filter(|h| h.k >= 2) {
        let w = 10.0f32.powf(h.db_rel_f1 / 20.0);
        weighted += w * h.cents_dev.abs();
        weight_sum += w;
    }
    if weight_sum > 0.0 {
        weighted / weight_sum
    } else {
        0.0
    }
}

/// Power-weighted mean frequency — the classic brightness measure.
pub fn spectral_centroid_hz(power: &[f32], fft_len: usize, sample_rate: u32) -> f32 {
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for (i, &p) in power.iter().enumerate() {
        num += (bin_hz(i, fft_len, sample_rate) * p) as f64;
        den += p as f64;
    }
    if den > 0.0 {
        (num / den) as f32
    } else {
        0.0
    }
}

/// Spectral flatness (geometric / arithmetic mean of power) over
/// 50 Hz..Nyquist, in dB. 0 dB = white noise, very negative = pure tone.
pub fn spectral_flatness_db(power: &[f32], fft_len: usize, sample_rate: u32) -> f32 {
    let first = power
        .iter()
        .enumerate()
        .position(|(i, _)| bin_hz(i, fft_len, sample_rate) >= 50.0)
        .unwrap_or(0);
    let band = &power[first..];
    if band.is_empty() {
        return 0.0;
    }
    let n = band.len() as f64;
    let mut log_sum = 0.0f64;
    let mut sum = 0.0f64;
    for &p in band {
        let p = (p as f64).max(1e-20);
        log_sum += p.ln();
        sum += p;
    }
    let geometric = (log_sum / n).exp();
    let arithmetic = sum / n;
    (10.0 * (geometric / arithmetic).log10()) as f32
}

/// Fraction of total band power (50 Hz..Nyquist) lying within ±3 bins of the
/// measured partials (the Hann-windowed peak spreads over a few bins).
/// 1.0 = fully tonal, low = noisy.
pub fn harmonic_ratio(
    power: &[f32],
    fft_len: usize,
    sample_rate: u32,
    harmonics: &[Harmonic],
) -> f32 {
    let first = power
        .iter()
        .enumerate()
        .position(|(i, _)| bin_hz(i, fft_len, sample_rate) >= 50.0)
        .unwrap_or(0);
    let total: f64 = power[first..].iter().map(|&p| p as f64).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let mut tonal = 0.0f64;
    for h in harmonics {
        let bin = (h.freq_hz * fft_len as f32 / sample_rate as f32).round() as usize;
        let lo = bin.saturating_sub(3).max(first);
        let hi = (bin + 4).min(power.len());
        if lo < hi {
            tonal += power[lo..hi].iter().map(|&p| p as f64).sum::<f64>();
        }
    }
    (tonal / total).min(1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::fft::power_spectrum;

    fn tone(freqs: &[(f32, f32)], sr: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                freqs
                    .iter()
                    .map(|&(f, a)| a * (std::f32::consts::TAU * f * t).sin())
                    .sum()
            })
            .collect()
    }

    #[test]
    fn goertzel_reads_amplitude() {
        let x = tone(&[(1000.0, 0.5)], 48_000, 9600);
        assert!((goertzel(&x, 48_000.0, 1000.0) - 0.5).abs() < 0.01);
        assert!(goertzel(&x, 48_000.0, 2000.0) < 0.01);
    }

    #[test]
    fn odd_harmonics_dominate_square_like_tone() {
        // Odd partials at 1/k (square-wave law): H3 must tower over H2/H4.
        let f0 = 200.0;
        let x = tone(
            &[(f0, 1.0), (3.0 * f0, 1.0 / 3.0), (5.0 * f0, 0.2)],
            48_000,
            48_000,
        );
        let hs = harmonics(&x, 48_000, f0);
        let h = |k: u32| hs.iter().find(|h| h.k == k).map(|h| h.db_rel_f1);
        assert!(h(3).unwrap() > -12.0, "H3 {:?}", h(3));
        assert!(h(2).unwrap() < -30.0, "H2 {:?}", h(2));
        assert!(h(4).unwrap() < -30.0, "H4 {:?}", h(4));
        assert!(inharmonicity_cents(&hs) < 5.0);

        let power = power_spectrum(&x).unwrap();
        let fft_len = x.len().next_power_of_two();
        let ratio = harmonic_ratio(&power, fft_len, 48_000, &hs);
        assert!(ratio > 0.9, "harmonic ratio {ratio}");
    }

    #[test]
    fn centroid_tracks_brightness() {
        let dark = tone(&[(200.0, 1.0)], 48_000, 16_384);
        let bright = tone(&[(200.0, 0.3), (4000.0, 1.0)], 48_000, 16_384);
        let fft_len = 16_384;
        let c_dark = spectral_centroid_hz(&power_spectrum(&dark).unwrap(), fft_len, 48_000);
        let c_bright = spectral_centroid_hz(&power_spectrum(&bright).unwrap(), fft_len, 48_000);
        assert!(c_dark < 400.0, "dark centroid {c_dark}");
        assert!(c_bright > 2000.0, "bright centroid {c_bright}");
    }

    #[test]
    fn flatness_separates_noise_from_tone() {
        let sine = tone(&[(440.0, 1.0)], 48_000, 16_384);
        let mut state = 0x1234_5678u32;
        let noise: Vec<f32> = (0..16_384)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
            })
            .collect();
        let fft_len = 16_384;
        let f_sine = spectral_flatness_db(&power_spectrum(&sine).unwrap(), fft_len, 48_000);
        let f_noise = spectral_flatness_db(&power_spectrum(&noise).unwrap(), fft_len, 48_000);
        assert!(f_sine < -40.0, "sine flatness {f_sine}");
        assert!(f_noise > -15.0, "noise flatness {f_noise}");
    }
}
