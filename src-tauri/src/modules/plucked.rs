//! Plucked-string synthesiser (Karplus-Strong): 8-voice polyphony, a noise burst
//! excites a tuned delay loop with a damping low-pass and energy decay.
//!
//! `broken` ages the instrument: each key above C3 gets a stable, hashed
//! detune that grows with the register — low notes stay honest, the top turns
//! sour the SAME way every time, like the bent tines of a worn music box
//! (the Pianoteq-"Condition" idea, at the instrument where pitch is decided).
//!
//! All DSP is hand-written. `process` is RT-SAFE: a fixed voice array, per-voice
//! delay buffers allocated once in `prepare`, no allocation or locks on the
//! audio path. Voice stealing takes the oldest voice.

use crate::engine::node::{
    AudioNode, NodeIo, NoteKind, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx,
    SignalKind,
};

const VOICES: usize = 8;
/// Lowest note the delay buffers must hold a full period of (≈ MIDI 16, 20.6 Hz).
const MIN_FREQ: f32 = 20.0;

const P_BRIGHTNESS: ParamId = 0;
const P_DECAY: ParamId = 1;
const P_RELEASE: ParamId = 2;
const P_LEVEL: ParamId = 3;
// Appended (ids are frozen — presets address params by name, order by id).
const P_BROKEN: ParamId = 4;

/// `broken` detune ceiling (cents) for the very top of the register at 1.0 —
/// a full 1.5 semitone: properly deranged. The knob's response is squared so
/// the lower half stays in the musical "slightly haunted" zone.
const BROKEN_MAX_CENTS: f32 = 150.0;

const PARAMS: [ParamSpec; 5] = [
    ParamSpec {
        id: P_BRIGHTNESS,
        name: "brightness",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.3,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_DECAY,
        name: "decay",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.7,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_RELEASE,
        name: "release",
        unit: "ms",
        min: 1.0,
        max: 1000.0,
        default: 60.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_LEVEL,
        name: "level",
        unit: "dB",
        min: -60.0,
        max: 0.0,
        default: -6.0,
        curve: ParamCurve::Db,
    },
    ParamSpec {
        id: P_BROKEN,
        name: "broken",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
];

/// Settings shared by all voices, mapped from the params into loop coefficients.
#[derive(Clone, Copy)]
struct Settings {
    /// Loop low-pass blend `b` in `y = b·x[n] + (1-b)·x[n-1]` (higher = brighter).
    brightness: f32,
    /// Per-sample loop feedback gain `R < 1` (higher = longer ring).
    decay: f32,
    /// Note-off damping time in seconds.
    release_s: f32,
    /// "Aged instrument" amount (0 = perfectly tuned).
    broken: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            brightness: 0.5,
            decay: 0.996,
            release_s: 0.06,
            broken: 0.0,
        }
    }
}

/// Detune (cents) of a "broken" key: zero below C3, growing toward the top of
/// the register, hashed per pitch so a given key is always out of tune the
/// SAME way — the ear reads a damaged instrument, not randomness. // RT-SAFE
fn broken_cents(pitch: u8, broken: f32) -> f32 {
    if broken <= 0.0 {
        return 0.0;
    }
    // Integer hash finalizer → a stable signed offset in [-1, 1] per key.
    let mut h = (pitch as u32).wrapping_mul(0x9e37_79b9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    let signed = (h as f32 / u32::MAX as f32) * 2.0 - 1.0;
    // Register weight: honest below C3 (48), fully worn at C7 (96).
    let weight = ((pitch as f32 - 48.0) / 48.0).clamp(0.0, 1.0);
    broken * broken * BROKEN_MAX_CENTS * weight * signed
}

/// One plucked-string voice: a circular delay line fed back through a one-zero
/// low-pass (damping) and a tuning all-pass (fractional delay).
#[derive(Clone)]
struct Voice {
    buffer: Vec<f32>,
    delay_len: usize,
    ptr: usize,
    /// Previous delay output, for the loop low-pass.
    prev: f32,
    /// Tuning all-pass state and coefficient.
    ap_a: f32,
    ap_x1: f32,
    ap_y1: f32,
    amp: f32,
    /// Note-off gate (1.0 held, ramps to 0 over the release time).
    gate: f32,
    gate_step: f32,
    releasing: bool,
    /// Leaky peak follower; the voice frees itself once the string has rung out.
    level: f32,
    pitch: u8,
    active: bool,
    age: u64,
    /// Per-voice xorshift RNG for the excitation burst.
    rng: u32,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            buffer: Vec::new(),
            delay_len: 1,
            ptr: 0,
            prev: 0.0,
            ap_a: 0.0,
            ap_x1: 0.0,
            ap_y1: 0.0,
            amp: 0.0,
            gate: 0.0,
            gate_step: 0.0,
            releasing: false,
            level: 0.0,
            pitch: 0,
            active: false,
            age: 0,
            rng: 1,
        }
    }
}

impl Voice {
    // RT-SAFE
    fn next_noise(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Pluck the string: tune the loop for `pitch`, fill the delay with a noise
    /// burst scaled by velocity. // RT-SAFE
    fn pluck(&mut self, pitch: u8, vel: u8, s: &Settings, sr: f32) {
        let freq = midi_to_freq(pitch) * (broken_cents(pitch, s.broken) / 1200.0).exp2();
        // Total loop delay = buffer + low-pass group delay (1-b) + all-pass frac.
        let period = (sr / freq).max(2.0);
        let lp_delay = 1.0 - s.brightness;
        let desired = (period - lp_delay).max(1.0);
        let n = (desired.floor() as usize).clamp(1, self.buffer.len().max(1) - 1);
        let frac = (desired - n as f32).clamp(0.0, 1.0);
        self.ap_a = (1.0 - frac) / (1.0 + frac);

        self.delay_len = n;
        self.ptr = 0;
        self.prev = 0.0;
        self.ap_x1 = 0.0;
        self.ap_y1 = 0.0;
        self.amp = vel as f32 / 127.0;
        for slot in self.buffer[..n].iter_mut() {
            *slot = 0.0;
        }
        // Excite only after zeroing so the burst is exactly `n` samples long.
        for i in 0..n {
            self.buffer[i] = self.next_noise() * self.amp;
        }
        self.gate = 1.0;
        self.gate_step = 1.0 / (s.release_s.max(0.001) * sr);
        self.releasing = false;
        self.level = self.amp.max(1.0e-3);
        self.pitch = pitch;
        self.active = true;
    }

    fn note_off(&mut self) {
        self.releasing = true;
    }

    // RT-SAFE
    fn render(&mut self, s: &Settings) -> f32 {
        let cur = self.buffer[self.ptr];
        // String damping: one-zero low-pass blend.
        let lp = s.brightness * cur + (1.0 - s.brightness) * self.prev;
        self.prev = cur;
        let fed = lp * s.decay;
        // Fractional-delay tuning all-pass.
        let ap = self.ap_a * fed + self.ap_x1 - self.ap_a * self.ap_y1;
        self.ap_x1 = fed;
        self.ap_y1 = ap;
        self.buffer[self.ptr] = flush(ap);
        self.ptr += 1;
        if self.ptr >= self.delay_len {
            self.ptr = 0;
        }

        if self.releasing {
            self.gate -= self.gate_step;
            if self.gate < 0.0 {
                self.gate = 0.0;
            }
        }
        let out = cur * self.gate;

        // Free the voice once it has rung out (held notes decay too).
        self.level *= 0.9995;
        let mag = out.abs();
        if mag > self.level {
            self.level = mag;
        }
        if self.level < 1.0e-4 {
            self.active = false;
        }
        out
    }
}

pub struct Plucked {
    sample_rate: f32,
    voices: Vec<Voice>,
    settings: Settings,
    level: crate::engine::params::LinearRamp,
    next_age: u64,
}

impl Default for Plucked {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            voices: vec![Voice::default(); VOICES],
            settings: Settings::default(),
            level: crate::engine::params::LinearRamp::new(1.0, 10.0, 48_000.0),
            next_age: 0,
        }
    }
}

impl Plucked {
    fn note_on(&mut self, pitch: u8, vel: u8) {
        let idx = self.alloc_voice();
        let age = self.next_age;
        self.next_age += 1;
        let sr = self.sample_rate;
        let settings = self.settings;
        let v = &mut self.voices[idx];
        v.age = age;
        v.pluck(pitch, vel, &settings, sr);
    }

    fn note_off(&mut self, pitch: u8) {
        for v in &mut self.voices {
            if v.active && v.pitch == pitch && !v.releasing {
                v.note_off();
            }
        }
    }

    /// Pick a free voice, else steal the oldest. // RT-SAFE
    fn alloc_voice(&mut self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        let mut idx = 0;
        let mut oldest = u64::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if v.age < oldest {
                oldest = v.age;
                idx = i;
            }
        }
        idx
    }

    #[cfg(test)]
    fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

impl AudioNode for Plucked {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        for ev in io.notes() {
            match ev.kind {
                NoteKind::On { pitch, vel } => self.note_on(pitch, vel),
                NoteKind::Off { pitch } => self.note_off(pitch),
            }
        }

        let settings = self.settings;
        let out = io.output(0);
        for slot in out.iter_mut() {
            let level = self.level.tick();
            let mut sum = 0.0;
            for v in &mut self.voices {
                if v.active {
                    sum += v.render(&settings);
                }
            }
            *slot = sum * level;
        }
        io.spread_output(0);
    }

    fn ports(&self) -> PortSpec {
        PortSpec {
            inputs: vec![Port {
                name: "notes",
                kind: SignalKind::Notes,
            }],
            outputs: vec![Port {
                name: "out",
                kind: SignalKind::Audio,
            }],
        }
    }

    fn params(&self) -> &[ParamSpec] {
        &PARAMS
    }

    fn set_param(&mut self, id: ParamId, value: f32) {
        let real = PARAMS[id as usize].denormalize(value);
        match id {
            // Map brightness 0..1 to a loop low-pass blend in [0.5, 0.95].
            P_BRIGHTNESS => self.settings.brightness = 0.5 + 0.45 * real,
            // Map decay 0..1 to a loop feedback gain in [0.99, 0.9999].
            P_DECAY => self.settings.decay = 0.99 + 0.0099 * real,
            P_RELEASE => self.settings.release_s = real / 1000.0,
            P_LEVEL => self.level.set_target(real),
            // Applied at pluck time: a change only affects the NEXT notes.
            P_BROKEN => self.settings.broken = real,
            _ => {}
        }
    }

    fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            if v.active {
                v.note_off();
            }
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        let cap = (sample_rate / MIN_FREQ).ceil() as usize + 2;
        self.voices = vec![Voice::default(); VOICES];
        for (i, v) in self.voices.iter_mut().enumerate() {
            v.buffer = vec![0.0; cap];
            // Distinct, non-zero xorshift seeds (Knuth multiplicative hash).
            v.rng = (i as u32).wrapping_mul(2_654_435_761).wrapping_add(1);
        }
        self.settings = Settings {
            brightness: 0.5 + 0.45 * default_real(P_BRIGHTNESS),
            decay: 0.99 + 0.0099 * default_real(P_DECAY),
            release_s: default_real(P_RELEASE) / 1000.0,
            broken: default_real(P_BROKEN),
        };
        self.level =
            crate::engine::params::LinearRamp::new(default_real(P_LEVEL), 10.0, sample_rate);
        self.next_age = 0;
    }
}

/// Real default value for a parameter (round-trips through the curve).
fn default_real(id: ParamId) -> f32 {
    let spec = &PARAMS[id as usize];
    spec.denormalize(spec.normalize(spec.default))
}

/// MIDI note number → frequency in Hz (A4 = 69 = 440 Hz).
fn midi_to_freq(pitch: u8) -> f32 {
    440.0 * 2.0f32.powf((pitch as f32 - 69.0) / 12.0)
}

/// Zero subnormal values to avoid denormal CPU stalls in the feedback loop. // RT-SAFE
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1.0e-15 {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node::NoteEvent;

    fn synth() -> Plucked {
        let mut s = Plucked::default();
        s.prepare(48_000.0, 256);
        s
    }

    fn render(s: &mut Plucked, notes: &[NoteEvent], frames: usize, out: &mut Vec<f32>) {
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: 48_000.0,
            frames,
            channels: 1,
            samples_per_beat: 0.0,
        };
        let mut io = NodeIo::new(notes, &[], out, frames, 1);
        s.process(&ctx, &mut io);
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
    }

    #[test]
    fn plucked_note_is_audible() {
        let mut s = synth();
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 52,
                vel: 110,
            },
        }];
        let mut buf = Vec::new();
        render(&mut s, &on, 2048, &mut buf);
        assert!(peak(&buf) > 0.05, "pluck is inaudible: {}", peak(&buf));
    }

    #[test]
    fn plucked_note_decays_like_a_string() {
        // A plucked string must ring down: late energy < early energy.
        let mut s = synth();
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 45,
                vel: 120,
            },
        }];
        let mut buf = Vec::new();
        render(&mut s, &on, 48_000, &mut buf); // 1 second
        let early = peak(&buf[..4_800]);
        let late = peak(&buf[43_200..]);
        assert!(early > 0.05, "no initial pluck: {early}");
        assert!(
            late < early * 0.6,
            "string did not decay: {early} -> {late}"
        );
    }

    #[test]
    fn eight_voices_then_oldest_is_stolen() {
        let mut s = synth();
        let mut buf = Vec::new();
        let ons: Vec<NoteEvent> = (52u8..60)
            .map(|p| NoteEvent {
                frame: 0,
                kind: NoteKind::On { pitch: p, vel: 100 },
            })
            .collect();
        render(&mut s, &ons, 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8);
        let ninth = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 64,
                vel: 100,
            },
        }];
        render(&mut s, &ninth, 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8);
    }

    /// Goertzel-style magnitude of a single bin, normalised by length.
    fn mag_at(buf: &[f32], freq: f32) -> f32 {
        let w = std::f32::consts::TAU * freq / 48_000.0;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, &x) in buf.iter().enumerate() {
            let p = w * n as f32;
            re += x * p.cos();
            im += x * p.sin();
        }
        (re * re + im * im).sqrt() / buf.len() as f32
    }

    #[test]
    fn broken_curve_is_stable_and_register_shaped() {
        // Honest below C3, deterministic per key, spread across the top keys.
        assert_eq!(broken_cents(40, 1.0), 0.0);
        assert_eq!(broken_cents(90, 1.0), broken_cents(90, 1.0));
        assert_eq!(broken_cents(90, 0.0), 0.0);
        let spread: Vec<f32> = (84u8..=97).map(|p| broken_cents(p, 1.0)).collect();
        assert!(
            spread.iter().any(|c| c.abs() > 20.0),
            "no key is audibly sour at full broken: {spread:?}"
        );
        let distinct = spread.windows(2).any(|w| (w[0] - w[1]).abs() > 5.0);
        assert!(
            distinct,
            "neighbouring keys are identically sour: {spread:?}"
        );
    }

    #[test]
    fn broken_detunes_a_top_note_by_the_predicted_amount() {
        // Pick the key with the biggest programmed offset in the top register
        // and verify the rendered string actually rings at the shifted pitch.
        let pitch = (84u8..=97)
            .max_by(|a, b| {
                broken_cents(*a, 1.0)
                    .abs()
                    .total_cmp(&broken_cents(*b, 1.0).abs())
            })
            .unwrap();
        let nominal = midi_to_freq(pitch);
        let expected = nominal * (broken_cents(pitch, 1.0) / 1200.0).exp2();

        let mut s = synth();
        s.set_param(P_BROKEN, 1.0);
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On { pitch, vel: 110 },
        }];
        let mut buf = Vec::new();
        render(&mut s, &on, 16_384, &mut buf);
        let at_expected = mag_at(&buf[4096..], expected);
        let at_nominal = mag_at(&buf[4096..], nominal);
        assert!(
            at_expected > 2.0 * at_nominal,
            "string did not ring at the broken pitch: expected {expected} Hz \
             ({at_expected}) vs nominal {nominal} Hz ({at_nominal})"
        );
    }

    #[test]
    fn note_off_damps_faster_than_held() {
        let mut s = synth();
        let mut held_buf = Vec::new();
        let on = [NoteEvent {
            frame: 0,
            kind: NoteKind::On {
                pitch: 50,
                vel: 110,
            },
        }];
        render(&mut s, &on, 4_096, &mut held_buf);
        // A second synth: same pluck, then released.
        let mut s2 = synth();
        let mut tmp = Vec::new();
        render(&mut s2, &on, 4_096, &mut tmp);
        let off = [NoteEvent {
            frame: 0,
            kind: NoteKind::Off { pitch: 50 },
        }];
        let mut released = Vec::new();
        render(&mut s2, &off, 8_000, &mut released);
        // After the release window the damped voice is quieter than the held one.
        let held_tail = peak(&held_buf[3_000..]);
        let released_tail = peak(&released[6_000..]);
        assert!(
            released_tail < held_tail,
            "note-off did not damp: held {held_tail}, released {released_tail}"
        );
    }
}
