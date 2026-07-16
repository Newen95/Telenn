//! Rhythm features: onset density and tempo.
//!
//! Onset envelope = half-wave-rectified log-spectral flux (STFT 1024, hop
//! 256); onsets are picked as local maxima above an adaptive threshold, and
//! the tempo comes from the envelope's autocorrelation over 60–180 BPM with a
//! bonus for lags whose double also correlates (beat vs half-beat ambiguity)
//! and parabolic peak refinement. Port of the prototype validated against
//! measured reference tracks.

use serde::Serialize;

use crate::analysis::fft::{bin_hz, power_spectrum};

pub const WINDOW: usize = 1024;
pub const HOP: usize = 256;
const BPM_LO: f32 = 60.0;
const BPM_HI: f32 = 180.0;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Rhythm {
    /// Estimated tempo (BPM); `None` when no periodicity stands out.
    pub tempo_bpm: Option<f32>,
    /// Detected onsets per second.
    pub onset_density: f32,
}

/// Half-wave-rectified log-spectral flux per hop, restricted to bins whose
/// frequency falls in `[lo_hz, hi_hz)`. The full band (`0.0..f32::INFINITY`)
/// is the classic onset envelope; narrower bands isolate one hit family
/// (kick from the low band, hats from the high band). Empty for short input.
pub fn band_flux(x: &[f32], sample_rate: u32, lo_hz: f32, hi_hz: f32) -> Vec<f32> {
    if x.len() < WINDOW * 2 {
        return Vec::new();
    }
    // Bin frequencies grow monotonically, so the band is one contiguous range.
    let n_bins = WINDOW / 2 + 1;
    let lo = (0..n_bins)
        .find(|&k| bin_hz(k, WINDOW, sample_rate) >= lo_hz)
        .unwrap_or(n_bins);
    let hi = (lo..n_bins)
        .find(|&k| bin_hz(k, WINDOW, sample_rate) >= hi_hz)
        .unwrap_or(n_bins);
    // Power relative to a full-window sine at the signal peak (Hann coherent
    // gain N/4): the log compression then reads calibrated dB-like values
    // instead of saturating on absolute FFT magnitudes. Without this, a
    // -40 dB cross-band bleed logs almost as loud as the band's own hits.
    let peak = x.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    let reference = if peak > 0.0 {
        (peak * WINDOW as f32 / 4.0) * (peak * WINDOW as f32 / 4.0)
    } else {
        1.0
    };
    let frames = (x.len() - WINDOW) / HOP;
    let mut prev: Option<Vec<f32>> = None;
    let mut env = Vec::with_capacity(frames);
    for i in 0..frames {
        let Ok(power) = power_spectrum(&x[i * HOP..i * HOP + WINDOW]) else {
            return Vec::new();
        };
        let logmag: Vec<f32> = power[lo..hi]
            .iter()
            .map(|&p| (1.0 + 1000.0 * p / reference).ln())
            .collect();
        if let Some(prev) = &prev {
            let flux: f32 = logmag.iter().zip(prev).map(|(c, p)| (c - p).max(0.0)).sum();
            env.push(flux);
        }
        prev = Some(logmag);
    }
    env
}

/// Seconds of envelope index `i`: the flux at `i` compares STFT frames `i`
/// and `i + 1`, and a transient registers strongest when it sits near the
/// centre of the Hann window, hence `(i + 1) * HOP + WINDOW / 2`. Every
/// consumer must use this one convention so the constant offset cancels out
/// of grid-relative positions.
pub fn frame_time_s(i: usize, sample_rate: u32) -> f32 {
    ((i + 1) * HOP + WINDOW / 2) as f32 / sample_rate as f32
}

/// Local maxima above `mean + std` (adaptive floor) with a 50 ms refractory
/// period, so one hit is one onset. Returns envelope indices (see
/// [`frame_time_s`] for the time convention).
pub fn pick_onsets(env: &[f32], fps: f32) -> Vec<usize> {
    if env.len() < 3 {
        return Vec::new();
    }
    let mean = env.iter().sum::<f32>() / env.len() as f32;
    let var = env.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / env.len() as f32;
    let threshold = mean + var.sqrt();
    let refractory = (0.05 * fps) as usize;
    let mut onsets = Vec::new();
    for i in 1..env.len() - 1 {
        if env[i] > threshold && env[i] >= env[i - 1] && env[i] > env[i + 1] {
            if let Some(&prev) = onsets.last() {
                if i - prev < refractory.max(1) {
                    continue;
                }
            }
            onsets.push(i);
        }
    }
    onsets
}

/// Tempo from the onset envelope's autocorrelation over the BPM window.
pub fn tempo(env: &[f32], fps: f32) -> Option<f32> {
    let n = env.len();
    let mean = env.iter().sum::<f32>() / n.max(1) as f32;
    let centred: Vec<f32> = env.iter().map(|v| v - mean).collect();
    let ac = |lag: usize| -> f32 {
        centred[..n - lag]
            .iter()
            .zip(&centred[lag..])
            .map(|(a, b)| a * b)
            .sum()
    };
    let lag_lo = (fps * 60.0 / BPM_HI) as usize;
    let lag_hi = (fps * 60.0 / BPM_LO) as usize;
    if lag_lo == 0 || lag_hi + 1 >= n {
        return None;
    }
    let norm = ac(0);
    if norm <= 0.0 {
        return None;
    }
    // Score = normalized autocorrelation, plus half the double-lag value so
    // the beat wins over the half-beat.
    let score = |lag: usize| -> f32 {
        let mut s = ac(lag) / norm;
        if 2 * lag < n {
            s += 0.5 * ac(2 * lag) / norm;
        }
        s
    };
    let mut best_lag = 0usize;
    let mut best = f32::MIN;
    for lag in lag_lo..=lag_hi {
        let s = score(lag);
        if s > best {
            best = s;
            best_lag = lag;
        }
    }
    if best <= 0.05 {
        return None; // nothing periodic stands out (pads, silence)
    }
    // Parabolic refinement on the raw autocorrelation around the winner.
    let refined = if best_lag > lag_lo && best_lag < lag_hi {
        let (a, b, c) = (
            ac(best_lag - 1) / norm,
            ac(best_lag) / norm,
            ac(best_lag + 1) / norm,
        );
        let denom = a - 2.0 * b + c;
        if denom.abs() > 1e-9 {
            best_lag as f32 + 0.5 * (a - c) / denom
        } else {
            best_lag as f32
        }
    } else {
        best_lag as f32
    };
    Some(60.0 * fps / refined)
}

/// Rhythm features of a mono signal.
pub fn analyze(x: &[f32], sample_rate: u32) -> Rhythm {
    let none = Rhythm {
        tempo_bpm: None,
        onset_density: 0.0,
    };
    if sample_rate == 0 {
        return none;
    }
    let env = band_flux(x, sample_rate, 0.0, f32::INFINITY);
    if env.is_empty() {
        return none;
    }
    let fps = sample_rate as f32 / HOP as f32;
    let duration = env.len() as f32 / fps;
    Rhythm {
        tempo_bpm: tempo(&env, fps),
        onset_density: pick_onsets(&env, fps).len() as f32 / duration.max(1e-6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    /// A click track: short decaying bursts on a strict BPM grid.
    fn clicks(bpm: f32, secs: f32) -> Vec<f32> {
        let n = (secs * SR as f32) as usize;
        let period = (60.0 / bpm * SR as f32) as usize;
        let mut x = vec![0.0f32; n];
        let mut t = 0usize;
        while t < n {
            for k in 0..200.min(n - t) {
                // A 1 kHz burst with a fast decay: a broadband, clean onset.
                let ph = std::f32::consts::TAU * 1000.0 * k as f32 / SR as f32;
                x[t + k] += ph.sin() * 0.8 * (-(k as f32) / 60.0).exp();
            }
            t += period;
        }
        x
    }

    #[test]
    fn a_click_track_reads_its_tempo_and_density() {
        let out = analyze(&clicks(120.0, 10.0), SR);
        let bpm = out.tempo_bpm.expect("no tempo found");
        assert!((bpm - 120.0).abs() < 2.0, "tempo {bpm} vs 120");
        assert!(
            (out.onset_density - 2.0).abs() < 0.4,
            "density {} vs 2/s",
            out.onset_density
        );
    }

    #[test]
    fn a_slow_click_track_is_not_octave_confused() {
        let out = analyze(&clicks(75.0, 12.0), SR);
        let bpm = out.tempo_bpm.expect("no tempo found");
        assert!((bpm - 75.0).abs() < 2.0, "tempo {bpm} vs 75");
    }

    #[test]
    fn silence_has_no_tempo_and_no_onsets() {
        let out = analyze(&vec![0.0f32; SR as usize * 4], SR);
        assert!(out.tempo_bpm.is_none());
        assert_eq!(out.onset_density, 0.0);
    }

    #[test]
    fn onset_positions_land_on_the_click_grid() {
        let env = band_flux(&clicks(120.0, 10.0), SR, 0.0, f32::INFINITY);
        let fps = SR as f32 / HOP as f32;
        let onsets = pick_onsets(&env, fps);
        assert!(onsets.len() >= 18, "only {} onsets", onsets.len());
        let period = fps * 0.5; // 120 BPM = one click every 0.5 s
        for pair in onsets.windows(2) {
            let d = (pair[1] - pair[0]) as f32;
            assert!((d - period).abs() <= 1.5, "spacing {d} vs {period}");
        }
    }

    /// Bursts of a tone or noise on a grid; `f0 = 0` means an LCG noise burst.
    fn bursts(f0: f32, secs: f32) -> Vec<f32> {
        let n = (secs * SR as f32) as usize;
        let period = SR as usize / 2;
        let len = SR as usize / 10; // 100 ms bursts
        let mut state = 0x2545_f491u32;
        let mut x = vec![0.0f32; n];
        let mut t = 0usize;
        while t < n {
            for k in 0..len.min(n - t) {
                let decay = (-(k as f32) / (len as f32 / 4.0)).exp();
                x[t + k] = if f0 > 0.0 {
                    (std::f32::consts::TAU * f0 * k as f32 / SR as f32).sin() * 0.8 * decay
                } else {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    ((state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0) * 0.8 * decay
                };
            }
            t += period;
        }
        x
    }

    #[test]
    fn band_flux_separates_low_and_high_content() {
        let low_hits = bursts(60.0, 4.0);
        let lo = band_flux(&low_hits, SR, 0.0, 150.0);
        let hi = band_flux(&low_hits, SR, 5000.0, 16000.0);
        let max = |v: &[f32]| v.iter().fold(0.0f32, |m, &x| m.max(x));
        assert!(
            max(&lo) > 5.0 * max(&hi),
            "60 Hz bursts: low {} vs high {}",
            max(&lo),
            max(&hi)
        );

        let noise_hits = bursts(0.0, 4.0);
        let lo = band_flux(&noise_hits, SR, 0.0, 150.0);
        let hi = band_flux(&noise_hits, SR, 5000.0, 16000.0);
        assert!(
            max(&hi) > 5.0 * max(&lo),
            "noise bursts: low {} vs high {}",
            max(&lo),
            max(&hi)
        );
    }

    #[test]
    fn full_band_flux_matches_the_analyze_envelope() {
        // The public full-band call is exactly what `analyze` consumes: same
        // clicks, same density, whatever the internal wiring becomes.
        let x = clicks(120.0, 10.0);
        let env = band_flux(&x, SR, 0.0, f32::INFINITY);
        let fps = SR as f32 / HOP as f32;
        let duration = env.len() as f32 / fps;
        let density = pick_onsets(&env, fps).len() as f32 / duration;
        let out = analyze(&x, SR);
        assert!((density - out.onset_density).abs() < 1e-6);
    }
}
