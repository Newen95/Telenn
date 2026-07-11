//! STFT spectrogram rendered to a log-frequency PNG — the closest thing to
//! "looking at" a sound: attack, harmonic tails, noise and resonances are
//! all visible at a glance.

use std::path::Path;

use super::fft::{fft, hann};
use super::{png, AnalysisError};

pub struct SpectrogramConfig {
    /// STFT window (power of two).
    pub win: usize,
    /// Hop between frames, samples.
    pub hop: usize,
    /// Image height: log-frequency rows from `f_min` up to Nyquist.
    pub rows: usize,
    /// Bottom of the frequency axis (A0 by default).
    pub f_min: f32,
    /// Dynamic range floor relative to the loudest cell, dB.
    pub db_floor: f32,
    /// Columns are max-pooled down to this width for long signals.
    pub max_cols: usize,
}

impl Default for SpectrogramConfig {
    fn default() -> Self {
        Self {
            win: 2048,
            hop: 512,
            rows: 384,
            f_min: 27.5,
            db_floor: -90.0,
            max_cols: 2000,
        }
    }
}

/// Render the spectrogram of `x` into a PNG at `path`.
pub fn render_png(
    x: &[f32],
    sample_rate: u32,
    cfg: &SpectrogramConfig,
    path: impl AsRef<Path>,
) -> Result<(), AnalysisError> {
    if x.is_empty() {
        return Err(AnalysisError::BadInput("empty signal".into()));
    }
    if sample_rate == 0 || cfg.rows < 2 || cfg.hop == 0 || !cfg.win.is_power_of_two() {
        return Err(AnalysisError::BadInput("bad spectrogram config".into()));
    }

    // Row → fractional FFT bin mapping (log axis, f_min at the bottom row).
    let nyquist = sample_rate as f32 / 2.0;
    let f_min = cfg.f_min.min(nyquist / 4.0).max(1.0);
    let ratio = nyquist / f_min;
    let row_bin: Vec<f32> = (0..cfg.rows)
        .map(|r| {
            let freq = f_min * ratio.powf(r as f32 / (cfg.rows - 1) as f32);
            freq * cfg.win as f32 / sample_rate as f32
        })
        .collect();

    // STFT columns, mapped to rows immediately (keeps memory at rows×cols).
    let window = hann(cfg.win);
    let n_frames = if x.len() >= cfg.win {
        (x.len() - cfg.win) / cfg.hop + 1
    } else {
        1
    };
    let mut columns: Vec<Vec<f32>> = Vec::with_capacity(n_frames);
    let mut re = vec![0.0f32; cfg.win];
    let mut im = vec![0.0f32; cfg.win];
    for frame in 0..n_frames {
        let start = frame * cfg.hop;
        re.iter_mut().for_each(|v| *v = 0.0);
        im.iter_mut().for_each(|v| *v = 0.0);
        for (i, w) in window.iter().enumerate() {
            if let Some(&s) = x.get(start + i) {
                re[i] = s * w;
            }
        }
        fft(&mut re, &mut im)?;
        let bins: Vec<f32> = (0..cfg.win / 2 + 1)
            .map(|i| re[i] * re[i] + im[i] * im[i])
            .collect();

        // One value per row: max over the bin span the row covers (preserves
        // peaks where the log axis compresses many bins into one row),
        // interpolation where a row is finer than a bin.
        let column: Vec<f32> = (0..cfg.rows)
            .map(|r| {
                let centre = row_bin[r];
                let lo = if r > 0 {
                    (row_bin[r - 1] + centre) / 2.0
                } else {
                    centre
                };
                let hi = if r + 1 < cfg.rows {
                    (row_bin[r + 1] + centre) / 2.0
                } else {
                    centre
                };
                let (lo_i, hi_i) = (lo.ceil() as usize, hi.floor() as usize);
                if lo_i <= hi_i && hi_i < bins.len() {
                    bins[lo_i..=hi_i].iter().fold(0.0f32, |m, &p| m.max(p))
                } else {
                    let base = (centre.floor() as usize).min(bins.len() - 2);
                    let frac = (centre - base as f32).clamp(0.0, 1.0);
                    bins[base] * (1.0 - frac) + bins[base + 1] * frac
                }
            })
            .collect();
        columns.push(column);
    }

    // Max-pool columns down to the image width.
    let cols = columns.len().min(cfg.max_cols);
    let pooled: Vec<Vec<f32>> = (0..cols)
        .map(|c| {
            let lo = c * columns.len() / cols;
            let hi = ((c + 1) * columns.len() / cols).max(lo + 1);
            (0..cfg.rows)
                .map(|r| columns[lo..hi].iter().fold(0.0f32, |m, col| m.max(col[r])))
                .collect()
        })
        .collect();

    // dB relative to the hottest cell, clamped to the floor, then coloured.
    let max_power = pooled
        .iter()
        .flatten()
        .fold(f32::MIN_POSITIVE, |m, &p| m.max(p));
    let mut pixels = vec![0u8; cols * cfg.rows * 3];
    for (c, column) in pooled.iter().enumerate() {
        for (r, &p) in column.iter().enumerate() {
            let db = 10.0 * (p.max(1e-30) / max_power).log10();
            let t = ((db - cfg.db_floor) / -cfg.db_floor).clamp(0.0, 1.0);
            let (red, green, blue) = colour(t);
            // Row 0 of the image is the top = highest frequency.
            let y = cfg.rows - 1 - r;
            let idx = (y * cols + c) * 3;
            pixels[idx] = red;
            pixels[idx + 1] = green;
            pixels[idx + 2] = blue;
        }
    }
    png::write_rgb(path, cols as u32, cfg.rows as u32, &pixels)
}

/// 5-stop colour ramp: black → deep blue → magenta → orange → white.
fn colour(t: f32) -> (u8, u8, u8) {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.0, [0.0, 0.0, 0.0]),
        (0.25, [25.0, 35.0, 110.0]),
        (0.5, [170.0, 40.0, 150.0]),
        (0.75, [240.0, 150.0, 50.0]),
        (1.0, [255.0, 255.0, 255.0]),
    ];
    let t = t.clamp(0.0, 1.0);
    for pair in STOPS.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
            return (
                (c0[0] + (c1[0] - c0[0]) * f) as u8,
                (c0[1] + (c1[1] - c0[1]) * f) as u8,
                (c0[2] + (c1[2] - c0[2]) * f) as u8,
            );
        }
    }
    (255, 255, 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_valid_png() {
        let sr = 48_000u32;
        let x: Vec<f32> = (0..sr as usize)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin() * 0.5)
            .collect();
        let path = std::env::temp_dir().join("telenn_spectrogram_test.png");
        render_png(&x, sr, &SpectrogramConfig::default(), &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
        // 1 s at hop 512 ≈ 90 frames wide, 384 rows tall.
        let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        assert!(w > 50 && w < 120, "width {w}");
        assert_eq!(h, 384);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn colour_ramp_is_monotonic_in_brightness() {
        let lum = |t: f32| {
            let (r, g, b) = colour(t);
            0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
        };
        assert!(lum(0.0) < lum(0.3));
        assert!(lum(0.3) < lum(0.7));
        assert!(lum(0.7) < lum(1.0));
    }
}
