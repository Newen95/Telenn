//! State owned exclusively by the audio callback.
//!
//! Everything here runs on the real-time thread and must stay RT-SAFE: no
//! allocation, no locks, no I/O. `RtState` drains control messages, advances the
//! sequencer, runs the compiled graph, mixes the mono track tails into stereo
//! (the `Mixer` stage), and fans the stereo pair out to the device channels.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rtrb::Producer;

use crate::control::sequencer::{Schedule, SequencerCursor};
use crate::control::{ControlMsg, TransportCmd};
use crate::engine::graph::CompiledGraph;
use crate::engine::mix::Mixer;
use crate::engine::node::{NodeIdx, NoteEvent, NoteKind};
use crate::engine::params::LinearRamp;
use crate::engine::swap::{Declick, Garbage};
use crate::engine::MAX_BLOCK;
use crate::telemetry::frame::{Meter, TelemetryFrame, MAX_METERS};

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
    /// The mono→stereo boundary: per-track gain/pan and the master gain.
    mixer: Mixer,
    /// One declick per stereo channel, absorbing the swap-boundary step.
    declick: [Declick; 2],
    /// Interleaved stereo mix scratch (graph + fading graph) before the device
    /// fan-out. Pre-allocated.
    mix: Vec<f32>,
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
    /// Audio→control telemetry ring (one frame per callback; dropped if full).
    telemetry: Producer<TelemetryFrame>,
    /// Shared cumulative xrun counter (written by the stream's error callback),
    /// read here to stamp each telemetry frame. Relaxed load is RT-safe.
    xruns: Arc<AtomicU32>,
}

impl RtState {
    pub fn new(
        sample_rate: f32,
        graph: CompiledGraph,
        schedule: Schedule,
        garbage: Producer<Garbage>,
        telemetry: Producer<TelemetryFrame>,
        xruns: Arc<AtomicU32>,
    ) -> Self {
        let mixer = Mixer::new(sample_rate, graph.track_mix(), graph.master_gain_db());
        Self {
            sample_rate,
            graph: Box::new(graph),
            fading: None,
            fade: LinearRamp::new(0.0, FADE_MS, sample_rate),
            mixer,
            declick: [Declick::new(sample_rate), Declick::new(sample_rate)],
            mix: vec![0.0; MAX_BLOCK * 2],
            schedule: Box::new(schedule),
            cursor: SequencerCursor::new(),
            playing: false,
            scheduled_on: ActiveNotes::new(),
            garbage,
            telemetry,
            xruns,
        }
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
                // Swap pointers (no allocation) and keep the playhead so the new
                // sequence resumes at the current position — only the event list
                // changes, the graph and its voices are untouched.
                std::mem::swap(&mut self.schedule, &mut incoming);
                self.cursor.reseat(&self.schedule);

                // Reconcile the sounding sequencer notes with the new sequence by
                // (node, pitch), so unchanged notes keep ringing across the
                // reload instead of being released and re-attacked:
                //   A = notes currently held (`scheduled_on`),
                //   N = notes the new sequence has in progress at the playhead.
                //   A∩N → keep (do nothing); A\N → release; N\A → strike.
                // A note whose only change is velocity stays in A∩N — left
                // untouched, the new velocity applying at its next trigger.
                // Re-attacking it just to update the attack loudness would
                // reintroduce the very glitch this reconciliation removes.
                //
                // Only while playing: when stopped a swap is how an edit reaches
                // the audio thread and must stay silent; the playhead sits at the
                // start with nothing held, so there is nothing to reconcile.
                if self.playing {
                    // Stack scratch (no allocation): the new in-progress set.
                    let mut buf = [(0usize, 0u8, 0u8); MAX_SCHEDULED_NOTES];
                    let n = self.cursor.active_notes(&self.schedule, &mut buf);
                    let new_active = &buf[..n];

                    // A\N: held notes no longer in progress → release (the voice
                    // rings out on its envelope; click-free).
                    for i in 0..self.scheduled_on.len {
                        let (node, pitch) = self.scheduled_on.notes[i];
                        let still_on = new_active
                            .iter()
                            .any(|&(nn, np, _)| nn == node && np == pitch);
                        if !still_on {
                            self.graph.push_note(
                                node,
                                NoteEvent {
                                    frame: 0,
                                    kind: NoteKind::Off { pitch },
                                },
                            );
                        }
                    }
                    // N\A: newly in-progress notes not already held → strike.
                    for &(node, pitch, vel) in new_active {
                        let already = self.scheduled_on.notes[..self.scheduled_on.len]
                            .iter()
                            .any(|&(hn, hp)| hn == node && hp == pitch);
                        if !already {
                            self.graph.push_note(
                                node,
                                NoteEvent {
                                    frame: 0,
                                    kind: NoteKind::On { pitch, vel },
                                },
                            );
                        }
                    }
                    // scheduled_on = N.
                    self.scheduled_on.clear();
                    for &(node, pitch, _) in new_active {
                        self.scheduled_on.on(node, pitch);
                    }
                } else {
                    // Stopped: nothing is held and nothing must sound. Keep the
                    // invariant that `scheduled_on` is empty so a later Play is clean.
                    self.scheduled_on.clear();
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
                // The new graph carries the project's mixer settings; glide the
                // strips to them (a mixer-only change swaps too, e.g. undo).
                self.mixer
                    .retarget(self.graph.track_mix(), self.graph.master_gain_db());
                // Smooth the residual routing/level step at the boundary.
                self.declick[0].arm();
                self.declick[1].arm();
            }
            ControlMsg::SetTrackMix { track, mix } => self.mixer.set_track(track, mix),
            ControlMsg::SetMasterGain { gain_db } => self.mixer.set_master_db(gain_db),
        }
    }

    /// Render the graph into `out` (interleaved, `channels` per frame): the
    /// mixer turns the mono track tails into a stereo pair, which fans out to
    /// the device — even channels get L, odd channels get R; a mono device gets
    /// the (L+R)/2 fold. Processed in `MAX_BLOCK` chunks so an unusually large
    /// callback never overruns the pre-allocated buffers. // RT-SAFE
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        if channels == 0 {
            return;
        }
        let total = out.len() / channels;
        let mut off = 0;
        let mut peak = 0.0f32;
        // Bracket only the graph computation (never the device wait, which lives
        // outside this call). `Instant::now` is a monotonic clock read — a VDSO
        // call on Windows/Linux: non-blocking, no allocation, no lock. // RT-SAFE
        let compute_start = Instant::now();
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
            // The mixer stage: mono track tails → interleaved stereo, with
            // per-track gain/pan and the master gain applied inside.
            self.mixer.mix(&self.graph, n, &mut self.mix);

            // Add the fading graph's tails (removed effects), panned with the
            // current strip gains and faded out. Collect it once silent or once
            // its fade window elapses.
            let mut collect = false;
            if let Some(fading) = self.fading.as_mut() {
                fading.process_block(self.sample_rate, n);
                fading.clear_notes();
                let energy = self
                    .mixer
                    .add_faded(fading, n, &mut self.fade, &mut self.mix);
                if self.fade.value() <= 1.0e-4 || energy < FADE_EPS {
                    collect = true;
                }
            }
            if collect {
                if let Some(done) = self.fading.take() {
                    discard(&mut self.garbage, Garbage::Graph(done));
                }
            }

            // Declick the swap-boundary step per channel, then fan the stereo
            // pair out to the device layout.
            let slice = &mut out[off * channels..(off + n) * channels];
            for (f, frame) in slice.chunks_mut(channels).enumerate() {
                let l = self.declick[0].apply(self.mix[2 * f]);
                let r = self.declick[1].apply(self.mix[2 * f + 1]);
                peak = peak.max(l.abs()).max(r.abs());
                if channels == 1 {
                    frame[0] = 0.5 * (l + r);
                } else {
                    for (c, ch) in frame.iter_mut().enumerate() {
                        *ch = if c % 2 == 0 { l } else { r };
                    }
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

        // Compute load = graph time / the block's real-time budget (N/SR), not
        // the wall-clock gap between callbacks. Above 1.0 means the block did not
        // fit its budget. // RT-SAFE
        let audio_load = if total > 0 {
            let budget = total as f32 / self.sample_rate;
            compute_start.elapsed().as_secs_f32() / budget
        } else {
            0.0
        };

        // Copy the per-node meters (capped to MAX_METERS) into the frame, keeping
        // the node-index alignment so the UI maps meter[i] → node id. // RT-SAFE
        let src = self.graph.meters();
        let meter_count = src.len().min(MAX_METERS);
        let mut meters = [Meter::default(); MAX_METERS];
        meters[..meter_count].copy_from_slice(&src[..meter_count]);

        // One telemetry frame per callback. Dropped if the ring is full — the
        // pump drains far faster in practice. // RT-SAFE
        let _ = self.telemetry.push(TelemetryFrame {
            playhead_samples: self.cursor.playhead(),
            playing: self.playing,
            master_peak: peak,
            audio_load,
            xruns: self.xruns.load(Ordering::Relaxed),
            meter_count: meter_count as u8,
            meters,
        });
    }
}

/// Render a project through the real `RtState` path (interleaved stereo, as a
/// 2-channel device would receive it), driven in fixed `block`-sized callbacks.
/// Shared by the equivalence test that pins the offline render against the
/// audio thread. // test-only
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
        1,
    )
    .expect("compile");
    let schedule = project.to_schedule(sample_rate, &index).expect("schedule");
    let total = schedule.loop_len as usize;

    let (prod, _cons) = rtrb::RingBuffer::<Garbage>::new(8);
    let (tprod, _t) = rtrb::RingBuffer::<TelemetryFrame>::new(64);
    let xruns = Arc::new(AtomicU32::new(0));
    let mut rt = RtState::new(sample_rate as f32, graph, schedule, prod, tprod, xruns);
    rt.handle(ControlMsg::Transport(TransportCmd::Play));

    let mut out = Vec::with_capacity(total * 2);
    let mut buf = vec![0.0f32; block * 2];
    let mut produced = 0usize;
    while produced < total {
        let n = (total - produced).min(block);
        let slice = &mut buf[..n * 2];
        slice.fill(0.0);
        rt.process(slice, 2);
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
        let (init, _index, _project, _path) = crate::control::build_demo().expect("demo builds");
        let (graph, schedule) = init(48_000.0, MAX_BLOCK).expect("graph compiles");
        let (prod, cons) = rtrb::RingBuffer::<Garbage>::new(8);
        let (tprod, _t) = rtrb::RingBuffer::<TelemetryFrame>::new(64);
        let xruns = Arc::new(AtomicU32::new(0));
        (
            RtState::new(48_000.0, graph, schedule, prod, tprod, xruns),
            cons,
        )
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
    fn telemetry_frame_carries_meters_load_and_xruns() {
        // Build an RtState keeping the telemetry consumer, plus a shared xrun
        // counter we bump to prove it is stamped onto the frame.
        let (init, _index, _project, _path) = crate::control::build_demo().expect("demo builds");
        let (graph, schedule) = init(48_000.0, MAX_BLOCK).expect("graph compiles");
        let (prod, _cons) = rtrb::RingBuffer::<Garbage>::new(8);
        let (tprod, mut tcons) = rtrb::RingBuffer::<TelemetryFrame>::new(64);
        let xruns = Arc::new(AtomicU32::new(0));
        let mut rt = RtState::new(48_000.0, graph, schedule, prod, tprod, Arc::clone(&xruns));

        xruns.store(3, Ordering::Relaxed); // as if the device reported 3 underruns
        rt.handle(ControlMsg::Transport(TransportCmd::Play));
        let mut out = vec![0.0f32; 256 * 2];
        rt.process(&mut out, 2);

        let frame = tcons.pop().expect("a telemetry frame was pushed");
        assert_eq!(
            frame.xruns, 3,
            "xruns must be stamped from the shared counter"
        );
        assert!(
            frame.meter_count as usize >= 1,
            "the demo graph has metered nodes"
        );
        assert!(
            frame.audio_load >= 0.0 && frame.audio_load.is_finite(),
            "audio load must be a finite ratio: {}",
            frame.audio_load
        );
        // The sequencer's downbeat is sounding, so some node meter is non-zero.
        let peak = frame.meters[..frame.meter_count as usize]
            .iter()
            .fold(0.0f32, |m, x| m.max(x.peak));
        assert!(peak > 0.0, "playing graph should register on a node meter");
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

    // --- Diff-aware reload reconciliation (no re-attack of unchanged notes) ---

    /// One sustained note as two scheduled events, on the instrument (node 0).
    fn note(pitch: u8, on: u64, off: u64) -> [ScheduledEvent; 2] {
        [
            ScheduledEvent {
                pos: on,
                node: 0,
                kind: NoteKind::On { pitch, vel: 100 },
            },
            ScheduledEvent {
                pos: off,
                node: 0,
                kind: NoteKind::Off { pitch },
            },
        ]
    }

    /// A playing `RtState` whose held notes are `schedule`'s, with the playhead
    /// nudged just past the downbeat and the note queue cleared — so a later
    /// `SetSchedule` reconciles against held notes, not the downbeat.
    fn playing_with(mut events: Vec<ScheduledEvent>) -> RtState {
        let (init, _index, _project, _path) = crate::control::build_demo().expect("demo builds");
        let (graph, _sched) = init(48_000.0, MAX_BLOCK).expect("graph compiles");
        let (prod, _cons) = rtrb::RingBuffer::<Garbage>::new(8);
        let (tprod, _t) = rtrb::RingBuffer::<TelemetryFrame>::new(64);
        let xruns = Arc::new(AtomicU32::new(0));
        // The schedule must be sorted by position (advance/active_notes stop at
        // the first event past the playhead).
        events.sort_by_key(|e| e.pos);
        let schedule = Schedule {
            events,
            loop_len: 1_000_000,
        };
        let mut rt = RtState::new(48_000.0, graph, schedule, prod, tprod, xruns);
        rt.handle(ControlMsg::Transport(TransportCmd::Play));
        // Process one block: the downbeat notes are struck (via `advance`) and
        // consumed, `scheduled_on` now holds them, and the queue is clear.
        let mut out = vec![0.0f32; 64];
        rt.process(&mut out, 1);
        rt
    }

    fn pushed_on(notes: &[NoteEvent], pitch: u8) -> bool {
        notes
            .iter()
            .any(|e| matches!(e.kind, NoteKind::On { pitch: p, .. } if p == pitch))
    }
    fn pushed_off(notes: &[NoteEvent], pitch: u8) -> bool {
        notes
            .iter()
            .any(|e| matches!(e.kind, NoteKind::Off { pitch: p } if p == pitch))
    }

    #[test]
    fn reload_keeps_an_unchanged_note_while_adding_others() {
        // The dominant live-coding case: a note is sounding while two notes are
        // added elsewhere in the sequence. The sounding note is unchanged, so the
        // reload must push NO note event for it — it keeps ringing, never
        // re-attacked. This is exactly what every Claude Code save looks like.
        let mut rt = playing_with(note(60, 0, 500_000).to_vec());

        let mut next = note(60, 0, 500_000).to_vec(); // the unchanged held note
        next.extend(note(67, 100_000, 110_000)); // added, far in the future
        next.extend(note(72, 120_000, 130_000)); // added, far in the future
        rt.handle(ControlMsg::SetSchedule(Box::new(Schedule {
            events: next,
            loop_len: 1_000_000,
        })));

        assert!(
            rt.graph.pending_notes(0).is_empty(),
            "an unchanged sounding note must not be released or re-struck on reload"
        );
    }

    #[test]
    fn reload_with_a_pitch_change_releases_old_and_strikes_new() {
        let mut rt = playing_with(note(60, 0, 500_000).to_vec());
        rt.handle(ControlMsg::SetSchedule(Box::new(Schedule {
            events: note(64, 0, 500_000).to_vec(), // same span, pitch 60 → 64
            loop_len: 1_000_000,
        })));
        let pending = rt.graph.pending_notes(0);
        assert!(pushed_off(pending, 60), "the old pitch must be released");
        assert!(pushed_on(pending, 64), "the new pitch must be struck");
        assert_eq!(pending.len(), 2, "nothing else should be touched");
    }

    #[test]
    fn reload_releases_only_the_removed_note() {
        // Two notes sounding; the reload drops one and keeps the other. Only the
        // removed note is released; the kept note is left untouched.
        let mut held = note(60, 0, 500_000).to_vec();
        held.extend(note(62, 0, 500_000));
        let mut rt = playing_with(held);

        rt.handle(ControlMsg::SetSchedule(Box::new(Schedule {
            events: note(60, 0, 500_000).to_vec(), // 62 removed
            loop_len: 1_000_000,
        })));
        let pending = rt.graph.pending_notes(0);
        assert!(pushed_off(pending, 62), "the removed note must be released");
        assert!(
            !pushed_off(pending, 60),
            "the kept note must not be released"
        );
        assert!(
            !pushed_on(pending, 60),
            "the kept note must not be re-struck"
        );
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn reload_strikes_a_note_newly_overlapping_the_playhead() {
        // Nothing held; the reload adds a note already in progress at the
        // playhead → it must be struck so it sounds without waiting for a downbeat.
        let mut rt = playing_with(Vec::new());
        rt.handle(ControlMsg::SetSchedule(Box::new(Schedule {
            events: note(60, 0, 500_000).to_vec(),
            loop_len: 1_000_000,
        })));
        let pending = rt.graph.pending_notes(0);
        assert!(
            pushed_on(pending, 60),
            "a newly in-progress note must be struck"
        );
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn reload_releases_a_note_shortened_under_the_playhead() {
        // A sounding note shortened so its new NoteOff now sits before the
        // playhead is no longer in progress → it must be released, not left
        // ringing (no stuck voice).
        let mut rt = playing_with(note(60, 0, 500_000).to_vec());
        rt.handle(ControlMsg::SetSchedule(Box::new(Schedule {
            events: note(60, 0, 32).to_vec(), // Off at 32, playhead is at 64
            loop_len: 1_000_000,
        })));
        let pending = rt.graph.pending_notes(0);
        assert!(
            pushed_off(pending, 60),
            "a note shortened under the playhead must be released"
        );
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn reload_restrikes_a_note_in_progress() {
        // Regression: reloading mid-sequence must restrike a note the new
        // sequence already has in progress at the playhead, so there is no gap.
        let (init, _index, _project, _path) = crate::control::build_demo().expect("demo builds");
        let (graph, _sched) = init(48_000.0, MAX_BLOCK).expect("graph compiles");
        let (prod, _cons) = rtrb::RingBuffer::<Garbage>::new(8);
        let empty = Schedule {
            events: vec![],
            loop_len: 192_000,
        };
        let (tprod, _t) = rtrb::RingBuffer::<TelemetryFrame>::new(64);
        let xruns = Arc::new(AtomicU32::new(0));
        let mut rt = RtState::new(48_000.0, graph, empty, prod, tprod, xruns);

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
        // The note is in progress at the playhead and was not held before, so the
        // reload must strike it (asserted on the pushed events, independent of the
        // demo mix level).
        assert!(
            pushed_on(rt.graph.pending_notes(0), 60),
            "in-progress note was not restruck on reload"
        );
    }

    #[test]
    fn stopped_schedule_swap_stays_silent() {
        // Regression: when stopped, a schedule swap is how a note edit (add/move/
        // resize/delete) reaches the audio thread. The playhead sits at the start,
        // over the sequence's first notes, but an edit must make no sound — only an
        // explicit preview (the create echo) does. So restriking is gated on play.
        let (mut rt, _garbage) = demo_rt();
        let mut out = vec![0.0f32; 4096 * 2];

        // A schedule whose notes are already in progress at the playhead (pos 0).
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
        // Never played: still stopped at the start.
        rt.handle(ControlMsg::SetSchedule(next));
        rt.process(&mut out, 2);
        let peak = out.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak < 1e-4, "a stopped edit made sound: {peak}");
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
            1,
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
            tails: vec![0],
            track_mix: vec![],
            master_gain_db: 0.0,
        };
        let graph_a = compile(&tail, test_support::create, 48_000.0, MAX_BLOCK, 1).unwrap();
        let (prod, _cons) = rtrb::RingBuffer::<Garbage>::new(8);
        let (tprod, _t) = rtrb::RingBuffer::<TelemetryFrame>::new(64);
        let xruns = Arc::new(AtomicU32::new(0));
        let mut rt = RtState::new(48_000.0, graph_a, Schedule::default(), prod, tprod, xruns);

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
            tails: vec![0],
            track_mix: vec![],
            master_gain_db: 0.0,
        };
        let graph_b = compile(&silence, test_support::create, 48_000.0, MAX_BLOCK, 1).unwrap();
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
