//! Tauri commands: the typed UI → control entry points.

use tauri::State;

use crate::control::{ControlMsg, TransportCmd};
use crate::engine::node::ParamId;
use crate::engine::AudioEngine;

#[tauri::command]
pub fn transport_play(engine: State<'_, AudioEngine>) {
    engine.send(ControlMsg::Transport(TransportCmd::Play));
}

#[tauri::command]
pub fn transport_stop(engine: State<'_, AudioEngine>) {
    engine.send(ControlMsg::Transport(TransportCmd::Stop));
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
