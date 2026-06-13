pub mod control;
pub mod engine;
pub mod modules;
pub mod telemetry;

use std::sync::Mutex;

use rtrb::RingBuffer;

use control::commands::{
    engine_status, export_wav, graph_insert_effect, graph_move_node, graph_nodes, graph_redo,
    graph_remove_node, graph_undo, load_project, module_params, note_off, note_on, project_view,
    set_node_param, transport_play, transport_stop,
};
use control::undo::EditHistory;
use telemetry::frame::TelemetryFrame;

/// Capacity of the audio→control telemetry ring (per ARCHITECTURE.md).
const TELEMETRY_RING_CAPACITY: usize = 64;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (init, index, project) = control::build_demo().expect("failed to build the demo project");
    let (telemetry_prod, telemetry_cons) =
        RingBuffer::<TelemetryFrame>::new(TELEMETRY_RING_CAPACITY);
    let engine = engine::AudioEngine::start(init, index, telemetry_prod)
        .expect("failed to start the audio engine");
    let history = Mutex::new(EditHistory::new(project));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(engine)
        .manage(history)
        .setup(move |app| {
            control::telemetry_pump::spawn(app.handle().clone(), telemetry_cons);
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
            graph_undo,
            graph_redo,
            graph_nodes,
            project_view
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
