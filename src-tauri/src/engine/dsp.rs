//! Shared DSP toolkit: small, hand-written building blocks reused across modules.
//!
//! These are the primitives that more than one module needs, factored out so they
//! never drift between copies. Everything here is RT-SAFE after `prepare`-time
//! allocation: no allocation, no locks in the tick/process paths.

use std::f32::consts::TAU;

/// Zero subnormal values to avoid denormal CPU stalls. Mandatory in any feedback
/// loop (filter states, delay feedback), whose tails decay toward subnormal
/// values that are catastrophically slow on x86. // RT-SAFE
#[inline]
pub fn flush(x: f32) -> f32 {
    if x.abs() < 1.0e-15 {
        0.0
    } else {
        x
    }
}

/// xorshift32 seed — any non-zero constant; fixed so offline renders repeat.
pub const RNG_SEED: u32 = 0x9e37_79b9;

/// Next white-noise sample in ±1 (xorshift32) — deterministic, allocation-free.
/// // RT-SAFE
pub fn next_white(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// RBJ cookbook biquad, transposed direct form II. // RT-SAFE
#[derive(Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn set_lowpass(&mut self, fc: f32, q: f32, sr: f32) {
        let (cos_w0, alpha) = Self::omega(fc, q, sr);
        let a0 = 1.0 + alpha;
        self.b1 = (1.0 - cos_w0) / a0;
        self.b0 = self.b1 / 2.0;
        self.b2 = self.b0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    pub fn set_highpass(&mut self, fc: f32, q: f32, sr: f32) {
        let (cos_w0, alpha) = Self::omega(fc, q, sr);
        let a0 = 1.0 + alpha;
        self.b1 = -(1.0 + cos_w0) / a0;
        self.b0 = -self.b1 / 2.0;
        self.b2 = self.b0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    /// Constant-skirt bandpass (peak gain = Q).
    pub fn set_bandpass(&mut self, fc: f32, q: f32, sr: f32) {
        let (cos_w0, alpha) = Self::omega(fc, q, sr);
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.b1 = 0.0;
        self.b2 = -self.b0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    pub fn set_notch(&mut self, fc: f32, q: f32, sr: f32) {
        let (cos_w0, alpha) = Self::omega(fc, q, sr);
        let a0 = 1.0 + alpha;
        self.b0 = 1.0 / a0;
        self.b1 = (-2.0 * cos_w0) / a0;
        self.b2 = self.b0;
        self.a1 = self.b1;
        self.a2 = (1.0 - alpha) / a0;
    }

    /// Shared RBJ intermediates: clamped normalized frequency → (cos ω₀, α).
    fn omega(fc: f32, q: f32, sr: f32) -> (f32, f32) {
        let w0 = TAU * (fc / sr).clamp(1.0e-4, 0.49);
        (w0.cos(), w0.sin() / (2.0 * q.max(0.1)))
    }

    // RT-SAFE
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        self.z1 = flush(self.z1);
        self.z2 = flush(self.z2);
        y
    }
}

/// Circular delay line with fractional (linear-interpolated) reads.
///
/// `prepare` allocates the buffer — call it off the audio thread; `read`/`write`
/// are RT-SAFE. Convention: read *before* writing the current input sample;
/// `read(d)` returns the sample written `d` calls to `write` ago (`d = 1` is the
/// most recent). Written samples are denormal-flushed, so a decaying feedback
/// loop through the line never stalls on subnormals.
#[derive(Default)]
pub struct DelayLine {
    buf: Vec<f32>,
    write: usize,
}

impl DelayLine {
    /// Size the line for at most `max_delay` samples and clear it. Allocates:
    /// `prepare`-time only, never on the audio thread.
    pub fn prepare(&mut self, max_delay: usize) {
        self.buf.clear();
        self.buf.resize(max_delay.max(1) + 1, 0.0);
        self.write = 0;
    }

    // RT-SAFE
    pub fn write(&mut self, x: f32) {
        if self.buf.is_empty() {
            return;
        }
        self.buf[self.write] = flush(x);
        self.write += 1;
        if self.write == self.buf.len() {
            self.write = 0;
        }
    }

    /// Fractional read `delay` samples back, linear interpolation. // RT-SAFE
    pub fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        if len == 0 {
            return 0.0;
        }
        let d = delay.clamp(1.0, (len - 1) as f32);
        let pos = self.write as f32 - d + len as f32;
        let i = pos as usize;
        let frac = pos - i as f32;
        let a = self.buf[i % len];
        let b = self.buf[(i + 1) % len];
        a + (b - a) * frac
    }

    /// Integer read `delay` samples back (comb/allpass taps). // RT-SAFE
    pub fn read_at(&self, delay: usize) -> f32 {
        let len = self.buf.len();
        if len == 0 {
            return 0.0;
        }
        let d = delay.clamp(1, len - 1);
        self.buf[(self.write + len - d) % len]
    }
}

/// Peak envelope follower: one-pole smoothing of the rectified signal, with
/// separate attack (level rising) and release (level falling) time constants.
/// Drives dynamics processing (compressor gain computer). // RT-SAFE
#[derive(Default)]
pub struct EnvFollower {
    env: f32,
    attack_coef: f32,
    release_coef: f32,
}

impl EnvFollower {
    /// Set the attack/release time constants (seconds). Not RT-critical but
    /// allocation-free, so callable from `set_param`.
    pub fn set_times(&mut self, attack_s: f32, release_s: f32, sr: f32) {
        self.attack_coef = (-1.0 / (attack_s.max(1.0e-4) * sr)).exp();
        self.release_coef = (-1.0 / (release_s.max(1.0e-4) * sr)).exp();
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
    }

    /// Feed one sample, get the current envelope. // RT-SAFE
    pub fn tick(&mut self, x: f32) -> f32 {
        let level = x.abs();
        let coef = if level > self.env {
            self.attack_coef
        } else {
            self.release_coef
        };
        self.env = flush(level + (self.env - level) * coef);
        self.env
    }
}

/// Minimum release to guarantee a click-free tail (seconds). A note-off can never
/// cut the amplitude instantly, however short the release knob is set.
pub const MIN_RELEASE_S: f32 = 0.005;

/// ADSR settings shared by all voices of an instrument (its envelope shape).
#[derive(Clone, Copy)]
pub struct AdsrSettings {
    pub attack_s: f32,
    pub decay_s: f32,
    pub sustain: f32,
    pub release_s: f32,
}

impl Default for AdsrSettings {
    fn default() -> Self {
        Self {
            attack_s: 0.005,
            decay_s: 0.2,
            sustain: 0.7,
            release_s: 0.15,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Per-voice amplitude envelope. Linear segments; the release floor keeps the
/// tail click-free even at the shortest setting.
#[derive(Clone, Copy)]
pub struct Env {
    stage: Stage,
    level: f32,
}

impl Default for Env {
    fn default() -> Self {
        Self {
            stage: Stage::Idle,
            level: 0.0,
        }
    }
}

impl Env {
    pub fn note_on(&mut self) {
        self.stage = Stage::Attack;
    }

    pub fn note_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    // RT-SAFE
    pub fn tick(&mut self, s: &AdsrSettings, sr: f32) -> f32 {
        match self.stage {
            Stage::Idle => self.level = 0.0,
            Stage::Attack => {
                self.level += 1.0 / (s.attack_s.max(0.001) * sr);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level -= (1.0 - s.sustain) / (s.decay_s.max(0.001) * sr);
                if self.level <= s.sustain {
                    self.level = s.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level = s.sustain,
            Stage::Release => {
                self.level -= 1.0 / (s.release_s.max(MIN_RELEASE_S) * sr);
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settled RMS of a unit sine at `freq` through a fresh copy of `bq`.
    fn sine_rms_through(bq: Biquad, freq: f32, sr: f32) -> f32 {
        let mut bq = bq;
        let n = 9600;
        let mut tail_energy = 0.0f32;
        let tail_start = n / 2;
        for i in 0..n {
            let x = (TAU * freq * i as f32 / sr).sin();
            let y = bq.process(x);
            if i >= tail_start {
                tail_energy += y * y;
            }
        }
        (tail_energy / (n - tail_start) as f32).sqrt()
    }

    #[test]
    fn biquad_types_shape_the_spectrum() {
        let sr = 48_000.0;
        const UNIT: f32 = 0.707; // RMS of a unit sine

        let mut hp = Biquad::default();
        hp.set_highpass(2000.0, 0.707, sr);
        assert!(
            sine_rms_through(hp, 100.0, sr) < 0.03,
            "hp passed the low end"
        );
        assert!(sine_rms_through(hp, 10_000.0, sr) > 0.6, "hp lost the top");

        let mut bp = Biquad::default();
        bp.set_bandpass(1000.0, 2.0, sr);
        let centre = sine_rms_through(bp, 1000.0, sr);
        assert!(centre > 0.5 * UNIT, "bp lost its centre: {centre}");
        assert!(
            sine_rms_through(bp, 60.0, sr) < 0.2 * centre,
            "bp passed the low skirt"
        );
        assert!(
            sine_rms_through(bp, 12_000.0, sr) < 0.2 * centre,
            "bp passed the high skirt"
        );

        let mut nt = Biquad::default();
        nt.set_notch(1000.0, 4.0, sr);
        assert!(
            sine_rms_through(nt, 1000.0, sr) < 0.05,
            "notch passed its centre"
        );
        assert!(
            sine_rms_through(nt, 100.0, sr) > 0.6,
            "notch lost the low end"
        );
        assert!(
            sine_rms_through(nt, 8000.0, sr) > 0.6,
            "notch lost the high end"
        );
    }

    #[test]
    fn delay_line_reads_back_what_was_written() {
        let mut dl = DelayLine::default();
        dl.prepare(16);
        for i in 0..8 {
            dl.write(i as f32);
        }
        assert_eq!(dl.read_at(1), 7.0);
        assert_eq!(dl.read_at(8), 0.0);
        assert_eq!(dl.read(1.0), 7.0);
        // Halfway between the samples written 3 and 2 writes ago (5.0 and 6.0).
        assert!((dl.read(2.5) - 5.5).abs() < 1.0e-6);
    }

    #[test]
    fn delay_line_wraps_without_losing_history() {
        let mut dl = DelayLine::default();
        dl.prepare(4);
        for i in 0..23 {
            dl.write(i as f32);
        }
        assert_eq!(dl.read_at(1), 22.0);
        assert_eq!(dl.read_at(4), 19.0);
    }

    #[test]
    fn unprepared_delay_line_is_silent() {
        let mut dl = DelayLine::default();
        dl.write(1.0);
        assert_eq!(dl.read(1.0), 0.0);
        assert_eq!(dl.read_at(1), 0.0);
    }
}
