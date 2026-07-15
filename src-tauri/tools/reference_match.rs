//! reference_match — measure a render against a reference track and print the
//! diff: loudness (BS.1770), dynamics, seven-band spectral balance, stereo
//! width and rhythm, with a verdict flagging every deviation beyond the
//! thresholds. The compose-style loop's closing tool: "how far is my mix from
//! the reference, in numbers".
//!
//! Usage:
//!   cargo run --bin reference_match -- <render.wav> <reference.wav> [--json p]
//!
//! Exit codes: 0 within thresholds, 1 usage error, 2 deviations flagged.
//! Whole-file statistics (no time alignment); both files must share a sample
//! rate — resample externally if needed.

// Shared CLI helpers; each bin uses a different subset (see util.rs header).
#[allow(dead_code)]
mod util;

use telenn_lib::analysis::report::match_report;
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
    let positional = args.finish_positional()?;
    let [render_path, reference_path] = positional.as_slice() else {
        return Err("usage: reference_match <render.wav> <reference.wav> [--json p]".into());
    };

    let render = Signal::from_wav(render_path).map_err(|e| format!("{render_path}: {e}"))?;
    let reference =
        Signal::from_wav(reference_path).map_err(|e| format!("{reference_path}: {e}"))?;
    if render.sample_rate != reference.sample_rate {
        return Err(format!(
            "sample rates differ ({} vs {} Hz): resample one side first",
            render.sample_rate, reference.sample_rate
        ));
    }

    println!(
        "render      {render_path}   {} Hz   {:.2} s",
        render.sample_rate,
        render.duration_s()
    );
    println!(
        "reference   {reference_path}   {} Hz   {:.2} s",
        reference.sample_rate,
        reference.duration_s()
    );
    let report = match_report(&render, &reference);
    print!("{report}");

    println!("== verdict ==");
    if report.flags.is_empty() {
        println!("MATCH — every metric within thresholds");
    } else {
        for flag in &report.flags {
            println!("OFF {flag}");
        }
    }

    if let Some(path) = json_path {
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        println!("json        {path}");
    }

    Ok(if report.flags.is_empty() {
        util::EXIT_OK
    } else {
        util::EXIT_DEFECTS
    })
}
