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
use crate::engine::MAX_BLOCK;

/// Upper bound on simultaneously sounding sequencer notes we track for release.
const MAX_SCHEDULED_NOTES: usize = 128;

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
    graph: CompiledGraph,
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
    /// Audio→control channel carrying displaced schedules for off-RT drop.
    garbage: Producer<Box<Schedule>>,
}

impl RtState {
    pub fn new(
        sample_rate: f32,
        graph: CompiledGraph,
        schedule: Schedule,
        garbage: Producer<Box<Schedule>>,
    ) -> Self {
        let mut master = LinearRamp::new(1.0, 10.0, sample_rate);
        master.set_target(1.0);
        Self {
            sample_rate,
            graph,
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
                // Return the old schedule for off-RT drop. If the garbage ring
                // is somehow full, leak rather than free on the audio thread.
                if let Err(rtrb::PushError::Full(old)) = self.garbage.push(incoming) {
                    std::mem::forget(old);
                }
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

    let (prod, _cons) = rtrb::RingBuffer::<Box<Schedule>>::new(8);
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

    // Compile the demo graph and build a matching schedule for tests, off-RT.
    fn demo_rt() -> (RtState, rtrb::Consumer<Box<Schedule>>) {
        let (init, index) = crate::control::build_demo().expect("demo builds");
        let (graph, schedule) = init(48_000.0, MAX_BLOCK).expect("graph compiles");
        let _ = index;
        let (prod, cons) = rtrb::RingBuffer::<Box<Schedule>>::new(8);
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
        let (init, _index) = crate::control::build_demo().expect("demo builds");
        let (graph, _sched) = init(48_000.0, MAX_BLOCK).expect("graph compiles");
        let (prod, _cons) = rtrb::RingBuffer::<Box<Schedule>>::new(8);
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
}
