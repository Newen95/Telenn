//! Offline quality stats for the embedded demo: per-track and full-mix DC
//! offset, peak and RMS. Run with `cargo run --example demo_stats` to check a
//! mix change against the export acceptance criteria (no clip, no DC).

fn stats(samples: &[f32]) -> (f32, f32, f32, f32) {
    let frames = (samples.len() / 2) as f32;
    let dc_l: f32 = samples.iter().step_by(2).sum::<f32>() / frames;
    let dc_r: f32 = samples.iter().skip(1).step_by(2).sum::<f32>() / frames;
    let peak = samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    (dc_l, dc_r, peak, rms)
}

fn main() {
    let full = telenn_lib::control::demo_project().expect("demo parses");

    let samples = telenn_lib::control::render::render_samples(&full, 48_000, 1).unwrap();
    let (l, r, p, rms) = stats(&samples);
    println!("FULL MIX     dc_l {l:+.6}  dc_r {r:+.6}  peak {p:.3}  rms {rms:.4}");

    for i in 0..full.tracks.len() {
        let mut solo = full.clone();
        let id = solo.tracks[i].id.clone();
        solo.tracks.retain(|t| t.id == id);
        let samples = telenn_lib::control::render::render_samples(&solo, 48_000, 1).unwrap();
        let (l, r, p, rms) = stats(&samples);
        println!("{id:<12} dc_l {l:+.6}  dc_r {r:+.6}  peak {p:.3}  rms {rms:.4}");
    }
}
