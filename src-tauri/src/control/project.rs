//! Project model: the serde mirror of `song.json` (the source of truth) and its
//! conversions to a compilable graph description and a playable schedule.
//!
//! Everything here runs off the audio thread (file I/O, allocation, validation).
//! Textual positions are `"bar.beat.tick"`; they are turned into integer sample
//! positions by [`sequencer::ticks_to_samples`] so playback never drifts.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::control::sequencer::{ticks_to_samples, Schedule, ScheduledEvent, PPQ};
use crate::engine::graph::{Connection, GraphDesc, NodeDesc};
use crate::engine::node::{NodeIdx, NoteKind};
use crate::modules::registry;

/// Module id of the implicit master node every project's tracks sum into. The
/// per-track mixer (gain/pan/sends) and buses arrive at M8; here a unity gain
/// just merges the tracks.
const MASTER_MODULE: &str = "gain";

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("cannot read project file: {0}")]
    Io(String),
    #[error("invalid project JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid position '{0}'")]
    BadPosition(String),
    #[error("unsupported time signature {0}/{1}")]
    BadTimeSignature(u32, u32),
    #[error("track '{0}' has an empty chain")]
    EmptyChain(String),
    #[error("unknown module '{0}'")]
    UnknownModule(String),
    #[error("note references unknown track '{0}'")]
    UnknownTrack(String),
}

/// A project, mirroring `song.json`. Unknown fields (and M6/M8 fields such as
/// `mixer`, `buses`, `modulations`, node `preset`) are ignored by serde for now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    #[serde(default = "default_version")]
    #[allow(dead_code)]
    pub version: u32,
    pub bpm: u32,
    #[serde(default = "default_time_signature")]
    pub time_signature: [u32; 2],
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub chain: Vec<NodeSpec>,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: String,
    pub module: String,
    /// Initial parameter values, by parameter name, normalized 0..1. A `BTreeMap`
    /// (not `HashMap`) so a saved project serialises with stable key order — the
    /// on-disk JSON stays diffable.
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    /// Stable identifier, addressed by the UI for edits. Empty in a hand-written
    /// file; [`crate::control::ids::normalize`] assigns one (and rewrites the file).
    #[serde(default)]
    pub id: String,
    /// Clip placement and length. In M3 clips are containers only: notes carry
    /// absolute positions and are not windowed by the clip yet.
    pub start: String,
    pub length: String,
    #[serde(default)]
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    /// Stable identifier, addressed by the UI for edits. Empty in a hand-written
    /// file; [`crate::control::ids::normalize`] assigns one (and rewrites the file).
    #[serde(default)]
    pub id: String,
    pub pitch: u8,
    /// Absolute song position `"bar.beat.tick"` (1-indexed bar/beat).
    pub start: String,
    /// Duration `"bar.beat.tick"` (0-indexed).
    pub length: String,
    #[serde(default = "default_vel")]
    pub vel: u8,
}

fn default_version() -> u32 {
    1
}
fn default_time_signature() -> [u32; 2] {
    [4, 4]
}
fn default_vel() -> u8 {
    100
}

/// Maps a track id to the flat index of the node its notes feed (the first node
/// of its chain). Computed by [`Project::to_graph`] and kept by the control side
/// so a later `load_project` can resolve notes against the running graph.
#[derive(Debug, Clone, Default)]
pub struct GraphIndex {
    pub instrument: HashMap<String, NodeIdx>,
}

/// Render-friendly snapshot of the project for the UI: positions resolved to
/// **ticks** (so the frontend never reparses `"bar.beat.tick"`). Distinct from
/// the on-disk shape so the render format can evolve independently.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub bpm: u32,
    pub ppq: u64,
    pub time_signature: [u32; 2],
    pub tracks: Vec<TrackView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackView {
    pub id: String,
    pub name: String,
    /// Flat graph index of the track's instrument node. Instruments occupy the
    /// leading indices (`to_graph` is instruments-first), so this equals the
    /// track's position; the UI uses it to play an audio echo while editing.
    pub instrument_node: NodeIdx,
    pub clips: Vec<ClipView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipView {
    pub id: String,
    pub start_ticks: u64,
    pub length_ticks: u64,
    pub notes: Vec<NoteView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteView {
    pub id: String,
    pub pitch: u8,
    /// Absolute start in ticks from the song start.
    pub start_ticks: u64,
    pub length_ticks: u64,
    pub vel: u8,
}

impl Project {
    /// Read and parse a `song.json` from disk. // off-RT
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let text = std::fs::read_to_string(path).map_err(|e| ProjectError::Io(e.to_string()))?;
        let project: Project = serde_json::from_str(&text)?;
        Ok(project)
    }

    /// Write the project back to disk as pretty JSON (the source of truth stays
    /// text and diffable; key order is stable). A trailing newline keeps git and
    /// most editors happy. // off-RT
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ProjectError> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        std::fs::write(path, text).map_err(|e| ProjectError::Io(e.to_string()))
    }

    pub fn ticks_per_bar_beat(&self) -> Result<(u64, u64), ProjectError> {
        Ok((self.ticks_per_bar()?, self.ticks_per_beat()?))
    }

    fn ticks_per_beat(&self) -> Result<u64, ProjectError> {
        let den = self.time_signature[1] as u64;
        // A beat is one `1/den` note; PPQ ticks span a quarter note.
        let quarter_ticks = PPQ * 4;
        if den == 0 || !quarter_ticks.is_multiple_of(den) {
            return Err(ProjectError::BadTimeSignature(
                self.time_signature[0],
                self.time_signature[1],
            ));
        }
        Ok(quarter_ticks / den)
    }

    fn ticks_per_bar(&self) -> Result<u64, ProjectError> {
        Ok(self.time_signature[0] as u64 * self.ticks_per_beat()?)
    }

    /// Build a graph description from the track chains, summing every chain into
    /// a unity master node. Returns the graph and the note-routing index.
    ///
    /// Layout is **instruments first** (indices `0..T-1`), then every track's
    /// effects, then the master. Keeping instrument indices independent of the
    /// effect chains means a live effect edit never shifts an instrument index,
    /// so the playing schedule and held notes stay valid across a hot-swap.
    pub fn to_graph(&self) -> Result<(GraphDesc, GraphIndex), ProjectError> {
        for track in &self.tracks {
            if track.chain.is_empty() {
                return Err(ProjectError::EmptyChain(track.id.clone()));
            }
        }

        let mut nodes: Vec<NodeDesc> = Vec::new();
        let mut connections: Vec<Connection> = Vec::new();
        let mut index = GraphIndex::default();

        // Pass 1: every track's instrument (chain head) → stable indices 0..T-1.
        let mut tails: Vec<NodeIdx> = Vec::with_capacity(self.tracks.len());
        for track in &self.tracks {
            let head = &track.chain[0];
            let idx = nodes.len();
            nodes.push(self.node_desc(head)?);
            index.instrument.insert(track.id.clone(), idx);
            tails.push(idx);
        }

        // Pass 2: each track's effects, chained from its instrument.
        for (t, track) in self.tracks.iter().enumerate() {
            for spec in &track.chain[1..] {
                let idx = nodes.len();
                nodes.push(self.node_desc(spec)?);
                connections.push(Connection {
                    src_node: tails[t],
                    src_port: 0,
                    dst_node: idx,
                    dst_port: 0,
                });
                tails[t] = idx;
            }
        }

        // The master node: every track's chain tail sums into its single input.
        let master = nodes.len();
        nodes.push(NodeDesc {
            id: "_master".into(),
            module_id: MASTER_MODULE.into(),
            params: Vec::new(),
        });
        for &tail in &tails {
            connections.push(Connection {
                src_node: tail,
                src_port: 0,
                dst_node: master,
                dst_port: 0,
            });
        }

        Ok((
            GraphDesc {
                nodes,
                connections,
                output: master,
            },
            index,
        ))
    }

    /// Render-friendly view with every position resolved to ticks. // off-RT
    pub fn view(&self) -> Result<ProjectView, ProjectError> {
        let tpbar = self.ticks_per_bar()?;
        let tpbeat = self.ticks_per_beat()?;
        let mut tracks = Vec::with_capacity(self.tracks.len());
        for (track_idx, track) in self.tracks.iter().enumerate() {
            let mut clips = Vec::with_capacity(track.clips.len());
            for clip in &track.clips {
                let mut notes = Vec::with_capacity(clip.notes.len());
                for note in &clip.notes {
                    notes.push(NoteView {
                        id: note.id.clone(),
                        pitch: note.pitch,
                        start_ticks: position_to_ticks(&note.start, tpbar, tpbeat)?,
                        length_ticks: duration_to_ticks(&note.length, tpbar, tpbeat)?,
                        vel: note.vel,
                    });
                }
                clips.push(ClipView {
                    id: clip.id.clone(),
                    start_ticks: position_to_ticks(&clip.start, tpbar, tpbeat)?,
                    length_ticks: duration_to_ticks(&clip.length, tpbar, tpbeat)?,
                    notes,
                });
            }
            tracks.push(TrackView {
                id: track.id.clone(),
                name: track.name.clone(),
                // Instruments-first layout: the instrument's flat index is the
                // track's position (see `to_graph`).
                instrument_node: track_idx,
                clips,
            });
        }
        Ok(ProjectView {
            bpm: self.bpm,
            ppq: PPQ,
            time_signature: self.time_signature,
            tracks,
        })
    }

    /// Build a `NodeDesc` from a chain node spec (validates module + params).
    fn node_desc(&self, spec: &NodeSpec) -> Result<NodeDesc, ProjectError> {
        Ok(NodeDesc {
            id: spec.id.clone(),
            module_id: spec.module.clone(),
            params: resolve_params(&spec.module, &spec.params)?,
        })
    }

    /// Build the playable schedule, resolving each note to its track's
    /// instrument node through `index` (so it works against a graph that is
    /// already running). // off-RT
    pub fn to_schedule(
        &self,
        sample_rate: u32,
        index: &GraphIndex,
    ) -> Result<Schedule, ProjectError> {
        let tpbar = self.ticks_per_bar()?;
        let tpbeat = self.ticks_per_beat()?;
        let mut events: Vec<ScheduledEvent> = Vec::new();
        let mut max_end_ticks = 0u64;

        for track in &self.tracks {
            let node = *index
                .instrument
                .get(&track.id)
                .ok_or_else(|| ProjectError::UnknownTrack(track.id.clone()))?;
            for clip in &track.clips {
                for note in &clip.notes {
                    let on = position_to_ticks(&note.start, tpbar, tpbeat)?;
                    let len = duration_to_ticks(&note.length, tpbar, tpbeat)?;
                    let off = on + len;
                    max_end_ticks = max_end_ticks.max(off);
                    events.push(ScheduledEvent {
                        pos: ticks_to_samples(on, self.bpm, sample_rate),
                        node,
                        kind: NoteKind::On {
                            pitch: note.pitch,
                            vel: note.vel,
                        },
                    });
                    events.push(ScheduledEvent {
                        pos: ticks_to_samples(off, self.bpm, sample_rate),
                        node,
                        kind: NoteKind::Off { pitch: note.pitch },
                    });
                }
            }
        }

        // Loop length: round the content end up to a whole bar (at least one).
        let loop_len = if max_end_ticks == 0 {
            0
        } else {
            let bars = max_end_ticks.div_ceil(tpbar).max(1);
            ticks_to_samples(bars * tpbar, self.bpm, sample_rate)
        };

        // The loop window is half-open `[0, loop_len)`, so an event landing
        // exactly on `loop_len` (a note ending on the final bar line) would never
        // fire and the note would ring across loops. Clamp every event just
        // inside the window — one sample early at most, inaudible.
        if loop_len > 0 {
            let last = loop_len - 1;
            for e in &mut events {
                e.pos = e.pos.min(last);
            }
        }

        // Sort by sample position; at a tie, NoteOff before NoteOn so a
        // re-triggered pitch releases the old voice before the new one starts.
        events.sort_by_key(|e| (e.pos, matches!(e.kind, NoteKind::On { .. }) as u8));

        Ok(Schedule { events, loop_len })
    }
}

/// Map a node's `name → value` parameter table to the `(ParamId, value)` pairs a
/// [`NodeDesc`] expects, looking ids up from the module's declared params.
/// Unknown parameter names are ignored. Validates the module exists.
fn resolve_params(
    module: &str,
    params: &BTreeMap<String, f32>,
) -> Result<Vec<(crate::engine::node::ParamId, f32)>, ProjectError> {
    let node =
        registry::create(module).ok_or_else(|| ProjectError::UnknownModule(module.to_string()))?;
    let specs = node.params();
    let mut out = Vec::new();
    for (name, &value) in params {
        if let Some(spec) = specs.iter().find(|s| s.name == name) {
            out.push((spec.id, value));
        }
    }
    Ok(out)
}

/// Parse `"a.b.c"` into three unsigned integers.
fn parse_triple(s: &str) -> Result<(u64, u64, u64), ProjectError> {
    let mut it = s.split('.');
    let parse = |part: Option<&str>| -> Option<u64> { part?.trim().parse().ok() };
    match (
        parse(it.next()),
        parse(it.next()),
        parse(it.next()),
        it.next(),
    ) {
        (Some(a), Some(b), Some(c), None) => Ok((a, b, c)),
        _ => Err(ProjectError::BadPosition(s.to_string())),
    }
}

/// Absolute position `"bar.beat.tick"` → ticks. Bar and beat are 1-indexed.
pub fn position_to_ticks(s: &str, tpbar: u64, tpbeat: u64) -> Result<u64, ProjectError> {
    let (bar, beat, tick) = parse_triple(s)?;
    if bar < 1 || beat < 1 {
        return Err(ProjectError::BadPosition(s.to_string()));
    }
    Ok((bar - 1) * tpbar + (beat - 1) * tpbeat + tick)
}

/// Duration `"bar.beat.tick"` → ticks. All components are 0-indexed counts.
pub fn duration_to_ticks(s: &str, tpbar: u64, tpbeat: u64) -> Result<u64, ProjectError> {
    let (bar, beat, tick) = parse_triple(s)?;
    Ok(bar * tpbar + beat * tpbeat + tick)
}

/// Ticks → absolute position `"bar.beat.tick"` (1-indexed bar/beat). Inverse of
/// [`position_to_ticks`].
pub fn ticks_to_position(ticks: u64, tpbar: u64, tpbeat: u64) -> String {
    let bar = ticks / tpbar;
    let rem = ticks % tpbar;
    let beat = rem / tpbeat;
    let tick = rem % tpbeat;
    format!("{}.{}.{}", bar + 1, beat + 1, tick)
}

/// Ticks → duration `"bar.beat.tick"` (0-indexed counts). Inverse of
/// [`duration_to_ticks`].
pub fn ticks_to_duration(ticks: u64, tpbar: u64, tpbeat: u64) -> String {
    let bar = ticks / tpbar;
    let rem = ticks % tpbar;
    let beat = rem / tpbeat;
    let tick = rem % tpbeat;
    format!("{}.{}.{}", bar, beat, tick)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = r#"{
        "version": 1,
        "bpm": 120,
        "time_signature": [4, 4],
        "tracks": [
            { "id": "lead", "chain": [ { "id": "n1", "module": "subtractive" } ],
              "clips": [ { "start": "1.1.0", "length": "1.0.0",
                "notes": [ { "pitch": 60, "start": "1.1.0", "length": "0.1.0", "vel": 100 } ] } ] }
        ]
    }"#;

    #[test]
    fn parses_positions_and_durations() {
        // 4/4: 3840 ticks/bar, 960 ticks/beat.
        assert_eq!(position_to_ticks("1.1.0", 3840, 960).unwrap(), 0);
        assert_eq!(position_to_ticks("2.1.0", 3840, 960).unwrap(), 3840);
        assert_eq!(
            position_to_ticks("1.3.480", 3840, 960).unwrap(),
            2 * 960 + 480
        );
        assert_eq!(duration_to_ticks("0.1.0", 3840, 960).unwrap(), 960);
        assert_eq!(duration_to_ticks("1.0.0", 3840, 960).unwrap(), 3840);
    }

    #[test]
    fn rejects_malformed_positions() {
        assert!(parse_triple("1.1").is_err());
        assert!(parse_triple("1.1.0.0").is_err());
        assert!(position_to_ticks("0.1.0", 3840, 960).is_err()); // bar 0 invalid
    }

    #[test]
    fn builds_graph_with_master_and_index() {
        let project: Project = serde_json::from_str(DEMO).unwrap();
        let (graph, index) = project.to_graph().unwrap();
        // One instrument + the master node.
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.output, 1);
        assert_eq!(index.instrument["lead"], 0);
        // The instrument feeds the master.
        assert_eq!(graph.connections.len(), 1);
        assert_eq!(graph.connections[0].dst_node, 1);
    }

    #[test]
    fn instruments_come_first_and_ids_propagate() {
        // Two tracks; the first carries an effect. Instruments must take the
        // leading indices (0, 1) regardless of effects, and node ids propagate.
        let song = r#"{ "bpm": 120, "tracks": [
            { "id": "lead", "chain": [
                { "id": "lead_synth", "module": "subtractive" },
                { "id": "lead_gain", "module": "gain" } ] },
            { "id": "bass", "chain": [ { "id": "bass_synth", "module": "subtractive" } ] }
        ] }"#;
        let project: Project = serde_json::from_str(song).unwrap();
        let (graph, index) = project.to_graph().unwrap();

        assert_eq!(index.instrument["lead"], 0);
        assert_eq!(index.instrument["bass"], 1);
        assert_eq!(graph.nodes[0].id, "lead_synth");
        assert_eq!(graph.nodes[1].id, "bass_synth");
        // Effect after the instruments, master last.
        assert_eq!(graph.nodes[2].id, "lead_gain");
        assert_eq!(graph.nodes[3].id, "_master");
        assert_eq!(graph.output, 3);
        // The layout still compiles to a valid RT graph.
        crate::engine::graph::compile(&graph, crate::modules::registry::create, 48_000.0, 64)
            .unwrap();
    }

    #[test]
    fn schedule_resolves_notes_and_loops_one_bar() {
        let project: Project = serde_json::from_str(DEMO).unwrap();
        let (_graph, index) = project.to_graph().unwrap();
        let schedule = project.to_schedule(48_000, &index).unwrap();
        // One note → an On at 0 and an Off one beat (24 000 samples) later.
        assert_eq!(schedule.events.len(), 2);
        assert_eq!(schedule.events[0].pos, 0);
        assert!(matches!(
            schedule.events[0].kind,
            NoteKind::On { pitch: 60, .. }
        ));
        assert_eq!(schedule.events[1].pos, 24_000);
        // Content ends within bar 1 → loop is one 4/4 bar = 96 000 samples.
        assert_eq!(schedule.loop_len, 96_000);
    }

    #[test]
    fn note_ending_on_the_bar_line_stays_inside_the_loop() {
        // A note spanning the whole bar ends exactly on `loop_len`; the half-open
        // loop window would drop that NoteOff and the note would ring across
        // loops. It must be clamped strictly inside the loop.
        let song = r#"{ "bpm": 120, "tracks": [
            { "id": "t", "chain": [ { "id": "n", "module": "subtractive" } ],
              "clips": [ { "start": "1.1.0", "length": "1.0.0",
                "notes": [ { "pitch": 60, "start": "1.1.0", "length": "1.0.0" } ] } ] } ] }"#;
        let project: Project = serde_json::from_str(song).unwrap();
        let (_graph, index) = project.to_graph().unwrap();
        let schedule = project.to_schedule(48_000, &index).unwrap();
        assert!(schedule.loop_len > 0);
        assert!(
            schedule.events.iter().all(|e| e.pos < schedule.loop_len),
            "an event sits on or past the loop boundary"
        );
    }

    #[test]
    fn view_resolves_positions_to_ticks() {
        let project: Project = serde_json::from_str(DEMO).unwrap();
        let view = project.view().unwrap();
        assert_eq!(view.bpm, 120);
        assert_eq!(view.ppq, 960);
        let note = &view.tracks[0].clips[0].notes[0];
        // "1.1.0" → tick 0 ; length "0.1.0" → one beat = 960 ticks.
        assert_eq!(note.start_ticks, 0);
        assert_eq!(note.length_ticks, 960);
        assert_eq!(note.pitch, 60);
    }

    #[test]
    fn position_ticks_round_trip() {
        // 4/4: 3840 ticks/bar, 960 ticks/beat. Encoders invert the parsers.
        for &(s, dur) in &[("1.1.0", false), ("2.3.480", false), ("3.0.240", true)] {
            let ticks = if dur {
                duration_to_ticks(s, 3840, 960).unwrap()
            } else {
                position_to_ticks(s, 3840, 960).unwrap()
            };
            let back = if dur {
                ticks_to_duration(ticks, 3840, 960)
            } else {
                ticks_to_position(ticks, 3840, 960)
            };
            assert_eq!(back, s, "round trip for {s} (dur={dur})");
        }
    }

    #[test]
    fn serialize_round_trips() {
        let project: Project = serde_json::from_str(DEMO).unwrap();
        let text = serde_json::to_string_pretty(&project).unwrap();
        let back: Project = serde_json::from_str(&text).unwrap();
        // The graph and schedule derived from the re-parsed project are identical.
        assert_eq!(
            project.to_graph().unwrap().0,
            back.to_graph().unwrap().0,
            "serialize/deserialize changed the graph"
        );
        let idx = project.to_graph().unwrap().1;
        assert_eq!(
            project.to_schedule(48_000, &idx).unwrap(),
            back.to_schedule(48_000, &idx).unwrap(),
            "serialize/deserialize changed the schedule"
        );
    }

    #[test]
    fn rejects_unknown_module() {
        let bad = r#"{ "bpm": 120, "tracks": [
            { "id": "x", "chain": [ { "id": "n1", "module": "nope" } ] } ] }"#;
        let project: Project = serde_json::from_str(bad).unwrap();
        assert!(matches!(
            project.to_graph(),
            Err(ProjectError::UnknownModule(_))
        ));
    }
}
