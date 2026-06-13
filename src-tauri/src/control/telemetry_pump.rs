//! Control-side telemetry pump: drains the audio→control telemetry ring and
//! emits a Tauri `telemetry` event at ~60 Hz. Runs on its own thread (off the
//! audio path), so allocation and event emission here are fine.

use std::thread;
use std::time::Duration;

use rtrb::Consumer;
use tauri::{AppHandle, Emitter};

use crate::telemetry::frame::{aggregate, TelemetryFrame};

/// UI refresh cap. The audio thread writes faster; each tick drains the whole
/// ring and emits the aggregate, so the UI never falls behind.
const PUMP_HZ: u64 = 60;
/// Event name the frontend listens on.
const EVENT: &str = "telemetry";

/// Spawn the pump thread. It runs for the lifetime of the app; when the engine
/// is dropped the ring goes silent and the pump simply emits nothing.
pub fn spawn(app: AppHandle, mut consumer: Consumer<TelemetryFrame>) {
    let period = Duration::from_millis(1000 / PUMP_HZ);
    let _ = thread::Builder::new()
        .name("telenn-telemetry".into())
        .spawn(move || loop {
            thread::sleep(period);
            // Drain the whole ring into one aggregate (latest playhead, max peak).
            if let Some(frame) = aggregate(std::iter::from_fn(|| consumer.pop().ok())) {
                let _ = app.emit(EVENT, frame);
            }
        });
}
