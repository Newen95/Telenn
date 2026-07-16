//! Partial transcription: bass line, drum pattern and chord grid on an
//! inferred beat grid.
//!
//! Only the layers a hand-written analysis can read reliably are extracted:
//! the bass (monophonic and dominant below 300 Hz in the target material),
//! the drums (detected in band-limited flux: kick in the lows, snare in the
//! mids confirmed broadband, hats in the highs confirmed noisy), and one
//! chord per measure from chroma. Everything else — voices, arpeggios buried
//! in reverb — is out of scope by design.
//!
//! Positions are `"bar.beat.tick"` strings (PPQ 960, 4/4) on a rigid grid:
//! tempo from onset autocorrelation, beat phase from onset alignment,
//! downbeat from low-band flux and bass-note starts. Every event keeps its
//! raw time and quantization error so a shifted grid stays visible; `--bpm`
//! and `--offset-s` overrides recover from a wrong inference.

use std::fmt;

use serde::Serialize;

use crate::analysis::chroma::{
    best_chord, chroma_frames, chroma_time_s, estimate_key, mean_chroma, Key, NAMES,
};
use crate::analysis::fft::power_spectrum;
use crate::analysis::pitch::estimate_f0_in;
use crate::analysis::rhythm::{band_flux, frame_time_s, pick_onsets, tempo, HOP, WINDOW};
use crate::analysis::spectral::spectral_flatness_above_db;
use crate::analysis::AnalysisError;
use crate::control::project::{ticks_to_duration, ticks_to_position};

/// Ticks per bar / per beat in the fixed 4/4 grid.
const TPBAR: u64 = 3840;
const TPBEAT: u64 = 960;

/// Bass tracker: 4th-order lowpass, then YIN on a 4x-decimated signal
/// (300 Hz content lives comfortably below the 6 kHz decimated Nyquist and
/// the difference function gets 16x cheaper).
const BASS_LP_HZ: f32 = 300.0;
const DECIM: usize = 4;
/// Decimated-rate analysis window (85 ms at 12 kHz — two periods of 30 Hz).
const BASS_WIN: usize = 1024;
const BASS_HOP: usize = 128;
const BASS_F_LO: f32 = 30.0;
const BASS_F_HI: f32 = 200.0;
/// YIN searches wider than the accepted range: a 300 Hz residue (snare body,
/// synth low-mids) must be *seen* at 300 Hz and rejected, not folded onto
/// its half by a 200 Hz search ceiling.
const BASS_F_SEARCH_HI: f32 = 400.0;
const BASS_CLARITY_MIN: f32 = 0.5;
/// Snap a bass-note start to a full-band onset this close (YIN frames are
/// ~11 ms; onsets are sharper).
const ONSET_SNAP_S: f32 = 0.04;

/// Drum detection bands and confirmations.
const KICK_HI_HZ: f32 = 150.0;
const SNARE_LO_HZ: f32 = 150.0;
const SNARE_HI_HZ: f32 = 2000.0;
const HAT_LO_HZ: f32 = 5000.0;
const HAT_HI_HZ: f32 = 16000.0;
/// Two events this close count as one hit.
const COINC_S: f32 = 0.02;
/// Post-attack window for the hat noisiness check.
const HAT_FEATURE_WIN_S: f32 = 0.04;
/// High-end flatness above this = noise = cymbal; harmonics score far lower.
const HAT_FLATNESS_MIN_DB: f32 = -18.0;
/// A kick candidate this close to a bass-note start may be the bass alone.
const BASS_KICK_COINC_S: f32 = 0.03;
/// Onsets weaker than this fraction of their band's loudest hit are noise
/// bleed from another band, not a drum — the adaptive `mean + std` floor of
/// `pick_onsets` is too permissive on a sparse envelope.
const MIN_STRENGTH: f32 = 0.15;
/// Same-class hits closer than this are one hit: a low tone barely longer
/// than the STFT window makes the flux wobble past the 50 ms refractory of
/// `pick_onsets`. Sixteenths at 128 BPM are 117 ms apart — still safe.
const CLASS_REFRACTORY_S: f32 = 0.1;

#[derive(Debug, Clone)]
pub struct Options {
    /// Tempo override; `None` infers it from the onset autocorrelation.
    pub bpm: Option<f32>,
    /// Absolute seconds of bar 1 beat 1; `None` infers phase and downbeat.
    pub offset_s: Option<f32>,
    /// Quantization denominator (16 = sixteenth grid). Must divide a bar.
    pub subdivision: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            bpm: None,
            offset_s: None,
            subdivision: 16,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Grid {
    pub bpm: f32,
    /// Absolute seconds of bar 1 beat 1.
    pub beat0_s: f32,
    pub time_signature: [u32; 2],
    pub ppq: u32,
    pub subdivision: u32,
    pub bpm_overridden: bool,
    pub offset_overridden: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BassNote {
    pub pitch: u8,
    /// `"C#2"` — the pitch as a name, for human eyes.
    pub note: String,
    /// Quantized `"bar.beat.tick"` position / duration triple.
    pub start: String,
    pub length: String,
    pub vel: u8,
    /// Mean YIN clarity over the note, 0..1.
    pub clarity: f32,
    /// Unquantized onset, absolute seconds.
    pub raw_s: f32,
    /// Raw ticks minus quantized ticks: how far the event sat off the grid.
    pub quant_error_ticks: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DrumHit {
    /// `"kick"`, `"snare"` or `"hat"`.
    pub class: String,
    pub start: String,
    /// Flux peak relative to the loudest hit of the same class, 0..1.
    pub strength: f32,
    pub raw_s: f32,
    pub quant_error_ticks: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeasureChord {
    pub bar: u32,
    /// Best triad, or `"N.C."` for a silent measure.
    pub chord: String,
    pub confidence: f32,
    pub alt: String,
    pub alt_confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Transcription {
    /// Filled by the caller (the library never sees the path).
    pub source: String,
    pub sample_rate: u32,
    pub duration_s: f32,
    /// Segment offset into the source file, informative only.
    pub start_s: f32,
    pub grid: Grid,
    pub key: Option<Key>,
    pub bass: Vec<BassNote>,
    pub drums: Vec<DrumHit>,
    pub chords: Vec<MeasureChord>,
}

/// Transcribe a mono signal. Fails when no tempo can be inferred and none
/// was supplied — everything downstream needs the grid.
pub fn transcribe(
    x: &[f32],
    sample_rate: u32,
    opts: &Options,
) -> Result<Transcription, AnalysisError> {
    if sample_rate == 0 {
        return Err(AnalysisError::BadInput("sample rate is zero".into()));
    }
    if opts.subdivision == 0 || !TPBAR.is_multiple_of(opts.subdivision as u64) {
        return Err(AnalysisError::BadInput(format!(
            "subdivision {} does not divide a 4/4 bar",
            opts.subdivision
        )));
    }
    let duration_s = x.len() as f32 / sample_rate as f32;
    let fps = sample_rate as f32 / HOP as f32;

    let full_env = band_flux(x, sample_rate, 0.0, f32::INFINITY);
    if full_env.is_empty() {
        return Err(AnalysisError::BadInput("input too short to analyze".into()));
    }

    let bpm = match opts.bpm {
        Some(b) if b > 0.0 => b,
        Some(b) => {
            return Err(AnalysisError::BadInput(format!("bad bpm {b}")));
        }
        None => tempo(&full_env, fps)
            .ok_or_else(|| AnalysisError::BadInput("no tempo found; pass --bpm".into()))?,
    };
    let beat_s = 60.0 / bpm;

    // Bass first: it is grid-agnostic (raw seconds) and its note starts help
    // place the downbeat.
    let onset_times: Vec<f32> = pick_onsets(&full_env, fps)
        .into_iter()
        .map(|i| frame_time_s(i, sample_rate))
        .collect();
    let low = lowpassed(x, sample_rate, BASS_LP_HZ);
    let dec: Vec<f32> = low.iter().step_by(DECIM).copied().collect();
    let dec_sr = sample_rate / DECIM as u32;
    let raw_bass = track_bass(&dec, dec_sr, &onset_times);

    let low_env = band_flux(x, sample_rate, 0.0, KICK_HI_HZ);
    let beat0_s = match opts.offset_s {
        Some(o) => o,
        None => {
            let phase_s = beat_phase_s(&full_env, fps, bpm, sample_rate);
            let starts: Vec<f32> = raw_bass.iter().map(|n| n.start_s).collect();
            let d = downbeat(&low_env, phase_s, beat_s, duration_s, &starts, sample_rate);
            phase_s + d as f32 * beat_s
        }
    };

    let step = (TPBAR / opts.subdivision as u64) as i64;
    let ticks_of =
        |t: f32| -> i64 { (((t - beat0_s) / beat_s) as f64 * TPBEAT as f64).round() as i64 };
    let quantize =
        |ticks: i64| -> i64 { (((ticks as f64) / step as f64).round() as i64 * step).max(0) };

    let bass: Vec<BassNote> = raw_bass
        .iter()
        .map(|n| {
            let raw_ticks = ticks_of(n.start_s);
            let q = quantize(raw_ticks);
            let end = quantize(ticks_of(n.end_s)).max(q + step);
            BassNote {
                pitch: n.pitch,
                note: midi_name(n.pitch),
                start: ticks_to_position(q as u64, TPBAR, TPBEAT),
                length: ticks_to_duration((end - q) as u64, TPBAR, TPBEAT),
                vel: n.vel,
                clarity: n.clarity,
                raw_s: n.start_s,
                quant_error_ticks: raw_ticks - q,
            }
        })
        .collect();

    let raw_hits = detect_drums(x, sample_rate, &low_env, fps, &raw_bass, &dec, dec_sr);
    let drums: Vec<DrumHit> = raw_hits
        .iter()
        .map(|h| {
            let raw_ticks = ticks_of(h.time_s);
            let q = quantize(raw_ticks);
            DrumHit {
                class: h.class.to_string(),
                start: ticks_to_position(q as u64, TPBAR, TPBEAT),
                strength: h.strength,
                raw_s: h.time_s,
                quant_error_ticks: raw_ticks - q,
            }
        })
        .collect();

    // One chord per measure, over every bar the grid covers.
    let cframes = chroma_frames(x, sample_rate);
    let n_bars = ((ticks_of(duration_s).max(0) as u64) / TPBAR).max(1) as u32;
    let bar_s = 4.0 * beat_s;
    let chords: Vec<MeasureChord> = (1..=n_bars)
        .map(|bar| {
            let t0 = beat0_s + (bar - 1) as f32 * bar_s;
            let in_bar: Vec<[f32; 12]> = cframes
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    let t = chroma_time_s(*i, sample_rate);
                    t >= t0 && t < t0 + bar_s
                })
                .map(|(_, c)| *c)
                .collect();
            match best_chord(&mean_chroma(&in_bar)) {
                Some(m) => MeasureChord {
                    bar,
                    chord: m.chord,
                    confidence: m.confidence,
                    alt: m.alt,
                    alt_confidence: m.alt_confidence,
                },
                None => MeasureChord {
                    bar,
                    chord: "N.C.".into(),
                    confidence: 0.0,
                    alt: String::new(),
                    alt_confidence: 0.0,
                },
            }
        })
        .collect();

    let key = estimate_key(&mean_chroma(&cframes));

    Ok(Transcription {
        source: String::new(),
        sample_rate,
        duration_s,
        start_s: 0.0,
        grid: Grid {
            bpm,
            beat0_s,
            time_signature: [4, 4],
            ppq: TPBEAT as u32,
            subdivision: opts.subdivision,
            bpm_overridden: opts.bpm.is_some(),
            offset_overridden: opts.offset_s.is_some(),
        },
        key,
        bass,
        drums,
        chords,
    })
}

/// `"C#2"` for MIDI 37 (octave -1 at MIDI 0, the usual convention).
fn midi_name(pitch: u8) -> String {
    format!(
        "{}{}",
        NAMES[(pitch % 12) as usize],
        (pitch / 12) as i32 - 1
    )
}

// ---------------------------------------------------------------- grid ----

/// Linear interpolation into an envelope at a fractional index.
fn env_at(env: &[f32], idx: f32) -> f32 {
    if env.is_empty() || idx < 0.0 {
        return 0.0;
    }
    let i = idx.floor() as usize;
    if i + 1 >= env.len() {
        return env[env.len() - 1];
    }
    let frac = idx - i as f32;
    env[i] * (1.0 - frac) + env[i + 1] * frac
}

/// Inverse of [`frame_time_s`], fractional.
fn time_to_frame(t: f32, sample_rate: u32) -> f32 {
    (t * sample_rate as f32 - (WINDOW / 2) as f32) / HOP as f32 - 1.0
}

/// Beat phase: the offset in `[0, beat period)` whose comb of beat positions
/// collects the most onset energy, refined parabolically.
fn beat_phase_s(env: &[f32], fps: f32, bpm: f32, sample_rate: u32) -> f32 {
    let period = fps * 60.0 / bpm;
    let n_offsets = (period.floor() as usize).max(1);
    let score = |o: f32| -> f32 {
        let mut s = 0.0;
        let mut t = o;
        while t < env.len() as f32 {
            s += env_at(env, t);
            t += period;
        }
        s
    };
    let scores: Vec<f32> = (0..n_offsets).map(|o| score(o as f32)).collect();
    let best = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let a = scores[(best + n_offsets - 1) % n_offsets];
    let b = scores[best];
    let c = scores[(best + 1) % n_offsets];
    let denom = a - 2.0 * b + c;
    let off = best as f32
        + if denom.abs() > 1e-9 {
            (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
        } else {
            0.0
        };
    // frame_time_s for a fractional index.
    ((off + 1.0) * HOP as f32 + (WINDOW / 2) as f32) / sample_rate as f32
}

/// Which of the 4 beats after `phase_s` is bar 1 beat 1: the candidate whose
/// downbeats collect the most low-band flux and the most bass-note starts.
fn downbeat(
    low_env: &[f32],
    phase_s: f32,
    beat_s: f32,
    duration_s: f32,
    bass_starts_s: &[f32],
    sample_rate: u32,
) -> usize {
    let mut best = (0usize, f32::MIN);
    for d in 0..4 {
        let mut flux = 0.0;
        let mut t = phase_s + d as f32 * beat_s;
        while t < duration_s {
            flux += env_at(low_env, time_to_frame(t, sample_rate));
            t += 4.0 * beat_s;
        }
        let on_downbeat = bass_starts_s
            .iter()
            .filter(|&&s| {
                let beats = ((s - phase_s) / beat_s - d as f32).rem_euclid(4.0);
                beats.min(4.0 - beats) * beat_s <= BASS_KICK_COINC_S
            })
            .count();
        let bass_frac = on_downbeat as f32 / bass_starts_s.len().max(1) as f32;
        let score = 0.5 * flux + 0.5 * bass_frac;
        if score > best.1 {
            best = (d, score);
        }
    }
    // Flux is unnormalized across candidates on purpose: only their order
    // matters and they share the same envelope.
    best.0
}

// ---------------------------------------------------------------- bass ----

/// RBJ lowpass biquad, direct form 1. Local on purpose: analysis must not
/// import the engine's RT filters, and offline code may allocate freely.
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn lowpass(sample_rate: u32, fc: f32) -> Self {
        let w0 = std::f32::consts::TAU * fc / sample_rate as f32;
        let (sin, cos) = w0.sin_cos();
        // Butterworth Q = 1/sqrt(2).
        let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos) / 2.0) / a0,
            b1: (1.0 - cos) / a0,
            b2: ((1.0 - cos) / 2.0) / a0,
            a1: (-2.0 * cos) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Two cascaded Butterworth lowpasses: ~24 dB/oct above `fc`.
fn lowpassed(x: &[f32], sample_rate: u32, fc: f32) -> Vec<f32> {
    let mut b1 = Biquad::lowpass(sample_rate, fc);
    let mut b2 = Biquad::lowpass(sample_rate, fc);
    x.iter().map(|&s| b2.process(b1.process(s))).collect()
}

struct RawBassNote {
    pitch: u8,
    start_s: f32,
    end_s: f32,
    vel: u8,
    clarity: f32,
}

/// Sliding YIN over the decimated lowpassed signal, median-filtered, then
/// segmented into notes on sustained pitch changes or unvoiced gaps.
fn track_bass(dec: &[f32], dec_sr: u32, onset_times: &[f32]) -> Vec<RawBassNote> {
    // Per-frame fractional MIDI + clarity; `None` = unvoiced.
    let mut frames: Vec<Option<(f32, f32)>> = Vec::new();
    let mut start = 0usize;
    while start + BASS_WIN <= dec.len() {
        let f = estimate_f0_in(
            &dec[start..start + BASS_WIN],
            dec_sr,
            BASS_F_LO,
            BASS_F_SEARCH_HI,
        )
        .filter(|f| f.clarity >= BASS_CLARITY_MIN && f.hz <= BASS_F_HI)
        .map(|f| (69.0 + 12.0 * (f.hz / 440.0).log2(), f.clarity));
        frames.push(f);
        start += BASS_HOP;
    }

    // Median filter (window 5) on the voiced values kills isolated octave
    // jumps without smearing real note changes.
    let midis: Vec<Option<f32>> = frames.iter().map(|f| f.map(|(m, _)| m)).collect();
    for (i, frame) in frames.iter_mut().enumerate() {
        if let Some((m, _)) = frame {
            let lo = i.saturating_sub(2);
            let hi = (i + 3).min(midis.len());
            let mut vals: Vec<f32> = midis[lo..hi].iter().flatten().copied().collect();
            vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            *m = vals[vals.len() / 2];
        }
    }

    let frame_s = |i: usize| -> f32 { (i * BASS_HOP) as f32 / dec_sr as f32 };
    let mut notes = Vec::new();
    let mut peaks: Vec<f32> = Vec::new();
    let mut i = 0usize;
    while i < frames.len() {
        let Some((midi, clar)) = frames[i] else {
            i += 1;
            continue;
        };
        let pitch = midi.round();
        let mut clar_sum = clar;
        let mut clar_n = 1usize;
        let mut last_voiced = i;
        let mut gap = 0usize;
        let mut j = i + 1;
        while j < frames.len() {
            match frames[j] {
                None => {
                    gap += 1;
                    if gap >= 2 {
                        break; // the note released
                    }
                }
                Some((m, c)) => {
                    gap = 0;
                    if (m.round() - pitch).abs() >= 1.0 {
                        // A new pitch must persist ~3 frames to end the note;
                        // shorter deviations are tracker glitches.
                        let target = m.round();
                        let persists = (j..j + 3).all(|k| {
                            frames
                                .get(k)
                                .and_then(|o| *o)
                                .is_some_and(|(mm, _)| (mm.round() - target).abs() < 0.5)
                        });
                        if persists {
                            break;
                        }
                    } else {
                        clar_sum += c;
                        clar_n += 1;
                        last_voiced = j;
                    }
                }
            }
            j += 1;
        }
        let mut start_s = frame_s(i);
        // Onsets are sharper than YIN frames: snap to the nearest one.
        if let Some(o) = onset_times
            .iter()
            .filter(|&&o| (o - start_s).abs() <= ONSET_SNAP_S)
            .min_by(|a, b| {
                (*a - start_s)
                    .abs()
                    .partial_cmp(&(*b - start_s).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            start_s = *o;
        }
        let seg = &dec[i * BASS_HOP..((last_voiced + 1) * BASS_HOP + BASS_WIN).min(dec.len())];
        let peak = seg.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
        notes.push(RawBassNote {
            pitch: pitch.clamp(0.0, 127.0) as u8,
            start_s,
            end_s: frame_s(last_voiced + 1),
            vel: 0, // filled below, needs the track-wide peak
            clarity: clar_sum / clar_n as f32,
        });
        peaks.push(peak);
        i = j.max(i + 1);
    }
    // Velocity: the loudest note maps to 110, the rest scale linearly.
    let track_max = peaks.iter().fold(0.0f32, |m, &v| m.max(v));
    for (n, &peak) in notes.iter_mut().zip(&peaks) {
        n.vel = if track_max > 0.0 {
            (40.0 + 70.0 * peak / track_max).round().clamp(1.0, 127.0) as u8
        } else {
            100
        };
    }
    notes
}

// --------------------------------------------------------------- drums ----

struct RawHit {
    class: &'static str,
    time_s: f32,
    strength: f32,
}

/// Band-limited flux detection: kick in the lows, snare in the mids
/// (confirmed by a coincident high-band rise — synth stabs are tonal),
/// hats in the highs (confirmed noisy). A snare suppresses coincident hat
/// candidates: its own noise burst explains the high band.
fn detect_drums(
    x: &[f32],
    sample_rate: u32,
    low_env: &[f32],
    fps: f32,
    bass: &[RawBassNote],
    dec: &[f32],
    dec_sr: u32,
) -> Vec<RawHit> {
    let snare_env = band_flux(x, sample_rate, SNARE_LO_HZ, SNARE_HI_HZ);
    let hat_env = band_flux(x, sample_rate, HAT_LO_HZ, HAT_HI_HZ);
    let coinc = (COINC_S * fps).ceil() as usize;

    let peak_near = |env: &[f32], i: usize| -> f32 {
        let lo = i.saturating_sub(coinc);
        let hi = (i + coinc + 1).min(env.len());
        env[lo..hi].iter().fold(0.0f32, |m, &v| m.max(v))
    };
    let stats = |env: &[f32]| -> (f32, f32) {
        if env.is_empty() {
            return (0.0, 0.0);
        }
        let mean = env.iter().sum::<f32>() / env.len() as f32;
        let var = env.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / env.len() as f32;
        (mean, var.sqrt())
    };
    let (hat_mean, hat_std) = stats(&hat_env);
    let hat_gate = hat_mean + 0.5 * hat_std;
    let max_of = |env: &[f32]| env.iter().fold(0.0f32, |m, &v| m.max(v));
    let (low_max, snare_max, hat_max) = (max_of(low_env), max_of(&snare_env), max_of(&hat_env));

    let mut hits: Vec<RawHit> = Vec::new();

    // Kick: low-band onsets, minus the ones the bass alone explains.
    let kick_max_env = low_max.max(1e-12);
    let mut last_kick = f32::NEG_INFINITY;
    for i in pick_onsets(low_env, fps) {
        let t = frame_time_s(i, sample_rate);
        let strength = low_env[i] / kick_max_env;
        if strength < MIN_STRENGTH || t - last_kick < CLASS_REFRACTORY_S {
            continue;
        }
        // Bass-explained means starting with a bass note *or* landing inside
        // a sounding one (a sustained low tone makes the flux wobble).
        let in_bass = bass
            .iter()
            .any(|n| t >= n.start_s - BASS_KICK_COINC_S && t <= n.end_s);
        if in_bass {
            // A bass note sounds here. A lone kick decays fast; a bass note
            // is pitched AND still ringing 150 ms later — drop the candidate
            // when both hold. A kick playing *under* the bass is dropped
            // too: amplitude cannot separate them in this band (documented
            // limitation — the low-onset grid survives in `bass`).
            let rms = |from_s: f32, to_s: f32| -> f32 {
                let from = ((from_s * dec_sr as f32) as usize).min(dec.len());
                let to = ((to_s * dec_sr as f32) as usize).min(dec.len());
                if to <= from {
                    return 0.0;
                }
                (dec[from..to].iter().map(|v| v * v).sum::<f32>() / (to - from) as f32).sqrt()
            };
            let attack = rms(t + 0.02, t + 0.08);
            let sustained = attack > 0.0 && rms(t + 0.15, t + 0.25) / attack > 0.5;
            let from = ((t + 0.06) * dec_sr as f32) as usize;
            let to = (((t + 0.12) * dec_sr as f32) as usize).min(dec.len());
            if sustained && to > from + 256 {
                let pitched = estimate_f0_in(&dec[from..to], dec_sr, BASS_F_LO, BASS_F_SEARCH_HI)
                    .is_some_and(|f| f.clarity > 0.6 && f.hz <= BASS_F_HI);
                if pitched {
                    continue;
                }
            }
        }
        last_kick = t;
        hits.push(RawHit {
            class: "kick",
            time_s: t,
            strength,
        });
    }

    // Snare: mid-band onsets with a coincident high-band rise.
    let mut snare_times: Vec<f32> = Vec::new();
    let snare_max_env = snare_max.max(1e-12);
    for i in pick_onsets(&snare_env, fps) {
        let strength = snare_env[i] / snare_max_env;
        let t = frame_time_s(i, sample_rate);
        if strength < MIN_STRENGTH
            || snare_times
                .last()
                .is_some_and(|&s| t - s < CLASS_REFRACTORY_S)
        {
            continue;
        }
        if peak_near(&hat_env, i) < hat_gate {
            continue; // tonal mid content (synth stab, bass harmonic)
        }
        snare_times.push(t);
        hits.push(RawHit {
            class: "snare",
            time_s: t,
            strength,
        });
    }

    // Hat: high-band onsets whose post-attack high end is noise, not
    // harmonics — and not already explained by a snare.
    let hat_max_env = hat_max.max(1e-12);
    let feature_len = (HAT_FEATURE_WIN_S * sample_rate as f32) as usize;
    let mut last_hat = f32::NEG_INFINITY;
    for i in pick_onsets(&hat_env, fps) {
        let strength = hat_env[i] / hat_max_env;
        let t = frame_time_s(i, sample_rate);
        if strength < MIN_STRENGTH || t - last_hat < CLASS_REFRACTORY_S {
            continue;
        }
        if snare_times.iter().any(|&s| (s - t).abs() <= COINC_S) {
            continue;
        }
        let from = (t * sample_rate as f32) as usize;
        let to = (from + feature_len).min(x.len());
        if to <= from + 256 {
            continue;
        }
        let Ok(power) = power_spectrum(&x[from..to]) else {
            continue;
        };
        let fft_len = (to - from).next_power_of_two();
        if spectral_flatness_above_db(&power, fft_len, sample_rate, 4000.0) < HAT_FLATNESS_MIN_DB {
            continue;
        }
        last_hat = t;
        hits.push(RawHit {
            class: "hat",
            time_s: t,
            strength,
        });
    }

    hits.sort_by(|a, b| {
        a.time_s
            .partial_cmp(&b.time_s)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits
}

// ------------------------------------------------------------- display ----

impl fmt::Display for Transcription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "== grid ==")?;
        write!(f, "bpm          {:.1}", self.grid.bpm)?;
        writeln!(
            f,
            "{}",
            if self.grid.bpm_overridden {
                "  (override)"
            } else {
                ""
            }
        )?;
        write!(f, "beat0        {:.3} s", self.grid.beat0_s)?;
        writeln!(
            f,
            "{}",
            if self.grid.offset_overridden {
                "  (override)"
            } else {
                ""
            }
        )?;
        writeln!(
            f,
            "signature    {}/{}   grid 1/{}",
            self.grid.time_signature[0], self.grid.time_signature[1], self.grid.subdivision
        )?;

        writeln!(f, "\n== key ==")?;
        match &self.key {
            Some(k) => writeln!(
                f,
                "{} {}  (confidence {:.2})",
                k.tonic, k.mode, k.confidence
            )?,
            None => writeln!(f, "none (silence or atonal)")?,
        }

        writeln!(f, "\n== bass ==  {} notes", self.bass.len())?;
        for n in &self.bass {
            writeln!(
                f,
                "{:<9} {:<4} len {:<8} vel {:>3}  clarity {:.2}  raw {:>8.3}s  err {:+}t",
                n.start, n.note, n.length, n.vel, n.clarity, n.raw_s, n.quant_error_ticks
            )?;
        }

        let counts = |class: &str| self.drums.iter().filter(|h| h.class == class).count();
        writeln!(
            f,
            "\n== drums ==  {} kicks, {} snares, {} hats",
            counts("kick"),
            counts("snare"),
            counts("hat")
        )?;
        for line in drum_grid_lines(&self.drums) {
            writeln!(f, "{line}")?;
        }

        writeln!(f, "\n== chords ==")?;
        for c in &self.chords {
            if c.chord == "N.C." {
                writeln!(f, "bar {:>3}  N.C.", c.bar)?;
            } else {
                writeln!(
                    f,
                    "bar {:>3}  {:<4} {:.2}  (alt {} {:.2})",
                    c.bar, c.chord, c.confidence, c.alt, c.alt_confidence
                )?;
            }
        }
        Ok(())
    }
}

/// One line per bar, 16 sixteenth cells grouped by beat: `K..h S..h …`.
/// Priority kick > snare > hat when several hits share a cell.
fn drum_grid_lines(drums: &[DrumHit]) -> Vec<String> {
    use crate::control::project::position_to_ticks;
    let mut last_bar = 0u64;
    let mut ticks: Vec<(u64, char)> = Vec::new();
    for h in drums {
        let Ok(t) = position_to_ticks(&h.start, TPBAR, TPBEAT) else {
            continue;
        };
        let c = match h.class.as_str() {
            "kick" => 'K',
            "snare" => 'S',
            _ => 'h',
        };
        ticks.push((t, c));
        last_bar = last_bar.max(t / TPBAR);
    }
    let rank = |c: char| match c {
        'K' => 0,
        'S' => 1,
        'h' => 2,
        _ => 3,
    };
    (0..=last_bar)
        .map(|bar| {
            let mut cells = ['.'; 16];
            for (t, c) in ticks.iter().filter(|(t, _)| t / TPBAR == bar) {
                let cell = ((t % TPBAR) / (TPBAR / 16)) as usize;
                if rank(*c) < rank(cells[cell]) {
                    cells[cell] = *c;
                }
            }
            let groups: Vec<String> = cells
                .chunks(4)
                .map(|g| g.iter().collect::<String>())
                .collect();
            format!("bar {:>3}  {}", bar + 1, groups.join(" "))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;
    const TAU: f32 = std::f32::consts::TAU;

    fn midi_hz(m: i32) -> f32 {
        440.0 * ((m - 69) as f32 / 12.0).exp2()
    }

    fn opts(bpm: f32, offset: f32) -> Options {
        Options {
            bpm: Some(bpm),
            offset_s: Some(offset),
            subdivision: 16,
        }
    }

    /// A bass note: sine + 30% second harmonic, 10 ms fades.
    fn bass_note(pitch: i32, secs: f32) -> Vec<f32> {
        let f = midi_hz(pitch);
        let n = (secs * SR as f32) as usize;
        let fade = SR as usize / 100;
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let env = if i < fade {
                    i as f32 / fade as f32
                } else if i + fade > n {
                    (n - i) as f32 / fade as f32
                } else {
                    1.0
                };
                ((TAU * f * t).sin() + 0.3 * (TAU * 2.0 * f * t).sin()) * 0.6 * env
            })
            .collect()
    }

    fn add_at(x: &mut [f32], at_s: f32, sig: &[f32]) {
        let from = (at_s * SR as f32) as usize;
        for (k, &v) in sig.iter().enumerate() {
            if from + k < x.len() {
                x[from + k] += v;
            }
        }
    }

    /// Soft kick: 55 Hz sine, 20 ms raised-cosine attack (keeps the spectral
    /// splash under 150 Hz), exponential decay.
    fn kick() -> Vec<f32> {
        let n = (0.15 * SR as f32) as usize;
        let attack = (0.02 * SR as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let a = if i < attack {
                    0.5 - 0.5 * (std::f32::consts::PI * i as f32 / attack as f32).cos()
                } else {
                    (-((i - attack) as f32) / (0.04 * SR as f32)).exp()
                };
                (TAU * 55.0 * t).sin() * 0.9 * a
            })
            .collect()
    }

    /// Deterministic differentiated white noise: high-tilted, near-zero lows.
    fn hp_noise(secs: f32, gain: f32) -> Vec<f32> {
        let n = (secs * SR as f32) as usize;
        let mut state = 0x2545_f491u32;
        let mut prev = 0.0f32;
        (0..n)
            .map(|i| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = (state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0;
                let d = v - prev;
                prev = v;
                let decay = (-(i as f32) / (n as f32 / 3.0)).exp();
                d * gain * decay
            })
            .collect()
    }

    /// Snare: 300 Hz body (10 ms fade-in keeps the attack splash above the
    /// kick band) + noise.
    fn snare() -> Vec<f32> {
        let n = (0.1 * SR as f32) as usize;
        let attack = (0.01 * SR as f32) as usize;
        let mut s: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let a = if i < attack {
                    0.5 - 0.5 * (std::f32::consts::PI * i as f32 / attack as f32).cos()
                } else {
                    (-((i - attack) as f32) / (0.03 * SR as f32)).exp()
                };
                (TAU * 300.0 * t).sin() * 0.5 * a
            })
            .collect();
        for (a, b) in s.iter_mut().zip(hp_noise(0.1, 0.4)) {
            *a += b;
        }
        s
    }

    fn hat() -> Vec<f32> {
        hp_noise(0.03, 0.5)
    }

    #[test]
    fn a_synthetic_bass_line_is_recovered() {
        // 8 notes on consecutive beats at 120 BPM (0.42 s + 80 ms gap).
        let pitches = [36, 36, 43, 41, 36, 36, 39, 38];
        let mut x = Vec::new();
        for &p in &pitches {
            x.extend(bass_note(p, 0.42));
            x.extend(std::iter::repeat_n(0.0, (0.08 * SR as f32) as usize));
        }
        x.extend(std::iter::repeat_n(0.0, SR as usize / 2));
        let t = transcribe(&x, SR, &opts(120.0, 0.0)).expect("transcription");
        let got: Vec<(i32, &str)> = t
            .bass
            .iter()
            .map(|n| (n.pitch as i32, n.start.as_str()))
            .collect();
        let want = [
            (36, "1.1.0"),
            (36, "1.2.0"),
            (43, "1.3.0"),
            (41, "1.4.0"),
            (36, "2.1.0"),
            (36, "2.2.0"),
            (39, "2.3.0"),
            (38, "2.4.0"),
        ];
        assert_eq!(got, want, "\nfull report:\n{t}");
        for n in &t.bass {
            assert!(
                n.quant_error_ticks.abs() < 120,
                "err {}",
                n.quant_error_ticks
            );
            assert!(n.clarity > 0.6, "clarity {}", n.clarity);
        }
    }

    #[test]
    fn a_drum_pattern_is_classified_and_placed() {
        // 2 bars at 120 BPM behind a 0.25 s pre-roll (an event at t = 0 sits
        // in the first STFT frame's blind spot): kick on 1 & 3, snare on
        // 2 & 4, hats on eighths.
        let pre = 0.25f32;
        let mut x = vec![0.0f32; SR as usize * 5];
        let (k, s, h) = (kick(), snare(), hat());
        for bar in 0..2 {
            let bar_t = pre + bar as f32 * 2.0;
            add_at(&mut x, bar_t, &k);
            add_at(&mut x, bar_t + 1.0, &k);
            add_at(&mut x, bar_t + 0.5, &s);
            add_at(&mut x, bar_t + 1.5, &s);
            for eighth in 0..8 {
                add_at(&mut x, bar_t + eighth as f32 * 0.25, &h);
            }
        }
        let t = transcribe(&x, SR, &opts(120.0, pre)).expect("transcription");
        let of = |class: &str| -> Vec<&str> {
            t.drums
                .iter()
                .filter(|d| d.class == class)
                .map(|d| d.start.as_str())
                .collect()
        };
        assert_eq!(
            of("kick"),
            ["1.1.0", "1.3.0", "2.1.0", "2.3.0"],
            "\nfull report:\n{t}"
        );
        assert_eq!(
            of("snare"),
            ["1.2.0", "1.4.0", "2.2.0", "2.4.0"],
            "\nfull report:\n{t}"
        );
        let hats = of("hat");
        // Hats on every eighth except under a snare (its noise burst already
        // explains the high band).
        for expected in ["1.1.480", "1.2.480", "1.3.0", "2.1.480", "2.4.480"] {
            assert!(hats.contains(&expected), "missing hat {expected}\n{t}");
        }
        for suppressed in ["1.2.0", "1.4.0", "2.2.0", "2.4.0"] {
            assert!(
                !hats.contains(&suppressed),
                "hat under snare {suppressed}\n{t}"
            );
        }
    }

    #[test]
    fn block_triads_read_as_a_chord_grid() {
        // 4 bars at 120 BPM: C, Am, F, G — root doubled an octave below.
        let bars = [
            [48, 60, 64, 67],
            [45, 57, 60, 64],
            [41, 53, 57, 60],
            [43, 55, 59, 62],
        ];
        let mut x = Vec::new();
        for bar in bars {
            let n = SR as usize * 2;
            x.extend((0..n).map(|i| {
                let t = i as f32 / SR as f32;
                bar.iter()
                    .map(|&m| (TAU * midi_hz(m) * t).sin())
                    .sum::<f32>()
                    * 0.2
            }));
        }
        let t = transcribe(&x, SR, &opts(120.0, 0.0)).expect("transcription");
        let labels: Vec<&str> = t.chords.iter().map(|c| c.chord.as_str()).collect();
        assert_eq!(labels, ["C", "Am", "F", "G"], "\nfull report:\n{t}");
    }

    #[test]
    fn grid_phase_is_recovered_from_a_prefixed_click_track() {
        // 1 kHz clicks at 120 BPM behind 0.25 s of silence, nothing else.
        let mut x = vec![0.0f32; SR as usize / 4];
        let n = SR as usize * 6;
        let period = SR as usize / 2;
        let mut grid = vec![0.0f32; n];
        let mut at = 0usize;
        while at < n {
            for k in 0..200.min(n - at) {
                let ph = TAU * 1000.0 * k as f32 / SR as f32;
                grid[at + k] += ph.sin() * 0.8 * (-(k as f32) / 60.0).exp();
            }
            at += period;
        }
        x.extend(grid);
        let t = transcribe(
            &x,
            SR,
            &Options {
                bpm: None,
                offset_s: None,
                subdivision: 16,
            },
        )
        .expect("transcription");
        assert!((t.grid.bpm - 120.0).abs() < 2.0, "bpm {}", t.grid.bpm);
        // beat0 must land on a click, whatever downbeat the tie-break picked.
        let rel = (t.grid.beat0_s - 0.25).rem_euclid(0.5);
        let dist = rel.min(0.5 - rel);
        assert!(dist < 0.015, "beat0 {} off the click grid", t.grid.beat0_s);
    }

    #[test]
    fn silence_fails_without_a_bpm_and_stays_empty_with_one() {
        let x = vec![0.0f32; SR as usize * 3];
        assert!(transcribe(&x, SR, &Options::default()).is_err());
        let t = transcribe(&x, SR, &opts(120.0, 0.0)).expect("transcription");
        assert!(t.bass.is_empty());
        assert!(t.drums.is_empty());
        assert!(t.chords.iter().all(|c| c.chord == "N.C."));
        assert!(t.key.is_none());
        let report = format!("{t}");
        assert!(!report.contains("NaN"), "{report}");
    }

    #[test]
    fn a_subdivision_that_does_not_divide_the_bar_is_rejected() {
        let x = vec![0.0f32; SR as usize];
        let bad = Options {
            bpm: Some(120.0),
            offset_s: Some(0.0),
            subdivision: 7,
        };
        assert!(transcribe(&x, SR, &bad).is_err());
    }
}
