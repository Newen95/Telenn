//! Tauri commands: the typed UI → control entry points.

use std::sync::Mutex;

use tauri::State;

use crate::control::project::{Project, ProjectView};
use crate::control::sequencer::Schedule;
use crate::control::undo::EditHistory;
use crate::control::{
    clip_edit, graph_edit, note_edit, render, ControlMsg, ProjectFile, TransportCmd,
};
use crate::engine::graph::{compile, GraphDesc};
use crate::engine::node::ParamId;
use crate::engine::{AudioEngine, MAX_BLOCK};
use crate::modules::registry;

/// Fixed sample rate for offline export: device-independent, so an exported WAV
/// is reproducible on any machine.
const EXPORT_SAMPLE_RATE: u32 = 48_000;
/// Loop iterations written by an export.
const EXPORT_LOOPS: u32 = 2;

#[tauri::command]
pub fn transport_play(engine: State<'_, AudioEngine>) {
    engine.send(ControlMsg::Transport(TransportCmd::Play));
}

#[tauri::command]
pub fn transport_stop(engine: State<'_, AudioEngine>) {
    engine.send(ControlMsg::Transport(TransportCmd::Stop));
}

/// Reload a project from disk and swap its sequence in without interrupting
/// playback. The graph topology stays fixed (a chain change needs a restart
/// until the M4 hot-swap): notes are resolved against the running graph.
#[tauri::command]
pub fn load_project(engine: State<'_, AudioEngine>, path: String) -> Result<(), String> {
    let project = Project::load(&path).map_err(|e| e.to_string())?;
    let schedule = project
        .to_schedule(engine.sample_rate(), engine.index())
        .map_err(|e| e.to_string())?;
    if !engine.send(ControlMsg::SetSchedule(Box::new(schedule))) {
        return Err("engine busy: command dropped".into());
    }
    Ok(())
}

/// Render a project to a 16-bit mono WAV offline (deterministic, device-free).
#[tauri::command]
pub fn export_wav(project_path: String, out_path: String) -> Result<(), String> {
    let project = Project::load(&project_path).map_err(|e| e.to_string())?;
    render::render_to_wav(&project, EXPORT_SAMPLE_RATE, EXPORT_LOOPS, &out_path)
        .map_err(|e| e.to_string())
}

/// Compile a graph description and hot-swap it onto the audio thread.
fn swap_to(engine: &AudioEngine, desc: &GraphDesc) -> Result<(), String> {
    let graph = compile(
        desc,
        registry::create,
        engine.sample_rate() as f32,
        MAX_BLOCK,
    )
    .map_err(|e| e.to_string())?;
    if !engine.send(ControlMsg::SwapGraph(Box::new(graph))) {
        return Err("engine busy: command dropped".into());
    }
    Ok(())
}

/// Build a project's schedule against the running graph's note routing.
fn schedule_for(engine: &AudioEngine, project: &Project) -> Result<Schedule, String> {
    project
        .to_schedule(engine.sample_rate(), engine.index())
        .map_err(|e| e.to_string())
}

/// Replace the playing sequence on the audio thread (no topology change).
fn set_schedule(engine: &AudioEngine, schedule: Schedule) -> Result<(), String> {
    if !engine.send(ControlMsg::SetSchedule(Box::new(schedule))) {
        return Err("engine busy: command dropped".into());
    }
    Ok(())
}

/// Persist the current project back to `song.json` (the source of truth). Best
/// effort: a write failure does not undo the edit already applied to the engine,
/// it just leaves the disk mirror to be refreshed by the next edit.
fn persist(history: &EditHistory, file: &ProjectFile) {
    let _ = history.project().save(&file.0);
}

/// Apply a validated **graph** edit (effect chain) to a trial clone and hot-swap
/// the resulting graph. A rejected edit leaves the project and audio untouched.
fn edit_and_swap(
    engine: &AudioEngine,
    history: &Mutex<EditHistory>,
    file: &ProjectFile,
    edit: impl FnOnce(&mut Project) -> Result<(), graph_edit::GraphEditError>,
) -> Result<(), String> {
    let mut history = history.lock().map_err(|_| "edit state poisoned")?;
    let mut trial = history.project().clone();
    edit(&mut trial).map_err(|e| e.to_string())?;
    let (desc, _index) = trial.to_graph().map_err(|e| e.to_string())?;
    swap_to(engine, &desc)?;
    history.commit(trial);
    persist(&history, file);
    Ok(())
}

/// Apply a validated **content** edit (notes/clips) to a trial clone and hot-swap
/// the rebuilt schedule. The topology is unchanged, so notes resolve against the
/// running graph and held voices are not cut. Returns the closure's value.
fn edit_schedule<T, E: std::fmt::Display>(
    engine: &AudioEngine,
    history: &Mutex<EditHistory>,
    file: &ProjectFile,
    edit: impl FnOnce(&mut Project) -> Result<T, E>,
) -> Result<T, String> {
    let mut history = history.lock().map_err(|_| "edit state poisoned")?;
    let mut trial = history.project().clone();
    let out = edit(&mut trial).map_err(|e| e.to_string())?;
    let schedule = schedule_for(engine, &trial)?;
    set_schedule(engine, schedule)?;
    history.commit(trial);
    persist(&history, file);
    Ok(out)
}

/// Re-apply a project state restored by undo/redo, sending the **minimal** audio
/// messages: a graph swap only if the topology changed, a schedule swap only if
/// the sequence changed. Avoids any spurious declick or note re-trigger when the
/// step only touched notes (or only the graph).
fn apply_restored(engine: &AudioEngine, prev: &Project, restored: &Project) -> Result<(), String> {
    let prev_desc = prev.to_graph().map_err(|e| e.to_string())?.0;
    let new_desc = restored.to_graph().map_err(|e| e.to_string())?.0;
    if new_desc != prev_desc {
        swap_to(engine, &new_desc)?;
    }
    let prev_sched = schedule_for(engine, prev)?;
    let new_sched = schedule_for(engine, restored)?;
    if new_sched != prev_sched {
        set_schedule(engine, new_sched)?;
    }
    Ok(())
}

#[tauri::command]
pub fn graph_insert_effect(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    track_id: String,
    module: String,
    id: String,
    params: std::collections::HashMap<String, f32>,
) -> Result<(), String> {
    edit_and_swap(&engine, &history, &file, |p| {
        graph_edit::insert_effect(p, &track_id, &module, &id, params)
    })
}

#[tauri::command]
pub fn graph_remove_node(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    id: String,
) -> Result<(), String> {
    edit_and_swap(&engine, &history, &file, |p| {
        graph_edit::remove_node(p, &id)
    })
}

#[tauri::command]
pub fn graph_move_node(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    id: String,
    dir: i32,
) -> Result<(), String> {
    edit_and_swap(&engine, &history, &file, |p| {
        graph_edit::move_node(p, &id, dir)
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // a Tauri command's args mirror the note fields
pub fn note_add(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    track_id: String,
    clip_id: String,
    pitch: u8,
    start_ticks: u64,
    length_ticks: u64,
    vel: u8,
) -> Result<String, String> {
    edit_schedule(&engine, &history, &file, |p| {
        note_edit::add(
            p,
            &track_id,
            &clip_id,
            pitch,
            start_ticks,
            length_ticks,
            vel,
        )
    })
}

#[tauri::command]
pub fn note_move(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    id: String,
    start_ticks: u64,
    pitch: u8,
) -> Result<(), String> {
    edit_schedule(&engine, &history, &file, |p| {
        note_edit::move_note(p, &id, start_ticks, pitch)
    })
}

#[tauri::command]
pub fn note_resize(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    id: String,
    start_ticks: u64,
    length_ticks: u64,
) -> Result<(), String> {
    edit_schedule(&engine, &history, &file, |p| {
        note_edit::resize_note(p, &id, start_ticks, length_ticks)
    })
}

#[tauri::command]
pub fn note_delete(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    id: String,
) -> Result<(), String> {
    edit_schedule(&engine, &history, &file, |p| note_edit::delete(p, &id))
}

#[tauri::command]
pub fn clip_move(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    id: String,
    delta_ticks: i64,
) -> Result<(), String> {
    edit_schedule(&engine, &history, &file, |p| {
        clip_edit::move_clip(p, &id, delta_ticks)
    })
}

#[tauri::command]
pub fn clip_resize(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    id: String,
    length_ticks: u64,
) -> Result<(), String> {
    edit_schedule(&engine, &history, &file, |p| {
        clip_edit::resize_clip(p, &id, length_ticks)
    })
}

#[tauri::command]
pub fn edit_undo(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
) -> Result<(), String> {
    let mut history = history.lock().map_err(|_| "edit state poisoned")?;
    let prev = history.project().clone();
    let restored = match history.undo() {
        Some(project) => project.clone(),
        None => return Ok(()), // nothing to undo
    };
    apply_restored(&engine, &prev, &restored)?;
    persist(&history, &file);
    Ok(())
}

#[tauri::command]
pub fn edit_redo(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
) -> Result<(), String> {
    let mut history = history.lock().map_err(|_| "edit state poisoned")?;
    let prev = history.project().clone();
    let restored = match history.redo() {
        Some(project) => project.clone(),
        None => return Ok(()),
    };
    apply_restored(&engine, &prev, &restored)?;
    persist(&history, &file);
    Ok(())
}

/// A node in the current graph, for the UI to list and target edits.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNodeInfo {
    pub track_id: String,
    pub id: String,
    pub module: String,
    pub is_instrument: bool,
}

/// The current project as a render-friendly view (positions in ticks) for the UI.
#[tauri::command]
pub fn project_view(history: State<'_, Mutex<EditHistory>>) -> Result<ProjectView, String> {
    let history = history.lock().map_err(|_| "edit state poisoned")?;
    history.project().view().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn graph_nodes(history: State<'_, Mutex<EditHistory>>) -> Vec<GraphNodeInfo> {
    let history = match history.lock() {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let mut nodes = Vec::new();
    for track in &history.project().tracks {
        for (i, node) in track.chain.iter().enumerate() {
            nodes.push(GraphNodeInfo {
                track_id: track.id.clone(),
                id: node.id.clone(),
                module: node.module.clone(),
                is_instrument: i == 0,
            });
        }
    }
    nodes
}

#[tauri::command]
pub fn note_on(engine: State<'_, AudioEngine>, node: usize, pitch: u8, vel: u8) {
    engine.send(ControlMsg::NoteOn { node, pitch, vel });
}

#[tauri::command]
pub fn note_off(engine: State<'_, AudioEngine>, node: usize, pitch: u8) {
    engine.send(ControlMsg::NoteOff { node, pitch });
}

#[tauri::command]
pub fn set_node_param(engine: State<'_, AudioEngine>, node: usize, param: ParamId, value: f32) {
    engine.send(ControlMsg::SetParam { node, param, value });
}

/// Minimal engine health read by the UI. The full telemetry pipeline (peaks,
/// playhead, event-driven) arrives at M5; M2 still only needs a polled snapshot.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub xruns: u32,
    pub sample_rate: u32,
}

#[tauri::command]
pub fn engine_status(engine: State<'_, AudioEngine>) -> EngineStatus {
    EngineStatus {
        xruns: engine.xruns(),
        sample_rate: engine.sample_rate(),
    }
}

/// A module parameter, as the UI needs it: identity plus the slider's starting
/// position (normalized 0..1). Ranges and curves stay declared in the module,
/// not hard-coded in the frontend.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamInfo {
    pub id: ParamId,
    pub name: String,
    pub unit: String,
    pub default: f32,
}

#[tauri::command]
pub fn module_params(module_id: String) -> Vec<ParamInfo> {
    match crate::modules::registry::create(&module_id) {
        Some(node) => node
            .params()
            .iter()
            .map(|p| ParamInfo {
                id: p.id,
                name: p.name.to_string(),
                unit: p.unit.to_string(),
                default: p.normalize(p.default),
            })
            .collect(),
        None => Vec::new(),
    }
}
