//! Wavetable synthesiser: 8-voice polyphony reading a morphing chain of five
//! band-limited tables — sine → triangle → saw → square → vox — with an
//! optional per-voice envelope pushing the morph position during the note
//! (the travelling spectrum of modern electronic pads and basses).
//!
//! Tables are procedural (additive synthesis in `prepare`, no assets) and
//! mip-mapped: each exists in 9 levels keeping 256, 128, … 1 harmonics. A
//! voice picks at note-on the deepest level whose top harmonic stays below
//! 0.45·sr and never changes it (no pitch-bend in the engine), so playback
//! cannot alias and the `position` morph only ever blends band-limited
//! content. All DSP is hand-written; `process` is RT-SAFE (tables are read
//! only, allocation happens in `prepare`).

use std::f32::consts::TAU;

use crate::engine::dsp::{AdsrSettings, Env, MIN_RELEASE_S};
use crate::engine::node::{
    AudioNode, NodeIo, NoteKind, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx,
    SignalKind,
};
use crate::engine::params::{LinearRamp, OnePole};

const VOICES: usize = 8;
/// Samples per table (power of two: the phase wrap is a mask).
const TABLE_LEN: usize = 2048;
/// Morph chain order; `position` 0 → 1 travels the whole chain.
const TABLES: usize = 5;
/// Mip levels: level m keeps `256 >> m` harmonics (… down to 1).
const MIPS: usize = 9;
/// Position glide time constant (seconds).
const SMOOTH_S: f32 = 0.015;

const P_POSITION: ParamId = 0;
const P_POS_ENV: ParamId = 1;
const P_POS_DECAY: ParamId = 2;
const P_ATTACK: ParamId = 3;
const P_DECAY: ParamId = 4;
const P_SUSTAIN: ParamId = 5;
const P_RELEASE: ParamId = 6;
const P_LEVEL: ParamId = 7;
// Appended (ids are frozen — presets address params by name, order by id).
const P_VIB_RATE: ParamId = 8;
const P_VIB_DEPTH: ParamId = 9;

const PARAMS: [ParamSpec; 10] = [
    ParamSpec {
        id: P_POSITION,
        name: "position",
        unit: "",
        min: 0.0,
        max: 1.0,
        default: 0.25,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_POS_ENV,
        name: "pos_env",
        unit: "",
        min: -1.0,
        max: 1.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_POS_DECAY,
        name: "pos_decay",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 300.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_ATTACK,
        name: "attack",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 5.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_DECAY,
        name: "decay",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 200.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_SUSTAIN,
        name: "sustain",
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
        max: 2000.0,
        default: 150.0,
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
    // Module-wide vibrato LFO (block-rate pitch wobble; depth 0 = off).
    ParamSpec {
        id: P_VIB_RATE,
        name: "vib_rate",
        unit: "Hz",
        min: 0.1,
        max: 12.0,
        default: 5.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_VIB_DEPTH,
        name: "vib_depth",
        unit: "ct",
        min: 0.0,
        max: 100.0,
        default: 0.0,
        curve: ParamCurve::Linear,
    },
];

/// Amplitude of harmonic `n` (1-based) in table `t` of the morph chain.
fn harmonic_amp(table: usize, n: usize) -> f32 {
    let nf = n as f32;
    match table {
        0 => {
            // sine: the fundamental only.
            if n == 1 {
                1.0
            } else {
                0.0
            }
        }
        1 => {
            // triangle: odd harmonics in 1/n², alternating sign.
            if n.is_multiple_of(2) {
                0.0
            } else {
                let sign = if ((n - 1) / 2).is_multiple_of(2) {
                    1.0
                } else {
                    -1.0
                };
                sign / (nf * nf)
            }
        }
        2 => 1.0 / nf, // saw: every harmonic in 1/n
        3 => {
            // square: odd harmonics in 1/n.
            if n.is_multiple_of(2) {
                0.0
            } else {
                1.0 / nf
            }
        }
        _ => {
            // vox: formant bumps — fundamental body plus two resonant crests.
            let g = |c: f32, w: f32| (-((nf - c) / w) * ((nf - c) / w)).exp();
            g(1.0, 1.2) + 0.7 * g(10.0, 2.5) + 0.3 * g(24.0, 4.0)
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    pitch: u8,
    active: bool,
    age: u64,
    /// Phase in turns (0..1) and its per-sample increment.
    phase: f32,
    inc: f32,
    /// Mip level picked at note-on (never changes during the note).
    mip: usize,
    amp: f32,
    env: Env,
    /// Position envelope (amp attack, its own decay, sustain 0).
    pos_env: Env,
}

pub struct Wavetable {
    sample_rate: f32,
    /// `TABLES × MIPS` tables of `TABLE_LEN` samples, flat; see `table_at`.
    tables: Vec<f32>,
    voices: [Voice; VOICES],
    position: OnePole,
    pos_env_amt: OnePole,
    /// Position-envelope decay, seconds.
    pos_decay_s: f32,
    level: LinearRamp,
    adsr: AdsrSettings,
    next_age: u64,
    vib_rate: OnePole,
    vib_depth: OnePole,
    /// Vibrato LFO phase in turns (0..1), advanced per block.
    vib_phase: f32,
}

impl Default for Wavetable {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            tables: Vec::new(),
            voices: [Voice::default(); VOICES],
            position: OnePole::new(0.25, 0.99),
            pos_env_amt: OnePole::new(0.0, 0.99),
            pos_decay_s: 0.3,
            level: LinearRamp::new(1.0, 10.0, 48_000.0),
            adsr: AdsrSettings::default(),
            next_age: 0,
            vib_rate: OnePole::new(5.0, 0.99),
            vib_depth: OnePole::new(0.0, 0.99),
            vib_phase: 0.0,
        }
    }
}

impl Wavetable {
    /// Slice of one band-limited table. // RT-SAFE (bounds are compile-time)
    fn table_at(&self, table: usize, mip: usize) -> &[f32] {
        let start = (table * MIPS + mip) * TABLE_LEN;
        &self.tables[start..start + TABLE_LEN]
    }

    /// Read `phase` (turns) from the morph chain at `pos` (0..1), mip fixed.
    /// // RT-SAFE
    fn read(&self, pos: f32, mip: usize, phase: f32) -> f32 {
        let seg = pos.clamp(0.0, 1.0) * (TABLES - 1) as f32;
        let t0 = (seg as usize).min(TABLES - 2);
        let frac_t = seg - t0 as f32;
        let x = phase * TABLE_LEN as f32;
        let i0 = (x as usize) & (TABLE_LEN - 1);
        let i1 = (i0 + 1) & (TABLE_LEN - 1);
        let frac_x = x - x.floor();
        let sample = |t: usize| {
            let tab = self.table_at(t, mip);
            tab[i0] * (1.0 - frac_x) + tab[i1] * frac_x
        };
        sample(t0) * (1.0 - frac_t) + sample(t0 + 1) * frac_t
    }

    fn note_on(&mut self, pitch: u8, vel: u8) {
        let sr = self.sample_rate;
        let idx = self.alloc_voice();
        let age = self.next_age;
        self.next_age += 1;
        let f0 = midi_to_freq(pitch);
        // Deepest mip whose top harmonic stays in band: 256>>m · f0 ≤ 0.45·sr.
        let mut mip = 0;
        while mip + 1 < MIPS && (256 >> mip) as f32 * f0 > 0.45 * sr {
            mip += 1;
        }
        let v = &mut self.voices[idx];
        v.pitch = pitch;
        v.active = true;
        v.age = age;
        v.phase = 0.0;
        v.inc = f0 / sr;
        v.mip = mip;
        v.amp = vel as f32 / 127.0;
        v.env = Env::default();
        v.env.note_on();
        v.pos_env = Env::default();
        v.pos_env.note_on();
    }

    fn note_off(&mut self, pitch: u8) {
        for v in &mut self.voices {
            if v.active && v.pitch == pitch {
                v.env.note_off();
                v.pos_env.note_off();
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

impl AudioNode for Wavetable {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        for ev in io.notes() {
            match ev.kind {
                NoteKind::On { pitch, vel } => self.note_on(pitch, vel),
                NoteKind::Off { pitch } => self.note_off(pitch),
            }
        }
        if self.tables.is_empty() {
            let out = io.output(0);
            out.fill(0.0); // unprepared: silent, never a panic
            io.spread_output(0);
            return;
        }

        let frames = io.frames();
        let sr = self.sample_rate;
        let position = self.position.advance(frames);
        let pos_amt = self.pos_env_amt.advance(frames);
        // Vibrato multiplier sampled once per block (staircase inaudible at
        // ≤ 12 Hz / 100 ct); depth 0 keeps the exact legacy increment.
        let vib_depth = self.vib_depth.advance(frames);
        let vib_rate = self.vib_rate.advance(frames);
        let vib = if vib_depth > 0.0 {
            let m = (vib_depth * (TAU * self.vib_phase).sin() / 1200.0).exp2();
            self.vib_phase = (self.vib_phase + vib_rate * frames as f32 / sr).fract();
            m
        } else {
            1.0
        };
        let pos_adsr = AdsrSettings {
            attack_s: self.adsr.attack_s,
            decay_s: self.pos_decay_s,
            sustain: 0.0,
            release_s: self.adsr.release_s,
        };
        let adsr = self.adsr;

        // Borrows stay short-lived: each voice mutation ends before `read`
        // takes its immutable look at the tables.
        let out = io.output(0);
        for slot in out.iter_mut() {
            let level = self.level.tick();
            let mut sum = 0.0;
            for vi in 0..VOICES {
                if !self.voices[vi].active {
                    continue;
                }
                let (pos_eff, phase, mip) = {
                    let v = &mut self.voices[vi];
                    let e = v.pos_env.tick(&pos_adsr, sr);
                    ((position + pos_amt * e).clamp(0.0, 1.0), v.phase, v.mip)
                };
                let s = self.read(pos_eff, mip, phase);
                let v = &mut self.voices[vi];
                v.phase += v.inc * vib;
                if v.phase >= 1.0 {
                    v.phase -= 1.0;
                }
                let env = v.env.tick(&adsr, sr);
                if !v.env.is_active() {
                    v.active = false;
                }
                sum += s * env * v.amp;
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
            P_POSITION => self.position.set_target(real),
            P_POS_ENV => self.pos_env_amt.set_target(real),
            P_POS_DECAY => self.pos_decay_s = real / 1000.0,
            P_ATTACK => self.adsr.attack_s = real / 1000.0,
            P_DECAY => self.adsr.decay_s = real / 1000.0,
            P_SUSTAIN => self.adsr.sustain = real,
            P_RELEASE => self.adsr.release_s = (real / 1000.0).max(MIN_RELEASE_S),
            P_LEVEL => self.level.set_target(real),
            P_VIB_RATE => self.vib_rate.set_target(real),
            P_VIB_DEPTH => self.vib_depth.set_target(real),
            _ => {}
        }
    }

    fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            if v.active {
                v.env.note_off();
                v.pos_env.note_off();
            }
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        // Generate every band-limited table by additive synthesis (off the
        // audio thread — this is the module's only allocation). Each table is
        // normalized to a 0.8 peak so the morph keeps a steady loudness.
        let mut tables = vec![0.0f32; TABLES * MIPS * TABLE_LEN];
        for t in 0..TABLES {
            for m in 0..MIPS {
                let h_max = 256usize >> m;
                let start = (t * MIPS + m) * TABLE_LEN;
                let slice = &mut tables[start..start + TABLE_LEN];
                for n in 1..=h_max {
                    let a = harmonic_amp(t, n);
                    if a == 0.0 {
                        continue;
                    }
                    for (k, s) in slice.iter_mut().enumerate() {
                        *s += a * (TAU * n as f32 * k as f32 / TABLE_LEN as f32).sin();
                    }
                }
                let peak = slice.iter().fold(0.0f32, |p, &x| p.max(x.abs()));
                if peak > 0.0 {
                    let g = 0.8 / peak;
                    for s in slice.iter_mut() {
                        *s *= g;
                    }
                }
            }
        }
        self.tables = tables;

        let coef = (-1.0 / (SMOOTH_S * sample_rate)).exp();
        self.position = OnePole::new(default_real(P_POSITION), coef);
        self.pos_env_amt = OnePole::new(default_real(P_POS_ENV), coef);
        self.pos_decay_s = default_real(P_POS_DECAY) / 1000.0;
        self.level = LinearRamp::new(default_real(P_LEVEL), 10.0, sample_rate);
        self.adsr = AdsrSettings {
            attack_s: default_real(P_ATTACK) / 1000.0,
            decay_s: default_real(P_DECAY) / 1000.0,
            sustain: default_real(P_SUSTAIN),
            release_s: (default_real(P_RELEASE) / 1000.0).max(MIN_RELEASE_S),
        };
        self.vib_rate = OnePole::new(default_real(P_VIB_RATE), coef);
        self.vib_depth = OnePole::new(default_real(P_VIB_DEPTH), coef);
        self.vib_phase = 0.0;
        self.voices = [Voice::default(); VOICES];
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node::NoteEvent;

    const SR: f32 = 48_000.0;

    fn synth() -> Wavetable {
        let mut s = Wavetable::default();
        s.prepare(SR, 256);
        s
    }

    fn set_real(s: &mut Wavetable, id: ParamId, real: f32) {
        s.set_param(id, PARAMS[id as usize].normalize(real));
    }

    fn render_notes(s: &mut Wavetable, notes: &[NoteEvent], frames: usize, out: &mut Vec<f32>) {
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: SR,
            frames,
            channels: 1,
            samples_per_beat: 0.0,
            beat_phase: 0.0,
        };
        let mut io = NodeIo::new(notes, &[], out, frames, 1);
        s.process(&ctx, &mut io);
    }

    /// Render in engine-sized blocks so block-rate state (vibrato) advances.
    fn render_chunked(s: &mut Wavetable, notes: &[NoteEvent], frames: usize, out: &mut Vec<f32>) {
        out.clear();
        let mut chunk = Vec::new();
        let mut done = 0;
        while done < frames {
            let n = 256.min(frames - done);
            render_notes(s, if done == 0 { notes } else { &[] }, n, &mut chunk);
            out.extend_from_slice(&chunk);
            done += n;
        }
    }

    #[test]
    fn vibrato_wobbles_the_pitch() {
        let mut s = synth();
        set_real(&mut s, P_POSITION, 1.0); // vox end: rich, but pitched
        set_real(&mut s, P_VIB_RATE, 2.0);
        set_real(&mut s, P_VIB_DEPTH, 50.0);
        let mut buf = Vec::new();
        render_chunked(&mut s, &[on(69)], 48_000, &mut buf);
        let (mut lo, mut hi) = (f32::MAX, 0.0f32);
        for start in (4800..43_200).step_by(2400) {
            if let Some(f0) = crate::analysis::pitch::estimate_f0(&buf[start..start + 2400], 48_000)
            {
                lo = lo.min(f0.hz);
                hi = hi.max(f0.hz);
            }
        }
        assert!(hi > 0.0 && lo < f32::MAX, "no pitch detected");
        let span = (hi - lo) / 440.0;
        assert!(span > 0.03, "vibrato span {span} too small ({lo}..{hi} Hz)");
    }

    fn on(pitch: u8) -> NoteEvent {
        NoteEvent {
            frame: 0,
            kind: NoteKind::On { pitch, vel: 110 },
        }
    }

    /// Goertzel-style magnitude of a single bin, normalised by length.
    fn mag_at(buf: &[f32], freq: f32) -> f32 {
        let w = TAU * freq / SR;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, &x) in buf.iter().enumerate() {
            let p = w * n as f32;
            re += x * p.cos();
            im += x * p.sin();
        }
        (re * re + im * im).sqrt() / buf.len() as f32
    }

    /// Settled sustain at `position` for pitch 57 (220 Hz).
    fn sustain_at(position: f32, pitch: u8, frames: usize) -> Vec<f32> {
        let mut s = synth();
        set_real(&mut s, P_POSITION, position);
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(pitch)], 4096, &mut buf); // settle glides
        let mut tail = Vec::new();
        render_notes(&mut s, &[], frames, &mut tail);
        tail
    }

    #[test]
    fn position_travels_from_sine_to_square() {
        let f0 = 220.0;
        // Position 0: a pure sine — no meaningful H2/H3.
        let sine = sustain_at(0.0, 57, 8192);
        let h1 = mag_at(&sine, f0);
        assert!(h1 > 0.01, "sine fundamental missing");
        assert!(mag_at(&sine, 3.0 * f0) < 0.02 * h1, "sine has H3");
        // Saw segment (position 0.5): even AND odd harmonics.
        let saw = sustain_at(0.5, 57, 8192);
        assert!(
            mag_at(&saw, 2.0 * f0) > 0.1 * mag_at(&saw, f0),
            "saw lost H2"
        );
        // Square segment (position 0.75): odd only.
        let square = sustain_at(0.75, 57, 8192);
        assert!(
            mag_at(&square, 3.0 * f0) > 5.0 * mag_at(&square, 2.0 * f0),
            "square is not odd-only"
        );
    }

    #[test]
    fn high_notes_do_not_alias() {
        // Pitch 105 (~3.7 kHz) at the square segment: the mip keeps only the
        // harmonics that fit below 0.45·sr, so the harmonics a naive square
        // would fold back into the band simply do not exist.
        let f0 = midi_to_freq(105);
        let buf = sustain_at(0.75, 105, 16_384);
        let h1 = mag_at(&buf, f0);
        assert!(h1 > 0.01, "fundamental missing at pitch 105");
        // H7 of a naive square (1/7 ≈ -17 dB) folds to |48000 - 7·f0|; with
        // the band-limited table there must be essentially nothing there.
        let alias = (SR - 7.0 * f0).abs();
        assert!(
            mag_at(&buf, alias) < 0.02 * h1,
            "aliased H7 image present at {alias} Hz"
        );
        // The excluded straight harmonic is absent too (band limit works).
        assert!(
            mag_at(&buf, 7.0 * f0) < 0.02 * h1,
            "H7 leaked above the band"
        );
    }

    #[test]
    fn position_envelope_makes_the_spectrum_travel() {
        use crate::analysis::{fft, spectral};
        let mut s = synth();
        set_real(&mut s, P_POSITION, 0.75); // start on the square
        set_real(&mut s, P_POS_ENV, -0.75); // envelope pulls toward the sine
        set_real(&mut s, P_POS_DECAY, 400.0);
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(57)], 96_000, &mut buf);
        let centroid = |slice: &[f32]| {
            let power = fft::power_spectrum(slice).expect("pow2");
            spectral::spectral_centroid_hz(&power, slice.len(), SR as u32)
        };
        // The envelope starts at the pulled position and decays back to the
        // knob: darker start, brighter settle.
        let early = centroid(&buf[512..8704]);
        let late = centroid(&buf[64_000..72_192]);
        assert!(
            late > 1.3 * early,
            "position envelope inert: early {early} Hz, late {late} Hz"
        );
    }

    #[test]
    fn position_sweep_is_click_free() {
        use crate::analysis::levels;
        let mut s = synth();
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(57)], 4096, &mut buf);
        let mut whole = Vec::new();
        for k in 0..128 {
            s.set_param(P_POSITION, k as f32 / 127.0);
            render_notes(&mut s, &[], 256, &mut buf);
            whole.extend_from_slice(&buf);
        }
        assert!(whole.iter().all(|x| x.is_finite()));
        let clicks = levels::detect_clicks(&whole, 48_000);
        assert!(clicks.is_empty(), "position sweep clicked: {clicks:?}");
    }

    #[test]
    fn release_is_click_free_and_voices_recycle() {
        let mut s = synth();
        s.set_param(P_RELEASE, 0.0); // floored to MIN_RELEASE_S
        let mut buf = Vec::new();
        render_notes(&mut s, &[on(64)], 2048, &mut buf);
        let off = [NoteEvent {
            frame: 0,
            kind: NoteKind::Off { pitch: 64 },
        }];
        let mut tail = Vec::new();
        render_notes(&mut s, &off, 48_000, &mut tail);
        let max_delta = tail
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(max_delta < 0.2, "release click, max delta {max_delta}");
        assert_eq!(s.active_voice_count(), 0, "voice not freed after release");

        let ons: Vec<NoteEvent> = (60u8..68).map(on).collect();
        render_notes(&mut s, &ons, 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8);
        render_notes(&mut s, &[on(72)], 64, &mut buf);
        assert_eq!(s.active_voice_count(), 8, "steal kept the count at 8");
    }
}
