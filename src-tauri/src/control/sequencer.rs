//! Sequence playback model: a precomputed, sorted list of timed note events and
//! the cursor that walks it block by block.
//!
//! The cursor is shared by the audio thread (`RtState`) and the offline renderer
//! so the two playback paths can never diverge (MVP_PLAN M3, step 4). The sample
//! counter is the master clock; positions are integers throughout — time is
//! never accumulated in floating point, so playback cannot drift.

use crate::engine::node::{NodeIdx, NoteEvent, NoteKind};

/// Pulses per quarter note. The textual `song.json` positions are expressed in
/// bars/beats/ticks at this resolution.
pub const PPQ: u64 = 960;

/// Convert a tick position to an absolute sample count. All-integer (`u128`
/// intermediate) so the conversion is exact and drift-free:
/// `samples = ticks * sr * 60 / (bpm * PPQ)`.
pub fn ticks_to_samples(ticks: u64, bpm: u32, sample_rate: u32) -> u64 {
    (ticks as u128 * sample_rate as u128 * 60 / (bpm as u128 * PPQ as u128)) as u64
}

/// One scheduled note event at an absolute sample position from the loop start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledEvent {
    pub pos: u64,
    pub node: NodeIdx,
    pub kind: NoteKind,
}

/// A compiled sequence: events sorted by position, plus the loop length in
/// samples. Built off the audio thread; owned by whoever plays it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schedule {
    /// Sorted by `pos`; at equal positions, `NoteOff` comes before `NoteOn`.
    pub events: Vec<ScheduledEvent>,
    /// Loop length in samples. The playhead wraps modulo this. Zero means an
    /// empty schedule with nothing to play.
    pub loop_len: u64,
}

/// Walks a [`Schedule`] forward in sample time, emitting the events that fall in
/// each processed block. Holds only the playhead and an index into the sorted
/// event list, so `advance` allocates nothing. // RT-SAFE
pub struct SequencerCursor {
    playhead: u64,
    /// Index of the next event to consider in the current loop iteration.
    /// Invariant: every event before it has `pos < playhead`.
    idx: usize,
}

impl Default for SequencerCursor {
    fn default() -> Self {
        Self::new()
    }
}

impl SequencerCursor {
    pub fn new() -> Self {
        Self {
            playhead: 0,
            idx: 0,
        }
    }

    pub fn playhead(&self) -> u64 {
        self.playhead
    }

    /// Rewind to the loop start. // RT-SAFE
    pub fn reset(&mut self) {
        self.playhead = 0;
        self.idx = 0;
    }

    /// Jump to an absolute sample position (wrapped into the loop) and re-seat
    /// the event index. Used by `Seek` and after a schedule swap so the new
    /// sequence resumes at the current position. // RT-SAFE
    pub fn seek(&mut self, pos: u64, schedule: &Schedule) {
        self.playhead = if schedule.loop_len > 0 {
            pos % schedule.loop_len
        } else {
            pos
        };
        self.reseat(schedule);
    }

    /// Re-seat `idx` to the first event at or after the current playhead (a
    /// binary search over the sorted list — no allocation). // RT-SAFE
    pub fn reseat(&mut self, schedule: &Schedule) {
        self.idx = schedule.events.partition_point(|e| e.pos < self.playhead);
    }

    /// Emit a NoteOn for every note already in progress at the current playhead
    /// (its NoteOn is at or before the playhead, its NoteOff strictly after). Run
    /// after a reseat (reload/seek) so resuming mid-sequence restrikes the
    /// sounding notes instead of leaving a gap until the next downbeat.
    ///
    /// Bounded by the number of events up to the playhead and only called on a
    /// (rare) reload; the on/off bookkeeping uses a fixed stack scratch, so it
    /// allocates nothing. // RT-SAFE
    pub fn strike_active(&self, schedule: &Schedule, mut emit: impl FnMut(NodeIdx, NoteEvent)) {
        const SCRATCH: usize = 128;
        let mut active: [(NodeIdx, u8, u8); SCRATCH] = [(0, 0, 0); SCRATCH];
        let mut len = 0usize;

        for ev in &schedule.events {
            if ev.pos > self.playhead {
                break; // events are sorted; nothing after the playhead is on yet
            }
            match ev.kind {
                NoteKind::On { pitch, vel } => {
                    if let Some(slot) = active[..len]
                        .iter_mut()
                        .find(|n| n.0 == ev.node && n.1 == pitch)
                    {
                        slot.2 = vel;
                    } else if len < SCRATCH {
                        active[len] = (ev.node, pitch, vel);
                        len += 1;
                    }
                }
                NoteKind::Off { pitch } => {
                    if let Some(i) = active[..len]
                        .iter()
                        .position(|n| n.0 == ev.node && n.1 == pitch)
                    {
                        len -= 1;
                        active[i] = active[len];
                    }
                }
            }
        }

        for &(node, pitch, vel) in &active[..len] {
            emit(
                node,
                NoteEvent {
                    frame: 0,
                    kind: NoteKind::On { pitch, vel },
                },
            );
        }
    }

    /// Advance the playhead by `frames` samples, emitting every event that falls
    /// in the covered window and handling the loop wrap. `emit` receives the
    /// target node and a block-relative note event. // RT-SAFE
    pub fn advance(
        &mut self,
        frames: usize,
        schedule: &Schedule,
        mut emit: impl FnMut(NodeIdx, NoteEvent),
    ) {
        let total = frames as u64;
        if schedule.loop_len == 0 {
            // Nothing to loop over; just move the clock forward.
            self.playhead = self.playhead.wrapping_add(total);
            return;
        }

        let mut consumed = 0u64;
        while consumed < total {
            let until_wrap = schedule.loop_len - self.playhead;
            let step = (total - consumed).min(until_wrap);
            let block_end = self.playhead + step;

            while self.idx < schedule.events.len() {
                let ev = schedule.events[self.idx];
                if ev.pos >= block_end {
                    break;
                }
                // `ev.pos >= playhead` holds by the cursor invariant, so the
                // block-relative frame offset is non-negative.
                let frame = (consumed + (ev.pos - self.playhead)) as usize;
                emit(
                    ev.node,
                    NoteEvent {
                        frame,
                        kind: ev.kind,
                    },
                );
                self.idx += 1;
            }

            self.playhead = block_end;
            consumed += step;
            if self.playhead == schedule.loop_len {
                self.playhead = 0;
                self.idx = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(pos: u64, pitch: u8, on: bool) -> ScheduledEvent {
        ScheduledEvent {
            pos,
            node: 0,
            kind: if on {
                NoteKind::On { pitch, vel: 100 }
            } else {
                NoteKind::Off { pitch }
            },
        }
    }

    #[test]
    fn ticks_to_samples_is_exact() {
        // One quarter note (960 ticks) at 120 bpm, 48 kHz = 0.5 s = 24 000.
        assert_eq!(ticks_to_samples(960, 120, 48_000), 24_000);
        // A whole 4/4 bar (4 quarters) = 96 000 samples.
        assert_eq!(ticks_to_samples(4 * 960, 120, 48_000), 96_000);
    }

    #[test]
    fn advance_emits_events_in_window_with_offsets() {
        let schedule = Schedule {
            events: vec![ev(0, 60, true), ev(100, 60, false), ev(300, 62, true)],
            loop_len: 1000,
        };
        let mut cur = SequencerCursor::new();
        let mut got: Vec<(usize, u64)> = Vec::new();
        // First block [0, 200): events at 0 and 100.
        cur.advance(200, &schedule, |_n, e| got.push((e.frame, 0)));
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), vec![0, 100]);
        assert_eq!(cur.playhead(), 200);
        // Second block [200, 400): event at 300, frame offset 100.
        got.clear();
        cur.advance(200, &schedule, |_n, e| got.push((e.frame, 0)));
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), vec![100]);
    }

    #[test]
    fn playhead_does_not_drift_over_100_loops() {
        let loop_len = 96_000u64; // one 4/4 bar at 120 bpm / 48 kHz
        let schedule = Schedule {
            events: vec![ev(0, 60, true)],
            loop_len,
        };
        let mut cur = SequencerCursor::new();
        // Advance with an awkward, non-divisor block size for many loops.
        let block = 257usize;
        let blocks = (loop_len * 100) / block as u64;
        let mut emitted = 0usize;
        for _ in 0..blocks {
            cur.advance(block, &schedule, |_n, _e| emitted += 1);
        }
        // The exact theoretical position after this many whole blocks.
        let advanced = blocks * block as u64;
        assert_eq!(cur.playhead(), advanced % loop_len);
        // The downbeat event fired once per completed loop, no duplicates/gaps.
        assert_eq!(emitted as u64, advanced / loop_len + 1);
    }
}
