//! State owned exclusively by the audio callback.
//!
//! Everything here runs on the real-time thread and must stay RT-SAFE: no
//! allocation, no locks, no I/O. The M1 engine is a single hard-wired 440 Hz
//! sine with a click-free master ramp; the graph arrives at M2.

use std::f32::consts::TAU;

use crate::control::{ControlMsg, TransportCmd};

/// Tone frequency for M1, in hertz.
const FREQ_HZ: f32 = 440.0;
/// Output amplitude, kept well below full scale to leave headroom.
const AMPLITUDE: f32 = 0.2;
/// Master gain ramp on play/stop, in milliseconds (avoids clicks).
const RAMP_MS: f32 = 10.0;

pub struct RtState {
    /// Phase accumulator in turns (0..1), not radians, to avoid large values.
    phase: f32,
    /// Phase increment per sample: `freq / sample_rate`.
    phase_inc: f32,
    /// Current master gain, moved toward `target_gain` one step per sample.
    gain: f32,
    /// Gain target: 1.0 while playing, 0.0 while stopped.
    target_gain: f32,
    /// Per-sample gain increment realising the `RAMP_MS` ramp.
    gain_step: f32,
}

impl RtState {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            phase_inc: FREQ_HZ / sample_rate,
            gain: 0.0,
            target_gain: 0.0,
            gain_step: 1.0 / (RAMP_MS / 1000.0 * sample_rate),
        }
    }

    /// Apply a control message. // RT-SAFE
    pub fn handle(&mut self, msg: ControlMsg) {
        match msg {
            ControlMsg::Transport(TransportCmd::Play) => self.target_gain = 1.0,
            ControlMsg::Transport(TransportCmd::Stop) => self.target_gain = 0.0,
        }
    }

    /// Render `out` (interleaved, `channels` per frame), writing the same mono
    /// signal to every channel. // RT-SAFE
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        for frame in out.chunks_mut(channels) {
            self.gain = step_toward(self.gain, self.target_gain, self.gain_step);

            let sample = (self.phase * TAU).sin() * AMPLITUDE * self.gain;
            self.phase += self.phase_inc;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }

            for slot in frame.iter_mut() {
                *slot = sample;
            }
        }
    }
}

/// Move `current` toward `target` by at most `step`. // RT-SAFE
fn step_toward(current: f32, target: f32, step: f32) -> f32 {
    if current < target {
        (current + step).min(target)
    } else if current > target {
        (current - step).max(target)
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Guard allocator: while the current thread has allocation denied, every
    // allocation bumps a global counter so a test can assert that an RT path
    // allocated nothing. The flag is thread-local (with a `const` initialiser
    // so its own setup never allocates), keeping parallel tests independent.
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

    fn render(rt: &mut RtState, blocks: usize, out: &mut Vec<f32>) {
        let mut buf = vec![0.0f32; 256 * 2];
        for _ in 0..blocks {
            buf.iter_mut().for_each(|s| *s = 0.0);
            rt.process(&mut buf, 2);
            out.extend_from_slice(&buf);
        }
    }

    #[test]
    fn process_does_not_allocate() {
        let mut rt = RtState::new(48_000.0);
        let mut buf = vec![0.0f32; 256 * 2]; // pre-allocate before the guard

        VIOLATIONS.store(0, Ordering::SeqCst);
        DENY.with(|d| d.set(true));
        rt.handle(ControlMsg::Transport(TransportCmd::Play));
        rt.process(&mut buf, 2);
        rt.handle(ControlMsg::Transport(TransportCmd::Stop));
        rt.process(&mut buf, 2);
        DENY.with(|d| d.set(false));

        assert_eq!(
            VIOLATIONS.load(Ordering::SeqCst),
            0,
            "the audio path allocated"
        );
    }

    #[test]
    fn transport_ramps_are_click_free() {
        let mut rt = RtState::new(48_000.0);
        let mut out = Vec::new();

        rt.handle(ControlMsg::Transport(TransportCmd::Play));
        render(&mut rt, 20, &mut out); // ramp up, then steady
        rt.handle(ControlMsg::Transport(TransportCmd::Stop));
        render(&mut rt, 20, &mut out); // ramp down to silence

        let ch0: Vec<f32> = out.iter().step_by(2).copied().collect();
        let max_delta = ch0
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_delta < 0.05, "inter-sample jump {max_delta} (click)");

        let tail = ch0.last().copied().unwrap_or(1.0).abs();
        assert!(tail < 1e-3, "output not silent after stop: {tail}");
    }
}
