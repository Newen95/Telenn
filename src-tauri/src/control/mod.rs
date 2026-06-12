//! Control layer (regular threads): sequencer, project I/O and undo, MIDI input,
//! offline WAV export, graph construction and the project file watcher.
//!
//! Communicates with the audio thread only through lock-free ring buffers.

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {}
}
