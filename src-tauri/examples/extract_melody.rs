//! Throwaway melody extractor: onset detection on the envelope, then YIN per
//! inter-onset segment. Prints `time  pitch  note` lines for transcription.
//! Run: cargo run --example extract_melody -- <file.wav>

use telenn_lib::analysis::envelope::{env_window, envelope};
use telenn_lib::analysis::pitch::estimate_f0;
use telenn_lib::analysis::report::note_name;
use telenn_lib::analysis::Signal;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: extract_melody <file.wav>");
    let signal = Signal::from_wav(&path).expect("readable wav");
    let mono = signal.mono();
    let sr = signal.sample_rate;
    let env = envelope(&mono, sr);
    let win = env_window(sr);

    // Onset = envelope rising by more than 2.5x over 10 ms, above a floor.
    let floor = env.iter().fold(0.0f32, |m, &v| m.max(v)) * 0.08;
    let mut onsets: Vec<usize> = Vec::new();
    let mut last_onset = 0usize;
    for i in 10..env.len() {
        let prev = env[i - 10].max(1e-6);
        if env[i] > floor && env[i] / prev > 2.5 && (onsets.is_empty() || i - last_onset > 120) {
            onsets.push(i);
            last_onset = i;
        }
    }

    println!("{} onsets", onsets.len());
    for (k, &onset) in onsets.iter().enumerate() {
        let start = onset * win + win * 5; // skip the attack transient (~5 ms)
        let end = if k + 1 < onsets.len() {
            (onsets[k + 1] * win).min(mono.len())
        } else {
            mono.len()
        };
        if end <= start + 2048 {
            continue;
        }
        let seg = &mono[start..end.min(start + 8192)];
        let t = onset as f32 * win as f32 / sr as f32;
        match estimate_f0(seg, sr) {
            Some(f0) if f0.clarity > 0.6 => println!(
                "{t:7.3}s  {:7.1} Hz  {}  (clarity {:.2})",
                f0.hz,
                note_name(f0.hz).unwrap_or_else(|| "?".into()),
                f0.clarity
            ),
            _ => println!("{t:7.3}s  (no pitch)"),
        }
    }
}
