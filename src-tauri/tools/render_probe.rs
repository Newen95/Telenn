//! render_probe — render a module (or a whole song) offline and measure it
//! in one command: the tight iteration loop for sound design.
//!
//! Probe mode (single instrument, optional effect chain):
//!   cargo run --bin render_probe -- --module fm \
//!     --set ratio=2 --set index=5.5 --fx delay --set delay.mix=0.4 \
//!     --pitches 48,60,72 --note-beats 2 --tail 2.0 --vel 100 --bpm 120 \
//!     --sr 48000 --out target/probes/fm.wav [--png p] [--json p] [--no-analysis]
//!
//!   `--set` takes REAL values (Hz, ms, dB, ratios); they are normalized via
//!   each parameter's declared curve. Bare names target the instrument,
//!   `module.name` targets a chain effect.
//!
//! Song mode (any song.json):
//!   cargo run --bin render_probe -- --song <song.json> [--track <id>]
//!     [--loops 2] [--tail 2.0] [--tone] --out mix.wav [--png p] [--json p]
//!
//! Exit codes: 0 clean, 1 usage error, 2 hard defects in the render.

// Shared CLI helpers; each bin uses a different subset (see util.rs header).
#[allow(dead_code)]
mod util;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use telenn_lib::analysis::report::{
    analyze_mix, analyze_tone, defects, note_name, Defect, Severity, ToneReport,
};
use telenn_lib::analysis::spectrogram::{render_png, SpectrogramConfig};
use telenn_lib::analysis::Signal;
use telenn_lib::control::project::{
    ticks_to_duration, ticks_to_position, Clip, NodeSpec, Note, Project, Track,
};
use telenn_lib::control::render::{render_samples_with_tail, write_wav};
use telenn_lib::engine::node::ParamSpec;
use telenn_lib::modules::registry;

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
    if let Some(song) = args.take_value("--song")? {
        song_mode(args, &song)
    } else {
        probe_mode(args)
    }
}

// --- Probe mode ---

struct ProbeSpec {
    module: String,
    fx: Vec<String>,
    /// module id -> (param name -> real value), from `--set`.
    sets: BTreeMap<String, BTreeMap<String, f32>>,
    pitches: Vec<u8>,
    note_beats: f32,
    tail_s: f32,
    vel: u8,
    bpm: u32,
    sr: u32,
}

fn probe_mode(mut args: util::Args) -> Result<i32, String> {
    let module = args
        .take_value("--module")?
        .ok_or("probe mode needs --module <id> (or use --song <song.json>)")?;
    if registry::create(&module).is_none() {
        return Err(unknown_module(&module));
    }
    let is_drum = module == "drum_sampler";

    let fx = args.take_values("--fx")?;
    for f in &fx {
        match registry::list().iter().find(|d| d.id == *f) {
            None => return Err(unknown_module(f)),
            Some(d) if d.category != registry::ModuleCategory::Effect => {
                return Err(format!("--fx {f}: not an effect"))
            }
            Some(_) => {}
        }
    }
    if fx.contains(&module) || has_duplicates(&fx) {
        return Err("duplicate module in the chain: params would be ambiguous".into());
    }

    // --set name=value / module.name=value, real values.
    let mut sets: BTreeMap<String, BTreeMap<String, f32>> = BTreeMap::new();
    for kv in args.take_values("--set")? {
        let (key, value) = kv
            .split_once('=')
            .ok_or(format!("--set {kv}: expected name=value"))?;
        let value = util::parse_f32("--set", value)?;
        let (target, name) = match key.split_once('.') {
            Some((m, n)) => (m.to_string(), n.to_string()),
            None => (module.clone(), key.to_string()),
        };
        if target != module && !fx.contains(&target) {
            return Err(format!("--set {kv}: module '{target}' is not in the chain"));
        }
        sets.entry(target).or_default().insert(name, value);
    }

    let pitches = match args.take_value("--pitches")? {
        Some(csv) => csv
            .split(',')
            .map(|p| {
                p.trim()
                    .parse::<u8>()
                    .map_err(|_| format!("--pitches: '{p}' is not a MIDI pitch (0-127)"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => vec![if is_drum { 36 } else { 60 }],
    };
    if pitches.is_empty() {
        return Err("--pitches: empty list".into());
    }

    let spec = ProbeSpec {
        note_beats: take_f32(&mut args, "--note-beats")?.unwrap_or(2.0),
        tail_s: take_f32(&mut args, "--tail")?.unwrap_or(2.0),
        vel: match args.take_value("--vel")? {
            Some(v) => v
                .parse::<u8>()
                .map_err(|_| format!("--vel: '{v}' is not 0-127"))?,
            None => 100,
        },
        bpm: match args.take_value("--bpm")? {
            Some(v) => v
                .parse::<u32>()
                .map_err(|_| format!("--bpm: bad value '{v}'"))?,
            None => 120,
        },
        sr: match args.take_value("--sr")? {
            Some(v) => v
                .parse::<u32>()
                .map_err(|_| format!("--sr: bad value '{v}'"))?,
            None => 48_000,
        },
        module,
        fx,
        sets,
        pitches,
    };

    let out = args
        .take_value("--out")?
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("target/probes/{}.wav", spec.module)));
    let png_path = args.take_value("--png")?;
    let json_path = args.take_value("--json")?;
    let no_analysis = args.take_flag("--no-analysis");
    let leftover = args.finish_positional()?;
    if !leftover.is_empty() {
        return Err(format!("unexpected argument '{}'", leftover[0]));
    }

    // Build the one-track project and echo exactly what will be rendered.
    let (project, resolved) = build_probe_project(&spec)?;
    println!(
        "probe       {} @ {} bpm, {} Hz, note {} beats, tail {} s",
        spec.module, spec.bpm, spec.sr, spec.note_beats, spec.tail_s
    );
    println!("chain       {}", chain_line(&project));
    for line in &resolved {
        println!("param       {line}");
    }

    let tail_frames = (spec.tail_s * spec.sr as f32) as u64;
    let samples =
        render_samples_with_tail(&project, spec.sr, 1, tail_frames).map_err(|e| e.to_string())?;
    ensure_parent(&out)?;
    write_wav(&samples, spec.sr, &out).map_err(|e| e.to_string())?;
    println!("wav         {}", out.display());

    if no_analysis {
        return Ok(util::EXIT_OK);
    }

    let signal = Signal::from_interleaved(&samples, spec.sr);
    let mono = signal.mono();
    let mix = analyze_mix(&signal);
    print!("{mix}");

    // Per-pitch tone reports: each note owns the segment up to the next
    // onset (or the buffer end), and its exact note-off time is known.
    let spb = 60.0 / spec.bpm as f32; // seconds per beat (4/4: beat = quarter)
    let spacing_beats = spacing_bars(&spec) as f32 * 4.0;
    let mut tones: Vec<(u8, ToneReport)> = Vec::new();
    for (i, &pitch) in spec.pitches.iter().enumerate() {
        let seg_start = ((i as f32 * spacing_beats * spb) * spec.sr as f32) as usize;
        let seg_end = if i + 1 < spec.pitches.len() {
            (((i + 1) as f32 * spacing_beats * spb) * spec.sr as f32) as usize
        } else {
            mono.len()
        };
        let seg = &mono[seg_start.min(mono.len())..seg_end.min(mono.len())];
        let tone = analyze_tone(seg, spec.sr, Some(spec.note_beats * spb));
        println!(
            "-- pitch {pitch} ({}) --",
            midi_name(pitch).unwrap_or_else(|| "?".into())
        );
        print!("{tone}");
        tones.push((pitch, tone));
    }

    // Verdict: mix defects + per-pitch decay truncation warnings.
    let mut found = defects(&mix, None);
    for (pitch, tone) in &tones {
        if let Some(d) = &tone.decay {
            if !d.reliable {
                found.push(Defect {
                    severity: Severity::Warning,
                    message: format!(
                        "pitch {pitch}: decay tail truncated, rt60 >= {:.2} s (lengthen --tail)",
                        d.rt60_s
                    ),
                });
            }
        }
    }
    print_verdict(&found);

    if let Some(path) = png_path {
        render_png(&mono, spec.sr, &SpectrogramConfig::default(), &path)
            .map_err(|e| e.to_string())?;
        println!("spectrogram {path}");
    }
    if let Some(path) = json_path {
        let tones_json: Vec<_> = tones
            .iter()
            .map(|(pitch, tone)| serde_json::json!({ "pitch": pitch, "tone": tone }))
            .collect();
        let payload = serde_json::json!({
            "module": spec.module,
            "fx": spec.fx,
            "params": resolved,
            "wav": out.display().to_string(),
            "mix": mix,
            "tones": tones_json,
            "defects": found,
        });
        let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        println!("json        {path}");
    }

    Ok(exit_code(&found))
}

/// Bars between two note onsets: the note plus its tail must fit.
fn spacing_bars(spec: &ProbeSpec) -> u64 {
    let beats_needed = spec.note_beats + spec.tail_s * spec.bpm as f32 / 60.0;
    ((beats_needed / 4.0).ceil() as u64).max(1)
}

/// Build the probe project and the human-readable resolved-param lines
/// ("fm.ratio = 2 (normalized 0.446)").
fn build_probe_project(spec: &ProbeSpec) -> Result<(Project, Vec<String>), String> {
    let mut resolved = Vec::new();
    let mut chain = Vec::new();
    for module_id in std::iter::once(&spec.module).chain(&spec.fx) {
        let instance = registry::create(module_id).ok_or_else(|| unknown_module(module_id))?;
        let mut params = BTreeMap::new();
        if let Some(sets) = spec.sets.get(module_id) {
            for (name, &real) in sets {
                let param = instance
                    .params()
                    .iter()
                    .find(|p| p.name == *name)
                    .ok_or_else(|| unknown_param(module_id, name, instance.params()))?;
                let normalized = param.normalize(real);
                resolved.push(format!(
                    "{module_id}.{name} = {real} {} (normalized {normalized:.3})",
                    param.unit
                ));
                params.insert(name.clone(), normalized);
            }
        }
        chain.push(NodeSpec {
            id: module_id.clone(),
            module: module_id.clone(),
            params,
        });
    }

    // One note per pitch, each on its own bar-aligned slot so tails never
    // overlap the next onset. 4/4 → one bar = 4 beats = 4·PPQ ticks.
    let (tpbar, tpbeat) = (4 * 960u64, 960u64);
    let spacing = spacing_bars(spec);
    let note_ticks = ((spec.note_beats * 960.0) as u64).max(1);
    let notes: Vec<Note> = spec
        .pitches
        .iter()
        .enumerate()
        .map(|(i, &pitch)| Note {
            id: format!("p{i}"),
            pitch,
            start: ticks_to_position(i as u64 * spacing * tpbar, tpbar, tpbeat),
            length: ticks_to_duration(note_ticks, tpbar, tpbeat),
            vel: spec.vel,
        })
        .collect();
    let clip_ticks = spec.pitches.len() as u64 * spacing * tpbar;

    let project = Project {
        version: 1,
        bpm: spec.bpm,
        time_signature: [4, 4],
        master_gain_db: 0.0,
        tracks: vec![Track {
            id: "probe".into(),
            name: "probe".into(),
            color: None,
            mixer: Default::default(),
            chain,
            clips: vec![Clip {
                id: "c0".into(),
                start: "1.1.0".into(),
                length: ticks_to_duration(clip_ticks, tpbar, tpbeat),
                notes,
            }],
        }],
    };
    Ok((project, resolved))
}

// --- Song mode ---

fn song_mode(mut args: util::Args, song: &str) -> Result<i32, String> {
    let mut project = Project::load(song).map_err(|e| format!("{song}: {e}"))?;
    if let Some(track_id) = args.take_value("--track")? {
        let before = project.tracks.len();
        project.tracks.retain(|t| t.id == track_id);
        if project.tracks.is_empty() {
            return Err(format!(
                "--track {track_id}: no such track (the song has {before})"
            ));
        }
    }
    let loops = match args.take_value("--loops")? {
        Some(v) => v
            .parse::<u32>()
            .map_err(|_| format!("--loops: bad value '{v}'"))?,
        None => 2,
    };
    let tail_s = take_f32(&mut args, "--tail")?.unwrap_or(2.0);
    let sr = match args.take_value("--sr")? {
        Some(v) => v
            .parse::<u32>()
            .map_err(|_| format!("--sr: bad value '{v}'"))?,
        None => 48_000,
    };
    let with_tone = args.take_flag("--tone");
    let out = args
        .take_value("--out")?
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/probes/song.wav"));
    let png_path = args.take_value("--png")?;
    let json_path = args.take_value("--json")?;
    let leftover = args.finish_positional()?;
    if !leftover.is_empty() {
        return Err(format!("unexpected argument '{}'", leftover[0]));
    }

    println!("song        {song} ({} tracks)", project.tracks.len());
    println!("chain       {}", chain_line(&project));
    let tail_frames = (tail_s * sr as f32) as u64;
    let samples =
        render_samples_with_tail(&project, sr, loops, tail_frames).map_err(|e| e.to_string())?;
    ensure_parent(&out)?;
    write_wav(&samples, sr, &out).map_err(|e| e.to_string())?;
    println!("wav         {}", out.display());

    let signal = Signal::from_interleaved(&samples, sr);
    let mono = signal.mono();
    let mix = analyze_mix(&signal);
    print!("{mix}");
    // Tone analysis is off by default: on a dense mix f0/harmonics are
    // meaningless. --tone opts in (e.g. a soloed melodic track).
    let tone = with_tone.then(|| analyze_tone(&mono, sr, None));
    if let Some(tone) = &tone {
        print!("{tone}");
    }

    let found = defects(&mix, tone.as_ref());
    print_verdict(&found);

    if let Some(path) = png_path {
        render_png(&mono, sr, &SpectrogramConfig::default(), &path).map_err(|e| e.to_string())?;
        println!("spectrogram {path}");
    }
    if let Some(path) = json_path {
        let payload = serde_json::json!({
            "song": song,
            "wav": out.display().to_string(),
            "mix": mix,
            "tone": tone,
            "defects": found,
        });
        let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        println!("json        {path}");
    }

    Ok(exit_code(&found))
}

// --- Shared helpers ---

fn take_f32(args: &mut util::Args, flag: &str) -> Result<Option<f32>, String> {
    args.take_value(flag)?
        .map(|v| util::parse_f32(flag, &v))
        .transpose()
}

fn has_duplicates(items: &[String]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    !items.iter().all(|i| seen.insert(i))
}

fn chain_line(project: &Project) -> String {
    project
        .tracks
        .iter()
        .map(|t| {
            let modules: Vec<&str> = t.chain.iter().map(|n| n.module.as_str()).collect();
            format!("{}: {}", t.id, modules.join(" -> "))
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn unknown_module(id: &str) -> String {
    let known: Vec<&str> = registry::list().iter().map(|d| d.id).collect();
    format!("unknown module '{id}'; known: {}", known.join(", "))
}

fn unknown_param(module: &str, name: &str, params: &[ParamSpec]) -> String {
    let valid: Vec<String> = params
        .iter()
        .map(|p| {
            if p.unit.is_empty() {
                format!("{} ({}..{})", p.name, p.min, p.max)
            } else {
                format!("{} ({}..{} {})", p.name, p.min, p.max, p.unit)
            }
        })
        .collect();
    format!(
        "module '{module}' has no param '{name}'; valid: {}",
        valid.join(", ")
    )
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn midi_name(pitch: u8) -> Option<String> {
    note_name(440.0 * 2f32.powf((pitch as f32 - 69.0) / 12.0))
        .map(|n| n.split(' ').next().unwrap_or_default().to_string())
}

fn print_verdict(found: &[Defect]) {
    println!("== verdict ==");
    if found.is_empty() {
        println!("OK — no defects");
    }
    for d in found {
        match d.severity {
            Severity::Error => println!("DEFECT {}", d.message),
            Severity::Warning => println!("WARN   {}", d.message),
        }
    }
}

fn exit_code(found: &[Defect]) -> i32 {
    if found.iter().any(|d| d.severity == Severity::Error) {
        util::EXIT_DEFECTS
    } else {
        util::EXIT_OK
    }
}
