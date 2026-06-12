//! Offline WAV render: deroulement of a project entirely off the real-time
//! thread, into a deterministic mono buffer and a `hound` WAV file.
//!
//! The render shares the exact sequencing logic of the audio thread (the
//! [`SequencerCursor`]) so an export can never drift from live playback. Being
//! device-free and fixed-rate, two renders of the same project are bit-for-bit
//! identical.

use std::path::Path;

use crate::control::project::{Project, ProjectError};
use crate::control::sequencer::SequencerCursor;
use crate::engine::graph::{compile, GraphError};
use crate::engine::MAX_BLOCK;
use crate::modules::registry;

/// Block size for the offline deroulement. Fixed, so the result is deterministic.
const RENDER_BLOCK: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error("wav write failed: {0}")]
    Wav(String),
}

/// Render `loops` iterations of the project's loop into a mono sample buffer.
/// Deterministic for a given `(project, sample_rate, loops)`.
pub fn render_samples(
    project: &Project,
    sample_rate: u32,
    loops: u32,
) -> Result<Vec<f32>, RenderError> {
    let (desc, index) = project.to_graph()?;
    let mut graph = compile(&desc, registry::create, sample_rate as f32, MAX_BLOCK)?;
    let schedule = project.to_schedule(sample_rate, &index)?;

    let total = schedule.loop_len.saturating_mul(loops as u64) as usize;
    let mut out = Vec::with_capacity(total);
    let mut cursor = SequencerCursor::new();
    let mut produced = 0usize;
    while produced < total {
        let n = (total - produced).min(RENDER_BLOCK);
        {
            let g = &mut graph;
            cursor.advance(n, &schedule, |node, ev| g.push_note(node, ev));
        }
        graph.process_block(sample_rate as f32, n);
        out.extend_from_slice(graph.output());
        graph.clear_notes();
        produced += n;
    }
    Ok(out)
}

/// Render the project and write it as a 16-bit mono PCM WAV.
pub fn render_to_wav(
    project: &Project,
    sample_rate: u32,
    loops: u32,
    path: impl AsRef<Path>,
) -> Result<(), RenderError> {
    let samples = render_samples(project, sample_rate, loops)?;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| RenderError::Wav(e.to_string()))?;
    for &s in &samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(v)
            .map_err(|e| RenderError::Wav(e.to_string()))?;
    }
    writer
        .finalize()
        .map_err(|e| RenderError::Wav(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Project {
        super::super::demo_project().expect("demo parses")
    }

    #[test]
    fn render_is_deterministic() {
        let a = render_samples(&demo(), 48_000, 1).unwrap();
        let b = render_samples(&demo(), 48_000, 1).unwrap();
        assert_eq!(a.len(), b.len());
        assert!(!a.is_empty(), "demo render is empty");
        assert_eq!(a, b, "two renders of the same project differ");
    }

    #[test]
    fn render_matches_realtime_path() {
        // The offline render and the audio thread share the SequencerCursor, so
        // at the same sample rate and block size their mono output is identical.
        // This pins "no logic duplicated with real time" (MVP_PLAN M3).
        let project = demo();
        let offline = render_samples(&project, 48_000, 1).unwrap();

        let rt = crate::engine::rt::render_offline_via_rtstate(&project, 48_000, RENDER_BLOCK);
        assert_eq!(offline.len(), rt.len());
        assert_eq!(offline, rt, "offline and real-time paths diverged");
    }
}
