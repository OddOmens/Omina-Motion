//! Undo and redo. Instead of every edit describing how to reverse itself, the history keeps
//! whole copies of the project, which hold no media and so are small. After each change it
//! records the project as it was before. Because it compares projects rather than trusting
//! each edit to report itself, no edit can be left out of undo by accident.
//!
//! The caller records only when no drag is in progress, so a whole drag (moving a layer,
//! scrubbing a value, dragging keyframes) undoes in one step.

use studio_core::model::Project;

/// Steps kept for undo. Older steps are dropped.
const LIMIT: usize = 200;

/// A project as it was, and the version number it had then.
struct Step {
    project: Project,
    version: u64,
}

pub struct History {
    /// The project as of the last recorded change: what the next change is compared against.
    baseline: Project,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Numbers each recorded state of the project, so the history can tell whether the project
    /// is back to how it was saved (after undoing, say) without comparing whole projects.
    version: u64,
    next_version: u64,
    saved_version: u64,
}

impl History {
    /// A fresh history for `project`, which counts as saved.
    pub fn new(project: &Project) -> Self {
        Self { baseline: project.clone(), undo: Vec::new(), redo: Vec::new(), version: 0, next_version: 1, saved_version: 0 }
    }

    /// Records `project` as a new step if it changed since the last one. Any redo steps are
    /// dropped, since they branched off from an earlier state.
    pub fn record(&mut self, project: &Project) {
        if *project == self.baseline {
            return;
        }
        let before = std::mem::replace(&mut self.baseline, project.clone());
        self.undo.push(Step { project: before, version: self.version });
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.version = self.next_version;
        self.next_version += 1;
    }

    /// Puts `project` back one step. Changes not recorded yet are recorded first, so they are
    /// what gets undone. Returns whether there was anything to undo.
    pub fn undo(&mut self, project: &mut Project) -> bool {
        self.record(project);
        let Some(step) = self.undo.pop() else { return false };
        self.redo.push(Step { project: std::mem::replace(project, step.project), version: self.version });
        self.restore(project, step.version);
        true
    }

    /// Re-applies the last undone step. Returns whether there was anything to redo.
    pub fn redo(&mut self, project: &mut Project) -> bool {
        self.record(project);
        let Some(step) = self.redo.pop() else { return false };
        self.undo.push(Step { project: std::mem::replace(project, step.project), version: self.version });
        self.restore(project, step.version);
        true
    }

    fn restore(&mut self, project: &Project, version: u64) {
        self.baseline = project.clone();
        self.version = version;
    }

    pub fn can_undo(&self, project: &Project) -> bool {
        !self.undo.is_empty() || *project != self.baseline
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Applies a change the app made by itself, not the user (such as fitting a text layer's
    /// box to its measured text), to the baseline too, so it does not become an undo step.
    pub fn absorb(&mut self, change: impl FnOnce(&mut Project)) {
        change(&mut self.baseline);
    }

    /// `absorb`, for a change that holds in every version of the project (a library color now
    /// being blue, a background job's result for settings a layer still has): it is applied to the
    /// undo and redo steps too, so undoing an unrelated edit doesn't bring back what it replaced.
    /// `change` must leave a project alone where it doesn't apply.
    pub fn absorb_everywhere(&mut self, change: impl Fn(&mut Project)) {
        change(&mut self.baseline);
        for step in self.undo.iter_mut().chain(&mut self.redo) {
            change(&mut step.project);
        }
    }

    /// Notes that `project` was just saved.
    pub fn mark_saved(&mut self, project: &Project) {
        self.record(project);
        self.saved_version = self.version;
    }

    /// Notes a save made in the middle of a drag. The drag is not recorded yet (it becomes one
    /// undo step when it ends), and what was saved is unfinished, so the project still counts
    /// as having unsaved changes until the next save.
    pub fn mark_saved_mid_change(&mut self) {
        self.saved_version = u64::MAX;
    }

    /// Whether `project` differs from what was last saved (or opened).
    pub fn has_unsaved_changes(&self, project: &Project) -> bool {
        self.version != self.saved_version || *project != self.baseline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::sample::sample_project;

    /// The sample project with its first composition renamed, as a stand-in for any edit.
    fn renamed(project: &Project, name: &str) -> Project {
        let mut changed = project.clone();
        changed.compositions[0].name = name.into();
        changed
    }

    #[test]
    fn undo_and_redo_walk_back_and_forth() {
        let original = sample_project();
        let mut history = History::new(&original);
        let first = renamed(&original, "First");
        history.record(&first);
        let mut project = renamed(&first, "Second");
        history.record(&project);

        assert!(history.undo(&mut project));
        assert_eq!(project, first);
        assert!(history.undo(&mut project));
        assert_eq!(project, original);
        assert!(!history.undo(&mut project), "nothing left to undo");
        assert!(history.redo(&mut project));
        assert!(history.redo(&mut project));
        assert_eq!(project.compositions[0].name, "Second");
        assert!(!history.redo(&mut project));
    }

    #[test]
    fn a_new_edit_after_undo_drops_the_redo_steps() {
        let original = sample_project();
        let mut history = History::new(&original);
        let mut project = renamed(&original, "First");
        history.record(&project);
        history.undo(&mut project);
        project = renamed(&project, "Other");
        history.record(&project);
        assert!(!history.can_redo());
    }

    #[test]
    fn unrecorded_changes_are_what_undo_reverts() {
        let original = sample_project();
        let mut history = History::new(&original);
        let mut project = renamed(&original, "Not recorded yet");
        assert!(history.can_undo(&project));
        assert!(history.undo(&mut project));
        assert_eq!(project, original);
    }

    #[test]
    fn unsaved_changes_follow_undo_back_to_the_saved_state() {
        let original = sample_project();
        let mut history = History::new(&original);
        assert!(!history.has_unsaved_changes(&original));
        let mut project = renamed(&original, "Edited");
        assert!(history.has_unsaved_changes(&project), "before it is even recorded");
        history.record(&project);
        history.mark_saved(&project);
        assert!(!history.has_unsaved_changes(&project));
        history.undo(&mut project);
        assert!(history.has_unsaved_changes(&project));
        history.redo(&mut project);
        assert!(!history.has_unsaved_changes(&project), "redo returns to exactly what was saved");
    }

    #[test]
    fn automatic_changes_are_not_undo_steps() {
        let original = sample_project();
        let mut history = History::new(&original);
        let mut project = original.clone();
        let fit = |project: &mut Project| project.compositions[0].layers[0].size.x += 1.0;
        fit(&mut project);
        history.absorb(fit);
        assert!(!history.can_undo(&project));
        assert!(!history.has_unsaved_changes(&project));
    }

    #[test]
    fn changes_that_hold_everywhere_reach_every_step() {
        let original = sample_project();
        let mut history = History::new(&original);
        let mut project = renamed(&original, "Edited");
        history.record(&project);
        let recolor = |project: &mut Project| project.compositions[0].layers[0].label = studio_core::model::LabelColor::Blue;
        recolor(&mut project);
        history.absorb_everywhere(recolor);
        assert!(history.undo(&mut project));
        assert_eq!(project.compositions[0].layers[0].label, studio_core::model::LabelColor::Blue, "undoing the rename keeps the new color");
        assert!(history.redo(&mut project));
        assert_eq!(project.compositions[0].layers[0].label, studio_core::model::LabelColor::Blue);
    }

    #[test]
    fn old_steps_are_dropped_past_the_limit() {
        let original = sample_project();
        let mut history = History::new(&original);
        let mut project = original.clone();
        for step in 0..LIMIT + 10 {
            project = renamed(&project, &format!("Step {step}"));
            history.record(&project);
        }
        let mut undone = 0;
        while history.undo(&mut project) {
            undone += 1;
        }
        assert_eq!(undone, LIMIT);
    }
}
