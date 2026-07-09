//! Control layer (regular threads): sequencer, project I/O and undo, MIDI input,
//! offline WAV export, graph construction and the project file watcher.
//!
//! Communicates with the audio thread only through lock-free ring buffers.

pub mod clip_edit;
pub mod commands;
pub mod graph_edit;
pub mod ids;
pub mod note_edit;
pub mod project;
pub mod render;
pub mod sequencer;
pub mod telemetry_pump;
pub mod undo;
pub mod watcher;

use std::path::{Path, PathBuf};

use crate::engine::graph::{compile, CompiledGraph};
use crate::engine::mix::TrackMix;
use crate::engine::node::{NodeIdx, ParamId};
use crate::engine::EngineError;
use crate::modules::registry;

use project::{GraphIndex, Project};
use sequencer::Schedule;

/// Absolute path of the editable `song.json` (the source of truth the UI writes
/// back to). Resolved from the crate dir so it is stable regardless of the
/// process working directory. Held in Tauri state for the edit commands.
pub struct ProjectFile(pub PathBuf);

/// Canonical location of the demo project on disk.
pub fn demo_song_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../projects/demo/song.json")
}

/// The demo project, embedded at build time so the engine always boots with a
/// playable sequence regardless of the working directory. The source of truth
/// is a tracked asset (`projects/` is gitignored user land): on first launch it
/// is materialized as `projects/demo/song.json` for the user to edit and
/// reload, and tests parse the embedded copy so a live edit session can never
/// break them.
const DEMO_SONG: &str = include_str!("../../assets/demo_song.json");

/// Compact, allocation-free messages sent from the control side to the audio
/// thread through an `rtrb` ring buffer. The callback drains all pending
/// messages at the start of each block before rendering.
///
/// `SwapGraph` (topology hot-swap) arrives at M4; M3 only swaps the schedule.
pub enum ControlMsg {
    Transport(TransportCmd),
    NoteOn {
        node: NodeIdx,
        pitch: u8,
        vel: u8,
    },
    NoteOff {
        node: NodeIdx,
        pitch: u8,
    },
    SetParam {
        node: NodeIdx,
        param: ParamId,
        value: f32,
    },
    /// Replace the playing sequence without interrupting sounding voices. The
    /// audio thread swaps the box in and returns the old one through the garbage
    /// ring to be dropped off the real-time thread.
    SetSchedule(Box<Schedule>),
    /// Replace the audio graph without interrupting playback (hot-swap). The
    /// audio thread moves persistent node instances into the new graph, fades
    /// out the old one (removed effects' tails), and returns it through the
    /// garbage ring.
    SwapGraph(Box<CompiledGraph>),
    /// Live mixer change for one track (fader/pan drag): retargets the strip's
    /// smoothers, no recompile, no swap.
    SetTrackMix {
        track: usize,
        mix: TrackMix,
    },
    /// Live master gain change (dB).
    SetMasterGain {
        gain_db: f32,
    },
    /// Metronome toggle + volume (session state, never persisted to the song).
    SetMetronome {
        enabled: bool,
        volume: f32,
    },
}

#[derive(Clone, Copy)]
pub enum TransportCmd {
    Play,
    Stop,
    /// Move the playhead to an absolute sample position.
    Seek(u64),
}

/// Boxed graph/schedule factory: compiles both once the real sample rate is
/// known, on the audio thread. Returning a `Box` keeps `AudioEngine::start`
/// independent of the project layer.
pub type EngineInit =
    Box<dyn FnOnce(f32, usize) -> Result<(CompiledGraph, Schedule), EngineError> + Send>;

/// Parse the embedded demo project. Deterministic (independent of any file on
/// disk), so tests can rely on it as a stable fixture.
pub fn demo_project() -> Result<project::Project, project::ProjectError> {
    Ok(serde_json::from_str(DEMO_SONG)?)
}

/// Build the engine initialiser for a project: the graph topology is computed
/// here (sample-rate independent); the closure finishes compilation and builds
/// the schedule once the device sample rate is known.
pub fn build_engine_init(project: Project) -> Result<(EngineInit, GraphIndex), EngineError> {
    let (desc, index) = project.to_graph()?;
    let index_for_engine = index.clone();
    let init: EngineInit = Box::new(move |sample_rate, max_block| {
        let graph = compile(&desc, registry::create, sample_rate, max_block, 1)?;
        let schedule = project.to_schedule(sample_rate as u32, &index)?;
        Ok((graph, schedule))
    });
    Ok((init, index_for_engine))
}

/// Build the engine initialiser plus the control-side state kept for later
/// commands: the note-routing index (`load_project`) and the editable project
/// (graph edits / undo).
pub fn build_demo() -> Result<(EngineInit, GraphIndex, Project, PathBuf), EngineError> {
    let path = demo_song_path();
    // Materialize the embedded demo on first launch (or fresh clone) so the
    // user has an editable file. Best effort: booting from the embedded copy
    // still works if the write fails.
    if !path.exists() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, DEMO_SONG);
    }
    // Load the editable file so UI edits round-trip; fall back to the embedded
    // copy if it is unreadable, so the app always boots.
    let mut project = Project::load(&path).or_else(|_| demo_project())?;
    // Assign ids to any id-less note/clip and rewrite the file (best effort: a
    // write failure must not stop the app from starting).
    if ids::normalize(&mut project) && path.exists() {
        let _ = project.save(&path);
    }

    let project_for_engine = project.clone();
    let (init, index) = build_engine_init(project)?;
    Ok((init, index, project_for_engine, path))
}
