//! song_lint — validate a song.json beyond what loading enforces.
//!
//! The loader is forgiving on purpose (unknown params are silently dropped,
//! duplicate ids are not rejected); an agent editing the file by hand wants
//! those surfaced BEFORE writing. Every rule has a stable id for the JSON
//! output.
//!
//! Usage:
//!   cargo run --bin song_lint -- <song.json> [--strict] [--json <path>]
//!
//! Exit codes: 0 no errors (warnings allowed; --strict promotes them),
//! 1 tool failure (unreadable file), 2 lint errors.

// Shared CLI helpers; each bin uses a different subset (see util.rs header).
#[allow(dead_code)]
mod util;

use std::collections::BTreeSet;

use serde::Serialize;
use telenn_lib::control::project::{duration_to_ticks, position_to_ticks, Project};
use telenn_lib::modules::registry::{self, ModuleCategory};

/// Pitches the drum sampler maps to a sample; anything else is silent.
const DRUM_PITCHES: [u8; 8] = [36, 38, 39, 42, 45, 46, 49, 50];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Serialize)]
struct Finding {
    rule: &'static str,
    severity: Severity,
    /// Human-readable location, e.g. "track 'drums' clip 'c3' note 'n41'".
    location: String,
    message: String,
}

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
    let strict = args.take_flag("--strict");
    let json_path = args.take_value("--json")?;
    let positional = args.finish_positional()?;
    let file = match positional.as_slice() {
        [f] => f.clone(),
        _ => return Err("usage: song_lint <song.json> [--strict] [--json p]".into()),
    };

    let text = std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?;
    let mut findings = Vec::new();

    match serde_json::from_str::<Project>(&text) {
        Err(e) => findings.push(Finding {
            rule: "parse",
            severity: Severity::Error,
            location: file.clone(),
            message: e.to_string(), // serde_json includes line/column
        }),
        Ok(project) => {
            lint(&project, &mut findings);
            print_summary(&project);
        }
    }

    for f in &findings {
        let tag = match f.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARN ",
        };
        println!("{tag} {}: [{}] {}", f.location, f.rule, f.message);
    }
    let errors = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = findings.len() - errors;
    println!("lint        {errors} error(s), {warnings} warning(s)");

    if let Some(path) = json_path {
        let text = serde_json::to_string_pretty(&findings).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
    }

    let fail = errors > 0 || (strict && warnings > 0);
    Ok(if fail {
        util::EXIT_DEFECTS
    } else {
        util::EXIT_OK
    })
}

fn error(rule: &'static str, location: String, message: String) -> Finding {
    Finding {
        rule,
        severity: Severity::Error,
        location,
        message,
    }
}

fn warn(rule: &'static str, location: String, message: String) -> Finding {
    Finding {
        rule,
        severity: Severity::Warning,
        location,
        message,
    }
}

fn lint(project: &Project, findings: &mut Vec<Finding>) {
    // Time signature / global structure. `to_graph` also catches unknown
    // modules and empty chains — run it once and surface whatever it says.
    let tpx = project.ticks_per_bar_beat();
    if let Err(e) = &tpx {
        findings.push(error("structure", "project".into(), e.to_string()));
    }
    if let Err(e) = project.to_graph() {
        findings.push(error("structure", "project".into(), e.to_string()));
    }

    // Duplicate ids, in their real namespaces: tracks; chain nodes (global,
    // the graph addresses them); clips+notes (shared namespace, see ids.rs).
    let mut track_ids = BTreeSet::new();
    let mut node_ids = BTreeSet::new();
    let mut object_ids = BTreeSet::new();
    for track in &project.tracks {
        let t_loc = format!("track '{}'", track.id);
        if !track_ids.insert(&track.id) {
            findings.push(error(
                "dup-track-id",
                t_loc.clone(),
                "duplicate track id".into(),
            ));
        }
        for node in &track.chain {
            if !node.id.is_empty() && !node_ids.insert(&node.id) {
                findings.push(error(
                    "dup-node-id",
                    format!("{t_loc} node '{}'", node.id),
                    "duplicate chain node id".into(),
                ));
            }
        }
        for clip in &track.clips {
            if clip.id.is_empty() {
                findings.push(warn(
                    "blank-id",
                    t_loc.clone(),
                    "clip without id (the app will assign one and rewrite the file)".into(),
                ));
            } else if !object_ids.insert(&clip.id) {
                findings.push(error(
                    "dup-clip-id",
                    format!("{t_loc} clip '{}'", clip.id),
                    "duplicate id (clips and notes share one namespace)".into(),
                ));
            }
            for note in &clip.notes {
                if note.id.is_empty() {
                    findings.push(warn(
                        "blank-id",
                        format!("{t_loc} clip '{}'", clip.id),
                        "note without id (the app will assign one and rewrite the file)".into(),
                    ));
                } else if !object_ids.insert(&note.id) {
                    findings.push(error(
                        "dup-note-id",
                        format!("{t_loc} clip '{}' note '{}'", clip.id, note.id),
                        "duplicate id (clips and notes share one namespace)".into(),
                    ));
                }
            }
        }
    }

    for track in &project.tracks {
        let t_loc = format!("track '{}'", track.id);

        // Chain shape and params.
        for (i, node) in track.chain.iter().enumerate() {
            let n_loc = format!("{t_loc} node '{}'", node.id);
            let Some(desc) = registry::list().iter().find(|d| d.id == node.module) else {
                continue; // unknown module already surfaced by to_graph
            };
            match (i, desc.category) {
                (0, ModuleCategory::Effect) => findings.push(error(
                    "chain-order",
                    n_loc.clone(),
                    format!(
                        "chain must start with an instrument, '{}' is an effect",
                        node.module
                    ),
                )),
                (_, ModuleCategory::Instrument) if i > 0 => findings.push(error(
                    "chain-order",
                    n_loc.clone(),
                    format!(
                        "instrument '{}' in effect position {i} (only position 0)",
                        node.module
                    ),
                )),
                _ => {}
            }
            if let Some(instance) = registry::create(&node.module) {
                let valid: Vec<&str> = instance.params().iter().map(|p| p.name).collect();
                for (name, &value) in &node.params {
                    if !valid.contains(&name.as_str()) {
                        findings.push(error(
                            "unknown-param",
                            n_loc.clone(),
                            format!(
                                "param '{name}' unknown for '{}' (silently dropped by the \
                                 loader); valid: {}",
                                node.module,
                                valid.join(", ")
                            ),
                        ));
                    }
                    if !(0.0..=1.0).contains(&value) {
                        findings.push(error(
                            "param-range",
                            n_loc.clone(),
                            format!(
                                "param '{name}' = {value}: song.json stores NORMALIZED 0..1 \
                                 values (real ranges are in `module_specs`)"
                            ),
                        ));
                    }
                }
            }
        }

        // Notes: positions, drum map, velocities, clip windowing, overlaps.
        let Ok((tpbar, tpbeat)) = tpx else { continue };
        let is_drum = track
            .chain
            .first()
            .is_some_and(|n| n.module == "drum_sampler");
        // (start, end, pitch, id) per note, for the overlap pass.
        let mut intervals: Vec<(u64, u64, u8, String)> = Vec::new();
        for clip in &track.clips {
            let c_loc = format!("{t_loc} clip '{}'", clip.id);
            let clip_bounds = position_to_ticks(&clip.start, tpbar, tpbeat)
                .and_then(|s| duration_to_ticks(&clip.length, tpbar, tpbeat).map(|l| (s, s + l)));
            if let Err(e) = &clip_bounds {
                findings.push(error("structure", c_loc.clone(), e.to_string()));
            }
            for note in &clip.notes {
                let n_loc = format!("{c_loc} note '{}'", note.id);
                let start = match position_to_ticks(&note.start, tpbar, tpbeat) {
                    Ok(s) => s,
                    Err(e) => {
                        findings.push(error("structure", n_loc.clone(), e.to_string()));
                        continue;
                    }
                };
                let length = match duration_to_ticks(&note.length, tpbar, tpbeat) {
                    Ok(l) => l,
                    Err(e) => {
                        findings.push(error("structure", n_loc.clone(), e.to_string()));
                        continue;
                    }
                };
                if is_drum && !DRUM_PITCHES.contains(&note.pitch) {
                    findings.push(error(
                        "drum-pitch",
                        n_loc.clone(),
                        format!(
                            "pitch {} is not in the drum map {DRUM_PITCHES:?} — the note is \
                             silent",
                            note.pitch
                        ),
                    ));
                }
                if note.vel == 0 {
                    findings.push(warn(
                        "vel-zero",
                        n_loc.clone(),
                        "velocity 0 renders silent".into(),
                    ));
                }
                if let Ok((clip_start, clip_end)) = clip_bounds {
                    if start < clip_start || start + length > clip_end {
                        findings.push(warn(
                            "note-in-clip",
                            n_loc.clone(),
                            "note extends outside its clip (positions are absolute; the \
                             engine plays it, the UI expects containment)"
                                .into(),
                        ));
                    }
                }
                intervals.push((start, start + length, note.pitch, note.id.clone()));
            }
        }
        // Same-pitch overlaps confuse the sequencer's on/off pairing. Sort
        // by (pitch, start) and sweep with the running furthest end.
        intervals.sort_by_key(|(start, _, pitch, _)| (*pitch, *start));
        let mut sweep: Option<(u8, u64, String)> = None;
        for (start, end, pitch, id) in intervals {
            match &mut sweep {
                Some((p, max_end, last_id)) if *p == pitch => {
                    if start < *max_end {
                        findings.push(warn(
                            "overlap",
                            format!("{t_loc} notes '{last_id}' and '{id}'"),
                            format!("same-pitch notes overlap (pitch {pitch})"),
                        ));
                    }
                    if end > *max_end {
                        *max_end = end;
                        *last_id = id;
                    }
                }
                _ => sweep = Some((pitch, end, id)),
            }
        }
    }
}

fn print_summary(project: &Project) {
    let tracks = project.tracks.len();
    let clips: usize = project.tracks.iter().map(|t| t.clips.len()).sum();
    let notes: usize = project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .map(|c| c.notes.len())
        .sum();
    println!(
        "song        {} bpm, {}/{}, {tracks} track(s), {clips} clip(s), {notes} note(s)",
        project.bpm, project.time_signature[0], project.time_signature[1]
    );

    let Ok((tpbar, tpbeat)) = project.ticks_per_bar_beat() else {
        return;
    };
    let mut content_end = 0u64;
    for track in &project.tracks {
        let mut pitches: Vec<u8> = Vec::new();
        for clip in &track.clips {
            if let (Ok(s), Ok(l)) = (
                position_to_ticks(&clip.start, tpbar, tpbeat),
                duration_to_ticks(&clip.length, tpbar, tpbeat),
            ) {
                content_end = content_end.max(s + l);
            }
            for note in &clip.notes {
                pitches.push(note.pitch);
                if let (Ok(s), Ok(l)) = (
                    position_to_ticks(&note.start, tpbar, tpbeat),
                    duration_to_ticks(&note.length, tpbar, tpbeat),
                ) {
                    content_end = content_end.max(s + l);
                }
            }
        }
        let range = match (pitches.iter().min(), pitches.iter().max()) {
            (Some(&lo), Some(&hi)) => format!("pitches {lo}..{hi}"),
            _ => "no notes".into(),
        };
        let modules: Vec<&str> = track.chain.iter().map(|n| n.module.as_str()).collect();
        println!(
            "track       '{}' [{}] {range}",
            track.id,
            modules.join(" -> ")
        );
    }
    let bars = content_end.div_ceil(tpbar);
    let seconds = (bars * tpbar) as f64 / tpbeat as f64 * 60.0 / project.bpm as f64;
    println!(
        "loop        {bars} bar(s), {seconds:.1} s at {} bpm",
        project.bpm
    );
}
