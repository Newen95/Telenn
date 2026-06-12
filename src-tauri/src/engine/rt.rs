//! State owned exclusively by the audio callback.
//!
//! Everything here runs on the real-time thread and must stay RT-SAFE: no
//! allocation, no locks, no I/O. `RtState` drains control messages into the
//! compiled graph, runs it, and writes the mono result to every output channel.

use crate::control::ControlMsg;
use crate::engine::graph::CompiledGraph;
use crate::engine::node::{NoteEvent, NoteKind};
use crate::engine::params::LinearRamp;
use crate::engine::MAX_BLOCK;

pub struct RtState {
    sample_rate: f32,
    graph: CompiledGraph,
    /// Master gain. Kept as a smoothed gain for forward use, but in M2 it stays
    /// at unity: the instrument is always live, transport no longer gates audio
    /// (the sequencer will, from M3). The ADSR keeps note transitions clickless.
    master: LinearRamp,
}

impl RtState {
    pub fn new(sample_rate: f32, graph: CompiledGraph) -> Self {
        let mut master = LinearRamp::new(1.0, 10.0, sample_rate);
        master.set_target(1.0);
        Self {
            sample_rate,
            graph,
            master,
        }
    }

    /// Apply a control message. // RT-SAFE
    pub fn handle(&mut self, msg: ControlMsg) {
        match msg {
            // Transport gating is the sequencer's job (M3); in M2 this is a
            // deliberate no-op so the instrument stays live.
            ControlMsg::Transport(_) => {}
            ControlMsg::NoteOn { node, pitch, vel } => self.graph.push_note(
                node,
                NoteEvent {
                    frame: 0,
                    kind: NoteKind::On { pitch, vel },
                },
            ),
            ControlMsg::NoteOff { node, pitch } => self.graph.push_note(
                node,
                NoteEvent {
                    frame: 0,
                    kind: NoteKind::Off { pitch },
                },
            ),
            ControlMsg::SetParam { node, param, value } => self.graph.set_param(node, param, value),
        }
    }

    /// Render the graph into `out` (interleaved, `channels` per frame), fanning
    /// the mono result out to every channel. Processed in `MAX_BLOCK` chunks so
    /// an unusually large callback never overruns the pre-allocated buffers.
    /// // RT-SAFE
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        if channels == 0 {
            return;
        }
        let total = out.len() / channels;
        let mut off = 0;
        while off < total {
            let n = (total - off).min(MAX_BLOCK);
            self.graph.process_block(self.sample_rate, n);
            let src = self.graph.output();
            let slice = &mut out[off * channels..(off + n) * channels];
            for (frame, &s) in slice.chunks_mut(channels).zip(src) {
                let value = s * self.master.tick();
                for c in frame.iter_mut() {
                    *c = value;
                }
            }
            off += n;
            // Notes were consumed by this chunk; clear so later chunks and the
            // next callback start fresh.
            self.graph.clear_notes();
        }
        if total == 0 {
            self.graph.clear_notes();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{build_test_graph, ControlMsg};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Guard allocator: while the current thread has allocation denied, every
    // allocation bumps a counter so a test can prove an RT path allocated
    // nothing. The flag is thread-local (with a `const` initialiser so its own
    // setup never allocates), keeping parallel tests independent.
    thread_local! {
        static DENY: Cell<bool> = const { Cell::new(false) };
    }
    static VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

    struct GuardAlloc;

    unsafe impl GlobalAlloc for GuardAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if DENY.with(Cell::get) {
                VIOLATIONS.fetch_add(1, Ordering::SeqCst);
            }
            System.alloc(layout)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout)
        }
    }

    #[global_allocator]
    static GUARD: GuardAlloc = GuardAlloc;

    #[test]
    fn graph_processing_does_not_allocate() {
        // Compile and set up entirely before arming the guard.
        let graph = build_test_graph(48_000.0, MAX_BLOCK).expect("graph compiles");
        let mut rt = RtState::new(48_000.0, graph);
        let mut out = vec![0.0f32; 256 * 2];

        VIOLATIONS.store(0, Ordering::SeqCst);
        DENY.with(|d| d.set(true));
        rt.handle(ControlMsg::NoteOn {
            node: 0,
            pitch: 60,
            vel: 100,
        });
        rt.process(&mut out, 2);
        rt.handle(ControlMsg::NoteOff { node: 0, pitch: 60 });
        rt.process(&mut out, 2);
        DENY.with(|d| d.set(false));

        assert_eq!(
            VIOLATIONS.load(Ordering::SeqCst),
            0,
            "the audio path allocated"
        );
    }
}
