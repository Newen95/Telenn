//! Telemetry: strictly one-way audio-to-UI metering channel.
//!
//! The audio thread writes fixed-size, `Copy` frames each block; the control
//! layer drains them and pushes UI events. The UI never asks the audio thread
//! for anything.

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {}
}
