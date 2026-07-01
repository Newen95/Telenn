//! Synthesises the built-in drum kit as 16-bit mono WAV files under
//! `assets/drumkit/`. Run once to (re)generate the kit:
//!
//! ```sh
//! cargo run --example gen_drumkit
//! ```
//!
//! The kit is entirely synthetic — no third-party samples, no licensing. The
//! generator is deterministic (fixed RNG seed), so the committed WAVs are exactly
//! reproducible from this source. `drum_sampler.rs` then plays these WAVs back as
//! a real sampler.

use std::f32::consts::TAU;
use std::path::Path;

const SR: u32 = 48_000;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/drumkit");
    std::fs::create_dir_all(&dir).expect("create drumkit dir");

    let kit: [(&str, Vec<f32>); 8] = [
        ("kick", kick()),
        ("snare", snare()),
        ("clap", clap()),
        ("closed_hat", hat(0.06, 120.0)),
        ("open_hat", hat(0.35, 12.0)),
        ("low_tom", tom(120.0, 90.0, 0.40, 10.0)),
        ("hi_tom", tom(180.0, 140.0, 0.30, 12.0)),
        ("crash", crash()),
    ];

    for (name, samples) in &kit {
        let path = dir.join(format!("{name}.wav"));
        write_wav(&path, samples);
        println!("wrote {} ({} samples)", path.display(), samples.len());
    }
}

/// Deterministic white noise in [-1, 1] from a seeded xorshift generator.
struct Noise(u32);
impl Noise {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// One-pole high-pass: keeps the bright part of a noise source (hats, claps).
fn highpass(x: &[f32], cutoff_coef: f32) -> Vec<f32> {
    let mut out = vec![0.0; x.len()];
    let (mut prev_x, mut prev_y) = (0.0f32, 0.0f32);
    for (o, &xn) in out.iter_mut().zip(x) {
        let y = cutoff_coef * (prev_y + xn - prev_x);
        *o = y;
        prev_x = xn;
        prev_y = y;
    }
    out
}

fn dur_samples(secs: f32) -> usize {
    (secs * SR as f32) as usize
}

/// Peak-normalise to 0.95 so every drum sits at a consistent, non-clipping level.
fn normalize(mut buf: Vec<f32>) -> Vec<f32> {
    let peak = buf.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    if peak > 1.0e-6 {
        let g = 0.95 / peak;
        for s in &mut buf {
            *s *= g;
        }
    }
    buf
}

fn kick() -> Vec<f32> {
    let n = dur_samples(0.35);
    let mut out = vec![0.0; n];
    let mut phase = 0.0f32;
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / SR as f32;
        // Pitch sweeps 150 Hz → 50 Hz; body decays fast, with a short click attack.
        let freq = 50.0 + 100.0 * (-t * 40.0).exp();
        phase += TAU * freq / SR as f32;
        let body = phase.sin() * (-t * 12.0).exp();
        let click = (-t * 700.0).exp() * 0.4;
        *s = body + click;
    }
    normalize(out)
}

fn snare() -> Vec<f32> {
    let n = dur_samples(0.25);
    let mut out = vec![0.0; n];
    let mut noise = Noise(0x1234_5678);
    let raw: Vec<f32> = (0..n).map(|_| noise.next()).collect();
    let bright = highpass(&raw, 0.85);
    let (mut p1, mut p2) = (0.0f32, 0.0f32);
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / SR as f32;
        p1 += TAU * 180.0 / SR as f32;
        p2 += TAU * 330.0 / SR as f32;
        let body = (p1.sin() + 0.6 * p2.sin()) * (-t * 22.0).exp();
        let rattle = bright[i] * (-t * 18.0).exp();
        *s = 0.5 * body + 0.8 * rattle;
    }
    normalize(out)
}

fn hat(secs: f32, decay: f32) -> Vec<f32> {
    let n = dur_samples(secs);
    let mut noise = Noise(0x0bad_f00d);
    let raw: Vec<f32> = (0..n).map(|_| noise.next()).collect();
    let bright = highpass(&raw, 0.92);
    let out: Vec<f32> = bright
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let t = i as f32 / SR as f32;
            x * (-t * decay).exp()
        })
        .collect();
    normalize(out)
}

fn clap() -> Vec<f32> {
    let n = dur_samples(0.25);
    let mut noise = Noise(0xdead_beef);
    let raw: Vec<f32> = (0..n).map(|_| noise.next()).collect();
    let bright = highpass(&raw, 0.88);
    // Three quick bursts then a softer tail — the classic clap envelope.
    let bursts = [0.0f32, 0.008, 0.016];
    let mut out = vec![0.0; n];
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / SR as f32;
        let mut env = 0.0;
        for &b in &bursts {
            if t >= b {
                env += (-(t - b) * 350.0).exp();
            }
        }
        let tail = (-t * 18.0).exp() * 0.4;
        *s = bright[i] * (env + tail);
    }
    normalize(out)
}

fn tom(f_start: f32, f_end: f32, secs: f32, decay: f32) -> Vec<f32> {
    let n = dur_samples(secs);
    let mut out = vec![0.0; n];
    let mut phase = 0.0f32;
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / SR as f32;
        let frac = (i as f32 / n as f32).min(1.0);
        let freq = f_start + (f_end - f_start) * frac;
        phase += TAU * freq / SR as f32;
        *s = phase.sin() * (-t * decay).exp();
    }
    normalize(out)
}

fn crash() -> Vec<f32> {
    let n = dur_samples(1.0);
    let mut noise = Noise(0xcafe_babe);
    let raw: Vec<f32> = (0..n).map(|_| noise.next()).collect();
    let bright = highpass(&raw, 0.95);
    let out: Vec<f32> = bright
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let t = i as f32 / SR as f32;
            // Fast attack, long shimmering decay.
            let env = (1.0 - (-t * 300.0).exp()) * (-t * 4.0).exp();
            x * env
        })
        .collect();
    normalize(out)
}

fn write_wav(path: &Path, samples: &[f32]) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("create wav");
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(v).expect("write sample");
    }
    writer.finalize().expect("finalize wav");
}
