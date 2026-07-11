//! module_specs — dump the module catalog: every registered module, its
//! ports, its parameters with REAL ranges (Hz, ms, dB), and the quirks that
//! are not visible in the numbers (drum pitch map, fm semantics…).
//!
//! This is the reference an agent reads before writing `--set` values or
//! song.json params — instead of re-reading each module's source.
//!
//! Usage:
//!   cargo run --bin module_specs [-- --json]

// Shared CLI helpers; each bin uses a different subset (see util.rs header).
#[allow(dead_code)]
mod util;

use telenn_lib::engine::node::SignalKind;
use telenn_lib::modules::registry::{self, ModuleCategory};

/// Module behaviours the numeric specs cannot express. Hand-maintained:
/// update alongside the module when one of these changes.
const QUIRKS: &str = "\
## Quirks (hand-maintained)

- **Params on disk are NORMALIZED.** `song.json` and preset files store every
  param as 0..1; the real value is `denormalize(v)` through the param's curve:
  linear: `min + (max-min)·v` · log: `min·(max/min)^v` · db: dB value
  `min + (max-min)·v`, then converted to a LINEAR GAIN `10^(dB/20)`.
  The CLIs (`render_probe --set`) take real values and normalize for you.
- **drum_sampler**: fixed pitch map — 36 kick, 38 snare, 39 clap,
  42 closed_hat, 45 low_tom, 46 open_hat, 49 crash, 50 hi_tom. Any other
  pitch is SILENT. Note-offs are ignored (one-shots); retrigger restarts the
  sample. `level` is the only param whose max exceeds 0 dB (+6).
- **fm**: `ratio` = modulator/carrier frequency ratio — integer ratios give
  harmonic tones, non-integer give inharmonic/bell timbres. `index` =
  modulation depth (radians): more index, brighter and noisier spectrum.
- **compressor**: `threshold` is declared `curve: linear` with unit dB ON
  PURPOSE — the gain computer consumes the dB number itself (normalize/
  denormalize stay a plain ramp over -60..0 dB), unlike `makeup`/`level`
  params (db curve → linear gain).
- **delay**: `feedback` is capped at 0.9 by design so the loop always
  decays; `time` is free-running milliseconds, NOT tempo-synced.
- **Loop & export**: the loop length is the content end rounded UP to a
  whole bar; the metronome is monitoring-only and never rendered/exported.
";

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
    let as_json = args.take_flag("--json");
    let leftover = args.finish_positional()?;
    if !leftover.is_empty() {
        return Err("usage: module_specs [--json]".into());
    }

    if as_json {
        print_json()
    } else {
        print_markdown();
        Ok(util::EXIT_OK)
    }
}

fn kind_name(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Notes => "notes",
        SignalKind::Audio => "audio",
        SignalKind::Modulation => "modulation",
    }
}

fn ports_line(module_id: &str) -> String {
    let Some(instance) = registry::create(module_id) else {
        return "?".into();
    };
    let spec = instance.ports();
    let fmt = |ports: &[telenn_lib::engine::node::Port]| {
        ports
            .iter()
            .map(|p| format!("{} ({})", p.name, kind_name(p.kind)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("{} -> {}", fmt(&spec.inputs), fmt(&spec.outputs))
}

fn print_markdown() {
    println!("# Module catalog\n");
    println!(
        "Params: `min`/`max`/`default` are REAL values; the `json default` \
         column is the same default as stored on disk (normalized 0..1).\n"
    );
    for desc in registry::list() {
        let category = match desc.category {
            ModuleCategory::Instrument => "instrument",
            ModuleCategory::Effect => "effect",
        };
        println!("## {} — {} ({category})\n", desc.id, desc.name);
        println!("ports: {}\n", ports_line(desc.id));
        let Some(instance) = registry::create(desc.id) else {
            continue;
        };
        println!("| param | unit | curve | min | max | default | json default |");
        println!("|-------|------|-------|-----|-----|---------|--------------|");
        for p in instance.params() {
            println!(
                "| {} | {} | {:?} | {} | {} | {} | {:.3} |",
                p.name,
                p.unit,
                p.curve,
                p.min,
                p.max,
                p.default,
                p.normalize(p.default)
            );
        }
        println!();
    }
    println!("{QUIRKS}");
}

fn print_json() -> Result<i32, String> {
    let catalog: Vec<serde_json::Value> = registry::list()
        .iter()
        .map(|desc| {
            let params = registry::create(desc.id)
                .map(|instance| instance.params().to_vec())
                .unwrap_or_default();
            let normalized_defaults: Vec<f32> =
                params.iter().map(|p| p.normalize(p.default)).collect();
            serde_json::json!({
                "id": desc.id,
                "name": desc.name,
                "category": desc.category,
                "ports": ports_line(desc.id),
                "params": params,
                "normalizedDefaults": normalized_defaults,
            })
        })
        .collect();
    let payload = serde_json::json!({ "modules": catalog, "quirks": QUIRKS });
    let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
    println!("{text}");
    Ok(util::EXIT_OK)
}
