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
    track.chain.push(NodeSpec {
        id: node_id.to_string(),
        module: module.to_string(),
        params,
    });
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
