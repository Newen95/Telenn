//! Graph edit history: undo/redo over project snapshots.
//!
//! Edits are project-level (effect chains), so the history stacks whole
//! `Project` snapshots. The caller validates and recompiles an edit before
//! committing it here; undo/redo return the project to recompile and hot-swap.

use crate::control::project::Project;

pub struct EditHistory {
    project: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl EditHistory {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Adopt an already-validated edit, pushing the previous state onto the undo
    /// stack and clearing the redo stack.
    pub fn commit(&mut self, next: Project) {
        let prev = std::mem::replace(&mut self.project, next);
        self.undo.push(prev);
        self.redo.clear();
    }

    /// Step back one edit; returns the restored project to recompile, or `None`.
    pub fn undo(&mut self) -> Option<&Project> {
        let prev = self.undo.pop()?;
        let current = std::mem::replace(&mut self.project, prev);
        self.redo.push(current);
        Some(&self.project)
    }

    /// Step forward one edit; returns the restored project, or `None`.
    pub fn redo(&mut self) -> Option<&Project> {
        let next = self.redo.pop()?;
        let current = std::mem::replace(&mut self.project, next);
        self.undo.push(current);
        Some(&self.project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::graph_edit;

    const DEMO: &str = r#"{ "bpm": 120, "tracks": [
        { "id": "lead", "chain": [ { "id": "lead_synth", "module": "subtractive" } ] }
    ] }"#;

    #[test]
    fn undo_redo_50_edits_round_trips() {
        let initial: Project = serde_json::from_str(DEMO).unwrap();
        let initial_desc = initial.to_graph().unwrap().0;
        let mut history = EditHistory::new(initial);

        for i in 0..50 {
            let mut next = history.project().clone();
            graph_edit::insert_effect(
                &mut next,
                "lead",
                "gain",
                &format!("fx{i}"),
                Default::default(),
            )
            .unwrap();
            history.commit(next);
        }
        // 50 effects added; the graph differs from the start.
        assert_ne!(history.project().to_graph().unwrap().0, initial_desc);

        for _ in 0..50 {
            assert!(history.undo().is_some());
        }
        assert_eq!(
            history.project().to_graph().unwrap().0,
            initial_desc,
            "undoing every edit must restore the initial graph"
        );

        // Redo walks back to the fully-edited state.
        for _ in 0..50 {
            assert!(history.redo().is_some());
        }
        assert_eq!(history.project().tracks[0].chain.len(), 51);
    }
}
