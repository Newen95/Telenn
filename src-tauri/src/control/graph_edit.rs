//! Pure edit operations on a [`Project`]'s tracks and effect chains.
//!
//! Each function mutates a project in place after validating the edit. The
//! control side applies these to a trial clone, recompiles, and hot-swaps; the
//! audio thread is never touched here. Within a chain the instrument (head) is
//! never moved or removed like an effect — it changes only through the track
//! operations ([`add_track`], [`set_instrument`], [`remove_track`]), which also
//! rebuild the schedule since they can shift instrument node indices.

use std::collections::HashMap;

use crate::control::ids;
use crate::control::project::{
    duration_to_ticks, position_to_ticks, ticks_to_duration, ticks_to_position, Clip, NodeSpec,
    Project, ProjectError, Track,
};
use crate::modules::registry;

/// Reserved id addressing the master bus: passed as a `track_id` it targets
/// `Project::master_chain` (effects only, no instrument head); it can never be
/// a node or track id.
pub const MASTER_ID: &str = "_master";

/// Which chain a node lives in: a track's, or the master bus's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainRef {
    Track(usize),
    Master,
}

impl ChainRef {
    /// First slot effects may occupy: tracks reserve index 0 for the
    /// instrument head; the master chain has no head.
    fn effects_floor(self) -> usize {
        match self {
            ChainRef::Track(_) => 1,
            ChainRef::Master => 0,
        }
    }
}

/// Resolve a `track_id` argument to a chain reference (`_master` = master bus).
fn chain_ref(project: &Project, track_id: &str) -> Result<ChainRef, GraphEditError> {
    if track_id == MASTER_ID {
        return Ok(ChainRef::Master);
    }
    Ok(ChainRef::Track(track_index(project, track_id)?))
}

fn chain_mut(project: &mut Project, r: ChainRef) -> &mut Vec<NodeSpec> {
    match r {
        ChainRef::Track(i) => &mut project.tracks[i].chain,
        ChainRef::Master => &mut project.master_chain,
    }
}

fn chain_of(project: &Project, r: ChainRef) -> &Vec<NodeSpec> {
    match r {
        ChainRef::Track(i) => &project.tracks[i].chain,
        ChainRef::Master => &project.master_chain,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GraphEditError {
    #[error("unknown track '{0}'")]
    UnknownTrack(String),
    #[error("unknown module '{0}'")]
    UnknownModule(String),
    #[error("node id '{0}' already exists")]
    DuplicateId(String),
    #[error("id '{0}' is reserved")]
    ReservedId(String),
    #[error("no node '{0}'")]
    NodeNotFound(String),
    #[error("'{0}' is a track instrument and cannot be moved or removed")]
    CannotEditInstrument(String),
    #[error("'{0}' is not an instrument module")]
    NotAnInstrument(String),
    #[error("'{0}' is not an effect module")]
    NotAnEffect(String),
    #[error("a chain needs at least an instrument")]
    EmptyChain,
    #[error("track id '{0}' already exists")]
    DuplicateTrack(String),
    #[error(transparent)]
    Project(#[from] ProjectError),
}

/// Validate that `module` is a registered instrument (a chain head).
fn require_instrument(module: &str) -> Result<(), GraphEditError> {
    let desc = registry::list()
        .iter()
        .find(|m| m.id == module)
        .ok_or_else(|| GraphEditError::UnknownModule(module.to_string()))?;
    if desc.category != registry::ModuleCategory::Instrument {
        return Err(GraphEditError::NotAnInstrument(module.to_string()));
    }
    Ok(())
}

/// Validate that `module` is a registered effect (a chain tail node).
fn require_effect(module: &str) -> Result<(), GraphEditError> {
    let desc = registry::list()
        .iter()
        .find(|m| m.id == module)
        .ok_or_else(|| GraphEditError::UnknownModule(module.to_string()))?;
    if desc.category != registry::ModuleCategory::Effect {
        return Err(GraphEditError::NotAnEffect(module.to_string()));
    }
    Ok(())
}

/// Append a new track whose chain head is a fresh instance of `module`. The
/// caller provides the ids (the UI generates them, like effect inserts); both
/// must be new — ids are never recycled.
pub fn add_track(
    project: &mut Project,
    track_id: &str,
    name: &str,
    module: &str,
    instrument_id: &str,
    color: Option<&str>,
) -> Result<(), GraphEditError> {
    if instrument_id == MASTER_ID || track_id == MASTER_ID {
        return Err(GraphEditError::ReservedId(track_id.to_string()));
    }
    require_instrument(module)?;
    if project.tracks.iter().any(|t| t.id == track_id) {
        return Err(GraphEditError::DuplicateTrack(track_id.to_string()));
    }
    if id_exists(project, instrument_id) {
        return Err(GraphEditError::DuplicateId(instrument_id.to_string()));
    }
    // A new track ships with one empty clip spanning the current loop: the
    // piano roll needs a clip to put notes in, and the timeline band IS the
    // clip. Matching the existing loop keeps the project length unchanged
    // (empty project → 16 bars).
    let (tpbar, tpbeat) = project.ticks_per_bar_beat()?;
    let mut extent = 0u64;
    for track in &project.tracks {
        for clip in &track.clips {
            let end = position_to_ticks(&clip.start, tpbar, tpbeat)?
                .saturating_add(duration_to_ticks(&clip.length, tpbar, tpbeat)?);
            extent = extent.max(end);
        }
    }
    if extent == 0 {
        extent = 16 * tpbar;
    }
    let clip = Clip {
        id: ids::fresh_clip_id(project),
        start: ticks_to_position(0, tpbar, tpbeat),
        length: ticks_to_duration(extent, tpbar, tpbeat),
        notes: Vec::new(),
    };
    project.tracks.push(Track {
        id: track_id.to_string(),
        name: name.to_string(),
        color: color.map(str::to_string),
        mixer: Default::default(),
        chain: vec![NodeSpec {
            id: instrument_id.to_string(),
            module: module.to_string(),
            params: Default::default(),
            config: Default::default(),
        }],
        clips: vec![clip],
    });
    Ok(())
}

/// Append a new track whose whole chain comes from a chain preset: `nodes[0]`
/// is the instrument (with params), the rest are effects in order. The caller
/// provides fresh ids for every node (the UI generates them, like effect
/// inserts) — one atomic edit, so a single undo removes the whole track.
pub fn add_track_with_chain(
    project: &mut Project,
    track_id: &str,
    name: &str,
    color: Option<&str>,
    nodes: &[NodeSpec],
) -> Result<(), GraphEditError> {
    let (head, effects) = nodes.split_first().ok_or(GraphEditError::EmptyChain)?;
    add_track(project, track_id, name, &head.module, &head.id, color)?;
    set_chain_tail(project, track_id, head, effects)
}

/// Replace a track's instrument (chain head) with a fresh instance of `module`
/// under a **new** id: the old instrument's voices ring out through the swap's
/// fading graph and its id is never reused. Effects, clips and mixer settings
/// stay — the new instrument simply plays the existing notes.
pub fn set_instrument(
    project: &mut Project,
    track_id: &str,
    module: &str,
    node_id: &str,
) -> Result<(), GraphEditError> {
    if node_id == MASTER_ID {
        return Err(GraphEditError::ReservedId(node_id.to_string()));
    }
    require_instrument(module)?;
    if id_exists(project, node_id) {
        return Err(GraphEditError::DuplicateId(node_id.to_string()));
    }
    let track = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .ok_or_else(|| GraphEditError::UnknownTrack(track_id.to_string()))?;
    track.chain[0] = NodeSpec {
        id: node_id.to_string(),
        module: module.to_string(),
        params: Default::default(),
        config: Default::default(),
    };
    Ok(())
}

/// Replace a track's whole chain (head + effects) from a chain preset, keeping
/// its clips, mixer, name and color. The old nodes are dropped and their ids
/// never reused; every incoming node needs a fresh id from the caller.
pub fn set_chain(
    project: &mut Project,
    track_id: &str,
    nodes: &[NodeSpec],
) -> Result<(), GraphEditError> {
    let (head, effects) = nodes.split_first().ok_or(GraphEditError::EmptyChain)?;
    set_instrument(project, track_id, &head.module, &head.id)?;
    let idx = track_index(project, track_id)?;
    project.tracks[idx].chain.truncate(1); // drop the old effects
    set_chain_tail(project, track_id, head, effects)
}

/// Shared tail of the chain-preset edits: pose the head's params (the track
/// edits ship an empty map) and append the preset's effects in order.
fn set_chain_tail(
    project: &mut Project,
    track_id: &str,
    head: &NodeSpec,
    effects: &[NodeSpec],
) -> Result<(), GraphEditError> {
    for fx in effects {
        // A hand-edited preset could smuggle an instrument into the tail;
        // `insert_effect` alone would accept any registered module.
        require_effect(&fx.module)?;
        let params: HashMap<String, f32> = fx.params.iter().map(|(k, v)| (k.clone(), *v)).collect();
        insert_effect(project, track_id, &fx.module, &fx.id, params)?;
    }
    let idx = track_index(project, track_id)?;
    project.tracks[idx].chain[0].params = head.params.clone();
    Ok(())
}

/// Remove a track — instrument, effects, clips and mixer strip. Later tracks'
/// instrument indices shift, so the caller must swap graph **and** schedule.
pub fn remove_track(project: &mut Project, track_id: &str) -> Result<(), GraphEditError> {
    let idx = track_index(project, track_id)?;
    project.tracks.remove(idx);
    Ok(())
}

/// Move a track to `index` in the project order (clamped to the end). Track
/// order is the user's arrangement — it drives the timeline lanes, the mixer
/// strips AND the instrument node indices, so the caller must swap graph and
/// schedule together.
pub fn move_track(
    project: &mut Project,
    track_id: &str,
    index: usize,
) -> Result<(), GraphEditError> {
    let from = track_index(project, track_id)?;
    let track = project.tracks.remove(from);
    let to = index.min(project.tracks.len());
    project.tracks.insert(to, track);
    Ok(())
}

/// Append a new effect (with initial normalized params) to the end of a track's
/// chain.
pub fn insert_effect(
    project: &mut Project,
    track_id: &str,
    module: &str,
    node_id: &str,
    params: HashMap<String, f32>,
) -> Result<(), GraphEditError> {
    insert_effect_at(project, track_id, module, node_id, params, usize::MAX)
}

/// Insert a new effect at `index` in a track's chain — or the master bus's
/// when `track_id` is [`MASTER_ID`]. `index` is clamped to the effects region
/// (`[1, len]` on a track: never the instrument slot; `[0, len]` on the
/// master, which has no head). `usize::MAX` means append. Master inserts are
/// strictly effects; the graph compiler enforces it again.
pub fn insert_effect_at(
    project: &mut Project,
    track_id: &str,
    module: &str,
    node_id: &str,
    params: HashMap<String, f32>,
    index: usize,
) -> Result<(), GraphEditError> {
    if node_id == MASTER_ID {
        return Err(GraphEditError::ReservedId(node_id.to_string()));
    }
    if registry::create(module).is_none() {
        return Err(GraphEditError::UnknownModule(module.to_string()));
    }
    if id_exists(project, node_id) {
        return Err(GraphEditError::DuplicateId(node_id.to_string()));
    }
    let dst = chain_ref(project, track_id)?;
    if dst == ChainRef::Master {
        require_effect(module)?;
    }
    let floor = dst.effects_floor();
    let chain = chain_mut(project, dst);
    let pos = index.clamp(floor, chain.len());
    chain.insert(
        pos,
        NodeSpec {
            id: node_id.to_string(),
            module: module.to_string(),
            params: params.into_iter().collect(),
            config: Default::default(),
        },
    );
    Ok(())
}

/// Remove an effect by id. The instrument (chain head) cannot be removed.
pub fn remove_node(project: &mut Project, node_id: &str) -> Result<(), GraphEditError> {
    let (cref, chain_idx) = locate(project, node_id)?;
    if matches!(cref, ChainRef::Track(_)) && chain_idx == 0 {
        return Err(GraphEditError::CannotEditInstrument(node_id.to_string()));
    }
    chain_mut(project, cref).remove(chain_idx);
    Ok(())
}

/// Move an effect one step earlier (`dir < 0`) or later (`dir > 0`) in its
/// chain. On a track the instrument slot (index 0) is never entered or left;
/// the master chain has no such slot.
pub fn move_node(project: &mut Project, node_id: &str, dir: i32) -> Result<(), GraphEditError> {
    let (cref, chain_idx) = locate(project, node_id)?;
    if matches!(cref, ChainRef::Track(_)) && chain_idx == 0 {
        return Err(GraphEditError::CannotEditInstrument(node_id.to_string()));
    }
    let floor = cref.effects_floor() as i32;
    let chain = chain_mut(project, cref);
    let target = chain_idx as i32 + dir.signum();
    // Stay within the effects region; a no-op at the edges.
    if target >= floor && (target as usize) < chain.len() {
        chain.swap(chain_idx, target as usize);
    }
    Ok(())
}

/// Move an effect to `index` in another (or the same) chain — a track's, or
/// the master bus's when `to_track` is [`MASTER_ID`] — **keeping its id** so
/// the hot-swap adopts the same node instance: its internal state (e.g. a
/// reverb tail) follows the move without a gap. The instrument (index 0 of a
/// track) can never be moved, nor can a node land on a target's instrument
/// slot; only effects may land on the master.
pub fn move_node_to(
    project: &mut Project,
    node_id: &str,
    to_track: &str,
    index: usize,
) -> Result<(), GraphEditError> {
    let (src, src_chain) = locate(project, node_id)?;
    if matches!(src, ChainRef::Track(_)) && src_chain == 0 {
        return Err(GraphEditError::CannotEditInstrument(node_id.to_string()));
    }
    let dst = chain_ref(project, to_track)?;
    if dst == ChainRef::Master {
        require_effect(&chain_of(project, src)[src_chain].module)?;
    }
    let spec = chain_mut(project, src).remove(src_chain);
    // Removing from the same chain before the target shifts later slots left.
    let mut pos = index;
    if src == dst && src_chain < pos {
        pos -= 1;
    }
    let floor = dst.effects_floor();
    let chain = chain_mut(project, dst);
    let pos = pos.clamp(floor, chain.len());
    chain.insert(pos, spec);
    Ok(())
}

/// Copy an effect into a chain at `index` under a **new** id (a fresh node
/// instance with the same params) — `to_track` may be [`MASTER_ID`]. The
/// instrument cannot be copied.
pub fn copy_node(
    project: &mut Project,
    node_id: &str,
    new_id: &str,
    to_track: &str,
    index: usize,
) -> Result<(), GraphEditError> {
    if new_id == MASTER_ID {
        return Err(GraphEditError::ReservedId(new_id.to_string()));
    }
    if id_exists(project, new_id) {
        return Err(GraphEditError::DuplicateId(new_id.to_string()));
    }
    let (src, src_chain) = locate(project, node_id)?;
    if matches!(src, ChainRef::Track(_)) && src_chain == 0 {
        return Err(GraphEditError::CannotEditInstrument(node_id.to_string()));
    }
    let source = &chain_of(project, src)[src_chain];
    let copy = NodeSpec {
        id: new_id.to_string(),
        module: source.module.clone(),
        params: source.params.clone(),
        // A copied node keeps its config: a duplicated sampler keeps its kit.
        config: source.config.clone(),
    };
    let dst = chain_ref(project, to_track)?;
    if dst == ChainRef::Master {
        require_effect(&copy.module)?;
    }
    let floor = dst.effects_floor();
    let chain = chain_mut(project, dst);
    let pos = index.clamp(floor, chain.len());
    chain.insert(pos, copy);
    Ok(())
}

/// Set a normalized parameter value on an effect by id, keyed by param name (the
/// stable, module-declared identifier). Persisted with the project; the live
/// audio value is sent separately so the change is heard while a knob is dragged.
pub fn set_param(
    project: &mut Project,
    node_id: &str,
    param: &str,
    value: f32,
) -> Result<(), GraphEditError> {
    let (cref, chain_idx) = locate(project, node_id)?;
    chain_mut(project, cref)[chain_idx]
        .params
        .insert(param.to_string(), value);
    Ok(())
}

fn track_index(project: &Project, track_id: &str) -> Result<usize, GraphEditError> {
    project
        .tracks
        .iter()
        .position(|t| t.id == track_id)
        .ok_or_else(|| GraphEditError::UnknownTrack(track_id.to_string()))
}

fn id_exists(project: &Project, node_id: &str) -> bool {
    project
        .tracks
        .iter()
        .flat_map(|t| t.chain.iter())
        .chain(project.master_chain.iter())
        .any(|n| n.id == node_id)
}

/// Find the chain (track or master) and slot of a node id.
fn locate(project: &Project, node_id: &str) -> Result<(ChainRef, usize), GraphEditError> {
    for (ti, track) in project.tracks.iter().enumerate() {
        if let Some(ci) = track.chain.iter().position(|n| n.id == node_id) {
            return Ok((ChainRef::Track(ti), ci));
        }
    }
    if let Some(ci) = project.master_chain.iter().position(|n| n.id == node_id) {
        return Ok((ChainRef::Master, ci));
    }
    Err(GraphEditError::NodeNotFound(node_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Project {
        let song = r#"{ "bpm": 120, "tracks": [
            { "id": "lead", "chain": [ { "id": "lead_synth", "module": "subtractive" } ] }
        ] }"#;
        serde_json::from_str(song).unwrap()
    }

    fn two_tracks() -> Project {
        let song = r#"{ "bpm": 120, "tracks": [
            { "id": "lead", "chain": [ { "id": "lead_synth", "module": "subtractive" } ] },
            { "id": "bass", "chain": [ { "id": "bass_synth", "module": "subtractive" } ] }
        ] }"#;
        serde_json::from_str(song).unwrap()
    }

    fn ids(project: &Project, track: usize) -> Vec<String> {
        project.tracks[track]
            .chain
            .iter()
            .map(|n| n.id.clone())
            .collect()
    }

    #[test]
    fn insert_then_remove_round_trips() {
        let mut p = demo();
        let before = p.to_graph().unwrap().0;
        insert_effect(&mut p, "lead", "gain", "lead_fx", Default::default()).unwrap();
        assert_eq!(p.tracks[0].chain.len(), 2);
        // The instrument index is unchanged by the insert.
        assert_eq!(p.to_graph().unwrap().1.instrument["lead"], 0);
        remove_node(&mut p, "lead_fx").unwrap();
        assert_eq!(p.to_graph().unwrap().0, before);
    }

    #[test]
    fn move_reorders_effects_only() {
        let mut p = demo();
        insert_effect(&mut p, "lead", "gain", "a", Default::default()).unwrap();
        insert_effect(&mut p, "lead", "gain", "b", Default::default()).unwrap();
        // chain: [lead_synth, a, b]; move b earlier → [lead_synth, b, a].
        move_node(&mut p, "b", -1).unwrap();
        let ids: Vec<&str> = p.tracks[0].chain.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["lead_synth", "b", "a"]);
    }

    #[test]
    fn instrument_cannot_be_removed_or_moved() {
        let mut p = demo();
        assert!(matches!(
            remove_node(&mut p, "lead_synth"),
            Err(GraphEditError::CannotEditInstrument(_))
        ));
        assert!(matches!(
            move_node(&mut p, "lead_synth", 1),
            Err(GraphEditError::CannotEditInstrument(_))
        ));
    }

    #[test]
    fn insert_effect_at_lands_at_the_index_after_the_instrument() {
        let mut p = demo();
        insert_effect(&mut p, "lead", "gain", "a", Default::default()).unwrap();
        insert_effect(&mut p, "lead", "gain", "b", Default::default()).unwrap();
        // chain: [lead_synth, a, b]; insert c at index 1 (right after instrument).
        insert_effect_at(&mut p, "lead", "gain", "c", Default::default(), 1).unwrap();
        assert_eq!(ids(&p, 0), ["lead_synth", "c", "a", "b"]);
        // index 0 is clamped up to 1: never on the instrument slot.
        insert_effect_at(&mut p, "lead", "gain", "d", Default::default(), 0).unwrap();
        assert_eq!(p.tracks[0].chain[0].id, "lead_synth");
    }

    #[test]
    fn move_node_to_relocates_across_tracks_keeping_its_id() {
        let mut p = two_tracks();
        insert_effect(&mut p, "lead", "gain", "verb", Default::default()).unwrap();
        // Move the effect from lead to bass; its id (and thus its node instance,
        // tail and all) is preserved — only the chain it lives in changes.
        move_node_to(&mut p, "verb", "bass", 1).unwrap();
        assert_eq!(ids(&p, 0), ["lead_synth"]);
        assert_eq!(ids(&p, 1), ["bass_synth", "verb"]);
    }

    #[test]
    fn move_node_to_reorders_within_a_track() {
        let mut p = demo();
        insert_effect(&mut p, "lead", "gain", "a", Default::default()).unwrap();
        insert_effect(&mut p, "lead", "gain", "b", Default::default()).unwrap();
        // [lead_synth, a, b]; move a to the end → [lead_synth, b, a].
        move_node_to(&mut p, "a", "lead", 3).unwrap();
        assert_eq!(ids(&p, 0), ["lead_synth", "b", "a"]);
    }

    #[test]
    fn move_node_to_refuses_the_instrument() {
        let mut p = two_tracks();
        assert!(matches!(
            move_node_to(&mut p, "lead_synth", "bass", 1),
            Err(GraphEditError::CannotEditInstrument(_))
        ));
    }

    #[test]
    fn copy_node_clones_params_under_a_new_id() {
        let mut p = two_tracks();
        let mut params = HashMap::new();
        params.insert("gain".to_string(), 0.3);
        insert_effect(&mut p, "lead", "gain", "verb", params).unwrap();
        copy_node(&mut p, "verb", "verb_copy", "bass", 1).unwrap();
        assert_eq!(ids(&p, 0), ["lead_synth", "verb"]); // original stays
        assert_eq!(ids(&p, 1), ["bass_synth", "verb_copy"]);
        let original = &p.tracks[0].chain[1];
        let copy = &p.tracks[1].chain[1];
        assert_eq!(original.module, copy.module);
        assert_eq!(original.params, copy.params); // same params, different id
    }

    #[test]
    fn copy_node_rejects_a_duplicate_id() {
        let mut p = two_tracks();
        insert_effect(&mut p, "lead", "gain", "verb", Default::default()).unwrap();
        assert!(matches!(
            copy_node(&mut p, "verb", "verb", "bass", 1),
            Err(GraphEditError::DuplicateId(_))
        ));
    }

    #[test]
    fn set_param_writes_value_by_name_under_the_node_id() {
        let mut p = demo();
        insert_effect(&mut p, "lead", "gain", "fx", Default::default()).unwrap();
        set_param(&mut p, "fx", "gain", 0.25).unwrap();
        assert_eq!(p.tracks[0].chain[1].params.get("gain"), Some(&0.25));
        // A second write overwrites (a knob settles on one value).
        set_param(&mut p, "fx", "gain", 0.9).unwrap();
        assert_eq!(p.tracks[0].chain[1].params.get("gain"), Some(&0.9));
        // Unknown node id is rejected.
        assert!(matches!(
            set_param(&mut p, "nope", "gain", 0.5),
            Err(GraphEditError::NodeNotFound(_))
        ));
    }

    #[test]
    fn add_track_appends_with_a_fresh_instrument() {
        let mut p = demo();
        add_track(
            &mut p,
            "drums",
            "Drums",
            "drum_sampler",
            "drums_kit",
            Some("#5b8a6f"),
        )
        .unwrap();
        assert_eq!(p.tracks.len(), 2);
        assert_eq!(ids(&p, 1), ["drums_kit"]);
        // One empty default clip, so notes can land right away. The demo
        // fixture has no clips → the default 16-bar span.
        assert_eq!(p.tracks[1].clips.len(), 1);
        assert!(p.tracks[1].clips[0].notes.is_empty());
        assert!(!p.tracks[1].clips[0].id.is_empty());
        assert_eq!(p.tracks[1].clips[0].length, "16.0.0");
        // Existing instrument indices are unchanged (instruments-first layout).
        let (_, index) = p.to_graph().unwrap();
        assert_eq!(index.instrument["lead"], 0);
        assert_eq!(index.instrument["drums"], 1);
    }

    #[test]
    fn add_track_rejects_effects_and_taken_ids() {
        let mut p = demo();
        assert!(matches!(
            add_track(&mut p, "t2", "T2", "delay", "x", None),
            Err(GraphEditError::NotAnInstrument(_))
        ));
        assert!(matches!(
            add_track(&mut p, "lead", "Lead", "fm", "x", None),
            Err(GraphEditError::DuplicateTrack(_))
        ));
        assert!(matches!(
            add_track(&mut p, "t2", "T2", "fm", "lead_synth", None),
            Err(GraphEditError::DuplicateId(_))
        ));
    }

    #[test]
    fn set_instrument_replaces_the_head_and_keeps_the_rest() {
        let mut p = demo();
        insert_effect(&mut p, "lead", "gain", "fx", Default::default()).unwrap();
        set_instrument(&mut p, "lead", "fm", "lead_fm").unwrap();
        assert_eq!(ids(&p, 0), ["lead_fm", "fx"]);
        assert_eq!(p.tracks[0].chain[0].module, "fm");
        // Fresh instance: default params, never the old instrument's.
        assert!(p.tracks[0].chain[0].params.is_empty());
        // The new id must be new (the old one is never recycled).
        assert!(matches!(
            set_instrument(&mut p, "lead", "fm", "fx"),
            Err(GraphEditError::DuplicateId(_))
        ));
        assert!(matches!(
            set_instrument(&mut p, "lead", "reverb", "z"),
            Err(GraphEditError::NotAnInstrument(_))
        ));
    }

    #[test]
    fn remove_track_drops_it_and_the_project_still_compiles() {
        let mut p = two_tracks();
        remove_node(&mut p, "lead_synth").unwrap_err(); // effects path still refuses heads
        remove_track(&mut p, "lead").unwrap();
        assert_eq!(p.tracks.len(), 1);
        assert_eq!(p.tracks[0].id, "bass");
        let (_, index) = p.to_graph().unwrap();
        assert_eq!(index.instrument["bass"], 0);
        // Removing the last track leaves an empty, still-valid project.
        remove_track(&mut p, "bass").unwrap();
        assert!(p.to_graph().is_ok());
        assert!(matches!(
            remove_track(&mut p, "bass"),
            Err(GraphEditError::UnknownTrack(_))
        ));
    }

    #[test]
    fn move_track_reorders_and_the_graph_indices_follow() {
        let mut p = two_tracks();
        move_track(&mut p, "bass", 0).unwrap();
        let order: Vec<&str> = p.tracks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(order, ["bass", "lead"]);
        // Instruments-first layout: indices follow the new track order.
        let (_, index) = p.to_graph().unwrap();
        assert_eq!(index.instrument["bass"], 0);
        assert_eq!(index.instrument["lead"], 1);
        // An out-of-range target clamps to the end; unknown tracks are refused.
        move_track(&mut p, "bass", 99).unwrap();
        assert_eq!(p.tracks[1].id, "bass");
        assert!(matches!(
            move_track(&mut p, "nope", 0),
            Err(GraphEditError::UnknownTrack(_))
        ));
    }

    #[test]
    fn rejects_unknown_module_and_duplicate_id() {
        let mut p = demo();
        assert!(matches!(
            insert_effect(&mut p, "lead", "nope", "x", Default::default()),
            Err(GraphEditError::UnknownModule(_))
        ));
        assert!(matches!(
            insert_effect(&mut p, "lead", "gain", "lead_synth", Default::default()),
            Err(GraphEditError::DuplicateId(_))
        ));
    }

    #[test]
    fn master_chain_accepts_effect_edits() {
        let mut p = demo();
        // Insert lands at [0, len] (no instrument head on the master).
        insert_effect_at(
            &mut p,
            MASTER_ID,
            "compressor",
            "m_comp",
            Default::default(),
            0,
        )
        .unwrap();
        insert_effect(&mut p, MASTER_ID, "gain", "m_gain", Default::default()).unwrap();
        let ids: Vec<&str> = p.master_chain.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["m_comp", "m_gain"]);

        // Reorder, param write, remove — all by node id, like track effects.
        move_node(&mut p, "m_gain", -1).unwrap();
        assert_eq!(p.master_chain[0].id, "m_gain");
        set_param(&mut p, "m_comp", "ratio", 0.6).unwrap();
        assert_eq!(p.master_chain[1].params.get("ratio"), Some(&0.6));
        remove_node(&mut p, "m_gain").unwrap();
        assert_eq!(p.master_chain.len(), 1);

        // The graph carries them (validated as effects at compile).
        let (desc, _) = p.to_graph().unwrap();
        assert_eq!(desc.master.len(), 1);
    }

    #[test]
    fn master_chain_rejects_instruments_and_reserved_ids() {
        let mut p = demo();
        assert!(matches!(
            insert_effect(&mut p, MASTER_ID, "plucked", "m_bad", Default::default()),
            Err(GraphEditError::NotAnEffect(_))
        ));
        assert!(matches!(
            add_track(&mut p, MASTER_ID, "Master", "fm", "x", None),
            Err(GraphEditError::ReservedId(_))
        ));
        // A duplicate id across track and master namespaces is refused.
        insert_effect(&mut p, MASTER_ID, "gain", "m_gain", Default::default()).unwrap();
        assert!(matches!(
            insert_effect(&mut p, "lead", "gain", "m_gain", Default::default()),
            Err(GraphEditError::DuplicateId(_))
        ));
    }

    #[test]
    fn move_node_to_crosses_track_and_master() {
        let mut p = demo();
        insert_effect(&mut p, "lead", "reverb", "verb", Default::default()).unwrap();
        // Track → master keeps the id (state adopted across the swap).
        move_node_to(&mut p, "verb", MASTER_ID, 0).unwrap();
        assert_eq!(ids(&p, 0), ["lead_synth"]);
        assert_eq!(p.master_chain[0].id, "verb");
        // Master → track: back after the instrument.
        move_node_to(&mut p, "verb", "lead", 1).unwrap();
        assert_eq!(ids(&p, 0), ["lead_synth", "verb"]);
        assert!(p.master_chain.is_empty());
        // The instrument still cannot go anywhere near the master.
        assert!(matches!(
            move_node_to(&mut p, "lead_synth", MASTER_ID, 0),
            Err(GraphEditError::CannotEditInstrument(_))
        ));
    }

    /// A chain-preset node spec with params, for the whole-chain edits.
    fn spec(id: &str, module: &str, params: &[(&str, f32)]) -> NodeSpec {
        NodeSpec {
            id: id.to_string(),
            module: module.to_string(),
            params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            config: Default::default(),
        }
    }

    #[test]
    fn add_track_with_chain_builds_the_full_chain() {
        let mut p = demo();
        let nodes = [
            spec("bx_ins", "plucked", &[("brightness", 0.35)]),
            spec("bx_rev", "reverb", &[("mix", 0.55)]),
        ];
        add_track_with_chain(&mut p, "bx", "Berceuse", None, &nodes).unwrap();
        assert_eq!(ids(&p, 1), ["bx_ins", "bx_rev"]);
        // Head params land too (plain add_track ships an empty map).
        assert_eq!(p.tracks[1].chain[0].params.get("brightness"), Some(&0.35));
        assert_eq!(p.tracks[1].chain[1].params.get("mix"), Some(&0.55));
        assert_eq!(p.tracks[1].clips.len(), 1); // default empty clip, as add_track
        assert!(p.to_graph().is_ok());
    }

    #[test]
    fn add_track_with_chain_rejects_bad_chains() {
        let mut p = demo();
        assert!(matches!(
            add_track_with_chain(&mut p, "t1", "T", None, &[]),
            Err(GraphEditError::EmptyChain)
        ));
        assert!(matches!(
            add_track_with_chain(&mut p, "t2", "T", None, &[spec("x1", "gain", &[])]),
            Err(GraphEditError::NotAnInstrument(_))
        ));
        // An instrument smuggled into the tail is refused (hand-edited preset).
        assert!(matches!(
            add_track_with_chain(
                &mut p,
                "t3",
                "T",
                None,
                &[spec("x2", "plucked", &[]), spec("x3", "fm", &[])]
            ),
            Err(GraphEditError::NotAnEffect(_))
        ));
        assert!(matches!(
            add_track_with_chain(&mut p, "t4", "T", None, &[spec("lead_synth", "fm", &[])]),
            Err(GraphEditError::DuplicateId(_))
        ));
    }

    #[test]
    fn set_chain_swaps_nodes_and_keeps_clips_and_mixer() {
        let song = r#"{ "bpm": 120, "tracks": [
            { "id": "lead", "mixer": { "gain_db": -3.0, "pan": 0.2 },
              "chain": [ { "id": "lead_synth", "module": "subtractive" },
                         { "id": "old_fx", "module": "delay" } ],
              "clips": [ { "id": "c1", "start": "1.1.0", "length": "4.0.0", "notes": [] } ] }
        ] }"#;
        let mut p: Project = serde_json::from_str(song).unwrap();
        let nodes = [
            spec("new_ins", "plucked", &[("decay", 0.98)]),
            spec("new_rev", "reverb", &[("size", 0.55)]),
        ];
        set_chain(&mut p, "lead", &nodes).unwrap();
        assert_eq!(ids(&p, 0), ["new_ins", "new_rev"]);
        assert_eq!(p.tracks[0].chain[0].params.get("decay"), Some(&0.98));
        // Everything that is not the chain survives: clips, mixer, name.
        assert_eq!(p.tracks[0].clips[0].id, "c1");
        assert_eq!(p.tracks[0].mixer.gain_db, -3.0);
        // Fresh ids again: `set_instrument` checks the id before the track.
        assert!(matches!(
            set_chain(&mut p, "nope", &[spec("other_ins", "fm", &[])]),
            Err(GraphEditError::UnknownTrack(_))
        ));
    }
}
