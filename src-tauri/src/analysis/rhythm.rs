//! Rhythm features: onset density and tempo.
//!
//! Onset envelope = half-wave-rectified log-spectral flux (STFT 1024, hop
//! 256); onsets are picked as local maxima above an adaptive threshold, and
//! the tempo comes from the envelope's autocorrelation over 60–180 BPM with a
//! bonus for lags whose double also correlates (beat vs half-beat ambiguity)
//! and parabolic peak refinement. Port of the prototype validated against
//! measured reference tracks.

use serde::Serialize;

use crate::analysis::fft::power_spectrum;

const WINDOW: usize = 1024;
const HOP: usize = 256;
const BPM_LO: f32 = 60.0;
const BPM_HI: f32 = 180.0;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Rhythm {
    /// Estimated tempo (BPM); `None` when no periodicity stands out.
    pub tempo_bpm: Option<f32>,
    /// Detected onsets per second.
    pub onset_density: f32,
}

/// Half-wave-rectified log-spectral flux per hop. Empty for short input.
fn onset_envelope(x: &[f32]) -> Vec<f32> {
    if x.len() < WINDOW * 2 {
        return Vec::new();
    }
    let frames = (x.len() - WINDOW) / HOP;
    let mut prev: Option<Vec<f32>> = None;
    let mut env = Vec::with_capacity(frames);
    for i in 0..frames {
        let Ok(power) = power_spectrum(&x[i * HOP..i * HOP + WINDOW]) else {
            return Vec::new();
        };
        let logmag: Vec<f32> = power.iter().map(|&p| (1.0 + 1000.0 * p).ln()).collect();
        if let Some(prev) = &prev {
            let flux: f32 = logmag.iter().zip(prev).map(|(c, p)| (c - p).max(0.0)).sum();
            env.push(flux);
        }
        prev = Some(logmag);
    }
    env
}

/// Count local maxima above `mean + std` (adaptive floor) with a 50 ms
/// refractory period, so one hit is one onset.
fn count_onsets(env: &[f32], fps: f32) -> usize {
    if env.len() < 3 {
        return 0;
    }
    let mean = env.iter().sum::<f32>() / env.len() as f32;
    let var = env.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / env.len() as f32;
    let threshold = mean + var.sqrt();
    let refractory = (0.05 * fps) as usize;
    let mut count = 0usize;
    let mut last: Option<usize> = None;
    for i in 1..env.len() - 1 {
        if env[i] > threshold && env[i] >= env[i - 1] && env[i] > env[i + 1] {
            if let Some(prev) = last {
                if i - prev < refractory.max(1) {
                    continue;
                }
            }
            count += 1;
            last = Some(i);
        }
    }
    count
}

/// Tempo from the onset envelope's autocorrelation over the BPM window.
fn tempo(env: &[f32], fps: f32) -> Option<f32> {
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
    let env = onset_envelope(x);
    if env.is_empty() {
        return none;
    }
    let fps = sample_rate as f32 / HOP as f32;
    let duration = env.len() as f32 / fps;
    Rhythm {
        tempo_bpm: tempo(&env, fps),
        onset_density: count_onsets(&env, fps) as f32 / duration.max(1e-6),
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
}
