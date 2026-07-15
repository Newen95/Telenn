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
    let mut graph = compile(
        &desc,
        registry::create,
        sample_rate as f32,
        MAX_BLOCK,
        crate::engine::CHANNELS,
    )?;
    let mut mixer = Mixer::new(
        sample_rate as f32,
        graph.track_mix(),
        graph.master_gain_db(),
    );
    let schedule = project.to_schedule(sample_rate, &index)?;

    let total = schedule.loop_len.saturating_mul(loops as u64) as usize;
    // The tail phase feeds no events but keeps the song's tempo: a synced
    // delay must not glide back to its ms fallback while ringing out.
    let samples_per_beat = schedule.samples_per_beat as f32;
    let mut out = Vec::with_capacity((total + tail_frames as usize) * 2);
    let mut cursor = SequencerCursor::new();
    pump(
        &mut graph,
        &mut mixer,
        &mut cursor,
        Some(&schedule),
        sample_rate,
        samples_per_beat,
        total,
        &mut out,
    );
    pump(
        &mut graph,
        &mut mixer,
        &mut cursor,
        None,
        sample_rate,
        samples_per_beat,
        tail_frames as usize,
        &mut out,
    );
    Ok(out)
}

/// Drive the graph + mixer for `total` frames, appending interleaved stereo
/// to `out`. `schedule = None` keeps processing without feeding events (the
/// tail phase — the cursor would wrap the loop and retrigger notes).
#[allow(clippy::too_many_arguments)]
fn pump(
    graph: &mut CompiledGraph,
    mixer: &mut Mixer,
    cursor: &mut SequencerCursor,
    schedule: Option<&Schedule>,
    sample_rate: u32,
    samples_per_beat: f32,
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
        graph.process_block(sample_rate as f32, n, samples_per_beat);
        mixer.mix(graph, sample_rate as f32, samples_per_beat, n, &mut stereo);
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
    fn master_chain_gain_scales_the_render() {
        // A -6.02 dB gain on the master chain halves every sample (once its
        // 10 ms param ramp has settled) — proof the chain actually processes
        // the mixed bus in both offline and RT paths.
        let project = demo();
        let base = render_samples(&project, 48_000, 1).unwrap();

        let mut mastered = project.clone();
        let gain_spec = crate::control::project::NodeSpec {
            id: "m_gain".into(),
            module: "gain".into(),
            // normalized for -6.0206 dB on the -60..+12 dB range
            params: [("gain".to_string(), (60.0 - 6.0206) / 72.0)]
                .into_iter()
                .collect(),
        };
        mastered.master_chain.push(gain_spec);
        let halved = render_samples(&mastered, 48_000, 1).unwrap();

        assert_eq!(base.len(), halved.len());
        let settled = 4_800; // past the 10 ms ramp
        let mut max_err = 0.0f32;
        for (b, h) in base[settled..].iter().zip(&halved[settled..]) {
            max_err = max_err.max((h - 0.5 * b).abs());
        }
        assert!(max_err < 1.0e-4, "master gain drifted: {max_err}");
    }

    #[test]
    fn render_matches_realtime_path_with_master_chain() {
        // The offline render and the audio thread share the staged mixer flow
        // (mix_tracks → process_master → apply_master), so with a master
        // compressor + gain their output stays bit-identical.
        let mut project = demo();
        for (i, module) in ["compressor", "gain"].iter().enumerate() {
            project
                .master_chain
                .push(crate::control::project::NodeSpec {
                    id: format!("m{i}"),
                    module: (*module).into(),
                    params: Default::default(),
                });
        }
        let offline = render_samples(&project, 48_000, 1).unwrap();
        let rt = crate::engine::rt::render_offline_via_rtstate(&project, 48_000, RENDER_BLOCK);
        assert_eq!(offline.len(), rt.len());
        assert_eq!(offline, rt, "master-chain paths diverged");
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
    fn stereo_render_actually_decorrelates() {
        // The demo carries a reverb: once the tap tables part ways, a centre-
        // panned render must NOT be dual-mono anymore — that is the point of
        // the stereo seam.
        let mut project = demo();
        for track in &mut project.tracks {
            track.mixer.pan = 0.0;
        }
        let samples = render_samples(&project, 48_000, 1).unwrap();
        let diverged = samples
            .chunks_exact(2)
            .any(|frame| (frame[0] - frame[1]).abs() > 1.0e-4);
        assert!(diverged, "stereo render is still dual-mono");
    }

    #[test]
    fn stereo_fold_matches_mono_render() {
        // Chorus and reverb are decorrelated so that the mono fold of their
        // stereo wet IS the legacy mono wet: folding a stereo render must
        // match a mono-compiled render of the same project to f32 rounding.
        // Two premises: nothing nonlinear sits after a decorrelating effect
        // in the demo's chains, and pans are centred — a non-zero pan weighs
        // L and R differently, which legitimately breaks fold equality on
        // decorrelated content.
        let mut project = demo();
        for track in &mut project.tracks {
            track.mixer.pan = 0.0;
        }
        let stereo = render_samples(&project, 48_000, 1).unwrap();

        let (desc, index) = project.to_graph().unwrap();
        let mut graph = compile(&desc, registry::create, 48_000.0, MAX_BLOCK, 1).unwrap();
        let mut mixer = Mixer::new(48_000.0, graph.track_mix(), graph.master_gain_db());
        let schedule = project.to_schedule(48_000, &index).unwrap();
        let total = schedule.loop_len as usize;
        let mut cursor = SequencerCursor::new();
        let mut mono = Vec::with_capacity(total * 2);
        pump(
            &mut graph,
            &mut mixer,
            &mut cursor,
            Some(&schedule),
            48_000,
            schedule.samples_per_beat as f32,
            total,
            &mut mono,
        );

        assert_eq!(stereo.len(), mono.len());
        let mut max_err = 0.0f32;
        for (frame, mono_frame) in stereo.chunks_exact(2).zip(mono.chunks_exact(2)) {
            let fold = 0.5 * (frame[0] + frame[1]);
            max_err = max_err.max((fold - mono_frame[0]).abs());
        }
        assert!(max_err < 1.0e-5, "mono fold drifted: {max_err}");
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

        // The centre L==R assertion needs a project with no decorrelating
        // effect, so strip every chain down to its instrument head.
        let mut dry = project.clone();
        for track in &mut dry.tracks {
            track.chain.truncate(1);
        }
        let centre = render_samples(&dry, 48_000, 1).unwrap();
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
