//! transcribe — partial transcription of a reference track: bass line,
//! drum pattern (kick/snare/hat) and per-measure chord grid on an inferred
//! beat grid, as `"bar.beat.tick"` positions comparable with song.json.
//! The compose-style loop's opening tool: "what does the reference actually
//! play, in notes".
//!
//! Usage:
//!   cargo run --bin transcribe -- <file.wav> [--bpm f] [--offset-s f]
//!       [--start-s f] [--dur-s f] [--subdivision n] [--melody] [--json path]
//!
//! `--bpm` and `--offset-s` override the grid inference when it lands wrong;
//! `--start-s`/`--dur-s` transcribe an excerpt (the grid restarts at 1.1.0).
//! `--melody` retargets the line tracker at a lead register (~B2..C#6) — for a
//! separated melody stem; the line lands in the `bass` output.
//! Exit codes: 0 done, 1 usage error (this is an extractor, not a linter).

// Shared CLI helpers; each bin uses a different subset (see util.rs header).
#[allow(dead_code)]
mod util;

use telenn_lib::analysis::transcribe::{transcribe, Options};
use telenn_lib::analysis::Signal;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    match try_run() {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("error: {msg}");
            util::EXIT_USAGE
        }
    }
}

fn try_run() -> Result<i32, String> {
    let mut args = util::Args::parse();
    let json_path = args.take_value("--json")?;
    let bpm = args
        .take_value("--bpm")?
        .map(|v| util::parse_f32("--bpm", &v))
        .transpose()?;
    let offset_s = args
        .take_value("--offset-s")?
        .map(|v| util::parse_f32("--offset-s", &v))
        .transpose()?;
    let start_s = args
        .take_value("--start-s")?
        .map(|v| util::parse_f32("--start-s", &v))
        .transpose()?
        .unwrap_or(0.0);
    let dur_s = args
        .take_value("--dur-s")?
        .map(|v| util::parse_f32("--dur-s", &v))
        .transpose()?;
    let subdivision = match args.take_value("--subdivision")? {
        Some(v) => v
            .parse::<u32>()
            .map_err(|_| format!("--subdivision: '{v}' is not an integer"))?,
        None => 16,
    };
    let melody = args.take_flag("--melody");
    let positional = args.finish_positional()?;
    let [path] = positional.as_slice() else {
        return Err(
            "usage: transcribe <file.wav> [--bpm f] [--offset-s f] [--start-s f] \
             [--dur-s f] [--subdivision n] [--melody] [--json path]"
                .into(),
        );
    };

    let signal = Signal::from_wav(path).map_err(|e| format!("{path}: {e}"))?;
    let mono = signal.mono();
    let sr = signal.sample_rate;

    // Sample-accurate excerpt; the grid restarts at bar 1 of the slice.
    let from = ((start_s * sr as f32) as usize).min(mono.len());
    let to = match dur_s {
        Some(d) if d > 0.0 => (from + (d * sr as f32) as usize).min(mono.len()),
        Some(d) => return Err(format!("--dur-s must be positive, got {d}")),
        None => mono.len(),
    };
    if to <= from {
        return Err(format!(
            "empty segment: --start-s {start_s} is past the end of the file"
        ));
    }

    let opts = Options {
        bpm,
        offset_s,
        subdivision,
        melody,
    };
    let mut transcription = transcribe(&mono[from..to], sr, &opts).map_err(|e| e.to_string())?;
    transcription.source = path.clone();
    transcription.start_s = start_s;

    println!(
        "file        {path}   {} Hz   {:.2} s{}",
        sr,
        signal.duration_s(),
        if from > 0 || to < mono.len() {
            format!(
                "   (segment {:.2}..{:.2} s)",
                start_s,
                start_s + (to - from) as f32 / sr as f32
            )
        } else {
            String::new()
        }
    );
    print!("{transcription}");

    if let Some(p) = json_path {
        let text = serde_json::to_string_pretty(&transcription).map_err(|e| e.to_string())?;
        std::fs::write(&p, text).map_err(|e| e.to_string())?;
        println!("\njson        {p}");
    }

    Ok(util::EXIT_OK)
}
