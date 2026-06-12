//! Real-time audio thread: graph execution, scheduler and hot-swap.
//!
//! Everything reachable from the audio callback must be RT-SAFE: no allocation,
//! no locks, no I/O, no blocking syscalls.

pub mod rt;
pub mod stream;

pub use stream::{AudioEngine, EngineError};
