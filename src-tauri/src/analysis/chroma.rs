//! Pitch-class content: chroma vectors, triad matching and global key.
//!
//! Chroma folds the STFT magnitude (log-compressed so the bass does not
//! drown the mids, 55–2000 Hz band) into the 12 pitch classes. Chords come
//! from weighted triad templates scored by cosine similarity, the key from
//! Krumhansl-Schmuckler profile correlation.

use serde::{Deserialize, Serialize};

use crate::analysis::fft::{bin_hz, power_spectrum};

pub const CHROMA_WINDOW: usize = 8192;
pub const CHROMA_HOP: usize = 4096;
/// Analysis band: below 55 Hz the FFT bins are too coarse to separate
/// neighbouring pitch classes, above 2 kHz stacked harmonics blur them.
const F_LO: f32 = 55.0;
const F_HI: f32 = 2000.0;

pub(crate) const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Krumhansl-Schmuckler probe-tone profiles, indexed from the tonic.
const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// One L1-normalized 12-bin chroma vector per STFT frame (window 8192, hop
/// 4096). Frame `i` is centred at [`chroma_time_s`]`(i)`. Empty for input
/// shorter than one window.
pub fn chroma_frames(x: &[f32], sample_rate: u32) -> Vec<[f32; 12]> {
    if sample_rate == 0 || x.len() < CHROMA_WINDOW {
        return Vec::new();
    }
    // Bin -> pitch class, computed once for the analysis band.
    let n_bins = CHROMA_WINDOW / 2 + 1;
    let classes: Vec<Option<usize>> = (0..n_bins)
        .map(|k| {
            let f = bin_hz(k, CHROMA_WINDOW, sample_rate);
            if !(F_LO..F_HI).contains(&f) {
                return None;
            }
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            Some((midi.round() as i32).rem_euclid(12) as usize)
        })
        .collect();
    let frames = (x.len() - CHROMA_WINDOW) / CHROMA_HOP + 1;
    let mut out = Vec::with_capacity(frames);
    for i in 0..frames {
        let Ok(power) = power_spectrum(&x[i * CHROMA_HOP..i * CHROMA_HOP + CHROMA_WINDOW]) else {
            return Vec::new();
        };
        // Compress relative to the frame peak: level-independent, and the
        // log stays gentle on -40 dB leakage instead of saturating on it.
        let peak = power
            .iter()
            .enumerate()
            .filter(|(k, _)| classes[*k].is_some())
            .map(|(_, &p)| p)
            .fold(0.0f32, f32::max);
        if peak <= 0.0 {
            out.push([0.0f32; 12]);
            continue;
        }
        let mut c = [0.0f32; 12];
        for (k, &p) in power.iter().enumerate() {
            if let Some(pc) = classes[k] {
                c[pc] += (1.0 + 1000.0 * p / peak).ln();
            }
        }
        // Spectral leakage raises all 12 classes by a common floor; removing
        // it keeps the vector sharp enough for template matching.
        let floor = c.iter().copied().fold(f32::INFINITY, f32::min);
        for v in &mut c {
            *v -= floor;
        }
        let sum: f32 = c.iter().sum();
        if sum > 1e-9 {
            for v in &mut c {
                *v /= sum;
            }
        }
        out.push(c);
    }
    out
}

/// Seconds of the centre of chroma frame `i`.
pub fn chroma_time_s(i: usize, sample_rate: u32) -> f32 {
    (i * CHROMA_HOP + CHROMA_WINDOW / 2) as f32 / sample_rate as f32
}

/// Mean of a set of chroma frames, L1-renormalized. All-zero when `frames`
/// is empty or silent.
pub fn mean_chroma(frames: &[[f32; 12]]) -> [f32; 12] {
    let mut acc = [0.0f32; 12];
    for frame in frames {
        for (a, v) in acc.iter_mut().zip(frame) {
            *a += v;
        }
    }
    let sum: f32 = acc.iter().sum();
    if sum > 1e-9 {
        for v in &mut acc {
            *v /= sum;
        }
    }
    acc
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChordMatch {
    /// `"C"`…`"B"` for major triads, `"Cm"`…`"Bm"` for minor.
    pub chord: String,
    /// Cosine similarity against the winning template, 0..1.
    pub confidence: f32,
    /// Runner-up chord and its score — a close second flags ambiguity.
    pub alt: String,
    pub alt_confidence: f32,
}

/// Best triad match for one chroma vector; `None` on near-silence.
pub fn best_chord(chroma: &[f32; 12]) -> Option<ChordMatch> {
    let norm = chroma.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm < 1e-6 {
        return None;
    }
    // Root 1.0 / fifth 0.9 / third 0.8: the third is the quality carrier but
    // often the weakest partial, so it must not dominate the score.
    let template_norm = (1.0f32 + 0.9 * 0.9 + 0.8 * 0.8).sqrt();
    let mut scored: Vec<(f32, String)> = Vec::with_capacity(24);
    for root in 0..12 {
        for (third, suffix) in [(4usize, ""), (3usize, "m")] {
            let dot =
                chroma[root] + 0.8 * chroma[(root + third) % 12] + 0.9 * chroma[(root + 7) % 12];
            scored.push((
                dot / (norm * template_norm),
                format!("{}{suffix}", NAMES[root]),
            ));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let (best, alt) = (&scored[0], &scored[1]);
    Some(ChordMatch {
        chord: best.1.clone(),
        confidence: best.0.clamp(0.0, 1.0),
        alt: alt.1.clone(),
        alt_confidence: alt.0.clamp(0.0, 1.0),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Key {
    pub tonic: String,
    /// `"major"` or `"minor"`.
    pub mode: String,
    /// Pearson correlation with the winning profile, clamped to 0..1.
    pub confidence: f32,
}

/// Global key of a mean chroma via Krumhansl-Schmuckler correlation over the
/// 24 rotations; `None` on near-silence.
pub fn estimate_key(chroma: &[f32; 12]) -> Option<Key> {
    if chroma.iter().map(|v| v * v).sum::<f32>().sqrt() < 1e-6 {
        return None;
    }
    fn pearson(a: &[f32; 12], b: &[f32; 12]) -> f32 {
        let ma = a.iter().sum::<f32>() / 12.0;
        let mb = b.iter().sum::<f32>() / 12.0;
        let (mut cov, mut va, mut vb) = (0.0f32, 0.0f32, 0.0f32);
        for k in 0..12 {
            let (da, db) = (a[k] - ma, b[k] - mb);
            cov += da * db;
            va += da * da;
            vb += db * db;
        }
        if va <= 0.0 || vb <= 0.0 {
            return 0.0;
        }
        cov / (va.sqrt() * vb.sqrt())
    }
    let mut best: Option<(f32, usize, &str)> = None;
    for tonic in 0..12 {
        for (profile, mode) in [(&MAJOR_PROFILE, "major"), (&MINOR_PROFILE, "minor")] {
            // Expected value at pitch class k is profile[(k - tonic) mod 12].
            let mut rotated = [0.0f32; 12];
            for (k, r) in rotated.iter_mut().enumerate() {
                *r = profile[(k + 12 - tonic) % 12];
            }
            let c = pearson(chroma, &rotated);
            if best.is_none_or(|(b, _, _)| c > b) {
                best = Some((c, tonic, mode));
            }
        }
    }
    let (c, tonic, mode) = best?;
    Some(Key {
        tonic: NAMES[tonic].to_string(),
        mode: mode.to_string(),
        confidence: c.clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn midi_hz(m: i32) -> f32 {
        440.0 * ((m - 69) as f32 / 12.0).exp2()
    }

    fn tone_mix(freqs: &[f32], secs: f32) -> Vec<f32> {
        let n = (secs * SR as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                freqs
                    .iter()
                    .map(|f| (std::f32::consts::TAU * f * t).sin())
                    .sum::<f32>()
                    * (0.8 / freqs.len() as f32)
            })
            .collect()
    }

    fn chord_of(midis: &[i32]) -> ChordMatch {
        let freqs: Vec<f32> = midis.iter().map(|&m| midi_hz(m)).collect();
        let frames = chroma_frames(&tone_mix(&freqs, 1.0), SR);
        best_chord(&mean_chroma(&frames)).expect("chord")
    }

    #[test]
    fn major_and_minor_triads_are_named() {
        let c = chord_of(&[60, 64, 67]); // C4 E4 G4
        assert_eq!(c.chord, "C");
        assert!(c.confidence > 0.85, "confidence {}", c.confidence);
        assert!(
            c.confidence - c.alt_confidence > 0.1,
            "ambiguous: {} {} vs {} {}",
            c.chord,
            c.confidence,
            c.alt,
            c.alt_confidence
        );

        let am = chord_of(&[57, 60, 64]); // A3 C4 E4
        assert_eq!(am.chord, "Am");
    }

    #[test]
    fn a_minor_progression_reads_its_key() {
        // i-VI-III-VII in C# minor (C#m, A, E, B) over a tonic bass pedal —
        // the pedal carries the tonal hierarchy, as a bass line does in real
        // material; without it the relative major is equally likely.
        let bars = [[61, 64, 68], [57, 61, 64], [64, 68, 71], [59, 63, 66]];
        let mut x = Vec::new();
        for bar in bars {
            let freqs: Vec<f32> = bar
                .iter()
                .copied()
                .chain([37]) // C#2 pedal
                .map(midi_hz)
                .collect();
            x.extend(tone_mix(&freqs, 1.0));
        }
        let frames = chroma_frames(&x, SR);
        let key = estimate_key(&mean_chroma(&frames)).expect("key");
        assert_eq!(key.tonic, "C#");
        assert_eq!(key.mode, "minor");
        assert!(key.confidence > 0.5, "confidence {}", key.confidence);
    }

    #[test]
    fn silence_yields_no_chord_no_key_no_nan() {
        let frames = chroma_frames(&vec![0.0f32; SR as usize], SR);
        assert!(!frames.is_empty());
        for frame in &frames {
            assert!(frame.iter().all(|v| v.is_finite() && *v == 0.0));
        }
        let mean = mean_chroma(&frames);
        assert!(best_chord(&mean).is_none());
        assert!(estimate_key(&mean).is_none());
    }

    #[test]
    fn short_input_yields_no_frames() {
        assert!(chroma_frames(&[0.0f32; 100], SR).is_empty());
        assert!(chroma_frames(&[], SR).is_empty());
    }
}
