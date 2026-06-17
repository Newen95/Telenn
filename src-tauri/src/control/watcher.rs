//! Project file watcher: reflects external edits of `song.json` (by Claude Code
//! or any editor) back into the running app. Runs on its own thread — off the
//! audio path, so file I/O and allocation here are fine.
//!
//! A single save emits a burst of filesystem events, so we coalesce them with a
//! short debounce and react once the directory has been quiet. The reload itself
//! (parse, hot-swap, history commit, self-write dedup) lives in
//! [`commands::reload_after_external_change`]; here we only drive it and forward
//! the resulting diff (or a parse error) to the UI.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Manager};

use crate::control::commands;
use crate::control::undo::EditHistory;
use crate::control::ProjectFile;
use crate::engine::AudioEngine;

/// Quiet window after the last filesystem event before we react (one save emits
/// several events; this collapses the burst into a single reload).
const DEBOUNCE: Duration = Duration::from_millis(100);
/// Emitted to the UI with the id-keyed diff after a successful external reload.
const CHANGED_EVENT: &str = "project_changed";
/// Emitted to the UI when the changed file fails to parse (the app keeps its
/// current state; nothing is reloaded).
const ERROR_EVENT: &str = "project_error";

/// Spawn the watcher thread for the project's directory. It runs for the app's
/// lifetime; when the app shuts down the event sender drops and the loop ends.
pub fn spawn(app: AppHandle, project_path: PathBuf) {
    let Some(dir) = project_path.parent().map(PathBuf::from) else {
        return;
    };
    let _ = thread::Builder::new()
        .name("telenn-watcher".into())
        .spawn(move || {
            let (tx, rx) = mpsc::channel();
            let mut watcher = match RecommendedWatcher::new(
                move |res| {
                    let _ = tx.send(res);
                },
                Config::default(),
            ) {
                Ok(w) => w,
                Err(_) => return,
            };
            // Non-recursive: only the directory's direct entries (song.json),
            // never the presets/ subtree. Dropping `watcher` stops watching, so
            // it must stay alive for the whole loop below.
            if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
                return;
            }

            loop {
                // Block until something happens, then drain the burst: keep
                // pulling events until the directory is quiet for the debounce
                // window, and only then react once.
                if rx.recv().is_err() {
                    break; // sender dropped: app shutting down
                }
                loop {
                    match rx.recv_timeout(DEBOUNCE) {
                        Ok(_) => continue,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                react(&app);
            }
        });
}

/// Reload the project and forward the outcome to the UI.
fn react(app: &AppHandle) {
    let engine = app.state::<AudioEngine>();
    let history = app.state::<Mutex<EditHistory>>();
    let file = app.state::<ProjectFile>();
    match commands::reload_after_external_change(engine.inner(), history.inner(), file.inner()) {
        // A real external change: reload done, flash whatever changed (the diff
        // may be empty for a non-note edit; the UI still refreshes its views).
        Ok(Some(diff)) => {
            let _ = app.emit(CHANGED_EVENT, diff);
        }
        // The app's own write echoing back: nothing to do.
        Ok(None) => {}
        // Unparseable file: keep the current state, surface the error.
        Err(e) => {
            let _ = app.emit(ERROR_EVENT, e);
        }
    }
}
