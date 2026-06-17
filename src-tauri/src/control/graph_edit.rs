//! Pure edit operations on a [`Project`]'s effect chains.
//!
//! Each function mutates a project in place after validating the edit. The
//! control side applies these to a trial clone, recompiles, and hot-swaps; the
//! audio thread is never touched here. M4 edits effects only — a track's
//! instrument (chain head) is never moved or removed, which keeps instrument
//! node indices stable across swaps.

use std::collections::HashMap;

use crate::control::project::{NodeSpec, Project};
use crate::modules::registry;

/// Reserved id of the implicit master node (see `project::to_graph`).
const MASTER_ID: &str = "_master";

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

/// Insert a new effect at `index` in a track's chain. `index` is clamped to the
/// effects region `[1, len]` (1 = right after the instrument, `len` = end), so a
/// new effect can never land on the instrument slot. `usize::MAX` means append.
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
    let track = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .ok_or_else(|| GraphEditError::UnknownTrack(track_id.to_string()))?;
    let pos = index.clamp(1, track.chain.len());
    track.chain.insert(
        pos,
        NodeSpec {
            id: node_id.to_string(),
            module: module.to_string(),
            params: params.into_iter().collect(),
        },
    );
    Ok(())
}

/// Remove an effect by id. The instrument (chain head) cannot be removed.
pub fn remove_node(project: &mut Project, node_id: &str) -> Result<(), GraphEditError> {
    let (track_idx, chain_idx) = locate(project, node_id)?;
    if chain_idx == 0 {
        return Err(GraphEditError::CannotEditInstrument(node_id.to_string()));
    }
    project.tracks[track_idx].chain.remove(chain_idx);
    Ok(())
}

/// Move an effect one step earlier (`dir < 0`) or later (`dir > 0`) in its
/// chain. The instrument slot (index 0) is never entered or left.
pub fn move_node(project: &mut Project, node_id: &str, dir: i32) -> Result<(), GraphEditError> {
    let (track_idx, chain_idx) = locate(project, node_id)?;
    if chain_idx == 0 {
        return Err(GraphEditError::CannotEditInstrument(node_id.to_string()));
    }
    let chain = &mut project.tracks[track_idx].chain;
    let target = chain_idx as i32 + dir.signum();
    // Stay within the effects region [1, len-1]; a no-op at the edges.
    if target >= 1 && (target as usize) < chain.len() {
        chain.swap(chain_idx, target as usize);
    }
    Ok(())
}

/// Move an effect to `index` in another (or the same) track's chain, **keeping its
/// id** so the hot-swap adopts the same node instance — its internal state (e.g. a
/// reverb tail) follows the move without a gap. The instrument (index 0) can never
/// be moved, nor can a node land on a target's instrument slot.
pub fn move_node_to(
    project: &mut Project,
    node_id: &str,
    to_track: &str,
    index: usize,
) -> Result<(), GraphEditError> {
    let (src_track, src_chain) = locate(project, node_id)?;
    if src_chain == 0 {
        return Err(GraphEditError::CannotEditInstrument(node_id.to_string()));
    }
    let dst_track = track_index(project, to_track)?;
    let spec = project.tracks[src_track].chain.remove(src_chain);
    // Removing from the same track before the target shifts later slots left.
    let mut pos = index;
    if src_track == dst_track && src_chain < pos {
        pos -= 1;
    }
    let chain = &mut project.tracks[dst_track].chain;
    let pos = pos.clamp(1, chain.len());
    chain.insert(pos, spec);
    Ok(())
}

/// Copy an effect into a track's chain at `index` under a **new** id (a fresh node
/// instance with the same params). The instrument cannot be copied.
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
    let (src_track, src_chain) = locate(project, node_id)?;
    if src_chain == 0 {
        return Err(GraphEditError::CannotEditInstrument(node_id.to_string()));
    }
    let source = &project.tracks[src_track].chain[src_chain];
    let copy = NodeSpec {
        id: new_id.to_string(),
        module: source.module.clone(),
        params: source.params.clone(),
    };
    let dst_track = track_index(project, to_track)?;
    let chain = &mut project.tracks[dst_track].chain;
    let pos = index.clamp(1, chain.len());
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
    let (track_idx, chain_idx) = locate(project, node_id)?;
    project.tracks[track_idx].chain[chain_idx]
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
        .any(|n| n.id == node_id)
}

/// Find the `(track, chain)` indices of a node id.
fn locate(project: &Project, node_id: &str) -> Result<(usize, usize), GraphEditError> {
    for (ti, track) in project.tracks.iter().enumerate() {
        if let Some(ci) = track.chain.iter().position(|n| n.id == node_id) {
            return Ok((ti, ci));
        }
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
}
