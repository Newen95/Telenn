//! Spectral balance over fixed musical bands.
//!
//! Welch-averaged power spectra (Hann windows, half-overlap) summed into the
//! seven bands a mix engineer reasons in — sub, bass, low-mid, mid, high-mid,
//! presence, air — each reported in dB relative to the total energy, so two
//! files of different loudness still compare band-for-band.

use serde::Serialize;

use crate::analysis::fft::{bin_hz, power_spectrum};

/// Analysis window (samples). Long enough to resolve the sub band at 44.1/48k
/// (~5.4 Hz per bin), short enough to average many windows on a 20 s render.
const WINDOW: usize = 8192;

/// The band edges (Hz) and their mixing names. The last band runs to Nyquist.
const EDGES: [(f32, f32, &str); 7] = [
    (20.0, 60.0, "sub"),
    (60.0, 150.0, "bass"),
    (150.0, 400.0, "low-mid"),
    (400.0, 1_000.0, "mid"),
    (1_000.0, 2_500.0, "high-mid"),
    (2_500.0, 6_000.0, "presence"),
    (6_000.0, f32::INFINITY, "air"),
];

#[derive(Debug, Clone, Serialize)]
pub struct BandEnergy {
    pub name: &'static str,
    pub lo_hz: f32,
    /// Upper edge; the air band reports the Nyquist frequency.
    pub hi_hz: f32,
    /// Band energy in dB relative to the total (always ≤ 0).
    pub db_rel: f32,
}

/// Welch-average the signal's power spectrum and fold it into the seven
/// mixing bands, each in dB relative to the total energy in 20 Hz..Nyquist.
/// Returns `None` for silence or input shorter than one window.
pub fn band_balance(x: &[f32], sample_rate: u32) -> Option<Vec<BandEnergy>> {
    if x.len() < WINDOW || sample_rate == 0 {
        return None;
    }
    let hop = WINDOW / 2;
    let mut power = vec![0.0f64; WINDOW / 2 + 1];
    let mut windows = 0usize;
    let mut start = 0usize;
    while start + WINDOW <= x.len() {
        let spectrum = power_spectrum(&x[start..start + WINDOW]).ok()?;
        for (acc, p) in power.iter_mut().zip(&spectrum) {
            *acc += *p as f64;
        }
        windows += 1;
        start += hop;
    }
    if windows == 0 {
        return None;
    }

    let nyquist = sample_rate as f32 / 2.0;
    let fft_len = WINDOW.next_power_of_two();
    let mut bands = vec![0.0f64; EDGES.len()];
    let mut total = 0.0f64;
    for (bin, &p) in power.iter().enumerate() {
        let f = bin_hz(bin, fft_len, sample_rate);
        if f < EDGES[0].0 {
            continue;
        }
        total += p;
        if let Some(k) = EDGES.iter().position(|&(lo, hi, _)| f >= lo && f < hi) {
            bands[k] += p;
        }
    }
    if total <= 0.0 {
        return None;
    }

    Some(
        EDGES
            .iter()
            .zip(&bands)
            .map(|(&(lo, hi, name), &e)| BandEnergy {
                name,
                lo_hz: lo,
                hi_hz: if hi.is_finite() { hi } else { nyquist },
                db_rel: 10.0 * ((e / total).max(1e-12) as f32).log10(),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const SR: u32 = 48_000;

    fn sine(freq: f32, secs: f32) -> Vec<f32> {
        let n = (secs * SR as f32) as usize;
        (0..n)
            .map(|i| (TAU * freq * i as f32 / SR as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn a_100hz_sine_lands_in_the_bass_band() {
        let bands = band_balance(&sine(100.0, 2.0), SR).expect("bands");
        let bass = bands.iter().find(|b| b.name == "bass").unwrap();
        // ≥ 90 % of the energy → ≥ -0.46 dB relative to total.
        assert!(bass.db_rel > -0.5, "bass band holds {} dB", bass.db_rel);
        let air = bands.iter().find(|b| b.name == "air").unwrap();
        assert!(air.db_rel < -40.0, "air band leaked: {} dB", air.db_rel);
    }

    #[test]
    fn a_5khz_sine_lands_in_presence() {
        let bands = band_balance(&sine(5_000.0, 2.0), SR).expect("bands");
        let presence = bands.iter().find(|b| b.name == "presence").unwrap();
        assert!(
            presence.db_rel > -0.5,
            "presence holds {} dB",
            presence.db_rel
        );
    }

    #[test]
    fn silence_and_short_input_yield_none() {
        assert!(band_balance(&vec![0.0; SR as usize], SR).is_none());
        assert!(band_balance(&[0.1; 128], SR).is_none());
    }
}
