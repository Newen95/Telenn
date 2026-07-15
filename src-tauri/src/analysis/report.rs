//! Assembled reports: `MixReport` for any render, `ToneReport` for single
//! notes, plus the defect policy that drives the CLIs' exit codes.

use std::fmt;

use serde::Serialize;

use super::envelope::{
    attack_time_s, decay_rt60, env_window, envelope, DecayEstimate, ENV_WINDOW_S,
};
use super::fft::power_spectrum;
use super::levels::{
    channel_stats, detect_clicks, end_discontinuity, stereo_correlation, ChannelStats, Click,
};
use super::pitch::{estimate_f0, F0};
use super::spectral::{
    harmonic_ratio, harmonics, inharmonicity_cents, spectral_centroid_hz, spectral_flatness_db,
    Harmonic,
};
use super::Signal;

/// Longest slice fed to the tonal analyses (Goertzel cost is linear in it).
const MAX_TONE_SLICE: usize = 32_768;

#[derive(Debug, Serialize)]
pub struct MixReport {
    pub sample_rate: u32,
    pub duration_s: f32,
    pub left: ChannelStats,
    pub right: ChannelStats,
    pub stereo_correlation: f32,
    /// RMS level difference L − R, dB. Positive = left louder.
    pub balance_db: f32,
    /// Clicks detected on the mono sum.
    pub clicks: Vec<Click>,
    pub end_discontinuity: Option<f32>,
}

pub fn analyze_mix(signal: &Signal) -> MixReport {
    let left = channel_stats(&signal.left);
    let right = channel_stats(&signal.right);
    let mono = signal.mono();
    let balance_db = if left.rms > 0.0 && right.rms > 0.0 {
        20.0 * (left.rms / right.rms).log10()
    } else {
        0.0
    };
    MixReport {
        sample_rate: signal.sample_rate,
        duration_s: signal.duration_s(),
        left,
        right,
        stereo_correlation: stereo_correlation(&signal.left, &signal.right),
        balance_db,
        clicks: detect_clicks(&mono, signal.sample_rate),
        end_discontinuity: match (
            end_discontinuity(&signal.left),
            end_discontinuity(&signal.right),
        ) {
            (Some(l), Some(r)) => Some(l.max(r)),
            (l, r) => l.or(r),
        },
    }
}

#[derive(Debug, Serialize)]
pub struct ToneReport {
    pub f0: Option<F0>,
    pub harmonics: Vec<Harmonic>,
    pub inharmonicity_cents: Option<f32>,
    pub centroid_hz: f32,
    pub flatness_db: f32,
    pub harmonic_ratio: Option<f32>,
    pub attack_s: Option<f32>,
    pub decay: Option<DecayEstimate>,
}

/// Tone analysis of a (mostly) single note. `note_off_s` should be the exact
/// note-off time when the caller rendered the probe; `None` falls back to
/// envelope heuristics (sustain = middle 40% of the active region, decay
/// measured from the envelope peak).
pub fn analyze_tone(mono: &[f32], sample_rate: u32, note_off_s: Option<f32>) -> ToneReport {
    let silent = ToneReport {
        f0: None,
        harmonics: Vec::new(),
        inharmonicity_cents: None,
        centroid_hz: 0.0,
        flatness_db: 0.0,
        harmonic_ratio: None,
        attack_s: None,
        decay: None,
    };
    if mono.is_empty() || sample_rate == 0 {
        return silent;
    }

    let env = envelope(mono, sample_rate);
    let win = env_window(sample_rate);
    let peak = env.iter().fold(0.0f32, |m, &v| m.max(v));
    if peak <= 1e-5 {
        return silent;
    }
    let first = env.iter().position(|&v| v >= 0.1 * peak).unwrap_or(0);
    let last = env.iter().rposition(|&v| v >= 0.1 * peak).unwrap_or(0);
    let active_start = first * win;
    let active_end = ((last + 1) * win).min(mono.len());
    let attack_s = attack_time_s(&env, ENV_WINDOW_S);

    // Sustain slice: steady part of the note, where pitch and spectrum are
    // meaningful (skip the attack transient and the release).
    let (sus_start, sus_end) = match note_off_s {
        Some(off) => {
            let start = active_start + (attack_s.unwrap_or(0.0) * sample_rate as f32) as usize;
            let end = ((off * sample_rate as f32) as usize).min(active_end);
            (start, end)
        }
        None => {
            let len = active_end - active_start;
            (active_start + len * 3 / 10, active_start + len * 7 / 10)
        }
    };
    let slice = if sus_end > sus_start && sus_end - sus_start >= 1024 {
        &mono[sus_start..sus_end.min(sus_start + MAX_TONE_SLICE)]
    } else {
        &mono[active_start..active_end.min(active_start + MAX_TONE_SLICE)]
    };
    if slice.is_empty() {
        return silent;
    }

    let f0 = estimate_f0(slice, sample_rate);
    let harmonics = match f0 {
        Some(f0) => harmonics(slice, sample_rate, f0.hz),
        None => Vec::new(),
    };
    let (centroid_hz, flatness_db, h_ratio) = match power_spectrum(slice) {
        Ok(power) => {
            let fft_len = slice.len().next_power_of_two();
            let ratio = (!harmonics.is_empty())
                .then(|| harmonic_ratio(&power, fft_len, sample_rate, &harmonics));
            (
                spectral_centroid_hz(&power, fft_len, sample_rate),
                spectral_flatness_db(&power, fft_len, sample_rate),
                ratio,
            )
        }
        Err(_) => (0.0, 0.0, None),
    };

    // Decay measured from note-off (or the envelope peak when unknown).
    let decay_from = note_off_s.unwrap_or_else(|| {
        let peak_idx = env
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap_or(0);
        peak_idx as f32 * ENV_WINDOW_S
    });

    ToneReport {
        f0,
        inharmonicity_cents: (!harmonics.is_empty()).then(|| inharmonicity_cents(&harmonics)),
        harmonics,
        centroid_hz,
        flatness_db,
        harmonic_ratio: h_ratio,
        attack_s,
        decay: decay_rt60(mono, sample_rate, decay_from),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Serialize)]
pub struct Defect {
    pub severity: Severity,
    pub message: String,
}

/// The defect policy: hard errors make a render unusable (exit code 2 in the
/// CLIs), warnings deserve a look but may be intentional.
pub fn defects(mix: &MixReport, tone: Option<&ToneReport>) -> Vec<Defect> {
    let mut out = Vec::new();
    let mut error = |message: String| {
        out.push(Defect {
            severity: Severity::Error,
            message,
        })
    };
    let clip_events = mix.left.clip_events + mix.right.clip_events;
    if clip_events > 0 {
        error(format!(
            "clipping: {clip_events} events ({} samples)",
            mix.left.clipped_samples + mix.right.clipped_samples
        ));
    }
    if !mix.clicks.is_empty() {
        error(format!(
            "clicks: {} (first at {:.3} s, jump {:.2})",
            mix.clicks.len(),
            mix.clicks[0].time_s,
            mix.clicks[0].jump
        ));
    }
    for (name, stats) in [("left", &mix.left), ("right", &mix.right)] {
        if stats.dc.abs() > 0.005 {
            error(format!("dc offset on {name}: {:+.4}", stats.dc));
        }
    }
    if mix.left.rms.max(mix.right.rms) < 1e-4 {
        error("near-silence: rms below -80 dBFS".into());
    }

    let mut warn = |message: String| {
        out.push(Defect {
            severity: Severity::Warning,
            message,
        })
    };
    let peak = mix.left.peak.max(mix.right.peak);
    if peak > 0.966 && clip_events == 0 {
        warn(format!("hot peak: {:.2} dBFS, no headroom", dbfs(peak)));
    }
    if let Some(v) = mix.end_discontinuity {
        warn(format!("render ends mid-signal (last sample {v:.3})"));
    }
    if let Some(decay) = tone.and_then(|t| t.decay.as_ref()) {
        if !decay.reliable {
            warn(format!(
                "decay tail truncated: rt60 >= {:.2} s is a lower bound (lengthen --tail)",
                decay.rt60_s
            ));
        }
    }
    out
}

/// One file's side of a reference comparison: the global, whole-file metrics
/// that make two mixes comparable regardless of length or content.
#[derive(Debug, Serialize)]
pub struct FileMetrics {
    pub peak_dbfs: f32,
    pub crest_db: f32,
    pub integrated_lufs: Option<f32>,
    pub short_term_max_lufs: Option<f32>,
    /// Seven-band spectral balance (dB rel total), `None` for silence.
    pub bands: Option<Vec<super::bands::BandEnergy>>,
    pub correlation: f32,
    /// Side/mid RMS ratio (0 = mono).
    pub width: f32,
    pub tempo_bpm: Option<f32>,
    pub onset_density: f32,
}

/// Measure the comparison metrics of one file.
pub fn file_metrics(signal: &Signal) -> FileMetrics {
    let stats_l = channel_stats(&signal.left);
    let stats_r = channel_stats(&signal.right);
    let mono = signal.mono();
    let loud = super::loudness::measure(&signal.left, &signal.right, signal.sample_rate);
    let rhythm = super::rhythm::analyze(&mono, signal.sample_rate);
    let peak = stats_l.peak.max(stats_r.peak);
    let rms = ((stats_l.rms * stats_l.rms + stats_r.rms * stats_r.rms) / 2.0).sqrt();
    FileMetrics {
        peak_dbfs: dbfs(peak),
        crest_db: if rms > 0.0 {
            20.0 * (peak / rms).log10()
        } else {
            0.0
        },
        integrated_lufs: loud.integrated_lufs,
        short_term_max_lufs: loud.short_term_max_lufs,
        bands: super::bands::band_balance(&mono, signal.sample_rate),
        correlation: stereo_correlation(&signal.left, &signal.right),
        width: super::levels::side_mid_ratio(&signal.left, &signal.right),
        tempo_bpm: rhythm.tempo_bpm,
        onset_density: rhythm.onset_density,
    }
}

/// Deviation thresholds beyond which a metric is flagged in the verdict.
const LU_TOL: f32 = 1.0;
const BAND_TOL_DB: f32 = 3.0;
const CORR_TOL: f32 = 0.3;
const TEMPO_TOL_BPM: f32 = 3.0;
const DENSITY_TOL_REL: f32 = 0.30;

/// A render measured against a reference file: both sides' metrics plus the
/// flagged deviations (each one line, ready to print).
#[derive(Debug, Serialize)]
pub struct MatchReport {
    pub render: FileMetrics,
    pub reference: FileMetrics,
    pub flags: Vec<String>,
}

pub fn match_report(render: &Signal, reference: &Signal) -> MatchReport {
    let a = file_metrics(render);
    let b = file_metrics(reference);
    let mut flags = Vec::new();

    if let (Some(x), Some(y)) = (a.integrated_lufs, b.integrated_lufs) {
        if (x - y).abs() > LU_TOL {
            flags.push(format!(
                "loudness: {:+.1} LU vs reference (render {x:.1}, ref {y:.1})",
                x - y
            ));
        }
    }
    if let (Some(ab), Some(bb)) = (&a.bands, &b.bands) {
        for (ba, bb) in ab.iter().zip(bb) {
            let d = ba.db_rel - bb.db_rel;
            if d.abs() > BAND_TOL_DB {
                flags.push(format!(
                    "{}: {d:+.1} dB vs reference (render {:.1}, ref {:.1})",
                    ba.name, ba.db_rel, bb.db_rel
                ));
            }
        }
    }
    if (a.correlation - b.correlation).abs() > CORR_TOL {
        flags.push(format!(
            "stereo: correlation {:.2} vs reference {:.2}",
            a.correlation, b.correlation
        ));
    }
    if let (Some(x), Some(y)) = (a.tempo_bpm, b.tempo_bpm) {
        if (x - y).abs() > TEMPO_TOL_BPM {
            flags.push(format!("tempo: render {x:.1} BPM vs reference {y:.1} BPM"));
        }
    }
    if b.onset_density > 0.0 {
        let rel = (a.onset_density - b.onset_density) / b.onset_density;
        if rel.abs() > DENSITY_TOL_REL {
            flags.push(format!(
                "rhythm: {:.1} onsets/s vs reference {:.1} ({:+.0} %)",
                a.onset_density,
                b.onset_density,
                rel * 100.0
            ));
        }
    }

    MatchReport {
        render: a,
        reference: b,
        flags,
    }
}

impl fmt::Display for MatchReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let opt = |v: Option<f32>| match v {
            Some(v) => format!("{v:7.1}"),
            None => "      —".into(),
        };
        let (a, b) = (&self.render, &self.reference);
        writeln!(f, "== loudness ==")?;
        writeln!(
            f,
            "integrated  render {} LUFS   ref {} LUFS",
            opt(a.integrated_lufs),
            opt(b.integrated_lufs)
        )?;
        writeln!(
            f,
            "short-term  render {} LUFS   ref {} LUFS",
            opt(a.short_term_max_lufs),
            opt(b.short_term_max_lufs)
        )?;
        writeln!(f, "== dynamics ==")?;
        writeln!(
            f,
            "peak        render {:7.2} dBFS   ref {:7.2} dBFS",
            a.peak_dbfs, b.peak_dbfs
        )?;
        writeln!(
            f,
            "crest       render {:5.1} dB      ref {:5.1} dB",
            a.crest_db, b.crest_db
        )?;
        writeln!(f, "== spectral balance (dB rel total) ==")?;
        if let (Some(ab), Some(bb)) = (&a.bands, &b.bands) {
            for (ba, bbx) in ab.iter().zip(bb) {
                writeln!(
                    f,
                    "{:<11} render {:6.1}   ref {:6.1}   Δ {:+5.1}",
                    ba.name,
                    ba.db_rel,
                    bbx.db_rel,
                    ba.db_rel - bbx.db_rel
                )?;
            }
        } else {
            writeln!(f, "(one side is silent or too short)")?;
        }
        writeln!(f, "== stereo ==")?;
        writeln!(
            f,
            "correlation render {:5.2}   ref {:5.2}",
            a.correlation, b.correlation
        )?;
        writeln!(
            f,
            "width       render {:5.2}   ref {:5.2}",
            a.width, b.width
        )?;
        writeln!(f, "== rhythm ==")?;
        writeln!(
            f,
            "tempo       render {} BPM   ref {} BPM",
            opt(a.tempo_bpm),
            opt(b.tempo_bpm)
        )?;
        writeln!(
            f,
            "onsets/s    render {:5.1}   ref {:5.1}",
            a.onset_density, b.onset_density
        )?;
        Ok(())
    }
}

/// Linear amplitude → dBFS (floored at −120 dB for silence).
pub fn dbfs(v: f32) -> f32 {
    if v > 0.0 {
        (20.0 * v.log10()).max(-120.0)
    } else {
        -120.0
    }
}

/// Nearest note name with cents deviation, e.g. "A3 +2 ct".
pub fn note_name(hz: f32) -> Option<String> {
    if hz <= 0.0 {
        return None;
    }
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let nearest = midi.round();
    if !(0.0..=127.0).contains(&nearest) {
        return None;
    }
    let cents = ((midi - nearest) * 100.0).round() as i32;
    let n = nearest as usize;
    Some(format!(
        "{}{} {:+} ct",
        NAMES[n % 12],
        (n / 12) as i32 - 1,
        cents
    ))
}

impl fmt::Display for MixReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "== levels ==")?;
        writeln!(
            f,
            "peak        L {:7.2} dBFS   R {:7.2} dBFS",
            dbfs(self.left.peak),
            dbfs(self.right.peak)
        )?;
        writeln!(
            f,
            "rms         L {:7.2} dBFS   R {:7.2} dBFS",
            dbfs(self.left.rms),
            dbfs(self.right.rms)
        )?;
        writeln!(
            f,
            "crest       L {:5.1} dB      R {:5.1} dB",
            self.left.crest_db, self.right.crest_db
        )?;
        writeln!(
            f,
            "dc          L {:+.6}     R {:+.6}",
            self.left.dc, self.right.dc
        )?;
        let clip_events = self.left.clip_events + self.right.clip_events;
        let clipping = if clip_events == 0 {
            "none".to_string()
        } else {
            format!(
                "{clip_events} events ({} samples)",
                self.left.clipped_samples + self.right.clipped_samples
            )
        };
        let clicks = if self.clicks.is_empty() {
            "none".to_string()
        } else {
            format!(
                "{} (first at {:.3} s)",
                self.clicks.len(),
                self.clicks[0].time_s
            )
        };
        writeln!(f, "clipping    {clipping}   clicks  {clicks}")?;
        writeln!(
            f,
            "stereo      correlation {:.2}   balance L-R {:+.1} dB",
            self.stereo_correlation, self.balance_db
        )?;
        match self.end_discontinuity {
            Some(v) => writeln!(f, "end         discontinuity (last sample {v:.3})"),
            None => writeln!(f, "end         clean"),
        }
    }
}

impl fmt::Display for ToneReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "== tone (mono sum) ==")?;
        match self.f0 {
            Some(f0) => writeln!(
                f,
                "f0          {:.1} Hz ({})   clarity {:.2}",
                f0.hz,
                note_name(f0.hz).unwrap_or_else(|| "?".into()),
                f0.clarity
            )?,
            None => writeln!(f, "f0          none (unpitched or noisy)")?,
        }
        if !self.harmonics.is_empty() {
            write!(f, "harmonics  ")?;
            for h in self.harmonics.iter().skip(1).take(8) {
                write!(f, " H{} {:.1}", h.k, h.db_rel_f1)?;
            }
            writeln!(f, "  dB rel F1")?;
        }
        if let Some(inh) = self.inharmonicity_cents {
            writeln!(f, "inharmonic  {inh:.1} cents")?;
        }
        write!(
            f,
            "centroid    {:.0} Hz   flatness {:.1} dB",
            self.centroid_hz, self.flatness_db
        )?;
        match self.harmonic_ratio {
            Some(r) => writeln!(f, "   harmonic ratio {r:.2}")?,
            None => writeln!(f)?,
        }
        match self.attack_s {
            Some(a) => writeln!(f, "attack      {:.1} ms", a * 1000.0)?,
            None => writeln!(f, "attack      n/a")?,
        }
        match &self.decay {
            Some(d) if d.reliable => {
                writeln!(
                    f,
                    "decay       RT60 {:.2} s (fit {:.0} dB)",
                    d.rt60_s, d.fit_range_db
                )
            }
            Some(d) => writeln!(f, "decay       RT60 >= {:.2} s (tail truncated)", d.rt60_s),
            None => writeln!(f, "decay       n/a"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_signal(freq: f32, amp: f32, sr: u32, secs: f32) -> Signal {
        let n = (sr as f32 * secs) as usize;
        let mono: Vec<f32> = (0..n)
            .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / sr as f32).sin())
            .collect();
        Signal {
            left: mono.clone(),
            right: mono,
            sample_rate: sr,
        }
    }

    #[test]
    fn clean_sine_mix_has_no_defects() {
        // Fade the tail so the render ends clean, like a real release does.
        let mut s = sine_signal(440.0, 0.5, 48_000, 1.0);
        let n = s.left.len();
        for i in n - 480..n {
            let g = (n - i) as f32 / 480.0;
            s.left[i] *= g;
            s.right[i] *= g;
        }
        let mix = analyze_mix(&s);
        let found = defects(&mix, None);
        assert!(found.is_empty(), "unexpected defects: {found:?}");
        assert!((mix.stereo_correlation - 1.0).abs() < 1e-6);
        assert!(mix.balance_db.abs() < 1e-6);
    }

    #[test]
    fn clipped_mix_reports_an_error() {
        let mut s = sine_signal(440.0, 1.0, 48_000, 0.5);
        for v in s.left.iter_mut().chain(s.right.iter_mut()) {
            *v = (*v * 2.0).clamp(-1.0, 1.0);
        }
        let mix = analyze_mix(&s);
        let found = defects(&mix, None);
        assert!(found
            .iter()
            .any(|d| d.severity == Severity::Error && d.message.contains("clipping")));
    }

    #[test]
    fn tone_report_on_synthetic_note() {
        // 220 Hz with a second harmonic, 20 ms linear attack, exponential
        // release after note-off at 1.0 s.
        let sr = 48_000u32;
        let n = 2 * sr as usize;
        let mono: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let amp = if t < 0.02 {
                    t / 0.02
                } else if t < 1.0 {
                    1.0
                } else {
                    (-(t - 1.0) / 0.1).exp()
                };
                0.5 * amp
                    * ((std::f32::consts::TAU * 220.0 * t).sin()
                        + 0.3 * (std::f32::consts::TAU * 440.0 * t).sin())
            })
            .collect();
        let tone = analyze_tone(&mono, sr, Some(1.0));
        let f0 = tone.f0.expect("f0 must be found");
        assert!((f0.hz - 220.0).abs() < 1.0, "f0 {}", f0.hz);
        let attack = tone.attack_s.expect("attack must be measured");
        assert!((attack - 0.016).abs() < 0.01, "attack {attack}");
        assert!(tone.centroid_hz > 200.0 && tone.centroid_hz < 500.0);
        assert!(tone.harmonic_ratio.unwrap_or(0.0) > 0.8);
        let decay = tone.decay.expect("decay must be measured");
        // tau = 0.1 s → RT60 ≈ 0.69 s.
        assert!((decay.rt60_s - 0.69).abs() < 0.2, "rt60 {}", decay.rt60_s);
    }

    #[test]
    fn note_names() {
        assert_eq!(note_name(440.0).unwrap(), "A4 +0 ct");
        assert_eq!(note_name(261.63).unwrap(), "C4 +0 ct");
        assert!(note_name(0.0).is_none());
    }
}
