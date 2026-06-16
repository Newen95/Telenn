//! Pure clip edit operations on a [`Project`], addressed by clip id.
//!
//! Clips are light containers (M3): notes carry absolute positions. So **moving**
//! a clip shifts the clip *and* every note it holds by the same delta (ids
//! preserved); **resizing** a clip changes only its length and never touches its
//! notes. Positions are in ticks, re-encoded to `"bar.beat.tick"`.

use crate::control::project::{
    position_to_ticks, ticks_to_duration, ticks_to_position, Clip, Project, ProjectError,
};

#[derive(Debug, thiserror::Error)]
pub enum ClipEditError {
    #[error("unknown clip '{0}'")]
    UnknownClip(String),
    #[error(transparent)]
    Project(#[from] ProjectError),
}

const MIN_LEN_TICKS: u64 = 1;

/// Move a clip by `delta_ticks` (signed). The clip and all its notes shift by the
/// same amount; the move is clamped so the earliest position never goes negative,
/// keeping the relative spacing intact.
pub fn move_clip(
    project: &mut Project,
    clip_id: &str,
    delta_ticks: i64,
) -> Result<(), ClipEditError> {
    let (tpbar, tpbeat) = project.ticks_per_bar_beat()?;
    let clip = locate(project, clip_id)?;

    // Earliest tick among the clip start and its notes bounds how far left we can go.
    let mut min_tick = position_to_ticks(&clip.start, tpbar, tpbeat)?;
    for note in &clip.notes {
        min_tick = min_tick.min(position_to_ticks(&note.start, tpbar, tpbeat)?);
    }
    let delta = delta_ticks.max(-(min_tick as i64));

    let shift = |s: &str| -> Result<String, ProjectError> {
        let t = position_to_ticks(s, tpbar, tpbeat)? as i64 + delta;
        Ok(ticks_to_position(t.max(0) as u64, tpbar, tpbeat))
    };
    clip.start = shift(&clip.start)?;
    for note in &mut clip.notes {
        note.start = shift(&note.start)?;
    }
    Ok(())
}

/// Resize a clip: change only its length (ticks). Notes (ids and positions) are
/// untouched.
pub fn resize_clip(
    project: &mut Project,
    clip_id: &str,
    length_ticks: u64,
) -> Result<(), ClipEditError> {
    let (tpbar, tpbeat) = project.ticks_per_bar_beat()?;
    let clip = locate(project, clip_id)?;
    clip.length = ticks_to_duration(length_ticks.max(MIN_LEN_TICKS), tpbar, tpbeat);
    Ok(())
}

fn locate<'a>(project: &'a mut Project, clip_id: &str) -> Result<&'a mut Clip, ClipEditError> {
    for track in &mut project.tracks {
        if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
            return Ok(clip);
        }
    }
    Err(ClipEditError::UnknownClip(clip_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Project {
        let json = r#"{ "bpm": 120, "tracks": [
            { "id": "lead", "chain": [ { "id": "s", "module": "subtractive" } ],
              "clips": [ { "id": "c1", "start": "2.1.0", "length": "1.0.0", "notes": [
                { "id": "n1", "pitch": 60, "start": "2.1.0", "length": "0.1.0" },
                { "id": "n2", "pitch": 62, "start": "2.3.0", "length": "0.1.0" } ] } ] } ] }"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn move_shifts_clip_and_notes_by_the_same_delta() {
        let mut p = demo();
        // Move forward one bar (3840 ticks).
        move_clip(&mut p, "c1", 3840).unwrap();
        let clip = &p.tracks[0].clips[0];
        assert_eq!(clip.start, "3.1.0");
        assert_eq!(clip.notes[0].start, "3.1.0");
        assert_eq!(clip.notes[1].start, "3.3.0");
        // Note ids preserved.
        assert_eq!(clip.notes[0].id, "n1");
        assert_eq!(clip.notes[1].id, "n2");
    }

    #[test]
    fn move_is_clamped_at_zero() {
        let mut p = demo();
        // Try to move far left; clip starts at bar 2 (3840 ticks), so it lands at 0.
        move_clip(&mut p, "c1", -100_000).unwrap();
        let clip = &p.tracks[0].clips[0];
        assert_eq!(clip.start, "1.1.0"); // tick 0
        assert_eq!(clip.notes[0].start, "1.1.0");
        // The second note kept its relative offset (one beat... it was +2 beats).
        assert_eq!(clip.notes[1].start, "1.3.0");
    }

    #[test]
    fn resize_changes_length_only_and_preserves_notes() {
        let mut p = demo();
        let before: Vec<(String, String)> = p.tracks[0].clips[0]
            .notes
            .iter()
            .map(|n| (n.id.clone(), n.start.clone()))
            .collect();
        resize_clip(&mut p, "c1", 2 * 3840).unwrap();
        let clip = &p.tracks[0].clips[0];
        assert_eq!(clip.length, "2.0.0");
        let after: Vec<(String, String)> = clip
            .notes
            .iter()
            .map(|n| (n.id.clone(), n.start.clone()))
            .collect();
        assert_eq!(before, after, "resize must not move or rename notes");
    }
}
