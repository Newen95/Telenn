//! Project file watcher: reflects external edits of `song.json` (by Claude Code
//! or any editor) back into the running app. Runs on its own thread — off the
//! audio path, so file I/O and allocation here are fine.
//!
//! A single save emits a burst of filesystem events, so we coalesce them with a
//! short debounce and react once the directory has been quiet. The reload itself
//! (parse, hot-swap, history commit, self-write dedup) lives in
//! [`commands::reload_after_external_change`]; here we only drive it and forward
//! the resulting diff (or a parse error) to the UI.
//!
//! Opening another project retargets the watch through [`WatcherControl`]: the
//! rewatch request rides the same channel as the filesystem events, so the loop
//! never needs to select over two receivers.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
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

enum Msg {
    /// A filesystem event fired in the watched directory.
    Fs,
    /// Retarget the watch to another project directory (project switch).
    Watch(PathBuf),
}

/// Handle to the watcher thread, held in Tauri state so `open_project` can
/// retarget the watch. Dropping every sender ends the thread.
pub struct WatcherControl(Sender<Msg>);

impl WatcherControl {
    /// Point the watcher at another project directory. Best effort: a dead
    /// watcher thread only costs external-edit reloads, never the open itself.
    pub fn rewatch(&self, dir: PathBuf) {
        let _ = self.0.send(Msg::Watch(dir));
    }
}

/// Spawn the watcher thread for the project's directory and return its control
/// handle. The thread runs for the app's lifetime; when the app shuts down the
/// senders drop and the loop ends.
pub fn spawn(app: AppHandle, project_path: PathBuf) -> WatcherControl {
    let (tx, rx) = mpsc::channel::<Msg>();
    let control = WatcherControl(tx.clone());
    let Some(initial_dir) = project_path.parent().map(PathBuf::from) else {
        return control;
    };
    let _ = thread::Builder::new()
        .name("telenn-watcher".into())
        .spawn(move || {
            let fs_tx = tx;
            let mut watcher = match RecommendedWatcher::new(
                move |_res| {
                    let _ = fs_tx.send(Msg::Fs);
                },
                Config::default(),
            ) {
                Ok(w) => w,
                Err(_) => return,
            };
            // Non-recursive: only the directory's direct entries (song.json),
            // never the presets/ subtree. Dropping `watcher` stops watching, so
            // it must stay alive for the whole loop below.
            let mut dir = initial_dir;
            if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
                return;
            }

            loop {
                // Block until something happens, then drain the burst: keep
                // pulling events until the directory is quiet for the debounce
                // window, and only then react once. A rewatch can arrive at any
                // point; stale events from the old directory are harmless (the
                // reload dedups against the active file's bytes).
                match rx.recv() {
                    Err(_) => break, // senders dropped: app shutting down
                    Ok(Msg::Watch(new_dir)) => {
                        retarget(&mut watcher, &mut dir, new_dir);
                        continue; // a switch alone is not a file change
                    }
                    Ok(Msg::Fs) => {}
                }
                loop {
                    match rx.recv_timeout(DEBOUNCE) {
                        Ok(Msg::Fs) => continue,
                        Ok(Msg::Watch(new_dir)) => {
                            retarget(&mut watcher, &mut dir, new_dir);
                            continue;
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                react(&app);
            }
        });
    control
}

/// Swap the watched directory; on failure keep whatever notify left us with —
/// the next rewatch (or app restart) recovers.
fn retarget(watcher: &mut RecommendedWatcher, dir: &mut PathBuf, new_dir: PathBuf) {
    if new_dir == *dir {
        return;
    }
    let _ = watcher.unwatch(dir);
    if watcher.watch(&new_dir, RecursiveMode::NonRecursive).is_ok() {
        *dir = new_dir;
    }
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
