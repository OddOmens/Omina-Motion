//! Each panel is its own cached GPUI view, so a frame only re-renders the panels
//! whose content changed. Edits notify the workspace, which every panel observes;
//! playback ticks bypass the workspace and refresh only the panels that show time.


use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::dock::{BasePanel, PanelEvent};
use gpui_kit::{
    AnyElement, App, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Styled as _,
    WeakEntity, Window, div,
};

use crate::workspace::{LeftPanel, PanelId, Workspace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Toolbar,
    /// The panel beside the rail: Media, Effects, Libraries, Stock and so on, whichever the rail picked.
    Side,
    Viewer,
    Timeline,
    Properties,
    Align,
    Nodes,
    Mixer,
}

impl PanelKind {
    /// The dockable panel this view draws; `None` for the toolbar, which stays fixed at the top.
    pub fn dock_id(self) -> Option<PanelId> {
        Some(match self {
            PanelKind::Toolbar => return None,
            PanelKind::Side => PanelId::Side,
            PanelKind::Viewer => PanelId::Composition,
            PanelKind::Timeline => PanelId::Timeline,
            PanelKind::Properties => PanelId::Properties,
            PanelKind::Align => PanelId::Align,
            PanelKind::Nodes => PanelId::Nodes,
            PanelKind::Mixer => PanelId::Mixer,
        })
    }
}

/// A panel view: renders one panel from the shared workspace state.
pub struct PanelView {
    workspace: WeakEntity<Workspace>,
    pub kind: PanelKind,
    /// The editor's own focus. A panel picked in the dock hands focus back to it, so keyboard
    /// shortcuts keep working wherever the panel has been moved.
    focus: FocusHandle,
    /// Whether the panel is on at the moment (one turned off in Settings ▸ Features, or the
    /// Nodes panel while it is closed, is not). The workspace sets it before the dock draws, so
    /// the dock can leave hidden panels out without reading the workspace mid-update.
    pub shown: Rc<Cell<bool>>,
    /// How many times this panel has rendered, so tests can check that caching works.
    #[cfg(test)]
    pub renders: usize,
    /// How long its renders took in all, for the stress benchmark.
    #[cfg(test)]
    pub render_time: std::time::Duration,
}

impl PanelView {
    pub fn new(workspace: &Entity<Workspace>, kind: PanelKind, focus: FocusHandle, cx: &mut Context<Self>) -> Self {
        // Any edit notifies the workspace; re-render this panel when that happens.
        cx.observe(workspace, |_, _, cx| cx.notify()).detach();
        Self {
            workspace: workspace.downgrade(),
            kind,
            focus,
            shown: Rc::new(Cell::new(true)),
            #[cfg(test)]
            renders: 0,
            #[cfg(test)]
            render_time: std::time::Duration::ZERO,
        }
    }
}

impl Render for PanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        #[cfg(test)]
        {
            self.renders += 1;
        }
        #[cfg(test)]
        let started = std::time::Instant::now();
        let element = self.workspace
            .update(cx, |workspace, cx| -> AnyElement {
                use super::*;
                match kind {
                    PanelKind::Toolbar => toolbar::render(workspace, cx).into_any_element(),
                    // The side panel shows whichever panel the rail picked.
                    PanelKind::Side => match workspace.ui.left_panel {
                        LeftPanel::Media => project::render(workspace, cx).into_any_element(),
                        LeftPanel::Effects => effects::render(workspace, cx).into_any_element(),
                        LeftPanel::Libraries => libraries::render(workspace, cx).into_any_element(),
                        LeftPanel::Templates => templates::render(workspace, cx).into_any_element(),
                        LeftPanel::Captions => captions::render(workspace, cx).into_any_element(),
                        LeftPanel::Elements => elements::render(workspace, cx).into_any_element(),
                        LeftPanel::Mockups => mockups::render(workspace, cx).into_any_element(),
                        LeftPanel::Stock => stock::render(workspace, cx).into_any_element(),
                        LeftPanel::History => history::render(workspace, cx).into_any_element(),
                        LeftPanel::Assistant => assistant::render(workspace, cx).into_any_element(),
                    },
                    PanelKind::Viewer => viewer::render(workspace, window, cx).into_any_element(),
                    PanelKind::Timeline => timeline::render(workspace, window, cx).into_any_element(),
                    PanelKind::Properties => properties::render(workspace, cx).into_any_element(),
                    PanelKind::Align => align::render(workspace, cx).into_any_element(),
                    PanelKind::Nodes => nodes::render(workspace, cx).into_any_element(),
                    PanelKind::Mixer => mixer::render(workspace, cx).into_any_element(),
                }
            })
            .unwrap_or_else(|_| div().into_any_element());
        #[cfg(test)]
        {
            self.render_time += started.elapsed();
        }
        element
    }
}

impl EventEmitter<PanelEvent> for PanelView {}

impl Focusable for PanelView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// How the dock sees a panel. Panels are never closed (each always has a place in the layout),
/// and are hidden instead while they are turned off. Any panel can fill the window for a
/// while (Maximize Panel, in the menu on its tab's grip).
impl BasePanel for PanelView {
    fn panel_name(&self) -> &'static str {
        self.kind.dock_id().map_or("Toolbar", PanelId::name)
    }

    fn visible(&self, _: &App) -> bool {
        self.shown.get()
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        true
    }
}

/// Drives playback. It renders nothing; while playing it advances the playhead from the
/// clock each frame and refreshes only the timed panels, leaving the rest cached.
pub struct PlaybackClock {
    workspace: WeakEntity<Workspace>,
    timed: Vec<EntityId>,
    /// The side panel, refreshed while the Project panel's preview plays in it.
    project: EntityId,
}

impl PlaybackClock {
    pub fn new(workspace: WeakEntity<Workspace>, timed: Vec<EntityId>, project: EntityId) -> Self {
        Self { workspace, timed, project }
    }
}

impl Render for PlaybackClock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Deliberately does not notify the workspace: that would refresh every panel.
        let (was_playing, playing) = self
            .workspace
            .update(cx, |workspace, _| (workspace.state.is_playing(), workspace.tick_playback()))
            .unwrap_or((false, false));
        // Refresh on every playing frame, and once more when playback stops so the Play button resets.
        if was_playing {
            let app: &mut gpui_kit::App = cx;
            for &panel in &self.timed {
                app.notify(panel);
            }
        }
        // The Project panel's preview plays on the same clock.
        let (was_previewing, previewing) =
            self.workspace.update(cx, |workspace, _| (workspace.ui.preview.playing.is_some(), workspace.tick_preview())).unwrap_or((false, false));
        if was_previewing {
            let app: &mut gpui_kit::App = cx;
            app.notify(self.project);
        }
        if playing || previewing {
            window.request_animation_frame();
        }
        div().size_0()
    }
}
