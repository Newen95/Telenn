pub mod control;
pub mod engine;
pub mod modules;
pub mod telemetry;

use std::sync::Mutex;

use rtrb::RingBuffer;

use control::commands::{
    clip_move, clip_resize, edit_redo, edit_undo, engine_status, export_wav, graph_insert_effect,
    graph_move_node, graph_nodes, graph_remove_node, list_modules, list_presets, load_project,
    metronome_set, mixer_commit_master, mixer_commit_track, mixer_set_master, mixer_set_track,
    module_params, node_commit_param, node_copy, node_insert, node_move_to, node_set_param,
    note_add, note_delete, note_move, note_off, note_on, note_resize, project_view, read_preset,
    save_preset, set_node_param, track_add, track_remove, track_set_instrument, transport_play,
    transport_stop,
};
use control::undo::EditHistory;
use telemetry::frame::TelemetryFrame;

/// Capacity of the audio→control telemetry ring (per ARCHITECTURE.md).
const TELEMETRY_RING_CAPACITY: usize = 64;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (init, index, project, project_path) =
        control::build_demo().expect("failed to build the demo project");
    let (telemetry_prod, telemetry_cons) =
        RingBuffer::<TelemetryFrame>::new(TELEMETRY_RING_CAPACITY);
    let engine = engine::AudioEngine::start(init, index, telemetry_prod)
        .expect("failed to start the audio engine");
    let history = Mutex::new(EditHistory::new(project));
    let watch_path = project_path.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(engine)
        .manage(history)
        .manage(control::ProjectFile(project_path))
        .setup(move |app| {
            control::telemetry_pump::spawn(app.handle().clone(), telemetry_cons);
            control::watcher::spawn(app.handle().clone(), watch_path);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            transport_play,
            transport_stop,
            note_on,
            note_off,
            set_node_param,
            engine_status,
            module_params,
            load_project,
            export_wav,
            graph_insert_effect,
            graph_remove_node,
            graph_move_node,
            node_move_to,
            node_copy,
            node_insert,
            node_set_param,
            node_commit_param,
            mixer_set_track,
            mixer_commit_track,
            mixer_set_master,
            mixer_commit_master,
            track_add,
            track_set_instrument,
            track_remove,
            metronome_set,
            list_modules,
            list_presets,
            read_preset,
            save_preset,
            note_add,
            note_move,
            note_resize,
            note_delete,
            clip_move,
            clip_resize,
            edit_undo,
            edit_redo,
            graph_nodes,
            project_view
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
