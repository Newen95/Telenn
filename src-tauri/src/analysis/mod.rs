//! Offline feature extraction for rendered audio.
//!
//! Everything here runs off the real-time thread (allocation and I/O are
//! fine) and operates on plain sample slices — nothing is imported from
//! `engine/`. Consumers: the verification CLIs under `tools/` and sound
//! non-regression tests asserting spectral properties.

pub mod bands;
pub mod chroma;
pub mod envelope;
pub mod fft;
pub mod levels;
pub mod loudness;
pub mod pitch;
pub mod png;
pub mod report;
pub mod rhythm;
pub mod spectral;
pub mod spectrogram;
pub mod transcribe;

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    #[error("cannot read wav: {0}")]
    Wav(String),
    #[error("bad input: {0}")]
    BadInput(String),
    #[error("io: {0}")]
    Io(String),
}

/// Deinterleaved audio ready for analysis.
pub struct Signal {
    pub left: Vec<f32>,
    /// Same content as `left` for mono sources.
    pub right: Vec<f32>,
    pub sample_rate: u32,
}

impl Signal {
    /// Split an interleaved stereo buffer (L,R per frame) into channels.
    pub fn from_interleaved(samples: &[f32], sample_rate: u32) -> Self {
        let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
        let right: Vec<f32> = samples.iter().skip(1).step_by(2).copied().collect();
        Self {
            left,
            right,
            sample_rate,
        }
    }

    /// Load a 1- or 2-channel WAV (16/24/32-bit int or 32-bit float).
    pub fn from_wav(path: impl AsRef<Path>) -> Result<Self, AnalysisError> {
        let mut reader =
            hound::WavReader::open(path).map_err(|e| AnalysisError::Wav(e.to_string()))?;
        let spec = reader.spec();
        if spec.channels == 0 || spec.channels > 2 {
            return Err(AnalysisError::Wav(format!(
                "unsupported channel count {}",
                spec.channels
            )));
        }

        let samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Float, 32) => reader
                .samples::<f32>()
                .collect::<Result<_, _>>()
                .map_err(|e| AnalysisError::Wav(e.to_string()))?,
            (hound::SampleFormat::Int, bits) if bits <= 32 => {
                let scale = 1.0 / (1i64 << (bits - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 * scale))
                    .collect::<Result<_, _>>()
                    .map_err(|e| AnalysisError::Wav(e.to_string()))?
            }
            (format, bits) => {
                return Err(AnalysisError::Wav(format!(
                    "unsupported sample format {format:?}/{bits}"
                )))
            }
        };

        if spec.channels == 1 {
            Ok(Self {
                right: samples.clone(),
                left: samples,
                sample_rate: spec.sample_rate,
            })
        } else {
            Ok(Self::from_interleaved(&samples, spec.sample_rate))
        }
    }

    /// Mono sum `(L+R)/2`.
    pub fn mono(&self) -> Vec<f32> {
        self.left
            .iter()
            .zip(&self.right)
            .map(|(l, r)| (l + r) * 0.5)
            .collect()
    }

    pub fn duration_s(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.left.len() as f32 / self.sample_rate as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaved_splits_channels() {
        let s = Signal::from_interleaved(&[0.1, -0.2, 0.3, -0.4], 48_000);
        assert_eq!(s.left, vec![0.1, 0.3]);
        assert_eq!(s.right, vec![-0.2, -0.4]);
        for (got, want) in s.mono().iter().zip([-0.05f32, -0.05]) {
            assert!((got - want).abs() < 1e-6, "{got} vs {want}");
        }
    }

    #[test]
    fn wav_roundtrip_via_hound() {
        let path = std::env::temp_dir().join("telenn_analysis_roundtrip.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..64 {
            let v = (i as f32 / 64.0 * std::f32::consts::TAU).sin() * 0.5;
            writer.write_sample((v * i16::MAX as f32) as i16).unwrap();
            writer.write_sample((v * i16::MAX as f32) as i16).unwrap();
        }
        writer.finalize().unwrap();

        let s = Signal::from_wav(&path).unwrap();
        assert_eq!(s.sample_rate, 48_000);
        assert_eq!(s.left.len(), 64);
        assert!((s.left[8] - s.right[8]).abs() < 1e-6);
        std::fs::remove_file(&path).ok();
    }
}
