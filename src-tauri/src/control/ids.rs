//! Stable identifiers for notes and clips.
//!
//! A hand-written `song.json` need not carry ids: every edit addresses notes and
//! clips by id, so we assign one to anything that lacks it. The rule (also in
//! `CONTRIBUTING.md`): existing ids are **preserved**, only id-less objects receive a
//! fresh one, and a project loaded with missing ids is rewritten to disk. Ids are
//! unique across the whole project (notes share the namespace with clips) so an
//! id alone resolves an object.

use std::collections::HashSet;

use crate::control::project::Project;

const NOTE_PREFIX: &str = "n";
const CLIP_PREFIX: &str = "c";

/// Allocates short, collision-free ids against a live set of taken ids. The
/// counter only moves forward, so normalising a whole project stays linear.
struct IdGen {
    used: HashSet<String>,
    counter: u64,
}

impl IdGen {
    fn from_project(project: &Project) -> Self {
        let mut used = HashSet::new();
        for track in &project.tracks {
            for clip in &track.clips {
                if !clip.id.is_empty() {
                    used.insert(clip.id.clone());
                }
                for note in &clip.notes {
                    if !note.id.is_empty() {
                        used.insert(note.id.clone());
                    }
                }
            }
        }
        Self { used, counter: 0 }
    }

    fn alloc(&mut self, prefix: &str) -> String {
        loop {
            self.counter += 1;
            let candidate = format!("{prefix}{}", self.counter);
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
        }
    }
}

/// Assign ids to every id-less note and clip, preserving existing ids. Returns
/// `true` if anything changed (the caller then rewrites the file). // off-RT
pub fn normalize(project: &mut Project) -> bool {
    let mut gen = IdGen::from_project(project);
    let mut changed = false;
    for track in &mut project.tracks {
        for clip in &mut track.clips {
            if clip.id.is_empty() {
                clip.id = gen.alloc(CLIP_PREFIX);
                changed = true;
            }
            for note in &mut clip.notes {
                if note.id.is_empty() {
                    note.id = gen.alloc(NOTE_PREFIX);
                    changed = true;
                }
            }
        }
    }
    changed
}

/// A fresh, unique note id for a newly created note. // off-RT
pub fn fresh_note_id(project: &Project) -> String {
    IdGen::from_project(project).alloc(NOTE_PREFIX)
}

/// A fresh, unique clip id for a newly created clip. // off-RT
pub fn fresh_clip_id(project: &Project) -> String {
    IdGen::from_project(project).alloc(CLIP_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(json: &str) -> Project {
        serde_json::from_str(json).unwrap()
    }

    const TWO_NOTES: &str = r#"{ "bpm": 120, "tracks": [
        { "id": "lead", "chain": [ { "id": "s", "module": "subtractive" } ],
          "clips": [ { "start": "1.1.0", "length": "1.0.0", "notes": [
            { "pitch": 60, "start": "1.1.0", "length": "0.1.0" },
            { "pitch": 62, "start": "1.2.0", "length": "0.1.0" } ] } ] } ] }"#;

    #[test]
    fn assigns_unique_ids_to_idless_objects() {
        let mut p = project(TWO_NOTES);
        assert!(normalize(&mut p));
        let clip = &p.tracks[0].clips[0];
        assert!(!clip.id.is_empty());
        let ids: Vec<&str> = clip.notes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.iter().all(|id| !id.is_empty()));
        assert_ne!(ids[0], ids[1], "ids must be unique");
    }

    #[test]
    fn preserves_existing_ids_and_avoids_collision() {
        // A note already owns "n1"; the id-less note must not be given "n1".
        let json = r#"{ "bpm": 120, "tracks": [
            { "id": "lead", "chain": [ { "id": "s", "module": "subtractive" } ],
              "clips": [ { "id": "c1", "start": "1.1.0", "length": "1.0.0", "notes": [
                { "id": "n1", "pitch": 60, "start": "1.1.0", "length": "0.1.0" },
                { "pitch": 62, "start": "1.2.0", "length": "0.1.0" } ] } ] } ] }"#;
        let mut p = project(json);
        assert!(normalize(&mut p));
        let notes = &p.tracks[0].clips[0].notes;
        assert_eq!(notes[0].id, "n1", "existing id preserved");
        assert_ne!(notes[1].id, "n1", "no collision with the existing id");
        assert_eq!(p.tracks[0].clips[0].id, "c1");
    }

    #[test]
    fn is_idempotent() {
        let mut p = project(TWO_NOTES);
        assert!(normalize(&mut p));
        assert!(!normalize(&mut p), "a second pass changes nothing");
    }
}
