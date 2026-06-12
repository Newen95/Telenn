pub mod control;
pub mod engine;
pub mod modules;
pub mod telemetry;

use std::sync::Mutex;

use control::commands::{
    engine_status, export_wav, graph_insert_effect, graph_move_node, graph_nodes, graph_redo,
    graph_remove_node, graph_undo, load_project, module_params, note_off, note_on, set_node_param,
    transport_play, transport_stop,
};
use control::undo::EditHistory;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (init, index, project) = control::build_demo().expect("failed to build the demo project");
    let engine = engine::AudioEngine::start(init, index).expect("failed to start the audio engine");
    let history = Mutex::new(EditHistory::new(project));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(engine)
        .manage(history)
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
            graph_nodes
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
