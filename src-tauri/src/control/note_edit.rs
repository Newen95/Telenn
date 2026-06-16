//! Pure note edit operations on a [`Project`], addressed by note id.
//!
//! Each function mutates a project in place after validating the edit; the
//! control side applies these to a trial clone, rebuilds the schedule and
//! hot-swaps it (the topology never changes, so notes resolve against the running
//! graph — see `control::commands`). Positions are taken in **ticks** and
//! re-encoded to the textual `"bar.beat.tick"` so `song.json` stays the source of
//! truth.

use crate::control::ids;
use crate::control::project::{
    duration_to_ticks, position_to_ticks, ticks_to_duration, ticks_to_position, Note, Project,
    ProjectError,
};

#[derive(Debug, thiserror::Error)]
pub enum NoteEditError {
    #[error("unknown track '{0}'")]
    UnknownTrack(String),
    #[error("unknown clip '{0}'")]
    UnknownClip(String),
    #[error("unknown note '{0}'")]
    UnknownNote(String),
    #[error(transparent)]
    Project(#[from] ProjectError),
}

/// Smallest note length, in ticks (a 64th-note-ish floor) so a note never has
/// zero or negative duration.
const MIN_LEN_TICKS: u64 = 1;

/// Create a note in `clip_id` of `track_id`. Returns the new note's id — or, if a
/// same-pitch note already overlaps this span, that note's id and no new note: the
/// same pitch cannot sound twice at once (a second NoteOn before its NoteOff), and
/// stacking would leave an invisible duplicate that keeps sounding after a delete.
pub fn add(
    project: &mut Project,
    track_id: &str,
    clip_id: &str,
    pitch: u8,
    start_ticks: u64,
    length_ticks: u64,
    vel: u8,
) -> Result<String, NoteEditError> {
    let (tpbar, tpbeat) = project.ticks_per_bar_beat()?;
    let len = length_ticks.max(MIN_LEN_TICKS);
    let end = start_ticks.saturating_add(len);
    let id = ids::fresh_note_id(project);
    let track = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .ok_or_else(|| NoteEditError::UnknownTrack(track_id.to_string()))?;
    let clip = track
        .clips
        .iter_mut()
        .find(|c| c.id == clip_id)
        .ok_or_else(|| NoteEditError::UnknownClip(clip_id.to_string()))?;
    for n in &clip.notes {
        if n.pitch != pitch {
            continue;
        }
        let n_start = position_to_ticks(&n.start, tpbar, tpbeat)?;
        let n_end = n_start.saturating_add(duration_to_ticks(&n.length, tpbar, tpbeat)?);
        if start_ticks < n_end && n_start < end {
            return Ok(n.id.clone()); // already a note here: ignore the duplicate
        }
    }
    clip.notes.push(Note {
        id: id.clone(),
        pitch,
        start: ticks_to_position(start_ticks, tpbar, tpbeat),
        length: ticks_to_duration(len, tpbar, tpbeat),
        vel,
    });
    Ok(id)
}

/// Move a note to a new start (ticks) and pitch.
pub fn move_note(
    project: &mut Project,
    note_id: &str,
    start_ticks: u64,
    pitch: u8,
) -> Result<(), NoteEditError> {
    let (tpbar, tpbeat) = project.ticks_per_bar_beat()?;
    let note = locate(project, note_id)?;
    note.start = ticks_to_position(start_ticks, tpbar, tpbeat);
    note.pitch = pitch;
    Ok(())
}

/// Resize a note: set its start and length (ticks), never its pitch. A right-edge
/// resize passes the unchanged start (length-only); a left-edge resize moves the
/// start while the end (start + length) stays fixed — both edges, one edit.
pub fn resize_note(
    project: &mut Project,
    note_id: &str,
    start_ticks: u64,
    length_ticks: u64,
) -> Result<(), NoteEditError> {
    let (tpbar, tpbeat) = project.ticks_per_bar_beat()?;
    let note = locate(project, note_id)?;
    note.start = ticks_to_position(start_ticks, tpbar, tpbeat);
    note.length = ticks_to_duration(length_ticks.max(MIN_LEN_TICKS), tpbar, tpbeat);
    Ok(())
}

/// Delete the note `note_id` and any same-pitch note overlapping it. The sweep is
/// defensive against legacy stacked duplicates (pre-dedup data): deleting must
/// leave no residual voice sounding under where the note was.
pub fn delete(project: &mut Project, note_id: &str) -> Result<(), NoteEditError> {
    let (tpbar, tpbeat) = project.ticks_per_bar_beat()?;
    for track in &mut project.tracks {
        for clip in &mut track.clips {
            let Some(i) = clip.notes.iter().position(|n| n.id == note_id) else {
                continue;
            };
            let target = &clip.notes[i];
            let pitch = target.pitch;
            let start = position_to_ticks(&target.start, tpbar, tpbeat)?;
            let end = start.saturating_add(duration_to_ticks(&target.length, tpbar, tpbeat)?);

            let mut remove = Vec::with_capacity(1);
            for n in &clip.notes {
                if n.id != note_id && n.pitch != pitch {
                    continue;
                }
                let n_start = position_to_ticks(&n.start, tpbar, tpbeat)?;
                let n_end = n_start.saturating_add(duration_to_ticks(&n.length, tpbar, tpbeat)?);
                if n.id == note_id || (start < n_end && n_start < end) {
                    remove.push(n.id.clone());
                }
            }
            clip.notes.retain(|n| !remove.contains(&n.id));
            return Ok(());
        }
    }
    Err(NoteEditError::UnknownNote(note_id.to_string()))
}

/// Mutable reference to the note with `note_id`.
fn locate<'a>(project: &'a mut Project, note_id: &str) -> Result<&'a mut Note, NoteEditError> {
    for track in &mut project.tracks {
        for clip in &mut track.clips {
            if let Some(note) = clip.notes.iter_mut().find(|n| n.id == note_id) {
                return Ok(note);
            }
        }
    }
    Err(NoteEditError::UnknownNote(note_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Project {
        // 4/4 → 3840 ticks/bar, 960 ticks/beat.
        let json = r#"{ "bpm": 120, "tracks": [
            { "id": "lead", "chain": [ { "id": "s", "module": "subtractive" } ],
              "clips": [ { "id": "c1", "start": "1.1.0", "length": "2.0.0", "notes": [
                { "id": "n1", "pitch": 60, "start": "1.1.0", "length": "0.1.0" } ] } ] } ] }"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn add_assigns_id_and_position() {
        let mut p = demo();
        let id = add(&mut p, "lead", "c1", 67, 960, 480, 100).unwrap();
        let note = p.tracks[0].clips[0]
            .notes
            .iter()
            .find(|n| n.id == id)
            .unwrap();
        assert_eq!(note.pitch, 67);
        assert_eq!(note.start, "1.2.0"); // 960 ticks = beat 2
        assert_eq!(note.length, "0.0.480");
    }

    #[test]
    fn move_changes_start_and_pitch_only() {
        let mut p = demo();
        move_note(&mut p, "n1", 3840, 62).unwrap();
        let note = &p.tracks[0].clips[0].notes[0];
        assert_eq!(note.start, "2.1.0"); // 3840 ticks = bar 2
        assert_eq!(note.pitch, 62);
        assert_eq!(note.length, "0.1.0"); // untouched
    }

    #[test]
    fn resize_right_keeps_start() {
        let mut p = demo();
        // Right-edge resize: caller passes the unchanged start, a new length.
        resize_note(&mut p, "n1", 0, 1920).unwrap();
        let note = &p.tracks[0].clips[0].notes[0];
        assert_eq!(note.length, "0.2.0"); // 1920 ticks = 2 beats
        assert_eq!(note.start, "1.1.0"); // tick 0, unchanged
    }

    #[test]
    fn resize_left_moves_start_keeping_end() {
        let mut p = demo();
        // n1 starts at tick 0, length 1 beat (end at tick 960). Drag the left
        // edge to tick 480: start moves, end stays at 960, so length halves.
        resize_note(&mut p, "n1", 480, 480).unwrap();
        let note = &p.tracks[0].clips[0].notes[0];
        assert_eq!(note.start, "1.1.480"); // 480 ticks into bar 1, beat 1
        assert_eq!(note.length, "0.0.480"); // 480 ticks
    }

    #[test]
    fn add_ignores_exact_overlap_duplicate() {
        let mut p = demo();
        // n1 is pitch 60 at ticks [0, 960). A same-pitch note overlapping it must
        // not stack: add returns the existing id and leaves a single note.
        let id = add(&mut p, "lead", "c1", 60, 0, 480, 100).unwrap();
        assert_eq!(id, "n1");
        assert_eq!(p.tracks[0].clips[0].notes.len(), 1);
    }

    #[test]
    fn add_allows_same_pitch_when_not_overlapping() {
        let mut p = demo();
        // Same pitch but starting after n1 ends (960): a legitimate second note.
        let id = add(&mut p, "lead", "c1", 60, 1920, 480, 100).unwrap();
        assert_ne!(id, "n1");
        assert_eq!(p.tracks[0].clips[0].notes.len(), 2);
    }

    #[test]
    fn delete_removes_the_note() {
        let mut p = demo();
        delete(&mut p, "n1").unwrap();
        assert!(p.tracks[0].clips[0].notes.is_empty());
        assert!(matches!(
            delete(&mut p, "n1"),
            Err(NoteEditError::UnknownNote(_))
        ));
    }

    #[test]
    fn delete_sweeps_stacked_duplicates() {
        // Legacy data: two same-pitch notes stacked at the same instant. Deleting
        // one must remove the hidden twin too, leaving no residual sounding voice.
        let json = r#"{ "bpm": 120, "tracks": [
            { "id": "lead", "chain": [ { "id": "s", "module": "subtractive" } ],
              "clips": [ { "id": "c1", "start": "1.1.0", "length": "2.0.0", "notes": [
                { "id": "n1", "pitch": 60, "start": "1.1.0", "length": "0.1.0" },
                { "id": "n2", "pitch": 60, "start": "1.1.0", "length": "0.1.0" } ] } ] } ] }"#;
        let mut p: Project = serde_json::from_str(json).unwrap();
        delete(&mut p, "n1").unwrap();
        assert!(p.tracks[0].clips[0].notes.is_empty());
    }

    #[test]
    fn rejects_unknown_targets() {
        let mut p = demo();
        assert!(matches!(
            add(&mut p, "nope", "c1", 60, 0, 480, 100),
            Err(NoteEditError::UnknownTrack(_))
        ));
        assert!(matches!(
            add(&mut p, "lead", "nope", 60, 0, 480, 100),
            Err(NoteEditError::UnknownClip(_))
        ));
        assert!(matches!(
            move_note(&mut p, "nope", 0, 60),
            Err(NoteEditError::UnknownNote(_))
        ));
    }
}
