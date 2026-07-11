//! Amplitude statistics and hard-defect detectors (clipping, clicks, DC).

use serde::Serialize;

/// Samples at or above this magnitude count as potentially clipped.
const CLIP_LEVEL: f32 = 0.999;
/// A run of at least this many consecutive clipped samples is a clip event
/// (true flat-topping, as opposed to a single hot peak).
const CLIP_RUN: usize = 3;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ChannelStats {
    pub peak: f32,
    pub rms: f32,
    /// `20·log10(peak/rms)` — low for squashed material, high for transients.
    pub crest_db: f32,
    pub dc: f32,
    pub clip_events: usize,
    pub clipped_samples: usize,
}

pub fn channel_stats(x: &[f32]) -> ChannelStats {
    if x.is_empty() {
        return ChannelStats {
            peak: 0.0,
            rms: 0.0,
            crest_db: 0.0,
            dc: 0.0,
            clip_events: 0,
            clipped_samples: 0,
        };
    }
    let peak = x.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    let rms = (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    let dc = x.iter().sum::<f32>() / x.len() as f32;

    let mut clip_events = 0usize;
    let mut clipped_samples = 0usize;
    let mut run = 0usize;
    for &s in x {
        if s.abs() >= CLIP_LEVEL {
            clipped_samples += 1;
            run += 1;
            if run == CLIP_RUN {
                clip_events += 1;
            }
        } else {
            run = 0;
        }
    }

    let crest_db = if rms > 0.0 && peak > 0.0 {
        20.0 * (peak / rms).log10()
    } else {
        0.0
    };
    ChannelStats {
        peak,
        rms,
        crest_db,
        dc,
        clip_events,
        clipped_samples,
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Click {
    pub time_s: f32,
    /// Sample-to-sample jump that triggered the detection.
    pub jump: f32,
}

/// Detect discontinuities via the first difference, judged against the mean
/// difference over the FOLLOWING 25 ms: a legitimate sharp attack (drum hit,
/// string pluck — even one rising instantly out of silence) is followed by
/// activity that raises that mean, while a genuine glitch is followed by a
/// return to smooth or silent material. A jump is a click when it is large
/// (> 0.5) and 6× the forward mean, or moderate (> 0.08) and 12× it.
/// Thresholds are heuristic, tuned for probe renders.
pub fn detect_clicks(x: &[f32], sample_rate: u32) -> Vec<Click> {
    if x.len() < 2 || sample_rate == 0 {
        return Vec::new();
    }
    let d: Vec<f32> = x.windows(2).map(|w| (w[1] - w[0]).abs()).collect();

    // Prefix sums of `d` for O(1) forward means.
    let mut prefix = Vec::with_capacity(d.len() + 1);
    prefix.push(0.0f64);
    for &v in &d {
        let last = *prefix.last().unwrap_or(&0.0);
        prefix.push(last + v as f64);
    }
    let long_window = (sample_rate as usize / 40).max(1); // 25 ms
    let short_window = (sample_rate as usize / 500).max(1); // 2 ms
    let forward_mean = |from: usize, window: usize| -> f32 {
        let lo = (from + 1).min(d.len() - 1);
        let hi = (from + window).min(d.len() - 1);
        ((prefix[hi + 1] - prefix[lo]) / (hi + 1 - lo) as f64) as f32
    };

    let mut clicks: Vec<Click> = Vec::new();
    let merge_gap = (sample_rate as usize / 1000).max(1); // 1 ms
    let mut last_flag: Option<usize> = None;
    for (i, &jump) in d.iter().enumerate() {
        // Two forward horizons: the short one sees a brief excitation burst
        // (a pluck's noise is only a few ms) that the long mean would dilute.
        let context = forward_mean(i, short_window).max(forward_mean(i, long_window));
        let is_click =
            (jump > 0.5 && jump > 6.0 * context) || (jump > 0.08 && jump > 12.0 * context);
        if !is_click {
            continue;
        }
        // Merge flags within 1 ms into a single event, keeping the biggest jump.
        match last_flag {
            Some(prev) if i - prev <= merge_gap => {
                if let Some(last) = clicks.last_mut() {
                    if jump > last.jump {
                        last.jump = jump;
                        last.time_s = (i + 1) as f32 / sample_rate as f32;
                    }
                }
            }
            _ => clicks.push(Click {
                time_s: (i + 1) as f32 / sample_rate as f32,
                jump,
            }),
        }
        last_flag = Some(i);
    }
    clicks
}

/// A render should end at (or ramp to) zero; a hot last sample means the
/// buffer was truncated mid-signal and will click on playback.
pub fn end_discontinuity(x: &[f32]) -> Option<f32> {
    let last = x.last()?.abs();
    (last > 0.02).then_some(last)
}

/// Normalized cross-correlation at lag 0. 1.0 = identical channels (mono),
/// 0.0 = unrelated, negative = out of phase.
pub fn stereo_correlation(l: &[f32], r: &[f32]) -> f32 {
    let n = l.len().min(r.len());
    if n == 0 {
        return 1.0;
    }
    let dot: f64 = l[..n]
        .iter()
        .zip(&r[..n])
        .map(|(a, b)| (a * b) as f64)
        .sum();
    let el: f64 = l[..n].iter().map(|a| (a * a) as f64).sum();
    let er: f64 = r[..n].iter().map(|b| (b * b) as f64).sum();
    let denom = (el * er).sqrt();
    if denom == 0.0 {
        return 1.0; // both silent: trivially identical
    }
    (dot / denom) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn clean_sine_has_no_defects() {
        let x: Vec<f32> = sine(440.0, 48_000, 48_000)
            .iter()
            .map(|s| s * 0.5)
            .collect();
        let stats = channel_stats(&x);
        assert_eq!(stats.clip_events, 0);
        assert!((stats.peak - 0.5).abs() < 1e-3);
        assert!((stats.rms - 0.3535).abs() < 1e-2, "rms {}", stats.rms);
        assert!(
            (stats.crest_db - 3.01).abs() < 0.1,
            "crest {}",
            stats.crest_db
        );
        assert!(stats.dc.abs() < 1e-4);
        assert!(detect_clicks(&x, 48_000).is_empty());
    }

    #[test]
    fn hard_clipped_sine_is_flagged() {
        let x: Vec<f32> = sine(440.0, 48_000, 48_000)
            .iter()
            .map(|s| (s * 3.0).clamp(-1.0, 1.0))
            .collect();
        let stats = channel_stats(&x);
        assert!(stats.clip_events > 0, "no clip events detected");
        assert!(stats.clipped_samples > 100);
    }

    #[test]
    fn step_is_detected_as_click() {
        let mut x = sine(220.0, 48_000, 24_000);
        for s in x.iter_mut() {
            *s *= 0.2;
        }
        x[12_000] += 0.8; // one-sample spike
        let clicks = detect_clicks(&x, 48_000);
        assert_eq!(clicks.len(), 1, "clicks: {clicks:?}");
        assert!((clicks[0].time_s - 0.25).abs() < 0.01);
    }

    #[test]
    fn dc_offset_is_measured() {
        let x: Vec<f32> = sine(440.0, 48_000, 48_000)
            .iter()
            .map(|s| s + 0.1)
            .collect();
        let stats = channel_stats(&x);
        assert!((stats.dc - 0.1).abs() < 1e-3, "dc {}", stats.dc);
    }

    #[test]
    fn end_discontinuity_flags_truncation() {
        assert!(end_discontinuity(&[0.0, 0.5, 0.4]).is_some());
        assert!(end_discontinuity(&[0.0, 0.5, 0.001]).is_none());
    }

    #[test]
    fn correlation_extremes() {
        let x = sine(440.0, 48_000, 4800);
        let inverted: Vec<f32> = x.iter().map(|s| -s).collect();
        assert!((stereo_correlation(&x, &x) - 1.0).abs() < 1e-6);
        assert!((stereo_correlation(&x, &inverted) + 1.0).abs() < 1e-6);
    }
}
