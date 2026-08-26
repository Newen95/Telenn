//! Control-side WAV loading shared by sample-based modules.
//!
//! Everything here runs from `configure`/`prepare` during graph build — never
//! on the audio thread. Handles 16/24/32-bit integer and float PCM, any
//! channel count (averaged to mono).

use std::io::Read;
use std::path::{Path, PathBuf};

/// Decode a WAV file into mono `f32` samples (channels averaged) plus its
/// sample rate. `None` on any read or format error — callers treat a bad file
/// like a missing one.
pub fn load_mono(path: &Path) -> Option<(Vec<f32>, f32)> {
    decode_mono(std::fs::File::open(path).ok()?)
}

/// Decode an in-memory WAV (embedded assets). Same contract as [`load_mono`].
pub fn decode_mono_bytes(bytes: &[u8]) -> Option<(Vec<f32>, f32)> {
    decode_mono(std::io::Cursor::new(bytes))
}

/// Resolve a config path against the project directory (absolute paths pass
/// through untouched).
pub fn resolve(base_dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base_dir.join(p)
    }
}

fn decode_mono(reader: impl Read) -> Option<(Vec<f32>, f32)> {
    let mut wav = hound::WavReader::new(reader).ok()?;
    let spec = wav.spec();
    let channels = spec.channels.max(1) as usize;
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => wav.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            // Full scale = 2^(bits-1) - 1, matching the historical embedded-kit
            // decode (`/ i16::MAX`) bit for bit on 16-bit files.
            let scale = 1.0 / ((1i64 << (spec.bits_per_sample - 1)) - 1) as f32;
            wav.samples::<i32>()
                .filter_map(Result::ok)
                .map(|s| s as f32 * scale)
                .collect()
        }
    };
    let mut mono = Vec::with_capacity(interleaved.len() / channels);
    for frame in interleaved.chunks_exact(channels) {
        mono.push(frame.iter().sum::<f32>() / channels as f32);
    }
    Some((mono, spec.sample_rate as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a 16-bit WAV with the given interleaved samples.
    fn write_wav(path: &Path, samples: &[f32], channels: u16, sr: u32) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for &s in samples {
            w.write_sample((s * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("telenn_wav_{}_{name}", std::process::id()))
    }

    #[test]
    fn stereo_averages_to_mono() {
        let path = temp_file("stereo.wav");
        // L = 0.8, R = 0.4 → mono 0.6.
        write_wav(&path, &[0.8, 0.4, 0.8, 0.4], 2, 44_100);
        let (mono, sr) = load_mono(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(sr, 44_100.0);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.6).abs() < 0.01, "got {}", mono[0]);
    }

    #[test]
    fn missing_file_is_none() {
        assert!(load_mono(Path::new("definitely/not/here.wav")).is_none());
    }

    #[test]
    fn resolve_joins_relative_only() {
        let base = Path::new("proj");
        assert_eq!(resolve(base, "samples/kit"), Path::new("proj/samples/kit"));
        let abs = std::env::temp_dir().join("kit");
        assert_eq!(resolve(base, abs.to_str().unwrap()), abs);
    }
}
