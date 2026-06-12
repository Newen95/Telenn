//! State owned exclusively by the audio callback.
//!
//! Everything here runs on the real-time thread and must stay RT-SAFE: no
//! allocation, no locks, no I/O. `RtState` drains control messages, advances the
//! sequencer, runs the compiled graph, and writes the mono result to every
//! output channel.

use rtrb::Producer;

use crate::control::sequencer::{Schedule, SequencerCursor};
use crate::control::{ControlMsg, TransportCmd};
use crate::engine::graph::CompiledGraph;
use crate::engine::node::{NodeIdx, NoteEvent, NoteKind};
use crate::engine::params::LinearRamp;
use crate::engine::swap::{Declick, Garbage};
use crate::engine::MAX_BLOCK;

/// Upper bound on simultaneously sounding sequencer notes we track for release.
const MAX_SCHEDULED_NOTES: usize = 128;
/// Fade-out time of the displaced graph after a hot-swap (bounds its lifetime;
/// decaying tails are collected earlier once they fall below `FADE_EPS`).
const FADE_MS: f32 = 2000.0;
/// Per-block output energy below which a fading graph is considered silent and
/// returned to the garbage ring.
const FADE_EPS: f32 = 1.0e-9;

/// Push a heavy object to the garbage ring for off-RT drop. If the ring is full
/// (a long drag burst the control side has not drained), leak rather than free
/// on the audio thread. // RT-SAFE
fn discard(garbage: &mut Producer<Garbage>, item: Garbage) {
    if let Err(rtrb::PushError::Full(item)) = garbage.push(item) {
        std::mem::forget(item);
    }
}

/// Fixed-capacity set of `(node, pitch)` notes the sequencer currently holds on.
/// Tracked so a schedule swap can release exactly the notes the old sequence
/// started — whose NoteOff lived in the schedule being removed — without
/// touching live keyboard notes, which are never recorded here. // RT-SAFE
struct ActiveNotes {
    notes: [(NodeIdx, u8); MAX_SCHEDULED_NOTES],
    len: usize,
}

impl ActiveNotes {
    fn new() -> Self {
        Self {
            notes: [(0, 0); MAX_SCHEDULED_NOTES],
            len: 0,
        }
    }

    // RT-SAFE
    fn on(&mut self, node: NodeIdx, pitch: u8) {
        if self.len < self.notes.len() {
            self.notes[self.len] = (node, pitch);
            self.len += 1;
        }
    }

    /// Remove the first matching note (swap-remove, order irrelevant). // RT-SAFE
    fn off(&mut self, node: NodeIdx, pitch: u8) {
        if let Some(i) = self.notes[..self.len]
            .iter()
            .position(|&n| n == (node, pitch))
        {
            self.len -= 1;
            self.notes[i] = self.notes[self.len];
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

pub struct RtState {
    sample_rate: f32,
    /// The live graph. A `Box` so a hot-swap replaces it by pointer (no
    /// allocation); the displaced graph is returned through `garbage`.
    graph: Box<CompiledGraph>,
    /// The previous graph after a swap, fading out. With its persistent nodes
    /// moved into `graph` and fed nothing, it renders only the removed effects'
    /// tails — faded to silence then returned to `garbage`. At most one.
    fading: Option<Box<CompiledGraph>>,
    fade: LinearRamp,
    declick: Declick,
    /// Mono mix scratch (graph + fading graph) before fan-out. Pre-allocated.
    mix: Vec<f32>,
    /// Master gain. Held at unity in the MVP: the instrument is always live and
    /// the sequencer gates playback, so the master never gates audio. The ADSR
    /// keeps note transitions clickless.
    master: LinearRamp,
    /// The playing sequence. A `Box` so a hot reload swaps pointers (no
    /// allocation on the audio thread); the displaced box is returned through
    /// `garbage` to be dropped off the real-time thread.
    schedule: Box<Schedule>,
    cursor: SequencerCursor,
    playing: bool,
    /// Notes the sequencer currently holds on (for release on stop/reload).
    scheduled_on: ActiveNotes,
    /// Audio→control channel carrying displaced schedules and graphs for off-RT drop.
    garbage: Producer<Garbage>,
}

impl RtState {
    pub fn new(
        sample_rate: f32,
        graph: CompiledGraph,
        schedule: Schedule,
        garbage: Producer<Garbage>,
    ) -> Self {
        let mut master = LinearRamp::new(1.0, 10.0, sample_rate);
        master.set_target(1.0);
        Self {
            sample_rate,
            graph: Box::new(graph),
            fading: None,
            fade: LinearRamp::new(0.0, FADE_MS, sample_rate),
            declick: Declick::new(sample_rate),
            mix: vec![0.0; MAX_BLOCK],
            master,
            schedule: Box::new(schedule),
            cursor: SequencerCursor::new(),
            playing: false,
            scheduled_on: ActiveNotes::new(),
            garbage,
        }
    }

    /// Release every note the sequencer is currently holding (their NoteOff
    /// lived in the active schedule). Live keyboard notes are untracked here, so
    /// they are left alone. // RT-SAFE
    fn release_scheduled_notes(&mut self) {
        for i in 0..self.scheduled_on.len {
            let (node, pitch) = self.scheduled_on.notes[i];
            self.graph.push_note(
                node,
                NoteEvent {
                    frame: 0,
                    kind: NoteKind::Off { pitch },
                },
            );
        }
        self.scheduled_on.clear();
    }

    /// Apply a control message. // RT-SAFE
    pub fn handle(&mut self, msg: ControlMsg) {
        match msg {
            ControlMsg::Transport(TransportCmd::Play) => self.playing = true,
            ControlMsg::Transport(TransportCmd::Stop) => {
                self.playing = false;
                self.cursor.reset();
                // Release everything (including the sequence's held notes whose
                // NoteOff we never reached) so nothing rings on after stop.
                self.graph.all_notes_off();
                self.scheduled_on.clear();
            }
            ControlMsg::Transport(TransportCmd::Seek(pos)) => self.cursor.seek(pos, &self.schedule),
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
            ControlMsg::SetSchedule(mut incoming) => {
                // Release the notes the old sequence was holding: their NoteOff
                // lived in the schedule we're about to drop, so without this they
                // would ring forever. The live keyboard is untracked and survives
                // the reload. The release is a normal envelope tail (click-free).
                self.release_scheduled_notes();
                // Swap pointers (no allocation) and keep the playhead so the new
                // sequence resumes at the current position — only the event list
                // changes, the graph and its voices are untouched.
                std::mem::swap(&mut self.schedule, &mut incoming);
                self.cursor.reseat(&self.schedule);
                // Restrike the notes the new sequence already has in progress at
                // the resume point, so there is no gap until the next downbeat.
                {
                    let cursor = &self.cursor;
                    let schedule = &self.schedule;
                    let graph = &mut self.graph;
                    let active = &mut self.scheduled_on;
                    cursor.strike_active(schedule, |node, ev| {
                        graph.push_note(node, ev);
                        if let NoteKind::On { pitch, .. } = ev.kind {
                            active.on(node, pitch);
                        }
                    });
                }
                // Return the old schedule for off-RT drop.
                discard(&mut self.garbage, Garbage::Schedule(incoming));
            }
            ControlMsg::SwapGraph(mut new) => {
                // Move every persistent node (matched by id) into the new graph,
                // so its real-time state is sample-continuous. The old graph then
                // holds only fresh (silent) persistent nodes plus the removed
                // ones — fed nothing, it renders just the removed effects' tails.
                new.adopt_nodes_from(&mut self.graph);
                let old = std::mem::replace(&mut self.graph, new);
                // Keep the old graph fading out so those tails ring rather than
                // being cut. Bound to one fading graph (collapse a burst of swaps).
                if let Some(prev) = self.fading.take() {
                    discard(&mut self.garbage, Garbage::Graph(prev));
                }
                self.fade = LinearRamp::new(1.0, FADE_MS, self.sample_rate);
                self.fade.set_target(0.0);
                self.fading = Some(old);
                // Smooth the residual routing/level step at the boundary.
                self.declick.arm();
            }
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

            // Emit this chunk's scheduled events before rendering. Live keyboard
            // notes were already queued by `handle`; both coexist (always-live
            // instrument). Split the borrows over disjoint fields.
            if self.playing {
                let cursor = &mut self.cursor;
                let schedule = &self.schedule;
                let graph = &mut self.graph;
                let active = &mut self.scheduled_on;
                cursor.advance(n, schedule, |node, ev| {
                    graph.push_note(node, ev);
                    match ev.kind {
                        NoteKind::On { pitch, .. } => active.on(node, pitch),
                        NoteKind::Off { pitch } => active.off(node, pitch),
                    }
                });
            }

            self.graph.process_block(self.sample_rate, n);
            let src = self.graph.output();
            self.mix[..n].copy_from_slice(src);

            // Mix the fading graph's tail (removed effects), faded out. Collect
            // it once silent or once its fade window elapses.
            let mut collect = false;
            if let Some(fading) = self.fading.as_mut() {
                fading.process_block(self.sample_rate, n);
                fading.clear_notes();
                let fsrc = fading.output();
                let mut energy = 0.0f32;
                for (m, &f) in self.mix[..n].iter_mut().zip(fsrc) {
                    let g = self.fade.tick();
                    let s = f * g;
                    *m += s;
                    energy += s * s;
                }
                if self.fade.value() <= 1.0e-4 || energy < FADE_EPS {
                    collect = true;
                }
            }
            if collect {
                if let Some(done) = self.fading.take() {
                    discard(&mut self.garbage, Garbage::Graph(done));
                }
            }

            // Declick the master boundary step, apply master gain, fan out.
            let slice = &mut out[off * channels..(off + n) * channels];
            for (frame, &m) in slice.chunks_mut(channels).zip(&self.mix[..n]) {
                let value = self.declick.apply(m) * self.master.tick();
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

/// Render a project through the real `RtState` path (mono, channel 0), driven in
/// fixed `block`-sized callbacks. Shared by the equivalence test that pins the
/// offline render against the audio thread. // test-only
#[cfg(test)]
pub fn render_offline_via_rtstate(
    project: &crate::control::project::Project,
    sample_rate: u32,
    block: usize,
) -> Vec<f32> {
    let (desc, index) = project.to_graph().expect("graph");
    let graph = crate::engine::graph::compile(
        &desc,
        crate::modules::registry::create,
        sample_rate as f32,
        MAX_BLOCK,
    )
    .expect("compile");
    let schedule = project.to_schedule(sample_rate, &index).expect("schedule");
    let total = schedule.loop_len as usize;

    let (prod, _cons) = rtrb::RingBuffer::<Garbage>::new(8);
    let mut rt = RtState::new(sample_rate as f32, graph, schedule, prod);
    rt.handle(ControlMsg::Transport(TransportCmd::Play));

    let mut out = Vec::with_capacity(total);
    let mut buf = vec![0.0f32; block];
    let mut produced = 0usize;
    while produced < total {
        let n = (total - produced).min(block);
        let slice = &mut buf[..n];
        slice.fill(0.0);
        rt.process(slice, 1);
        out.extend_from_slice(slice);
        produced += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::sequencer::{Schedule, ScheduledEvent};
    use crate::control::ControlMsg;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Guard allocator: while the current thread has allocation denied, every
    // allocation *and free* bumps a counter, so a test can prove an RT path
    // neither allocated nor dropped heap memory (e.g. a graph swap must hand the
    // old graph to the garbage ring, never `drop` it). The flag is thread-local
    // (with a `const` initialiser so its own setup never allocates), keeping
    // parallel tests independent.
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
            if DENY.with(Cell::get) {
                VIOLATIONS.fetch_add(1, Ordering::SeqCst);
            }
            System.dealloc(ptr, layout)
        }
    }

    #[global_allocator]
    static GUARD: GuardAlloc = GuardAlloc;

    // Compile the demo graph and build a matching schedule for tests, off-RT.
    fn demo_rt() -> (RtState, rtrb::Consumer<Garbage>) {
        let (init, _index, _project) = crate::control::build_demo().expect("demo builds");
        let (graph, schedule) = init(48_000.0, MAX_BLOCK).expect("graph compiles");
        let (prod, cons) = rtrb::RingBuffer::<Garbage>::new(8);
        (RtState::new(48_000.0, graph, schedule, prod), cons)
    }

    #[test]
    fn playing_graph_and_schedule_swap_do_not_allocate() {
        let (mut rt, mut garbage) = demo_rt();
        let mut out = vec![0.0f32; 256 * 2];
        // Pre-build the replacement schedule entirely before arming the guard.
        let replacement = Box::new(Schedule {
            events: vec![ScheduledEvent {
                pos: 0,
                node: 0,
                kind: NoteKind::On {
                    pitch: 64,
                    vel: 100,
                },
            }],
            loop_len: 96_000,
        });

        VIOLATIONS.store(0, Ordering::SeqCst);
        DENY.with(|d| d.set(true));
        rt.handle(ControlMsg::Transport(TransportCmd::Play));
        rt.process(&mut out, 2);
        rt.handle(ControlMsg::SetSchedule(replacement));
        rt.process(&mut out, 2);
        DENY.with(|d| d.set(false));

        assert_eq!(
            VIOLATIONS.load(Ordering::SeqCst),
            0,
            "the audio path allocated"
        );
        // The displaced schedule came back for off-RT drop.
        assert!(garbage.pop().is_ok(), "old schedule was not returned");
    }

    #[test]
    fn schedule_swap_keeps_a_held_note_alive() {
        // A note held by the live keyboard must survive a hot reload: the swap
        // only replaces the event list, never the graph or its voices.
        let (mut rt, _garbage) = demo_rt();
        let mut out = vec![0.0f32; 256 * 2];
        rt.handle(ControlMsg::NoteOn {
            node: 0,
            pitch: 60,
            vel: 110,
        });
        rt.process(&mut out, 2); // voice reaches sustain region
        let before = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(before > 0.01, "note should be sounding before reload");

        rt.handle(ControlMsg::SetSchedule(Box::<Schedule>::default()));
        rt.process(&mut out, 2);
        let after = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(after > 0.01, "held note was cut by the schedule swap");
    }

    #[test]
    fn schedule_swap_releases_sequenced_notes() {
        // Regression: a note the sequencer turned on must be released on reload
        // (its NoteOff lived in the schedule being dropped), otherwise it rings
        // on forever over the new sequence.
        let (mut rt, _garbage) = demo_rt();
        let mut out = vec![0.0f32; 256 * 2];
        rt.handle(ControlMsg::Transport(TransportCmd::Play));
        rt.process(&mut out, 2); // the demo's downbeat notes are now sounding
        let playing = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(
            playing > 0.01,
            "sequence should be sounding before the swap"
        );

        // Swap to an empty schedule: with nothing new scheduled, the held
        // sequence notes must release and the output fall silent.
        rt.handle(ControlMsg::SetSchedule(Box::<Schedule>::default()));
        let mut tail = vec![0.0f32; 48_000 * 2];
        rt.process(&mut tail, 2);
        let end = tail[tail.len() - 64..]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(end < 1e-3, "sequenced notes rang on after the swap: {end}");
    }

    #[test]
    fn reload_restrikes_a_note_in_progress() {
        // Regression: reloading mid-sequence must restrike a note the new
        // sequence already has in progress at the playhead, so there is no gap.
        let (init, _index, _project) = crate::control::build_demo().expect("demo builds");
        let (graph, _sched) = init(48_000.0, MAX_BLOCK).expect("graph compiles");
        let (prod, _cons) = rtrb::RingBuffer::<Garbage>::new(8);
        let empty = Schedule {
            events: vec![],
            loop_len: 192_000,
        };
        let mut rt = RtState::new(48_000.0, graph, empty, prod);

        rt.handle(ControlMsg::Transport(TransportCmd::Play));
        let mut out = vec![0.0f32; 4096 * 2];
        rt.process(&mut out, 2); // playhead advances over silence
        let silent = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(silent < 1e-4, "nothing should sound yet");

        // New schedule: a long note that started before, ends after, the playhead.
        let next = Box::new(Schedule {
            events: vec![
                ScheduledEvent {
                    pos: 0,
                    node: 0,
                    kind: NoteKind::On {
                        pitch: 60,
                        vel: 110,
                    },
                },
                ScheduledEvent {
                    pos: 96_000,
                    node: 0,
                    kind: NoteKind::Off { pitch: 60 },
                },
            ],
            loop_len: 192_000,
        });
        rt.handle(ControlMsg::SetSchedule(next));
        let mut after = vec![0.0f32; 4096 * 2];
        rt.process(&mut after, 2);
        let peak = after.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(
            peak > 0.01,
            "in-progress note was not restruck on reload: {peak}"
        );
    }

    fn max_delta(samples: &[f32]) -> f32 {
        samples
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    }

    /// The demo plus a gain effect inserted on the lead chain. Instrument ids
    /// (`lead_synth`, `bass_synth`) and the master are shared, so they are
    /// adopted across the swap; `lead_fx` is new.
    fn demo_with_lead_gain() -> Box<CompiledGraph> {
        let song = r#"{ "bpm": 120, "tracks": [
            { "id": "lead", "chain": [
                { "id": "lead_synth", "module": "subtractive" },
                { "id": "lead_fx", "module": "gain" } ] },
            { "id": "bass", "chain": [ { "id": "bass_synth", "module": "subtractive" } ] }
        ] }"#;
        let project: crate::control::project::Project = serde_json::from_str(song).unwrap();
        let (desc, _index) = project.to_graph().unwrap();
        let graph = crate::engine::graph::compile(
            &desc,
            crate::modules::registry::create,
            48_000.0,
            MAX_BLOCK,
        )
        .unwrap();
        Box::new(graph)
    }

    #[test]
    fn graph_swap_does_not_allocate_or_free() {
        let (mut rt, mut garbage) = demo_rt();
        let mut out = vec![0.0f32; 256 * 2];
        // Compile the replacement graph entirely before arming the guard.
        let new_graph = demo_with_lead_gain();

        VIOLATIONS.store(0, Ordering::SeqCst);
        DENY.with(|d| d.set(true));
        rt.handle(ControlMsg::SwapGraph(new_graph));
        rt.process(&mut out, 2);
        rt.process(&mut out, 2);
        DENY.with(|d| d.set(false));

        assert_eq!(
            VIOLATIONS.load(Ordering::SeqCst),
            0,
            "the swap allocated or freed on the audio thread"
        );
        // The displaced graph came back for off-RT drop, never dropped in the callback.
        assert!(
            garbage.pop().is_ok(),
            "old graph was not returned to garbage"
        );
    }

    #[test]
    fn held_note_survives_graph_swap() {
        // A note held on the instrument must keep sounding across a graph edit:
        // the instrument box is moved into the new graph, not recreated.
        let (mut rt, _garbage) = demo_rt();
        let mut out = vec![0.0f32; 256 * 2];
        rt.handle(ControlMsg::NoteOn {
            node: 0,
            pitch: 60,
            vel: 110,
        });
        rt.process(&mut out, 2);
        let before = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(before > 0.01, "note should sound before the swap");

        rt.handle(ControlMsg::SwapGraph(demo_with_lead_gain()));
        rt.process(&mut out, 2);
        let after = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(after > 0.01, "held note was cut by the graph swap");
    }

    #[test]
    fn swap_output_has_no_click() {
        // Inserting an effect must not add a discontinuity beyond the signal's
        // own per-sample deltas (the declick absorbs the boundary step).
        let (mut rt, _garbage) = demo_rt();
        rt.handle(ControlMsg::NoteOn {
            node: 0,
            pitch: 48,
            vel: 100,
        });
        let mut buf = vec![0.0f32; 4096 * 2];
        rt.process(&mut buf, 2); // reach sustain

        let mut base = Vec::new();
        for _ in 0..4 {
            rt.process(&mut buf, 2);
            base.extend(buf.iter().step_by(2).copied());
        }
        let base_delta = max_delta(&base);

        let mut cap = Vec::new();
        rt.process(&mut buf, 2);
        cap.extend(buf.iter().step_by(2).copied());
        rt.handle(ControlMsg::SwapGraph(demo_with_lead_gain()));
        for _ in 0..4 {
            rt.process(&mut buf, 2);
            cap.extend(buf.iter().step_by(2).copied());
        }
        let swap_delta = max_delta(&cap);
        assert!(
            swap_delta <= base_delta + 0.02,
            "graph swap added a discontinuity: base {base_delta}, swap {swap_delta}"
        );
    }

    #[test]
    fn removed_tail_rings_out_smoothly() {
        // Removing a tail effect (proxy: a decaying source) must not cut its
        // tail: the orphaned old graph rings out, faded to silence (no cliff).
        use crate::engine::graph::{compile, GraphDesc, NodeDesc};
        use crate::engine::swap::test_support;

        let tail = GraphDesc {
            nodes: vec![NodeDesc {
                id: "t".into(),
                module_id: "tail".into(),
                params: vec![],
            }],
            connections: vec![],
            output: 0,
        };
        let graph_a = compile(&tail, test_support::create, 48_000.0, MAX_BLOCK).unwrap();
        let (prod, _cons) = rtrb::RingBuffer::<Garbage>::new(8);
        let mut rt = RtState::new(48_000.0, graph_a, Schedule::default(), prod);

        let mut buf = vec![0.0f32; 256];
        rt.process(&mut buf, 1);
        let before = buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(
            before > 0.1,
            "tail should be sounding before removal: {before}"
        );

        let silence = GraphDesc {
            nodes: vec![NodeDesc {
                id: "s".into(),
                module_id: "silence".into(),
                params: vec![],
            }],
            connections: vec![],
            output: 0,
        };
        let graph_b = compile(&silence, test_support::create, 48_000.0, MAX_BLOCK).unwrap();
        rt.handle(ControlMsg::SwapGraph(Box::new(graph_b)));

        let mut cap = Vec::new();
        for _ in 0..400 {
            let mut b = vec![0.0f32; 256];
            rt.process(&mut b, 1);
            cap.extend_from_slice(&b);
        }
        assert!(
            max_delta(&cap) < 0.05,
            "tail removal clicked: {}",
            max_delta(&cap)
        );
        let end = cap[cap.len() - 256..]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(end < 1e-3, "tail did not ring out to silence: {end}");
    }
}
