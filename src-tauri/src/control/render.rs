//! Offline WAV render: deroulement of a project entirely off the real-time
//! thread, into a deterministic stereo buffer and a `hound` WAV file.
//!
//! The render shares the exact sequencing logic of the audio thread (the
//! [`SequencerCursor`]) *and* the same [`Mixer`] stage, so an export honours
//! gain/pan/master and can never drift from live playback. Being device-free
//! and fixed-rate, two renders of the same project are bit-for-bit identical.

use std::path::Path;

use crate::control::project::{Project, ProjectError};
use crate::control::sequencer::{Schedule, SequencerCursor};
use crate::engine::graph::{compile, CompiledGraph, GraphError};
use crate::engine::mix::Mixer;
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

/// Render `loops` iterations of the project's loop into an interleaved stereo
/// sample buffer (L,R per frame), through the same `Mixer` stage as the audio
/// thread. Deterministic for a given `(project, sample_rate, loops)`.
pub fn render_samples(
    project: &Project,
    sample_rate: u32,
    loops: u32,
) -> Result<Vec<f32>, RenderError> {
    render_samples_with_tail(project, sample_rate, loops, 0)
}

/// Like [`render_samples`], then keep processing `tail_frames` more frames
/// with no scheduled events, so releases and effect tails (delay, reverb)
/// ring out instead of being cut at the loop boundary. A note still held at
/// the end of the last loop never receives its note-off and keeps sounding
/// through the tail — probes should place note-offs inside the loop.
pub fn render_samples_with_tail(
    project: &Project,
    sample_rate: u32,
    loops: u32,
    tail_frames: u64,
) -> Result<Vec<f32>, RenderError> {
    let (desc, index) = project.to_graph()?;
    let mut graph = compile(&desc, registry::create, sample_rate as f32, MAX_BLOCK, 1)?;
    let mut mixer = Mixer::new(
        sample_rate as f32,
        graph.track_mix(),
        graph.master_gain_db(),
    );
    let schedule = project.to_schedule(sample_rate, &index)?;

    let total = schedule.loop_len.saturating_mul(loops as u64) as usize;
    let mut out = Vec::with_capacity((total + tail_frames as usize) * 2);
    let mut cursor = SequencerCursor::new();
    pump(
        &mut graph,
        &mut mixer,
        &mut cursor,
        Some(&schedule),
        sample_rate,
        total,
        &mut out,
    );
    pump(
        &mut graph,
        &mut mixer,
        &mut cursor,
        None,
        sample_rate,
        tail_frames as usize,
        &mut out,
    );
    Ok(out)
}

/// Drive the graph + mixer for `total` frames, appending interleaved stereo
/// to `out`. `schedule = None` keeps processing without feeding events (the
/// tail phase — the cursor would wrap the loop and retrigger notes).
fn pump(
    graph: &mut CompiledGraph,
    mixer: &mut Mixer,
    cursor: &mut SequencerCursor,
    schedule: Option<&Schedule>,
    sample_rate: u32,
    total: usize,
    out: &mut Vec<f32>,
) {
    let mut stereo = vec![0.0f32; RENDER_BLOCK * 2];
    let mut produced = 0usize;
    while produced < total {
        let n = (total - produced).min(RENDER_BLOCK);
        if let Some(schedule) = schedule {
            let g = &mut *graph;
            cursor.advance(n, schedule, |node, ev| g.push_note(node, ev));
        }
        graph.process_block(sample_rate as f32, n);
        mixer.mix(graph, n, &mut stereo);
        out.extend_from_slice(&stereo[..n * 2]);
        graph.clear_notes();
        produced += n;
    }
}

/// Write an interleaved stereo buffer as a 16-bit PCM WAV.
pub fn write_wav(
    samples: &[f32],
    sample_rate: u32,
    path: impl AsRef<Path>,
) -> Result<(), RenderError> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(path, spec).map_err(|e| RenderError::Wav(e.to_string()))?;
    for &s in samples {
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

/// Render the project and write it as a 16-bit stereo PCM WAV.
pub fn render_to_wav(
    project: &Project,
    sample_rate: u32,
    loops: u32,
    path: impl AsRef<Path>,
) -> Result<(), RenderError> {
    let samples = render_samples(project, sample_rate, loops)?;
    write_wav(&samples, sample_rate, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Project {
        super::super::demo_project().expect("demo parses")
    }

    #[test]
    fn tail_render_extends_without_events() {
        // tail 0 is bit-identical to the plain render; a tail adds exactly
        // the requested frames and stays finite (no NaN, no runaway level).
        let base = render_samples(&demo(), 48_000, 1).unwrap();
        let with_zero_tail = render_samples_with_tail(&demo(), 48_000, 1, 0).unwrap();
        assert_eq!(base, with_zero_tail);

        let tail = 4_800u64;
        let extended = render_samples_with_tail(&demo(), 48_000, 1, tail).unwrap();
        assert_eq!(extended.len(), base.len() + tail as usize * 2);
        assert_eq!(&extended[..base.len()], &base[..]);
        let tail_peak = extended[base.len()..]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(
            tail_peak.is_finite() && tail_peak <= 1.0,
            "tail peak {tail_peak}"
        );
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
        // The offline render and the audio thread share the SequencerCursor AND
        // the Mixer stage, so at the same sample rate and block size their
        // interleaved stereo output is identical. This pins "no logic duplicated
        // with real time" (MVP_PLAN M3), now including gain/pan/master.
        let project = demo();
        let offline = render_samples(&project, 48_000, 1).unwrap();

        let rt = crate::engine::rt::render_offline_via_rtstate(&project, 48_000, RENDER_BLOCK);
        assert_eq!(offline.len(), rt.len());
        assert_eq!(offline, rt, "offline and real-time paths diverged");
    }

    #[test]
    fn stereo_render_has_no_clip_and_no_dc() {
        let samples = render_samples(&demo(), 48_000, 1).unwrap();
        assert_eq!(samples.len() % 2, 0, "stereo output must be interleaved");
        assert!(!samples.is_empty());

        let peak = samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak <= 1.0, "render clips: peak {peak}");

        // Per-channel DC offset stays negligible relative to the signal. The
        // bound tolerates the low-frequency leakage of a 55 Hz bass over a
        // finite render (truncated periods bias the mean by ~1e-3) while still
        // catching a real stuck offset (0.5% FS).
        let frames = (samples.len() / 2) as f32;
        let dc_l: f32 = samples.iter().step_by(2).sum::<f32>() / frames;
        let dc_r: f32 = samples.iter().skip(1).step_by(2).sum::<f32>() / frames;
        assert!(dc_l.abs() < 5e-3, "left channel DC offset {dc_l}");
        assert!(dc_r.abs() < 5e-3, "right channel DC offset {dc_r}");
    }

    #[test]
    fn pan_routes_a_track_between_channels() {
        // The demo's stored mixer values are whatever the user last saved, so
        // every setting under test is forced explicitly here. Hard-pan the two
        // tracks apart: each channel must carry energy and the channels differ;
        // recentred, both channels are identical (within f32 rounding).
        let mut project = demo();
        project.master_gain_db = 0.0;
        for track in &mut project.tracks {
            track.mixer = crate::engine::mix::TrackMix::default();
        }
        let mut panned = project.clone();
        panned.tracks[0].mixer.pan = -1.0;
        if panned.tracks.len() > 1 {
            panned.tracks[1].mixer.pan = 1.0;
        }

        let samples = render_samples(&panned, 48_000, 1).unwrap();
        let energy_l: f32 = samples.iter().step_by(2).map(|s| s * s).sum();
        let energy_r: f32 = samples.iter().skip(1).step_by(2).map(|s| s * s).sum();
        assert!(energy_l > 0.0, "hard-left track vanished");
        assert!(energy_r > 0.0, "hard-right track vanished");

        let centre = render_samples(&project, 48_000, 1).unwrap();
        for frame in centre.chunks_exact(2) {
            assert!(
                (frame[0] - frame[1]).abs() < 1.0e-6,
                "centre pan differs between channels: {} vs {}",
                frame[0],
                frame[1]
            );
        }
        let (l, r): (Vec<f32>, Vec<f32>) = (
            samples.iter().step_by(2).copied().collect(),
            samples.iter().skip(1).step_by(2).copied().collect(),
        );
        assert_ne!(l, r, "hard pans must separate the channels");
    }
}
