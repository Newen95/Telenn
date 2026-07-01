//! Tauri commands: the typed UI → control entry points.

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::State;

use crate::control::project::{Preset, Project, ProjectDiff, ProjectView};
use crate::control::sequencer::Schedule;
use crate::control::undo::EditHistory;
use crate::control::{
    clip_edit, graph_edit, ids, note_edit, render, ControlMsg, ProjectFile, TransportCmd,
};
use crate::engine::graph::{compile, GraphDesc};
use crate::engine::mix::TrackMix;
use crate::engine::node::{ParamCurve, ParamId};
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

/// Render a project to a 16-bit stereo WAV offline (deterministic, device-free).
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
        1,
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

/// Reload `song.json` after the file watcher saw it change on disk. Returns the
/// id-keyed diff to flash in the UI, or `None` when the file is byte-identical to
/// the in-memory project (the app's own write echoing back — never a reload loop).
///
/// The file wins: the reloaded state is hot-swapped onto the audio thread
/// (graph and/or schedule, whichever changed) and committed to the undo history
/// so the external change can be undone. Id-less notes are normalised and the
/// file rewritten (that rewrite is itself a deduped echo).
pub(crate) fn reload_after_external_change(
    engine: &AudioEngine,
    history: &Mutex<EditHistory>,
    file: &ProjectFile,
) -> Result<Option<ProjectDiff>, String> {
    let disk = std::fs::read_to_string(&file.0).map_err(|e| e.to_string())?;
    let mut history = history.lock().map_err(|_| "edit state poisoned")?;

    // Our own write echoing back? Compare against the exact bytes we would write.
    if let Ok(current) = history.project().to_json() {
        if current == disk {
            return Ok(None);
        }
    }

    let mut next: Project = serde_json::from_str(&disk).map_err(|e| e.to_string())?;
    let normalized = ids::normalize(&mut next);
    let prev = history.project().clone();
    apply_restored(engine, &prev, &next)?;
    let diff = ProjectDiff::between(&prev, &next);
    history.commit(next);
    // Persist the id assignment so the on-disk file matches memory; without this
    // the next watcher tick would see a mismatch and reload in a loop.
    if normalized {
        persist(&history, file);
    }
    Ok(Some(diff))
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

/// Move an effect to another (or the same) chain at `index`, keeping its id (so
/// the hot-swap adopts the same instance and its tail follows the move).
#[tauri::command]
pub fn node_move_to(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    id: String,
    to_track: String,
    index: usize,
) -> Result<(), String> {
    edit_and_swap(&engine, &history, &file, |p| {
        graph_edit::move_node_to(p, &id, &to_track, index)
    })
}

/// Copy an effect into a chain at `index` under a fresh id (same params).
#[tauri::command]
pub fn node_copy(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    id: String,
    new_id: String,
    to_track: String,
    index: usize,
) -> Result<(), String> {
    edit_and_swap(&engine, &history, &file, |p| {
        graph_edit::copy_node(p, &id, &new_id, &to_track, index)
    })
}

/// Insert a new effect (a dropped module or preset) at `index` in a chain.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // a Tauri command mirrors the node fields
pub fn node_insert(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    track_id: String,
    index: usize,
    module: String,
    id: String,
    params: std::collections::HashMap<String, f32>,
) -> Result<(), String> {
    edit_and_swap(&engine, &history, &file, |p| {
        graph_edit::insert_effect_at(p, &track_id, &module, &id, params, index)
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

/// Resolve an effect's live `(node index, param id)` from a stable node id and a
/// param name, against the *current* project (whose layout matches the running
/// graph). Lets the UI address params by id+name and never recompute the
/// instruments-first flat index itself.
fn locate_param(project: &Project, node_id: &str, param: &str) -> Result<(usize, ParamId), String> {
    let (desc, _index) = project.to_graph().map_err(|e| e.to_string())?;
    let node = desc
        .nodes
        .iter()
        .position(|n| n.id == node_id)
        .ok_or_else(|| format!("unknown node '{node_id}'"))?;
    let module = &desc.nodes[node].module_id;
    let inst = registry::create(module).ok_or_else(|| format!("unknown module '{module}'"))?;
    let pid = inst
        .params()
        .iter()
        .find(|p| p.name == param)
        .map(|p| p.id)
        .ok_or_else(|| format!("unknown param '{param}'"))?;
    Ok((node, pid))
}

/// Live parameter change on a chain node, addressed by id and param name. Sends
/// the normalized value straight to the audio thread (smoothed in the node): no
/// recompile, no swap, no persist — meant to stream while a knob is dragged so
/// the sound moves in real time, even mid-playback.
#[tauri::command]
pub fn node_set_param(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    node_id: String,
    param: String,
    value: f32,
) -> Result<(), String> {
    let history = history.lock().map_err(|_| "edit state poisoned")?;
    let (node, pid) = locate_param(history.project(), &node_id, &param)?;
    engine.send(ControlMsg::SetParam {
        node,
        param: pid,
        value,
    });
    Ok(())
}

/// Commit a parameter change: persist it to `song.json` and record one undo step
/// (a whole knob gesture = one step). No graph swap — a param change keeps the
/// topology, so the live node already holds the value; the value is resent once
/// more so the running node ends exactly where the knob settled.
#[tauri::command]
pub fn node_commit_param(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    node_id: String,
    param: String,
    value: f32,
) -> Result<(), String> {
    let mut history = history.lock().map_err(|_| "edit state poisoned")?;
    let (node, pid) = locate_param(history.project(), &node_id, &param)?;
    let mut trial = history.project().clone();
    graph_edit::set_param(&mut trial, &node_id, &param, value).map_err(|e| e.to_string())?;
    engine.send(ControlMsg::SetParam {
        node,
        param: pid,
        value,
    });
    history.commit(trial);
    persist(&history, &file);
    Ok(())
}

/// Track index of a track id in the project (the mixer strip order).
fn locate_track(project: &Project, track_id: &str) -> Result<usize, String> {
    project
        .tracks
        .iter()
        .position(|t| t.id == track_id)
        .ok_or_else(|| format!("unknown track '{track_id}'"))
}

/// Live mixer change for one track, streamed while a fader/pan is dragged: the
/// strip's smoothers retarget in real time — no recompile, no swap, no persist.
#[tauri::command]
pub fn mixer_set_track(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    track_id: String,
    gain_db: f32,
    pan: f32,
) -> Result<(), String> {
    let history = history.lock().map_err(|_| "edit state poisoned")?;
    let track = locate_track(history.project(), &track_id)?;
    engine.send(ControlMsg::SetTrackMix {
        track,
        mix: TrackMix { gain_db, pan },
    });
    Ok(())
}

/// Commit a track's mixer settings: persist to `song.json` and record one undo
/// step (a whole fader gesture = one step). The value is resent live so the
/// strip ends exactly where the fader settled.
#[tauri::command]
pub fn mixer_commit_track(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    track_id: String,
    gain_db: f32,
    pan: f32,
) -> Result<(), String> {
    let mut history = history.lock().map_err(|_| "edit state poisoned")?;
    let track = locate_track(history.project(), &track_id)?;
    let mut trial = history.project().clone();
    trial.tracks[track].mixer = TrackMix { gain_db, pan };
    engine.send(ControlMsg::SetTrackMix {
        track,
        mix: TrackMix { gain_db, pan },
    });
    history.commit(trial);
    persist(&history, &file);
    Ok(())
}

/// Live master gain change (dB), streamed while the master fader is dragged.
#[tauri::command]
pub fn mixer_set_master(engine: State<'_, AudioEngine>, gain_db: f32) {
    engine.send(ControlMsg::SetMasterGain { gain_db });
}

/// Commit the master gain: persist and record one undo step.
#[tauri::command]
pub fn mixer_commit_master(
    engine: State<'_, AudioEngine>,
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    gain_db: f32,
) -> Result<(), String> {
    let mut history = history.lock().map_err(|_| "edit state poisoned")?;
    let mut trial = history.project().clone();
    trial.master_gain_db = gain_db;
    engine.send(ControlMsg::SetMasterGain { gain_db });
    history.commit(trial);
    persist(&history, &file);
    Ok(())
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

/// A module parameter, as the UI needs it: identity, the slider's starting
/// position (normalized 0..1), and the declared range/curve so the UI can
/// display the real value next to the normalized slider. Ranges and curves stay
/// declared in the module, never hard-coded in the frontend.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamInfo {
    pub id: ParamId,
    pub name: String,
    pub unit: String,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    pub curve: ParamCurve,
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
                min: p.min,
                max: p.max,
                curve: p.curve,
            })
            .collect(),
        None => Vec::new(),
    }
}

/// Every registered module (id, display name, category) for the browser.
#[tauri::command]
pub fn list_modules() -> Vec<registry::ModuleDesc> {
    registry::list().to_vec()
}

/// The project's `presets/` directory (next to its `song.json`).
fn presets_dir(file: &ProjectFile) -> Option<PathBuf> {
    file.0.parent().map(|p| p.join("presets"))
}

/// Reject names that could escape the presets directory or break the filename.
fn safe_preset_name(name: &str) -> Result<&str, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.contains(['/', '\\']) || trimmed.contains("..") {
        return Err("invalid preset name".into());
    }
    Ok(trimmed)
}

/// Names of the presets saved in the project (filenames without the `.json`).
#[tauri::command]
pub fn list_presets(file: State<'_, ProjectFile>) -> Vec<String> {
    let Some(dir) = presets_dir(&file) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new(); // no presets dir yet
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            (path.extension().and_then(|x| x.to_str()) == Some("json"))
                .then(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                })
                .flatten()
        })
        .collect();
    names.sort();
    names
}

/// Read a saved preset (module + params) so the UI can drop it into a chain.
#[tauri::command]
pub fn read_preset(file: State<'_, ProjectFile>, name: String) -> Result<Preset, String> {
    let safe = safe_preset_name(&name)?;
    let dir = presets_dir(&file).ok_or("no project directory")?;
    Preset::load(dir.join(format!("{safe}.json"))).map_err(|e| e.to_string())
}

/// Save an existing node's module + params as a named preset.
#[tauri::command]
pub fn save_preset(
    history: State<'_, Mutex<EditHistory>>,
    file: State<'_, ProjectFile>,
    name: String,
    node_id: String,
) -> Result<(), String> {
    let safe = safe_preset_name(&name)?;
    let history = history.lock().map_err(|_| "edit state poisoned")?;
    let spec = history
        .project()
        .tracks
        .iter()
        .flat_map(|t| &t.chain)
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("unknown node '{node_id}'"))?;
    let preset = Preset {
        module: spec.module.clone(),
        params: spec.params.clone(),
    };
    let dir = presets_dir(&file).ok_or("no project directory")?;
    preset
        .save(dir.join(format!("{safe}.json")))
        .map_err(|e| e.to_string())
}
