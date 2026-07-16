//! Compositional metrics and melodic line distance.
//!
//! Every source (a song.json track, a transcribed bass line) is normalized
//! to a [`Line`] of tick-domain events, so all metrics and distances are
//! tempo-independent by construction. Matching works on (interval, IOI)
//! tokens — transposition-invariant, rhythm-aware — with two safeguards
//! against the ostinato pathology (two unrelated one-note pedals are
//! token-identical): n-grams are weighted by interval variety, and
//! low-information comparisons never raise similarity flags.
//!
//! The hard plagiarism evidence is a long shared token run with interval
//! variety; the similarity score is a softer, statistical signal. The guard
//! finds evidence — it cannot prove absence (transcription noise splits or
//! detunes notes and breaks exact token matches). Chord-grid comparison is
//! out of scope for now: song.json carries no chord annotation and a chord
//! progression is genre furniture, not melodic identity.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use crate::analysis::transcribe::{BassNote, DrumHit};
use crate::analysis::AnalysisError;
use crate::control::project::{duration_to_ticks, position_to_ticks, ticks_to_position, Track};

/// Onset-gap clamp: gaps beyond two 4/4 bars are arrangement, not melody —
/// letting them vary freely would fragment matches over silence length.
pub const IOI_MAX_STEPS: u32 = 32;
/// Cells of the per-bar onset histogram (sixteenth grid).
pub const GRID_CELLS: usize = 16;
/// n-gram sizes used by the similarity score.
pub const NGRAM_MIN: usize = 3;
pub const NGRAM_MAX: usize = 5;
/// n-gram size of the self-repetition metric.
pub const SELF_REP_N: usize = 4;
/// Shared (interval, IOI) run lengths, in intervals (notes = len + 1), that
/// raise the hard flag. Genre formulas can legitimately collide up to 6-7
/// intervals; an exact 8-interval run with matching rhythm is no accident.
pub const SHARED_RUN_DEFECT: usize = 8;
pub const SHARED_RUN_WARN: usize = 6;
/// Interval-only shared run (rhythm ignored) that flags rhythm-displaced
/// copying — informational, never a defect on its own.
pub const INTERVAL_RUN_WARN: usize = 10;
/// Similarity flag thresholds; both apply only to informative comparisons.
pub const SIMILARITY_DEFECT: f32 = 0.85;
pub const SIMILARITY_WARN: f32 = 0.6;
pub const MIN_NOTES_FOR_SIMILARITY_FLAG: usize = 10;
pub const MIN_DISTINCT_INTERVALS_FOR_FLAG: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct NoteEvent {
    pub pitch: u8,
    pub start_ticks: u64,
    pub length_ticks: u64,
    pub vel: u8,
}

/// A monophonic-ish note sequence, sorted by (start, pitch).
#[derive(Debug, Clone, Serialize)]
pub struct Line {
    pub label: String,
    pub events: Vec<NoteEvent>,
}

/// One melodic step: signed semitones to the next note and the onset gap
/// quantized to sixteenth steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Token {
    pub interval: i32,
    pub ioi_steps: u32,
}

impl Line {
    /// Flatten a song.json track (notes carry absolute positions; clips are
    /// containers only) into a sorted line.
    pub fn from_track(track: &Track, tpbar: u64, tpbeat: u64) -> Result<Self, AnalysisError> {
        let mut events = Vec::new();
        for clip in &track.clips {
            for note in &clip.notes {
                let context = |e| {
                    AnalysisError::BadInput(format!("track '{}' note '{}': {e}", track.id, note.id))
                };
                events.push(NoteEvent {
                    pitch: note.pitch,
                    start_ticks: position_to_ticks(&note.start, tpbar, tpbeat).map_err(context)?,
                    length_ticks: duration_to_ticks(&note.length, tpbar, tpbeat)
                        .map_err(context)?,
                    vel: note.vel,
                });
            }
        }
        events.sort_by_key(|e| (e.start_ticks, e.pitch));
        Ok(Self {
            label: track.id.clone(),
            events,
        })
    }

    /// Convert a transcribed bass line (already on the PPQ-960 grid).
    pub fn from_bass(
        label: &str,
        bass: &[BassNote],
        tpbar: u64,
        tpbeat: u64,
    ) -> Result<Self, AnalysisError> {
        let mut events = Vec::new();
        for note in bass {
            let context =
                |e| AnalysisError::BadInput(format!("bass note at '{}': {e}", note.start));
            events.push(NoteEvent {
                pitch: note.pitch,
                start_ticks: position_to_ticks(&note.start, tpbar, tpbeat).map_err(context)?,
                length_ticks: duration_to_ticks(&note.length, tpbar, tpbeat).map_err(context)?,
                vel: note.vel,
            });
        }
        events.sort_by_key(|e| (e.start_ticks, e.pitch));
        Ok(Self {
            label: label.to_string(),
            events,
        })
    }

    /// (interval, IOI) tokens between consecutive events.
    pub fn tokens(&self, tpbeat: u64) -> Vec<Token> {
        let step = (tpbeat / 4).max(1);
        self.events
            .windows(2)
            .map(|w| Token {
                interval: w[1].pitch as i32 - w[0].pitch as i32,
                ioi_steps: quantize_ioi(w[1].start_ticks - w[0].start_ticks, step),
            })
            .collect()
    }

    pub fn stats(&self, tpbar: u64, tpbeat: u64) -> LineStats {
        let note_count = self.events.len();
        let span_bars = span_bars(
            self.events.first().map(|e| e.start_ticks),
            self.events
                .iter()
                .map(|e| e.start_ticks + e.length_ticks)
                .max(),
            tpbar,
        );
        let density = if span_bars > 0 {
            note_count as f32 / span_bars as f32
        } else {
            0.0
        };
        let mut pitches: Vec<u8> = self.events.iter().map(|e| e.pitch).collect();
        pitches.sort_unstable();
        let mut interval_histogram = BTreeMap::new();
        for w in self.events.windows(2) {
            *interval_histogram
                .entry(w[1].pitch as i32 - w[0].pitch as i32)
                .or_insert(0usize) += 1;
        }
        let mut onset_grid = [0usize; GRID_CELLS];
        for e in &self.events {
            let cell = ((e.start_ticks % tpbar) as usize * GRID_CELLS) / tpbar as usize;
            onset_grid[cell.min(GRID_CELLS - 1)] += 1;
        }
        let tokens = self.tokens(tpbeat);
        let mut ioi_histogram = BTreeMap::new();
        for t in &tokens {
            *ioi_histogram.entry(t.ioi_steps).or_insert(0usize) += 1;
        }
        let total_grams = tokens.len().saturating_sub(SELF_REP_N - 1);
        let self_repetition = if total_grams >= 2 {
            let distinct = gram_counts(&tokens, SELF_REP_N).len();
            1.0 - distinct as f32 / total_grams as f32
        } else {
            0.0
        };
        LineStats {
            note_count,
            span_bars,
            density_notes_per_bar: density,
            pitch_min: pitches.first().copied().unwrap_or(0),
            pitch_median: pitches
                .get(note_count.saturating_sub(1) / 2)
                .copied()
                .unwrap_or(0),
            pitch_max: pitches.last().copied().unwrap_or(0),
            distinct_intervals: interval_histogram.len(),
            interval_histogram,
            onset_grid,
            ioi_histogram,
            self_repetition,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LineStats {
    pub note_count: usize,
    /// First-onset bar through last-end bar, inclusive.
    pub span_bars: u64,
    pub density_notes_per_bar: f32,
    pub pitch_min: u8,
    /// Lower-middle median.
    pub pitch_median: u8,
    pub pitch_max: u8,
    pub distinct_intervals: usize,
    /// Signed semitone steps between consecutive notes.
    pub interval_histogram: BTreeMap<i32, usize>,
    /// Onset counts per sixteenth cell of the bar, all bars summed.
    pub onset_grid: [usize; GRID_CELLS],
    /// Inter-onset intervals in sixteenth steps, clamped.
    pub ioi_histogram: BTreeMap<u32, usize>,
    /// 1 - distinct/total 4-grams of tokens: 0 = never repeats, ~1 = ostinato.
    pub self_repetition: f32,
}

impl fmt::Display for LineStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "notes        {} over {} bar(s), {:.1}/bar",
            self.note_count, self.span_bars, self.density_notes_per_bar
        )?;
        if self.note_count == 0 {
            return Ok(());
        }
        writeln!(
            f,
            "register     {}..{} (median {})",
            self.pitch_min, self.pitch_max, self.pitch_median
        )?;
        write!(f, "intervals    {} distinct:", self.distinct_intervals)?;
        for (iv, n) in &self.interval_histogram {
            write!(f, " {iv:+}x{n}")?;
        }
        writeln!(f)?;
        write!(f, "ioi (16ths) ")?;
        for (ioi, n) in &self.ioi_histogram {
            write!(f, " {ioi}x{n}")?;
        }
        writeln!(f)?;
        write!(f, "onset grid  ")?;
        for (i, n) in self.onset_grid.iter().enumerate() {
            if i % 4 == 0 {
                write!(f, " ")?;
            }
            write!(f, "{}", cell_char(*n))?;
        }
        writeln!(f)?;
        writeln!(
            f,
            "self-rep     {:.2} ({SELF_REP_N}-gram redundancy)",
            self.self_repetition
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DrumClassStats {
    pub class: String,
    pub count: usize,
    pub per_bar: f32,
    pub grid: [usize; GRID_CELLS],
}

#[derive(Debug, Clone, Serialize)]
pub struct DrumStats {
    pub label: String,
    pub span_bars: u64,
    pub classes: Vec<DrumClassStats>,
}

impl fmt::Display for DrumStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.classes {
            write!(
                f,
                "{:<7} {:>4} hits {:>5.2}/bar  ",
                c.class, c.count, c.per_bar
            )?;
            for (i, n) in c.grid.iter().enumerate() {
                if i > 0 && i % 4 == 0 {
                    write!(f, " ")?;
                }
                write!(f, "{}", cell_char(*n))?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

/// The song.json drum map (pitches outside it are silent in the sampler but
/// still counted here, under "other").
const DRUM_CLASSES: [(u8, &str); 8] = [
    (36, "kick"),
    (38, "snare"),
    (39, "clap"),
    (42, "ch"),
    (45, "lt"),
    (46, "oh"),
    (49, "crash"),
    (50, "ht"),
];

pub fn drum_stats_from_track(
    track: &Track,
    tpbar: u64,
    tpbeat: u64,
) -> Result<DrumStats, AnalysisError> {
    let mut events = Vec::new();
    for clip in &track.clips {
        for note in &clip.notes {
            let start = position_to_ticks(&note.start, tpbar, tpbeat).map_err(|e| {
                AnalysisError::BadInput(format!("track '{}' note '{}': {e}", track.id, note.id))
            })?;
            let class = DRUM_CLASSES
                .iter()
                .find(|&&(p, _)| p == note.pitch)
                .map_or("other", |&(_, c)| c);
            events.push((class.to_string(), start));
        }
    }
    Ok(build_drum_stats(&track.id, &events, tpbar))
}

pub fn drum_stats_from_hits(
    label: &str,
    hits: &[DrumHit],
    tpbar: u64,
    tpbeat: u64,
) -> Result<DrumStats, AnalysisError> {
    let mut events = Vec::new();
    for hit in hits {
        let start = position_to_ticks(&hit.start, tpbar, tpbeat)
            .map_err(|e| AnalysisError::BadInput(format!("drum hit at '{}': {e}", hit.start)))?;
        events.push((hit.class.clone(), start));
    }
    Ok(build_drum_stats(label, &events, tpbar))
}

fn build_drum_stats(label: &str, events: &[(String, u64)], tpbar: u64) -> DrumStats {
    let span_bars = span_bars(
        events.iter().map(|&(_, t)| t).min(),
        events.iter().map(|&(_, t)| t + 1).max(),
        tpbar,
    );
    // Known classes first, in map order, then anything else as encountered.
    let mut order: Vec<&str> = Vec::new();
    for &(_, name) in &DRUM_CLASSES {
        if events.iter().any(|(c, _)| c == name) {
            order.push(name);
        }
    }
    for (c, _) in events {
        if !order.contains(&c.as_str()) {
            order.push(c);
        }
    }
    let classes = order
        .iter()
        .map(|&name| {
            let mut grid = [0usize; GRID_CELLS];
            let mut count = 0usize;
            for &(_, start) in events.iter().filter(|(c, _)| c == name) {
                let cell = ((start % tpbar) as usize * GRID_CELLS) / tpbar as usize;
                grid[cell.min(GRID_CELLS - 1)] += 1;
                count += 1;
            }
            DrumClassStats {
                class: name.to_string(),
                count,
                per_bar: if span_bars > 0 {
                    count as f32 / span_bars as f32
                } else {
                    0.0
                },
                grid,
            }
        })
        .collect();
    DrumStats {
        label: label.to_string(),
        span_bars,
        classes,
    }
}

/// The longest shared (interval, IOI) run between two lines — the readable
/// plagiarism evidence.
#[derive(Debug, Clone, Serialize)]
pub struct SharedRun {
    /// In intervals; notes = len + 1.
    pub len: usize,
    pub intervals: Vec<i32>,
    pub ioi_steps: Vec<u32>,
    pub distinct_intervals: usize,
    pub a_note_index: usize,
    pub b_note_index: usize,
    /// `"bar.beat.tick"` of the run's first note on each side.
    pub a_start: String,
    pub b_start: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub a: String,
    pub b: String,
    pub similarity: f32,
    pub distance: f32,
    /// The (n, containment) pairs actually computed.
    pub per_n: Vec<(usize, f32)>,
    pub shared_run: Option<SharedRun>,
    /// Longest common interval run ignoring rhythm.
    pub interval_run_len: usize,
    /// True when either line is too short or has < 2 distinct intervals;
    /// similarity flags are meaningless there (see module docs).
    pub low_information: bool,
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} vs {}: distance {:.2} (similarity {:.2}",
            self.a, self.b, self.distance, self.similarity
        )?;
        for (n, c) in &self.per_n {
            write!(f, " n{n}={c:.2}")?;
        }
        write!(f, ")")?;
        if self.low_information {
            write!(f, " [low information]")?;
        }
        writeln!(f)?;
        match &self.shared_run {
            Some(run) => {
                write!(
                    f,
                    "shared run: {} intervals ({} notes) at {} / {}, steps",
                    run.len,
                    run.len + 1,
                    run.a_start,
                    run.b_start
                )?;
                // Long runs are self-evident; keep the report scannable.
                for iv in run.intervals.iter().take(24) {
                    write!(f, " {iv:+}")?;
                }
                if run.intervals.len() > 24 {
                    write!(f, " …")?;
                }
                writeln!(f)?;
            }
            None => writeln!(f, "shared run: none")?,
        }
        writeln!(f, "interval-only run: {} intervals", self.interval_run_len)
    }
}

/// Compare two lines: n-gram similarity, longest shared run, both invariant
/// under transposition and tempo.
pub fn compare(a: &Line, b: &Line, tpbar: u64, tpbeat: u64) -> Comparison {
    let ta = a.tokens(tpbeat);
    let tb = b.tokens(tpbeat);
    let mut per_n = Vec::new();
    for n in NGRAM_MIN..=NGRAM_MAX {
        if let Some(c) = weighted_containment(&ta, &tb, n) {
            per_n.push((n, c));
        }
    }
    let similarity = if per_n.is_empty() {
        0.0
    } else {
        per_n.iter().map(|&(_, c)| c).sum::<f32>() / per_n.len() as f32
    };
    let low_information = ta.len() < NGRAM_MIN
        || tb.len() < NGRAM_MIN
        || distinct_interval_count(&ta) < 2
        || distinct_interval_count(&tb) < 2;
    let shared_run = longest_common_slice(&ta, &tb).map(|(len, ai, bi)| {
        let run = &ta[ai..ai + len];
        SharedRun {
            len,
            intervals: run.iter().map(|t| t.interval).collect(),
            ioi_steps: run.iter().map(|t| t.ioi_steps).collect(),
            distinct_intervals: distinct_interval_count(run),
            a_note_index: ai,
            b_note_index: bi,
            a_start: ticks_to_position(a.events[ai].start_ticks, tpbar, tpbeat),
            b_start: ticks_to_position(b.events[bi].start_ticks, tpbar, tpbeat),
        }
    });
    let ia: Vec<i32> = ta.iter().map(|t| t.interval).collect();
    let ib: Vec<i32> = tb.iter().map(|t| t.interval).collect();
    let interval_run_len = longest_common_slice(&ia, &ib).map_or(0, |(len, _, _)| len);
    Comparison {
        a: a.label.clone(),
        b: b.label.clone(),
        similarity,
        distance: 1.0 - similarity,
        per_n,
        shared_run,
        interval_run_len,
        low_information,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagLevel {
    Warning,
    Defect,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardFinding {
    pub level: FlagLevel,
    pub rule: &'static str,
    pub message: String,
}

/// Apply the plagiarism rules to one comparison. The shared-run rule is the
/// hard evidence and needs interval variety; everything else is advisory and
/// suppressed on low-information comparisons.
pub fn guard(cmp: &Comparison, a: &LineStats, b: &LineStats) -> Vec<GuardFinding> {
    let mut findings = Vec::new();
    if let Some(run) = &cmp.shared_run {
        if run.distinct_intervals >= 2 {
            if run.len >= SHARED_RUN_DEFECT {
                findings.push(GuardFinding {
                    level: FlagLevel::Defect,
                    rule: "shared-run",
                    message: format!(
                        "'{}' and '{}' share a {}-interval run ({} notes, rhythm included) \
                         at {} / {}",
                        cmp.a,
                        cmp.b,
                        run.len,
                        run.len + 1,
                        run.a_start,
                        run.b_start
                    ),
                });
            } else if run.len >= SHARED_RUN_WARN {
                findings.push(GuardFinding {
                    level: FlagLevel::Warning,
                    rule: "shared-run",
                    message: format!(
                        "'{}' and '{}' share a {}-interval run at {} / {} — genre formulas \
                         collide here, check by ear",
                        cmp.a, cmp.b, run.len, run.a_start, run.b_start
                    ),
                });
            }
        }
    }
    if !cmp.low_information {
        if cmp.interval_run_len >= INTERVAL_RUN_WARN {
            findings.push(GuardFinding {
                level: FlagLevel::Warning,
                rule: "interval-run",
                message: format!(
                    "'{}' and '{}' share a {}-interval melodic run ignoring rhythm",
                    cmp.a, cmp.b, cmp.interval_run_len
                ),
            });
        }
        let eligible = a.note_count >= MIN_NOTES_FOR_SIMILARITY_FLAG
            && b.note_count >= MIN_NOTES_FOR_SIMILARITY_FLAG
            && a.distinct_intervals >= MIN_DISTINCT_INTERVALS_FOR_FLAG
            && b.distinct_intervals >= MIN_DISTINCT_INTERVALS_FOR_FLAG;
        if eligible {
            if cmp.similarity >= SIMILARITY_DEFECT {
                findings.push(GuardFinding {
                    level: FlagLevel::Defect,
                    rule: "similarity",
                    message: format!(
                        "'{}' reads as a variation of '{}' (similarity {:.2})",
                        cmp.a, cmp.b, cmp.similarity
                    ),
                });
            } else if cmp.similarity >= SIMILARITY_WARN {
                findings.push(GuardFinding {
                    level: FlagLevel::Warning,
                    rule: "similarity",
                    message: format!(
                        "'{}' leans on '{}' (similarity {:.2})",
                        cmp.a, cmp.b, cmp.similarity
                    ),
                });
            }
        }
    }
    findings
}

/// The novelty floor: how far the references sit from each other. A composed
/// line closer to a reference than this floor imitates that reference more
/// than the artist imitates themselves.
pub fn novelty_floor(pairwise: &[f32]) -> Option<f32> {
    pairwise.iter().copied().reduce(f32::min)
}

fn quantize_ioi(dt: u64, step: u64) -> u32 {
    (((dt + step / 2) / step) as u32).min(IOI_MAX_STEPS)
}

fn span_bars(first_start: Option<u64>, last_end: Option<u64>, tpbar: u64) -> u64 {
    match (first_start, last_end) {
        (Some(start), Some(end)) if tpbar > 0 => {
            let first_bar = start / tpbar;
            let last_bar = end.saturating_sub(1).max(start) / tpbar;
            last_bar - first_bar + 1
        }
        _ => 0,
    }
}

fn cell_char(count: usize) -> char {
    match count {
        0 => '.',
        1..=9 => char::from(b'0' + count as u8),
        _ => '*',
    }
}

fn gram_counts<T: Ord>(tokens: &[T], n: usize) -> BTreeMap<&[T], usize> {
    let mut counts = BTreeMap::new();
    for gram in tokens.windows(n) {
        *counts.entry(gram).or_insert(0usize) += 1;
    }
    counts
}

fn distinct_interval_count(tokens: &[Token]) -> usize {
    let mut vals: Vec<i32> = tokens.iter().map(|t| t.interval).collect();
    vals.sort_unstable();
    vals.dedup();
    vals.len()
}

fn gram_weight(gram: &[Token]) -> f32 {
    distinct_interval_count(gram) as f32
}

/// Symmetric weighted containment of n-grams: the matched share of the
/// smaller side's gram mass. Containment, not Jaccard — a short phrase
/// quoted inside a long line must score ~1, that is the plagiarism case.
fn weighted_containment(a: &[Token], b: &[Token], n: usize) -> Option<f32> {
    if a.len() < n || b.len() < n {
        return None;
    }
    let ca = gram_counts(a, n);
    let cb = gram_counts(b, n);
    let mass = |c: &BTreeMap<&[Token], usize>| {
        c.iter()
            .map(|(g, &cnt)| gram_weight(g) * cnt as f32)
            .sum::<f32>()
    };
    let denom = mass(&ca).min(mass(&cb));
    if denom <= 0.0 {
        return None;
    }
    let matched: f32 = ca
        .iter()
        .filter_map(|(g, &cnt)| cb.get(g).map(|&c2| gram_weight(g) * cnt.min(c2) as f32))
        .sum();
    let c = matched / denom;
    // `clamp` keeps a signed zero (-0.0 < 0.0 is false); map it to +0.0.
    Some(if c <= 0.0 { 0.0 } else { c.min(1.0) })
}

/// Longest common contiguous run: (length, start in a, start in b).
/// Two-row DP; sequences are at most a few hundred tokens.
fn longest_common_slice<T: PartialEq>(a: &[T], b: &[T]) -> Option<(usize, usize, usize)> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    let mut best = (0usize, 0usize, 0usize);
    for (i, ta) in a.iter().enumerate() {
        cur.fill(0);
        for (j, tb) in b.iter().enumerate() {
            if ta == tb {
                let len = prev[j] + 1;
                cur[j + 1] = len;
                if len > best.0 {
                    best = (len, i + 1 - len, j + 1 - len);
                }
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    if best.0 == 0 {
        None
    } else {
        Some(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::project::{Clip, Note};

    const TPBAR: u64 = 3840;
    const TPBEAT: u64 = 960;
    /// Eighth-note spacing used by most fixtures.
    const EIGHTH: u64 = 480;

    fn line(notes: &[(u8, u64, u64)]) -> Line {
        Line {
            label: "test".into(),
            events: notes
                .iter()
                .map(|&(pitch, start_ticks, length_ticks)| NoteEvent {
                    pitch,
                    start_ticks,
                    length_ticks,
                    vel: 100,
                })
                .collect(),
        }
    }

    /// Pitches laid out as straight eighths.
    fn seq(pitches: &[u8]) -> Line {
        line(
            &pitches
                .iter()
                .enumerate()
                .map(|(i, &p)| (p, i as u64 * EIGHTH, EIGHTH))
                .collect::<Vec<_>>(),
        )
    }

    fn note(pitch: u8, start: &str, length: &str) -> Note {
        Note {
            id: String::new(),
            pitch,
            start: start.into(),
            length: length.into(),
            vel: 100,
        }
    }

    fn track(clips: Vec<Clip>) -> Track {
        serde_json::from_value(serde_json::json!({
            "id": "t1",
            "chain": [],
            "clips": clips,
        }))
        .expect("track fixture")
    }

    fn clip(notes: Vec<Note>) -> Clip {
        Clip {
            id: String::new(),
            start: "1.1.0".into(),
            length: "1.0.0".into(),
            notes,
        }
    }

    #[test]
    fn from_track_flattens_clips_and_sorts() {
        let t = track(vec![
            clip(vec![
                note(43, "2.1.0", "0.0.480"),
                note(36, "1.1.0", "0.0.480"),
            ]),
            clip(vec![note(40, "1.3.0", "0.0.480")]),
        ]);
        let l = Line::from_track(&t, TPBAR, TPBEAT).expect("line");
        let ticks: Vec<u64> = l.events.iter().map(|e| e.start_ticks).collect();
        assert_eq!(ticks, vec![0, 1920, 3840]);
        let pitches: Vec<u8> = l.events.iter().map(|e| e.pitch).collect();
        assert_eq!(pitches, vec![36, 40, 43]);
    }

    #[test]
    fn from_bass_parses_bar_beat_tick_positions() {
        let bass = vec![bass_note(30, "1.1.0"), bass_note(32, "1.3.480")];
        let l = Line::from_bass("bass", &bass, TPBAR, TPBEAT).expect("line");
        assert_eq!(l.events[0].start_ticks, 0);
        assert_eq!(l.events[1].start_ticks, 2400);
    }

    fn bass_note(pitch: u8, start: &str) -> BassNote {
        BassNote {
            pitch,
            note: String::new(),
            start: start.into(),
            length: "0.0.480".into(),
            vel: 100,
            clarity: 0.9,
            raw_s: 0.0,
            quant_error_ticks: 0,
        }
    }

    #[test]
    fn stats_density_register_and_grid() {
        let l = line(&[
            (36, 0, 480),
            (43, 960, 480),
            (40, 3840, 480),
            (48, 3 * 3840 + 2880, 480),
        ]);
        let s = l.stats(TPBAR, TPBEAT);
        assert_eq!(s.note_count, 4);
        assert_eq!(s.span_bars, 4);
        assert!((s.density_notes_per_bar - 1.0).abs() < 1e-6);
        assert_eq!((s.pitch_min, s.pitch_median, s.pitch_max), (36, 40, 48));
        assert_eq!(s.onset_grid[0], 2);
        assert_eq!(s.onset_grid[4], 1);
        assert_eq!(s.onset_grid[12], 1);
    }

    #[test]
    fn interval_histogram_is_signed() {
        let s = seq(&[40, 47, 40]).stats(TPBAR, TPBEAT);
        assert_eq!(s.interval_histogram.get(&7), Some(&1));
        assert_eq!(s.interval_histogram.get(&-7), Some(&1));
        assert_eq!(s.distinct_intervals, 2);
    }

    #[test]
    fn ioi_quantizes_to_sixteenths_and_clamps() {
        let l = line(&[
            (40, 0, 100),
            (40, 230, 100),
            (40, 590, 100),
            (40, 590 + 9600, 100),
        ]);
        let steps: Vec<u32> = l.tokens(TPBEAT).iter().map(|t| t.ioi_steps).collect();
        assert_eq!(steps, vec![1, 2, IOI_MAX_STEPS]);
    }

    #[test]
    fn self_repetition_high_for_ostinato_low_for_varied() {
        let ostinato: Vec<u8> = [36u8, 36, 43, 41].repeat(8);
        let varied: Vec<u8> = vec![
            36, 43, 38, 50, 41, 47, 39, 52, 44, 36, 49, 42, 55, 40, 46, 58,
        ];
        let rep_ost = seq(&ostinato).stats(TPBAR, TPBEAT).self_repetition;
        let rep_var = seq(&varied).stats(TPBAR, TPBEAT).self_repetition;
        assert!(rep_ost > 0.7, "ostinato self-rep {rep_ost}");
        assert!(rep_var < 0.1, "varied self-rep {rep_var}");
    }

    #[test]
    fn identical_lines_have_distance_zero_and_full_run() {
        let l = seq(&[
            36, 43, 38, 50, 41, 47, 39, 52, 44, 36, 49, 42, 55, 40, 46, 58,
        ]);
        let cmp = compare(&l, &l, TPBAR, TPBEAT);
        assert!(cmp.distance < 1e-6, "distance {}", cmp.distance);
        let run = cmp.shared_run.expect("run");
        assert_eq!(run.len, l.events.len() - 1);
    }

    #[test]
    fn transposed_copy_has_distance_zero() {
        let pitches = [36u8, 43, 38, 50, 41, 47, 39, 52, 44, 36, 49, 42];
        let up: Vec<u8> = pitches.iter().map(|&p| p + 5).collect();
        let cmp = compare(&seq(&pitches), &seq(&up), TPBAR, TPBEAT);
        assert!(cmp.distance < 1e-6, "distance {}", cmp.distance);
    }

    #[test]
    fn unrelated_lines_have_distance_near_one() {
        let a = seq(&[36, 37, 39, 42, 46, 51, 57, 64, 72, 60, 49, 39, 30]);
        let b = seq(&[40, 64, 40, 64, 40, 64, 40, 64, 40, 64, 40, 64, 40]);
        let cmp = compare(&a, &b, TPBAR, TPBEAT);
        assert!(cmp.distance > 0.7, "distance {}", cmp.distance);
    }

    #[test]
    fn distance_is_symmetric() {
        let a = seq(&[36, 43, 38, 50, 41, 47, 39, 52]);
        let b = seq(&[36, 43, 38, 50, 60, 58, 55, 50, 45, 41]);
        let ab = compare(&a, &b, TPBAR, TPBEAT);
        let ba = compare(&b, &a, TPBAR, TPBEAT);
        assert_eq!(ab.distance, ba.distance);
    }

    /// The varied 9-note quote planted by the run tests: 8 intervals, all
    /// different from the prefixes/suffixes around it.
    const QUOTE: [u8; 9] = [50, 53, 51, 56, 54, 49, 57, 52, 58];

    #[test]
    fn short_quote_inside_long_line_scores_containment_high() {
        let quote = [60u8, 63, 61, 66, 64, 59, 67, 62, 68, 65, 70, 66];
        let mut long: Vec<u8> = vec![
            36, 38, 41, 39, 44, 40, 45, 43, 48, 42, 47, 46, 51, 49, 52, 50, 55, 53, 56, 54,
        ];
        long.extend_from_slice(&quote);
        long.extend_from_slice(&[
            30, 34, 31, 37, 33, 29, 36, 32, 38, 35, 28, 39, 27, 40, 26, 41,
        ]);
        let cmp = compare(&seq(&quote), &seq(&long), TPBAR, TPBEAT);
        assert!(cmp.similarity > 0.8, "similarity {}", cmp.similarity);
    }

    #[test]
    fn shared_run_reports_length_and_positions() {
        let mut a: Vec<u8> = vec![30, 32, 31, 33];
        a.extend_from_slice(&QUOTE);
        a.extend_from_slice(&[40, 42]);
        let mut b: Vec<u8> = vec![70, 65];
        b.extend_from_slice(&QUOTE);
        b.extend_from_slice(&[60]);
        let cmp = compare(&seq(&a), &seq(&b), TPBAR, TPBEAT);
        let run = cmp.shared_run.expect("run");
        assert_eq!(run.len, 8);
        assert_eq!(run.a_note_index, 4);
        assert_eq!(run.b_note_index, 2);
        assert_eq!(run.a_start, "1.3.0");
        assert_eq!(run.b_start, "1.2.0");
    }

    #[test]
    fn two_different_one_note_ostinatos_never_flag() {
        let a = seq(&[36u8; 16]);
        let b = seq(&[43u8; 16]);
        let cmp = compare(&a, &b, TPBAR, TPBEAT);
        assert!(cmp.distance < 0.1, "pedals are token-identical");
        assert!(cmp.low_information);
        let findings = guard(&cmp, &a.stats(TPBAR, TPBEAT), &b.stats(TPBAR, TPBEAT));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn guard_defect_on_long_run_warn_on_medium() {
        let mut a: Vec<u8> = vec![30, 32, 31, 33];
        a.extend_from_slice(&QUOTE);
        a.extend_from_slice(&[40, 42]);
        let mut b: Vec<u8> = vec![70, 65];
        b.extend_from_slice(&QUOTE);
        b.extend_from_slice(&[60]);
        let (la, lb) = (seq(&a), seq(&b));
        let cmp = compare(&la, &lb, TPBAR, TPBEAT);
        let findings = guard(&cmp, &la.stats(TPBAR, TPBEAT), &lb.stats(TPBAR, TPBEAT));
        assert!(
            findings
                .iter()
                .any(|f| f.level == FlagLevel::Defect && f.rule == "shared-run"),
            "{findings:?}"
        );

        // Same construction with a 7-note quote (6 intervals): warning only.
        let mut a: Vec<u8> = vec![30, 32, 31];
        a.extend_from_slice(&QUOTE[..7]);
        a.extend_from_slice(&[40, 42]);
        let mut b: Vec<u8> = vec![70, 65];
        b.extend_from_slice(&QUOTE[..7]);
        b.extend_from_slice(&[60]);
        let (la, lb) = (seq(&a), seq(&b));
        let cmp = compare(&la, &lb, TPBAR, TPBEAT);
        let findings = guard(&cmp, &la.stats(TPBAR, TPBEAT), &lb.stats(TPBAR, TPBEAT));
        assert!(
            !findings.iter().any(|f| f.level == FlagLevel::Defect),
            "{findings:?}"
        );
        assert!(
            findings
                .iter()
                .any(|f| f.level == FlagLevel::Warning && f.rule == "shared-run"),
            "{findings:?}"
        );
    }

    #[test]
    fn pedal_run_of_any_length_does_not_defect() {
        let a = seq(&[36u8; 30]);
        let mut b_pitches: Vec<u8> = vec![50, 53, 51, 56, 54];
        b_pitches.extend_from_slice(&[40u8; 21]);
        let b = seq(&b_pitches);
        let cmp = compare(&a, &b, TPBAR, TPBEAT);
        let run = cmp.shared_run.as_ref().expect("run");
        assert!(run.len >= 20, "pedal run len {}", run.len);
        let findings = guard(&cmp, &a.stats(TPBAR, TPBEAT), &b.stats(TPBAR, TPBEAT));
        assert!(
            !findings.iter().any(|f| f.level == FlagLevel::Defect),
            "{findings:?}"
        );
    }

    #[test]
    fn no_grams_means_distance_one() {
        let a = seq(&[36, 43]);
        let b = seq(&[40, 45]);
        let cmp = compare(&a, &b, TPBAR, TPBEAT);
        assert_eq!(cmp.distance, 1.0);
        assert!(cmp.low_information);
        assert!(cmp.per_n.is_empty());
    }

    #[test]
    fn drum_stats_per_class_and_grid() {
        let t = track(vec![clip(vec![
            note(36, "1.1.0", "0.0.240"),
            note(36, "1.3.0", "0.0.240"),
            note(38, "1.2.0", "0.0.240"),
            note(38, "1.4.0", "0.0.240"),
            note(42, "1.1.480", "0.0.240"),
            note(42, "1.2.480", "0.0.240"),
        ])]);
        let s = drum_stats_from_track(&t, TPBAR, TPBEAT).expect("stats");
        assert_eq!(s.span_bars, 1);
        let kick = s.classes.iter().find(|c| c.class == "kick").expect("kick");
        assert_eq!(kick.count, 2);
        assert_eq!((kick.grid[0], kick.grid[8]), (1, 1));
        let hits = vec![
            drum_hit("kick", "1.1.0"),
            drum_hit("snare", "1.2.0"),
            drum_hit("hat", "1.1.480"),
        ];
        let s = drum_stats_from_hits("drums", &hits, TPBAR, TPBEAT).expect("stats");
        let hat = s.classes.iter().find(|c| c.class == "hat").expect("hat");
        assert_eq!(hat.grid[2], 1);
    }

    fn drum_hit(class: &str, start: &str) -> DrumHit {
        DrumHit {
            class: class.into(),
            start: start.into(),
            strength: 1.0,
            raw_s: 0.0,
            quant_error_ticks: 0,
        }
    }

    #[test]
    fn novelty_floor_is_min_pairwise() {
        assert_eq!(novelty_floor(&[0.4, 0.2, 0.9]), Some(0.2));
        assert_eq!(novelty_floor(&[]), None);
    }

    #[test]
    fn displays_carry_no_nan_on_empty_line() {
        let empty = line(&[]);
        let stats = format!("{}", empty.stats(TPBAR, TPBEAT));
        assert!(!stats.contains("NaN"), "{stats}");
        let cmp = format!("{}", compare(&empty, &empty, TPBAR, TPBEAT));
        assert!(!cmp.contains("NaN"), "{cmp}");
    }
}
