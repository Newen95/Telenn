//! Real-time audio thread: graph execution, scheduler and hot-swap.
//!
//! Everything reachable from the audio callback must be RT-SAFE: no allocation,
//! no locks, no I/O, no blocking syscalls.

pub mod dsp;
pub mod graph;
pub mod metronome;
pub mod mix;
pub mod node;
pub mod params;
pub mod rt;
pub mod stream;
pub mod swap;

pub use stream::{AudioEngine, EngineError};

/// Largest block the engine pre-allocates for. Device callbacks are expected to
/// stay well under this; larger blocks are processed in chunks.
pub const MAX_BLOCK: usize = 4096;
