//! Amplitude envelope and time-domain features (attack, decay).

use serde::Serialize;

/// Envelope resolution: peak magnitude per 1 ms window.
pub const ENV_WINDOW_S: f32 = 0.001;

/// Envelope window length in samples for `sample_rate`.
pub fn env_window(sample_rate: u32) -> usize {
    ((sample_rate as f32 * ENV_WINDOW_S) as usize).max(1)
}

/// Peak-per-window amplitude envelope, [`ENV_WINDOW_S`] windows.
pub fn envelope(x: &[f32], sample_rate: u32) -> Vec<f32> {
    let win = env_window(sample_rate);
    x.chunks(win)
        .map(|c| c.iter().fold(0.0f32, |m, &s| m.max(s.abs())))
        .collect()
}

/// 10% → 90% rise time before the envelope peak, in seconds.
/// `win_s` is the envelope window duration ([`ENV_WINDOW_S`] normally).
pub fn attack_time_s(env: &[f32], win_s: f32) -> Option<f32> {
    let (peak_idx, &peak) = env.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    if peak <= 0.0 {
        return None;
    }
    // Walk back from the peak to the last window below 10%, then forward to
    // the first at 90% — robust to slow swells that cross 10% several times.
    let t10 = env[..peak_idx]
        .iter()
        .rposition(|&v| v < 0.1 * peak)
        .map_or(0, |i| i + 1);
    let t90 = (t10..=peak_idx).find(|&i| env[i] >= 0.9 * peak)?;
    Some((t90 - t10) as f32 * win_s)
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct DecayEstimate {
    pub rt60_s: f32,
    /// dB span of the energy-decay curve the line was fitted on.
    pub fit_range_db: f32,
    /// False when the decay curve never reached −25 dB before the buffer
    /// ended: the tail is truncated and `rt60_s` is a lower bound.
    pub reliable: bool,
}

/// RT60 via Schroeder backward integration from `from_s` (note-off when the
/// caller knows it, envelope peak otherwise). Least-squares line fit on the
/// energy-decay curve between −5 dB and −25 dB, extrapolated to −60 dB.
///
/// Truncation is judged by comparing short-window RMS at the head and foot of
/// the tail — Schroeder integration always plunges at the buffer end, so the
/// curve itself cannot tell a finished decay from a cut one.
pub fn decay_rt60(x: &[f32], sample_rate: u32, from_s: f32) -> Option<DecayEstimate> {
    if sample_rate == 0 {
        return None;
    }
    let start = ((from_s * sample_rate as f32) as usize).min(x.len());
    let tail = &x[start..];
    if tail.len() < 32 {
        return None;
    }

    let rms = |s: &[f32]| (s.iter().map(|v| (v * v) as f64).sum::<f64>() / s.len() as f64).sqrt();
    let win = ((sample_rate / 100) as usize).clamp(16, tail.len()); // 10 ms
    let head = rms(&tail[..win]);
    let foot = rms(&tail[tail.len() - win..]);
    if head <= 0.0 {
        return None;
    }
    let drop_db = (20.0 * (foot / head).max(1e-30).log10()) as f32;
    if drop_db > -25.0 {
        // The buffer ends before the decay completed: the curve fit would be
        // biased, so estimate a lower bound from the observed drop instead.
        if drop_db > -15.0 {
            return None; // decay barely started — nothing meaningful to fit
        }
        let duration = tail.len() as f32 / sample_rate as f32;
        return Some(DecayEstimate {
            rt60_s: 60.0 * duration / -drop_db,
            fit_range_db: -drop_db,
            reliable: false,
        });
    }

    // Backward-integrated energy: EDC(n) = sum_{k>=n} x^2 / total.
    let mut cumulative = vec![0.0f64; tail.len()];
    let mut acc = 0.0f64;
    for (i, &s) in tail.iter().enumerate().rev() {
        acc += (s as f64) * (s as f64);
        cumulative[i] = acc;
    }
    let total = cumulative[0];
    if total <= 0.0 {
        return None;
    }
    let edc_db: Vec<f32> = cumulative
        .iter()
        .map(|&e| (10.0 * (e / total).log10()) as f32)
        .collect();

    let n5 = edc_db.iter().position(|&db| db <= -5.0)?;
    let end = edc_db.iter().position(|&db| db <= -25.0)?;
    let reliable = true;
    if end <= n5 {
        return None;
    }

    // Least-squares slope of EDC(t) over the fit segment, in dB/s.
    let m = (end - n5 + 1) as f64;
    let (mut sum_t, mut sum_y, mut sum_tt, mut sum_ty) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (i, &db) in edc_db[n5..=end].iter().enumerate() {
        let t = (n5 + i) as f64 / sample_rate as f64;
        sum_t += t;
        sum_y += db as f64;
        sum_tt += t * t;
        sum_ty += t * db as f64;
    }
    let denom = m * sum_tt - sum_t * sum_t;
    if denom == 0.0 {
        return None;
    }
    let slope = (m * sum_ty - sum_t * sum_y) / denom;
    if slope >= 0.0 {
        return None; // not decaying
    }
    Some(DecayEstimate {
        rt60_s: (-60.0 / slope) as f32,
        fit_range_db: edc_db[n5] - edc_db[end],
        reliable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_ramp_attack() {
        // 50 ms linear ramp to full scale, then hold: 10→90% of a linear ramp
        // is 80% of its duration = 40 ms.
        let sr = 48_000u32;
        let ramp = (sr as usize) / 20;
        let mut x: Vec<f32> = (0..ramp).map(|i| i as f32 / ramp as f32).collect();
        x.resize(ramp + sr as usize / 10, 1.0);
        let env = envelope(&x, sr);
        let attack = attack_time_s(&env, ENV_WINDOW_S).unwrap();
        assert!((attack - 0.040).abs() < 0.005, "attack {attack}");
    }

    #[test]
    fn exponential_tail_rt60() {
        // x(t) = e^(-t/tau) modulated sine. RT60 = tau * ln(10^(60/20)) ≈ 6.91·tau.
        // tau chosen so RT60 = 1.2 s.
        let sr = 48_000u32;
        let tau = 1.2 / 6.9078;
        let n = (2.5 * sr as f32) as usize;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (-t / tau).exp() * (std::f32::consts::TAU * 440.0 * t).sin()
            })
            .collect();
        let est = decay_rt60(&x, sr, 0.0).unwrap();
        assert!(est.reliable);
        assert!(
            (est.rt60_s - 1.2).abs() / 1.2 < 0.15,
            "rt60 {} (expected ~1.2)",
            est.rt60_s
        );
    }

    #[test]
    fn truncated_tail_is_unreliable() {
        // tau = 0.39 s decays only ~20 dB over this 0.9 s buffer: the
        // estimate must be flagged unreliable but stay in the right ballpark
        // (true RT60 ≈ 2.7 s, and the linear lower bound matches it here).
        let sr = 48_000u32;
        let tau = 0.39f32;
        let n = (0.9 * sr as f32) as usize;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (-t / tau).exp() * (std::f32::consts::TAU * 440.0 * t).sin()
            })
            .collect();
        let est = decay_rt60(&x, sr, 0.0).expect("a 20 dB drop is measurable");
        assert!(!est.reliable, "a truncated tail must not claim reliability");
        assert!(
            (est.rt60_s - 2.7).abs() / 2.7 < 0.25,
            "rt60 lower bound {} (expected ~2.7)",
            est.rt60_s
        );
    }

    #[test]
    fn barely_started_decay_yields_none() {
        let sr = 48_000u32;
        let x: Vec<f32> = (0..sr as usize / 2)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (-t / 2.0).exp() * (std::f32::consts::TAU * 440.0 * t).sin()
            })
            .collect();
        assert!(decay_rt60(&x, sr, 0.0).is_none());
    }

    #[test]
    fn silence_has_no_decay() {
        assert!(decay_rt60(&[0.0; 4800], 48_000, 0.0).is_none());
    }
}
