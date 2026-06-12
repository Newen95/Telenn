//! Control layer (regular threads): sequencer, project I/O and undo, MIDI input,
//! offline WAV export, graph construction and the project file watcher.
//!
//! Communicates with the audio thread only through lock-free ring buffers.

pub mod commands;

/// Compact, allocation-free messages sent from the control side to the audio
/// thread through an `rtrb` ring buffer. The callback drains all pending
/// messages at the start of each block before rendering.
///
/// This enum grows over later milestones (per ARCHITECTURE.md: `Seek`,
/// `NoteOn`, `NoteOff`, `SetParam`, `SwapGraph`). M1 only needs transport.
pub enum ControlMsg {
    Transport(TransportCmd),
}

#[derive(Clone, Copy)]
pub enum TransportCmd {
    Play,
    Stop,
}
