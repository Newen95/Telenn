//! Telemetry: strictly one-way audio-to-UI metering channel.
//!
//! The audio thread writes fixed-size, `Copy` frames each block into an `rtrb`
//! ring; the control-side pump (`control::telemetry_pump`) drains them and emits
//! a Tauri event. The UI never asks the audio thread for anything.

pub mod frame;
