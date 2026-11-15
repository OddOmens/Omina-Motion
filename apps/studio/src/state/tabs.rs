//! Composition tabs: the compositions open across the top of the Composition panel, as
//! documents are in Canvas. Each tab keeps its own playhead, work area and timeline zoom, so
//! switching back to one finds it as it was left. Closing a tab only stops showing that
//! composition; the composition itself stays in the project.

use studio_core::model::Id;
use studio_core::time::FrameRange;

use super::EditorState;

/// Where a tab that isn't showing was left.
#[derive(Clone, Copy, Debug)]
pub(super) struct TabView {
    current_frame: i64,
    work_area: FrameRange,
    view_range: FrameRange,
}

impl EditorState {
    /// The open tabs in order, leaving out any composition that has been deleted since. The
    /// open composition is always among them.
    pub fn tabs(&self) -> Vec<Id> {
        let mut tabs: Vec<Id> = self.open_tabs.iter().copied().filter(|id| self.project.composition(*id).is_some()).collect();
        if !tabs.contains(&self.active_comp) {
            tabs.push(self.active_comp);
        }
        tabs
    }

    /// Whether the tab for `id` can be closed: not when it is the only one, since the panels
    /// always show a composition.
    pub fn can_close_tab(&self, id: Id) -> bool {
        let tabs = self.tabs();
        tabs.len() > 1 && tabs.contains(&id)
    }

    /// Closes the tab for `id`. When it was the one showing, the tab to its right opens (or,
    /// for the last tab, the one to its left). Returns whether it closed.
    pub fn close_tab(&mut self, id: Id) -> bool {
        if !self.can_close_tab(id) {
            return false;
        }
        let tabs = self.tabs();
        let index = tabs.iter().position(|tab| *tab == id).expect("checked by can_close_tab");
        self.open_tabs = tabs.into_iter().filter(|tab| *tab != id).collect();
        self.tab_views.remove(&id);
        if id == self.active_comp {
            let next = self.open_tabs[index.min(self.open_tabs.len() - 1)];
            self.open_composition(next);
        }
        true
    }

    /// Forgets every tab, for a project that replaces this one: its compositions are new.
    pub(super) fn reset_tabs(&mut self) {
        self.open_tabs.clear();
        self.tab_views.clear();
    }

    /// Notes where the open composition's tab is, before another one opens.
    pub(super) fn remember_tab_view(&mut self) {
        let view = TabView { current_frame: self.current_frame, work_area: self.work_area, view_range: self.view_range };
        self.tab_views.insert(self.active_comp, view);
    }

    /// Adds a tab for `id` at the end, if it has none, and gives back where that tab was left
    /// (`None` for a new tab), kept within the composition's `duration` in frames.
    pub(super) fn take_tab_view(&mut self, id: Id, duration: i64) -> Option<(i64, FrameRange, FrameRange)> {
        self.open_tabs.retain(|tab| self.project.composition(*tab).is_some());
        if !self.open_tabs.contains(&id) {
            self.open_tabs.push(id);
        }
        let view = self.tab_views.remove(&id)?;
        let frames = duration as f64;
        Some((view.current_frame.clamp(0, duration - 1), view.work_area.clamped(frames, 1.0), view.view_range.clamped(frames, super::MIN_VIEW_FRAMES)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::edit::CompositionSettings;

    /// A second composition beside the sample's main one, opened.
    fn with_second_comp(state: &mut EditorState) -> Id {
        let settings = CompositionSettings { name: "Second".into(), ..state.composition_settings() };
        state.create_composition(&settings).unwrap();
        state.active_comp
    }

    #[test]
    fn opening_a_composition_adds_a_tab_that_remembers_its_playhead() {
        let mut state = EditorState::new();
        let main = state.active_comp;
        state.current_frame = 40;
        let second = with_second_comp(&mut state);
        assert_eq!(state.tabs(), vec![main, second]);
        assert_eq!(state.current_frame, 0, "a new tab starts at its first frame");
        state.current_frame = 12;
        state.open_composition(main);
        assert_eq!(state.current_frame, 40, "back where it was left");
        state.open_composition(second);
        assert_eq!(state.current_frame, 12);
        assert_eq!(state.tabs(), vec![main, second], "opening an open composition adds no tab");
    }

    #[test]
    fn closing_a_tab_opens_its_neighbour_and_the_last_tab_stays() {
        let mut state = EditorState::new();
        let main = state.active_comp;
        let second = with_second_comp(&mut state);
        assert!(state.close_tab(second));
        assert_eq!((state.active_comp, state.tabs()), (main, vec![main]));
        assert!(state.project.composition(second).is_some(), "closing a tab keeps the composition");
        assert!(!state.can_close_tab(main) && !state.close_tab(main), "the only tab can't close");
    }

    #[test]
    fn a_deleted_composition_loses_its_tab() {
        let mut state = EditorState::new();
        let main = state.active_comp;
        let second = with_second_comp(&mut state);
        state.open_composition(main);
        let plan = studio_core::edit::plan_item_deletion(&state.project, &[second], main);
        state.delete_items(&plan);
        assert_eq!(state.tabs(), vec![main]);
    }

    #[test]
    fn a_new_project_starts_with_one_tab() {
        let mut state = EditorState::new();
        with_second_comp(&mut state);
        state.new_project(&[], crate::app_settings::AppSettings::default().new_composition);
        assert_eq!(state.tabs(), vec![state.active_comp]);
    }
}
