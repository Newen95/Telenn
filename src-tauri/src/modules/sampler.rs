//! Sampler: pitched multisample playback from WAV zones on disk.
//!
//! Config key `zones` names a directory (project-relative) of WAV files, one
//! per root note, named by MIDI number (`048.wav`) or note name (`c3.wav`,
//! `f#2.wav`, `bb1.wav` — convention C4 = 60). A played note picks the nearest
//! root and repitches it by the semitone ratio (linear-interp resampling), so a
//! handful of zones covers the keyboard. An ADSR shapes each voice on top of
//! the raw sample: long zones can be clipped short by the release, short zones
//! simply end (no sustain loop — pick sources longer than the notes).
//!
//! `configure` loads the zones (control side, file I/O allowed); `process` is
//! RT-SAFE — it only reads pre-decoded buffers and advances playheads.

use std::path::Path;

use crate::engine::dsp::{AdsrSettings, Env};
use crate::engine::node::{
    AudioNode, NodeIo, NoteKind, ParamCurve, ParamId, ParamSpec, Port, PortSpec, ProcessCtx,
    SignalKind,
};
use crate::engine::params::LinearRamp;
use crate::modules::wav;

const P_ATTACK: ParamId = 0;
const P_DECAY: ParamId = 1;
const P_SUSTAIN: ParamId = 2;
const P_RELEASE: ParamId = 3;
const P_LEVEL: ParamId = 4;

const PARAMS: [ParamSpec; 5] = [
    ParamSpec {
        id: P_ATTACK,
        name: "attack",
        unit: "ms",
        min: 1.0,
        max: 2000.0,
        default: 2.0,
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
        default: 1.0,
        curve: ParamCurve::Linear,
    },
    ParamSpec {
        id: P_RELEASE,
        name: "release",
        unit: "ms",
        min: 1.0,
        max: 4000.0,
        default: 200.0,
        curve: ParamCurve::Log,
    },
    ParamSpec {
        id: P_LEVEL,
        name: "level",
        unit: "dB",
        min: -60.0,
        max: 6.0,
        default: 0.0,
        curve: ParamCurve::Db,
    },
];

const MAX_VOICES: usize = 12;
/// Voice-steal declick length (seconds), same idiom as the drum sampler.
const FADE_S: f32 = 0.003;

/// One loaded multisample zone.
struct Zone {
    root: u8,
    sample: Vec<f32>,
    wav_sr: f32,
}

#[derive(Clone, Copy)]
struct Voice {
    zone: usize,
    pos: f32,
    step: f32,
    amp: f32,
    note: u8,
    env: Env,
    /// Last rendered value, carried into the steal declick fade.
    last: f32,
    /// Allocation stamp: the smallest is the oldest voice (steal order).
    age: u64,
    active: bool,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            zone: 0,
            pos: 0.0,
            step: 1.0,
            amp: 0.0,
            note: 0,
            env: Env::default(),
            last: 0.0,
            age: 0,
            active: false,
        }
    }
}

pub struct Sampler {
    zones: Vec<Zone>,
    voices: [Voice; MAX_VOICES],
    adsr: AdsrSettings,
    level: LinearRamp,
    sr: f32,
    stamp: u64,
    /// Steal declick accumulator (cut voice values fading to zero).
    fade: f32,
    fade_step: f32,
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            zones: Vec::new(),
            voices: [Voice::default(); MAX_VOICES],
            adsr: AdsrSettings {
                attack_s: 0.002,
                decay_s: 0.2,
                sustain: 1.0,
                release_s: 0.2,
            },
            level: LinearRamp::new(1.0, 10.0, 48_000.0),
            sr: 48_000.0,
            stamp: 0,
            fade: 0.0,
            fade_step: 1.0 / (FADE_S * 48_000.0),
        }
    }
}

impl Sampler {
    // RT-SAFE
    fn note_on(&mut self, note: u8, vel: u8) {
        // Nearest root wins; at a tie the lower zone keeps basses from thinning.
        let mut best: Option<(usize, i16, u8)> = None;
        for (i, z) in self.zones.iter().enumerate() {
            let dist = (note as i16 - z.root as i16).abs();
            if best.is_none_or(|(_, d, r)| (dist, z.root) < (d, r)) {
                best = Some((i, dist, z.root));
            }
        }
        let Some((zone_idx, _, root)) = best else {
            return;
        };
        let ratio = 2.0_f32.powf((note as f32 - root as f32) / 12.0);
        let step = ratio * self.zones[zone_idx].wav_sr / self.sr;

        // A free voice, else steal the oldest (its cut value joins the fade).
        let mut pick = 0;
        let mut found_free = false;
        let mut oldest = u64::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if !v.active {
                pick = i;
                found_free = true;
                break;
            }
            if v.age < oldest {
                oldest = v.age;
                pick = i;
            }
        }
        if !found_free {
            self.fade += self.voices[pick].last;
        }

        self.stamp += 1;
        self.voices[pick] = Voice {
            zone: zone_idx,
            pos: 0.0,
            step,
            amp: vel as f32 / 127.0,
            note,
            env: Env::default(),
            last: 0.0,
            age: self.stamp,
            active: true,
        };
        self.voices[pick].env.note_on();
    }

    // RT-SAFE
    fn note_off(&mut self, note: u8) {
        for v in &mut self.voices {
            if v.active && v.note == note {
                v.env.note_off();
            }
        }
    }
}

impl AudioNode for Sampler {
    fn process(&mut self, _ctx: &ProcessCtx, io: &mut NodeIo) {
        for ev in io.notes() {
            match ev.kind {
                NoteKind::On { pitch, vel } => self.note_on(pitch, vel),
                NoteKind::Off { pitch } => self.note_off(pitch),
            }
        }

        let adsr = self.adsr;
        let sr = self.sr;
        let zones = &self.zones;
        let out = io.output(0);
        for s in out.iter_mut() {
            let level = self.level.tick();
            let mut sum = self.fade;
            if self.fade > 0.0 {
                self.fade = (self.fade - self.fade_step).max(0.0);
            } else if self.fade < 0.0 {
                self.fade = (self.fade + self.fade_step).min(0.0);
            }
            for v in &mut self.voices {
                if !v.active {
                    continue;
                }
                let e = v.env.tick(&adsr, sr);
                let zone = &zones[v.zone];
                let x = sample_at(&zone.sample, v.pos) * v.amp * e;
                v.last = x;
                sum += x;
                v.pos += v.step;
                if !v.env.is_active() || v.pos >= zone.sample.len() as f32 - 1.0 {
                    v.active = false;
                    v.last = 0.0;
                }
            }
            *s = sum * level;
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
        let spec = &PARAMS[id as usize];
        let real = spec.denormalize(value);
        match id {
            P_ATTACK => self.adsr.attack_s = real / 1000.0,
            P_DECAY => self.adsr.decay_s = real / 1000.0,
            P_SUSTAIN => self.adsr.sustain = real,
            P_RELEASE => self.adsr.release_s = real / 1000.0,
            P_LEVEL => self.level.set_target(real),
            _ => {}
        }
    }

    fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            if v.active {
                v.env.note_off();
            }
        }
    }

    fn configure(&mut self, config: &[(String, String)], base_dir: &Path) {
        for (key, value) in config {
            if key != "zones" {
                continue;
            }
            self.zones.clear();
            let dir = wav::resolve(base_dir, value);
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let is_wav = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
                if !is_wav {
                    continue;
                }
                let Some(root) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(parse_root)
                else {
                    continue;
                };
                if let Some((sample, wav_sr)) = wav::load_mono(&path) {
                    self.zones.push(Zone {
                        root,
                        sample,
                        wav_sr,
                    });
                }
            }
            self.zones.sort_by_key(|z| z.root);
        }
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        self.fade = 0.0;
        self.fade_step = 1.0 / (FADE_S * sample_rate);
        self.level = LinearRamp::new(default_real(P_LEVEL), 10.0, sample_rate);
        for v in &mut self.voices {
            *v = Voice::default();
        }
    }
}

/// Read a sample at a fractional position with linear interpolation. // RT-SAFE
fn sample_at(buf: &[f32], pos: f32) -> f32 {
    let i = pos as usize;
    if i + 1 >= buf.len() {
        return 0.0;
    }
    let frac = pos - i as f32;
    buf[i] * (1.0 - frac) + buf[i + 1] * frac
}

/// Parse a zone file stem into its MIDI root: a plain number (`48`, `048`) or a
/// note name with optional accidental (`c3`, `f#2`, `bb1`; C4 = 60).
fn parse_root(stem: &str) -> Option<u8> {
    let s = stem.trim().to_ascii_lowercase();
    if let Ok(n) = s.parse::<u8>() {
        return (n < 128).then_some(n);
    }
    let bytes = s.as_bytes();
    let base = match bytes.first()? {
        b'c' => 0,
        b'd' => 2,
        b'e' => 4,
        b'f' => 5,
        b'g' => 7,
        b'a' => 9,
        b'b' => 11,
        _ => return None,
    };
    let mut i = 1;
    let mut acc = 0i32;
    match bytes.get(i) {
        Some(b'#') => {
            acc = 1;
            i += 1;
        }
        // A flat only when digits follow ("bb1"); a bare "b1" is the note B.
        Some(b'b') if bytes.len() > i + 1 => {
            acc = -1;
            i += 1;
        }
        _ => {}
    }
    let octave: i32 = s[i..].parse().ok()?;
    let midi = 12 * (octave + 1) + base + acc;
    (0..128).contains(&midi).then_some(midi as u8)
}

/// Real default value for a parameter (round-trips through the curve).
fn default_real(id: ParamId) -> f32 {
    let spec = &PARAMS[id as usize];
    spec.denormalize(spec.normalize(spec.default))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::node::NoteEvent;
    use std::path::PathBuf;

    fn write_sine(path: &Path, freq: f32, sr: u32, len_s: f32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        let n = (len_s * sr as f32) as usize;
        for i in 0..n {
            let x = (std::f32::consts::TAU * freq * i as f32 / sr as f32).sin();
            w.write_sample((x * 0.8 * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    fn zone_dir(name: &str, zones: &[(&str, f32, u32)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("telenn_zones_{}_{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (file, freq, sr) in zones {
            write_sine(&dir.join(file), *freq, *sr, 1.0);
        }
        dir
    }

    fn sampler(dir: &Path) -> Sampler {
        let mut s = Sampler::default();
        let config = vec![("zones".to_string(), dir.to_string_lossy().into_owned())];
        s.configure(&config, Path::new(""));
        s.prepare(48_000.0, 256);
        s
    }

    fn render(s: &mut Sampler, notes: &[NoteEvent], frames: usize, out: &mut Vec<f32>) {
        out.resize(frames, 0.0);
        let ctx = ProcessCtx {
            sample_rate: 48_000.0,
            frames,
            channels: 1,
            samples_per_beat: 0.0,
            beat_phase: 0.0,
        };
        let mut io = NodeIo::new(notes, &[], out, frames, 1);
        s.process(&ctx, &mut io);
    }

    fn on(pitch: u8) -> NoteEvent {
        NoteEvent {
            frame: 0,
            kind: NoteKind::On { pitch, vel: 110 },
        }
    }

    fn off(pitch: u8) -> NoteEvent {
        NoteEvent {
            frame: 0,
            kind: NoteKind::Off { pitch },
        }
    }

    /// Fundamental from zero crossings over the steady part of the buffer.
    fn f0(buf: &[f32], sr: f32) -> f32 {
        let window = &buf[buf.len() / 4..];
        let crossings = window
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        crossings as f32 / 2.0 * sr / window.len() as f32
    }

    #[test]
    fn zone_plays_true_at_its_root() {
        let dir = zone_dir("root", &[("a2.wav", 220.0, 48_000)]);
        let mut s = sampler(&dir);
        std::fs::remove_dir_all(&dir).ok();
        let mut buf = Vec::new();
        render(&mut s, &[on(45)], 9600, &mut buf); // A2
        let f = f0(&buf, 48_000.0);
        assert!((f - 220.0).abs() < 7.0, "root pitch off: {f} Hz");
    }

    #[test]
    fn note_repitches_by_semitone_ratio() {
        let dir = zone_dir("repitch", &[("a2.wav", 220.0, 48_000)]);
        let mut s = sampler(&dir);
        std::fs::remove_dir_all(&dir).ok();
        let mut buf = Vec::new();
        render(&mut s, &[on(57)], 9600, &mut buf); // A3, one octave up
        let f = f0(&buf, 48_000.0);
        assert!((f - 440.0).abs() < 14.0, "octave repitch off: {f} Hz");
    }

    #[test]
    fn source_sample_rate_does_not_shift_pitch() {
        let dir = zone_dir("rate", &[("a2.wav", 220.0, 44_100)]);
        let mut s = sampler(&dir);
        std::fs::remove_dir_all(&dir).ok();
        let mut buf = Vec::new();
        render(&mut s, &[on(45)], 9600, &mut buf);
        let f = f0(&buf, 48_000.0);
        assert!((f - 220.0).abs() < 7.0, "44.1k zone detuned at 48k: {f} Hz");
    }

    #[test]
    fn nearest_zone_wins() {
        // Zones at A2 and A3; A#2 must repitch the A2 zone (+1 st ≈ 233 Hz),
        // not drag the A3 zone down to 208 Hz.
        let dir = zone_dir(
            "nearest",
            &[("a2.wav", 220.0, 48_000), ("a3.wav", 440.0, 48_000)],
        );
        let mut s = sampler(&dir);
        std::fs::remove_dir_all(&dir).ok();
        let mut buf = Vec::new();
        render(&mut s, &[on(46)], 9600, &mut buf);
        let f = f0(&buf, 48_000.0);
        assert!((f - 233.0).abs() < 8.0, "wrong zone picked: {f} Hz");
    }

    #[test]
    fn numeric_stems_name_roots_too() {
        let dir = zone_dir("numeric", &[("045.wav", 220.0, 48_000)]);
        let mut s = sampler(&dir);
        std::fs::remove_dir_all(&dir).ok();
        let mut buf = Vec::new();
        render(&mut s, &[on(45)], 9600, &mut buf);
        assert!(buf.iter().any(|x| x.abs() > 0.1), "numeric zone silent");
    }

    #[test]
    fn release_fades_out_click_free() {
        use crate::analysis::levels;
        let dir = zone_dir("release", &[("a2.wav", 220.0, 48_000)]);
        let mut s = sampler(&dir);
        std::fs::remove_dir_all(&dir).ok();
        let mut whole = Vec::new();
        let mut buf = Vec::new();
        render(&mut s, &[on(45)], 4800, &mut buf);
        whole.extend_from_slice(&buf);
        render(&mut s, &[off(45)], 19_200, &mut buf); // 400 ms ≫ 200 ms release
        whole.extend_from_slice(&buf);
        let tail = whole[whole.len() - 2400..]
            .iter()
            .fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(tail < 1.0e-3, "release did not end the voice: {tail}");
        let clicks = levels::detect_clicks(&whole, 48_000);
        assert!(clicks.is_empty(), "release clicked: {clicks:?}");
    }

    #[test]
    fn no_zones_is_silent_and_finite() {
        let mut s = Sampler::default();
        s.prepare(48_000.0, 256);
        let mut buf = Vec::new();
        render(&mut s, &[on(45)], 4096, &mut buf);
        assert!(buf.iter().all(|x| x.is_finite()));
        assert!(buf.iter().all(|x| x.abs() < 1.0e-6), "empty sampler spoke");
    }

    #[test]
    fn note_names_parse_to_midi() {
        assert_eq!(parse_root("c4"), Some(60));
        assert_eq!(parse_root("a2"), Some(45));
        assert_eq!(parse_root("f#2"), Some(42));
        assert_eq!(parse_root("bb1"), Some(34));
        assert_eq!(parse_root("b1"), Some(35));
        assert_eq!(parse_root("048"), Some(48));
        assert_eq!(parse_root("kick"), None);
    }
}
