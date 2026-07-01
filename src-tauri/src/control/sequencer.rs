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
    /// Samples per beat (from bpm + time signature), the metronome's grid.
    /// Zero means unknown — the metronome stays silent.
    pub samples_per_beat: u64,
    /// Beats per bar (time-signature numerator); the bar start is accented.
    pub beats_per_bar: u32,
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

    /// Fill `out` with every note in progress at the current playhead — its
    /// NoteOn at or before the playhead, its NoteOff strictly after — as
    /// `(node, pitch, vel)`, and return how many. A note whose NoteOff has
    /// already passed (e.g. shortened under the playhead) is not in progress and
    /// is left out. Used after a reseat (reload/seek) to reconcile the sounding
    /// notes with the new sequence.
    ///
    /// Bounded by `out.len()`; the on/off bookkeeping writes only into `out`, so
    /// it allocates nothing. // RT-SAFE
    pub fn active_notes(&self, schedule: &Schedule, out: &mut [(NodeIdx, u8, u8)]) -> usize {
        let mut len = 0usize;
        for ev in &schedule.events {
            if ev.pos > self.playhead {
                break; // events are sorted; nothing after the playhead is on yet
            }
            match ev.kind {
                NoteKind::On { pitch, vel } => {
                    if let Some(slot) = out[..len]
                        .iter_mut()
                        .find(|n| n.0 == ev.node && n.1 == pitch)
                    {
                        slot.2 = vel;
                    } else if len < out.len() {
                        out[len] = (ev.node, pitch, vel);
                        len += 1;
                    }
                }
                NoteKind::Off { pitch } => {
                    if let Some(i) = out[..len]
                        .iter()
                        .position(|n| n.0 == ev.node && n.1 == pitch)
                    {
                        len -= 1;
                        out[i] = out[len];
                    }
                }
            }
        }
        len
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
            ..Default::default()
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
    fn active_notes_returns_notes_in_progress_at_the_playhead() {
        // Note A spans [0, 400) at pitch 60; note B spans [200, 600) at pitch 62.
        let schedule = Schedule {
            events: vec![
                ev(0, 60, true),
                ev(200, 62, true),
                ev(400, 60, false),
                ev(600, 62, false),
            ],
            loop_len: 1000,
            ..Default::default()
        };
        let mut cur = SequencerCursor::new();
        let mut out = [(0usize, 0u8, 0u8); 16];

        // At 300: both A and B are in progress.
        cur.seek(300, &schedule);
        let n = cur.active_notes(&schedule, &mut out);
        let mut got: Vec<u8> = out[..n].iter().map(|x| x.1).collect();
        got.sort_unstable();
        assert_eq!(got, vec![60, 62]);

        // At 100: only A.
        cur.seek(100, &schedule);
        let n = cur.active_notes(&schedule, &mut out);
        assert_eq!(n, 1);
        assert_eq!(out[0].1, 60);

        // At 500: A has ended (its Off at 400 has passed), only B remains.
        cur.seek(500, &schedule);
        let n = cur.active_notes(&schedule, &mut out);
        assert_eq!(n, 1);
        assert_eq!(out[0].1, 62);
    }

    #[test]
    fn active_notes_excludes_a_note_whose_off_has_passed() {
        // A note shortened so its Off (200) sits before the playhead (300) is no
        // longer in progress — the reconciliation will release it, not keep it.
        let schedule = Schedule {
            events: vec![ev(0, 60, true), ev(200, 60, false)],
            loop_len: 1000,
            ..Default::default()
        };
        let mut cur = SequencerCursor::new();
        cur.seek(300, &schedule);
        let mut out = [(0usize, 0u8, 0u8); 16];
        assert_eq!(cur.active_notes(&schedule, &mut out), 0);
    }

    #[test]
    fn active_notes_captures_the_latest_velocity() {
        let schedule = Schedule {
            events: vec![
                ScheduledEvent {
                    pos: 0,
                    node: 0,
                    kind: NoteKind::On { pitch: 60, vel: 80 },
                },
                ScheduledEvent {
                    pos: 100,
                    node: 0,
                    kind: NoteKind::On {
                        pitch: 60,
                        vel: 120,
                    },
                },
            ],
            loop_len: 1000,
            ..Default::default()
        };
        let mut cur = SequencerCursor::new();
        cur.seek(200, &schedule);
        let mut out = [(0usize, 0u8, 0u8); 16];
        let n = cur.active_notes(&schedule, &mut out);
        assert_eq!(n, 1);
        assert_eq!(out[0].2, 120, "the later velocity wins");
    }

    #[test]
    fn playhead_does_not_drift_over_100_loops() {
        let loop_len = 96_000u64; // one 4/4 bar at 120 bpm / 48 kHz
        let schedule = Schedule {
            events: vec![ev(0, 60, true)],
            loop_len,
            ..Default::default()
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
