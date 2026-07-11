//! analyze_wav — objective ears for a rendered WAV.
//!
//! Prints an agent-readable report (levels, tone, verdict), optionally a
//! machine-readable JSON and a log-frequency spectrogram PNG.
//!
//! Usage:
//!   cargo run --bin analyze_wav -- <file.wav> [options]
//!     --json <path>      write the full report as JSON
//!     --png <path>       write a spectrogram PNG
//!     --note-off <sec>   note-off time, sharpens the decay estimate
//!     --no-tone          skip tone analysis (dense mixes)
//!
//! Exit codes: 0 clean, 1 usage/read error, 2 hard defects (clip, click, DC,
//! silence).

// Shared CLI helpers; each bin uses a different subset (see util.rs header).
#[allow(dead_code)]
mod util;

use telenn_lib::analysis::report::{analyze_mix, analyze_tone, defects, Severity};
use telenn_lib::analysis::spectrogram::{render_png, SpectrogramConfig};
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
    let png_path = args.take_value("--png")?;
    let note_off = args
        .take_value("--note-off")?
        .map(|v| util::parse_f32("--note-off", &v))
        .transpose()?;
    let no_tone = args.take_flag("--no-tone");
    let positional = args.finish_positional()?;
    let file = match positional.as_slice() {
        [f] => f.clone(),
        _ => {
            return Err(
                "usage: analyze_wav <file.wav> [--json p] [--png p] [--note-off s] [--no-tone]"
                    .into(),
            )
        }
    };

    let signal = Signal::from_wav(&file).map_err(|e| e.to_string())?;
    let mono = signal.mono();
    let mix = analyze_mix(&signal);
    let tone = (!no_tone).then(|| analyze_tone(&mono, signal.sample_rate, note_off));

    println!(
        "file        {file}   {} Hz   {:.2} s",
        signal.sample_rate,
        signal.duration_s()
    );
    print!("{mix}");
    if let Some(tone) = &tone {
        print!("{tone}");
    }

    let found = defects(&mix, tone.as_ref());
    println!("== verdict ==");
    if found.is_empty() {
        println!("OK — no defects");
    }
    for d in &found {
        match d.severity {
            Severity::Error => println!("DEFECT {}", d.message),
            Severity::Warning => println!("WARN   {}", d.message),
        }
    }

    if let Some(path) = png_path {
        render_png(
            &mono,
            signal.sample_rate,
            &SpectrogramConfig::default(),
            &path,
        )
        .map_err(|e| e.to_string())?;
        println!("spectrogram {path}");
    }
    if let Some(path) = json_path {
        let payload = serde_json::json!({
            "file": file,
            "mix": mix,
            "tone": tone,
            "defects": found,
        });
        let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        println!("json        {path}");
    }

    let has_errors = found.iter().any(|d| d.severity == Severity::Error);
    Ok(if has_errors {
        util::EXIT_DEFECTS
    } else {
        util::EXIT_OK
    })
}
