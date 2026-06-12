//! Real-time audio thread: graph execution, scheduler and hot-swap.
//!
//! Everything reachable from the audio callback must be RT-SAFE: no allocation,
//! no locks, no I/O, no blocking syscalls.

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {}
}
