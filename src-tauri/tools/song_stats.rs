//! song_stats — compositional metrics and an anti-plagiarism guard.
//!
//! Where reference_match compares mixes, this compares *writing*: note
//! density, register, interval and rhythm distributions, self-repetition,
//! and — against transcribed references — melodic distance, the longest
//! shared (interval, rhythm) run (the hard evidence) and the novelty floor
//! (a composed line should sit at least as far from every reference as the
//! references sit from each other). The guard finds evidence, it cannot
//! prove absence: transcription noise breaks exact matches. Chord-grid
//! comparison is a possible v2 (song.json carries no chord annotation).
//!
//! Both the input and every --vs accept a song.json or a transcribe --json
//! output; comparing a transcription to itself is the detector's sanity
//! check. Tokens use the primary input's beat resolution on both sides
//! (identical at PPQ 960 / 4/4, the only grid the transcriber emits).
//!
//! Usage:
//!   cargo run --bin song_stats -- <song.json | transcription.json>
//!       [--vs <transcription.json>]... [--strict] [--json <path>]
//!
//! Exit codes: 0 no defects (warnings allowed; --strict promotes them),
//! 1 tool failure (unreadable file), 2 plagiarism defects.

// Shared CLI helpers; each bin uses a different subset (see util.rs header).
#[allow(dead_code)]
mod util;

use std::path::Path;

use serde::Serialize;
use telenn_lib::analysis::composition::{
    compare, drum_stats_from_hits, drum_stats_from_track, guard, novelty_floor, Comparison,
    DrumStats, FlagLevel, GuardFinding, Line, LineStats,
};
use telenn_lib::analysis::transcribe::Transcription;
use telenn_lib::control::project::Project;

/// Fixed grid of a transcription (PPQ 960, 4/4).
const REF_TPBAR: u64 = 3840;
const REF_TPBEAT: u64 = 960;

enum Input {
    Song(Project),
    Reference(Transcription),
}

struct Extracted {
    kind: &'static str,
    bpm: f32,
    time_signature: [u32; 2],
    tpbar: u64,
    tpbeat: u64,
    /// Melodic lines with their stats, computed once.
    lines: Vec<(Line, LineStats)>,
    drums: Vec<DrumStats>,
}

#[derive(Serialize)]
struct LabeledStats {
    label: String,
    stats: LineStats,
}

#[derive(Serialize)]
struct Pair {
    a: String,
    b: String,
    distance: f32,
}

#[derive(Serialize)]
struct Spread {
    pairwise: Vec<Pair>,
    novelty_floor: f32,
}

#[derive(Serialize)]
struct StatsReport {
    file: String,
    kind: &'static str,
    bpm: f32,
    time_signature: [u32; 2],
    lines: Vec<LabeledStats>,
    drums: Vec<DrumStats>,
    comparisons: Vec<Comparison>,
    reference_spread: Option<Spread>,
    findings: Vec<GuardFinding>,
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
    let vs_paths = args.take_values("--vs")?;
    let positional = args.finish_positional()?;
    let file = match positional.as_slice() {
        [f] => f.clone(),
        _ => {
            return Err("usage: song_stats <song.json | transcription.json> \
                 [--vs <transcription.json>]... [--strict] [--json p]"
                .into())
        }
    };

    let input = load_input(&file)?;
    let main = extract(&input, None)?;

    println!("== input ==");
    println!("file         {file}");
    println!("kind         {}", main.kind);
    println!(
        "grid         {:.1} bpm, {}/{}",
        main.bpm, main.time_signature[0], main.time_signature[1]
    );
    println!(
        "content      {} melodic line(s), {} drum layer(s)",
        main.lines.len(),
        main.drums.len()
    );

    for (line, stats) in &main.lines {
        println!("\n== line {} ==", line.label);
        print!("{stats}");
    }
    for drums in &main.drums {
        println!("\n== drums {} ==", drums.label);
        print!("{drums}");
    }

    // References: compare every composed line against every reference line.
    let mut ref_lines: Vec<(String, Vec<(Line, LineStats)>)> = Vec::new();
    let mut comparisons = Vec::new();
    let mut findings = Vec::new();
    for path in &vs_paths {
        let stem = file_stem(path);
        let reference = extract(&load_input(path)?, Some(&stem))?;
        println!("\n== vs {stem} ==");
        if reference.lines.is_empty() {
            println!("no melodic line found in {path}");
        }
        for (ref_line, ref_stats) in &reference.lines {
            println!(
                "{}: {} notes over {} bar(s), register {}..{}, self-rep {:.2}",
                ref_line.label,
                ref_stats.note_count,
                ref_stats.span_bars,
                ref_stats.pitch_min,
                ref_stats.pitch_max,
                ref_stats.self_repetition
            );
            for (line, stats) in &main.lines {
                let cmp = compare(line, ref_line, main.tpbar, main.tpbeat);
                print!("{cmp}");
                findings.extend(guard(&cmp, stats, ref_stats));
                comparisons.push(cmp);
            }
        }
        ref_lines.push((stem, reference.lines));
    }

    // Novelty floor: pairwise distances between reference lines of
    // *different* files (two voices of one reference are one piece).
    let mut spread = None;
    if ref_lines.len() >= 2 {
        println!("\n== reference spread ==");
        let mut pairwise = Vec::new();
        for i in 0..ref_lines.len() {
            for j in i + 1..ref_lines.len() {
                for (a, _) in &ref_lines[i].1 {
                    for (b, _) in &ref_lines[j].1 {
                        let d = compare(a, b, main.tpbar, main.tpbeat).distance;
                        println!("{} vs {}: distance {d:.2}", a.label, b.label);
                        pairwise.push(Pair {
                            a: a.label.clone(),
                            b: b.label.clone(),
                            distance: d,
                        });
                    }
                }
            }
        }
        let distances: Vec<f32> = pairwise.iter().map(|p| p.distance).collect();
        if let Some(floor) = novelty_floor(&distances) {
            println!("novelty floor {floor:.2} (min inter-reference distance)");
            for (line, _) in &main.lines {
                let closest = comparisons
                    .iter()
                    .filter(|c| c.a == line.label)
                    .map(|c| (c.distance, c.b.clone()))
                    .min_by(|x, y| x.0.total_cmp(&y.0));
                if let Some((d, closest_ref)) = closest {
                    let ok = if d >= floor {
                        "above floor"
                    } else {
                        "BELOW floor"
                    };
                    println!("{}: min ref distance {d:.2} ({ok})", line.label);
                    if d < floor {
                        findings.push(GuardFinding {
                            level: FlagLevel::Warning,
                            rule: "novelty-floor",
                            message: format!(
                                "'{}' is closer to '{closest_ref}' ({d:.2}) than the \
                                 references are to each other ({floor:.2})",
                                line.label
                            ),
                        });
                    }
                }
            }
            spread = Some(Spread {
                pairwise,
                novelty_floor: floor,
            });
        }
    }

    let defects = findings
        .iter()
        .filter(|f| f.level == FlagLevel::Defect)
        .count();
    let warnings = findings.len() - defects;
    if !vs_paths.is_empty() {
        println!("\n== verdict ==");
        for f in &findings {
            let tag = match f.level {
                FlagLevel::Defect => "DEFECT",
                FlagLevel::Warning => "WARN  ",
            };
            println!("{tag} [{}] {}", f.rule, f.message);
        }
        println!("guard        {defects} defect(s), {warnings} warning(s)");
    }

    if let Some(path) = json_path {
        let report = StatsReport {
            file,
            kind: main.kind,
            bpm: main.bpm,
            time_signature: main.time_signature,
            lines: main
                .lines
                .into_iter()
                .map(|(line, stats)| LabeledStats {
                    label: line.label,
                    stats,
                })
                .collect(),
            drums: main.drums,
            comparisons,
            reference_spread: spread,
            findings,
        };
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
    }

    let fail = defects > 0 || (strict && warnings > 0);
    Ok(if fail {
        util::EXIT_DEFECTS
    } else {
        util::EXIT_OK
    })
}

/// Sniff the file shape from its top-level fields: a song has `tracks`,
/// a transcription has `grid`. Trying one parser and falling back would
/// swallow a malformed song.json behind the wrong error message.
fn load_input(path: &str) -> Result<Input, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    if value.get("tracks").is_some() {
        let project: Project = serde_json::from_str(&text)
            .map_err(|e| format!("{path}: not a valid song.json ({e}); run song_lint"))?;
        Ok(Input::Song(project))
    } else if value.get("grid").is_some() {
        let t: Transcription = serde_json::from_str(&text)
            .map_err(|e| format!("{path}: not a valid transcription ({e})"))?;
        Ok(Input::Reference(t))
    } else {
        Err(format!(
            "{path}: neither a song.json (no `tracks`) nor a transcription (no `grid`)"
        ))
    }
}

fn extract(input: &Input, label_prefix: Option<&str>) -> Result<Extracted, String> {
    let label = |base: &str| match label_prefix {
        Some(prefix) => format!("{prefix}:{base}"),
        None => base.to_string(),
    };
    match input {
        Input::Song(project) => {
            let (tpbar, tpbeat) = project.ticks_per_bar_beat().map_err(|e| e.to_string())?;
            let mut lines = Vec::new();
            let mut drums = Vec::new();
            for track in &project.tracks {
                let is_drum = track
                    .chain
                    .first()
                    .is_some_and(|n| n.module == "drum_sampler");
                let has_notes = track.clips.iter().any(|c| !c.notes.is_empty());
                if !has_notes {
                    continue;
                }
                if is_drum {
                    let mut stats =
                        drum_stats_from_track(track, tpbar, tpbeat).map_err(|e| e.to_string())?;
                    stats.label = label(&track.id);
                    drums.push(stats);
                } else {
                    let mut line =
                        Line::from_track(track, tpbar, tpbeat).map_err(|e| e.to_string())?;
                    line.label = label(&track.id);
                    let stats = line.stats(tpbar, tpbeat);
                    lines.push((line, stats));
                }
            }
            Ok(Extracted {
                kind: "song",
                bpm: project.bpm as f32,
                time_signature: project.time_signature,
                tpbar,
                tpbeat,
                lines,
                drums,
            })
        }
        Input::Reference(t) => {
            let mut lines = Vec::new();
            if !t.bass.is_empty() {
                let line = Line::from_bass(&label("bass"), &t.bass, REF_TPBAR, REF_TPBEAT)
                    .map_err(|e| e.to_string())?;
                let stats = line.stats(REF_TPBAR, REF_TPBEAT);
                lines.push((line, stats));
            }
            let mut drums = Vec::new();
            if !t.drums.is_empty() {
                drums.push(
                    drum_stats_from_hits(&label("drums"), &t.drums, REF_TPBAR, REF_TPBEAT)
                        .map_err(|e| e.to_string())?,
                );
            }
            Ok(Extracted {
                kind: "transcription",
                bpm: t.grid.bpm,
                time_signature: t.grid.time_signature,
                tpbar: REF_TPBAR,
                tpbeat: REF_TPBEAT,
                lines,
                drums,
            })
        }
    }
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}
