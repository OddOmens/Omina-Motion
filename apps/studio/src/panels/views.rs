//! Each panel is its own cached GPUI view, so a frame only re-renders the panels
//! whose content changed. Edits notify the workspace, which every panel observes;
//! playback ticks bypass the workspace and refresh only the panels that show time.


use gpui_kit::{
    AnyElement, Context, Entity, EntityId, IntoElement, Render, Styled as _, WeakEntity, Window, div,
};

use crate::workspace::Workspace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Toolbar,
    Project,
    Viewer,
    Timeline,
    Properties,
    Effects,
    Libraries,
    Align,
    Templates,
    Captions,
}

/// A panel view: renders one panel from the shared workspace state.
pub struct PanelView {
    workspace: WeakEntity<Workspace>,
    kind: PanelKind,
    /// How many times this panel has rendered, so tests can check that caching works.
    #[cfg(test)]
    pub renders: usize,
}

impl PanelView {
    pub fn new(workspace: &Entity<Workspace>, kind: PanelKind, cx: &mut Context<Self>) -> Self {
        // Any edit notifies the workspace; re-render this panel when that happens.
        cx.observe(workspace, |_, _, cx| cx.notify()).detach();
        Self {
            workspace: workspace.downgrade(),
            kind,
            #[cfg(test)]
            renders: 0,
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
        self.workspace
            .update(cx, |workspace, cx| -> AnyElement {
                use super::*;
                match kind {
                    PanelKind::Toolbar => toolbar::render(workspace, cx).into_any_element(),
                    PanelKind::Project => project::render(workspace, cx).into_any_element(),
                    PanelKind::Viewer => viewer::render(workspace, window, cx).into_any_element(),
                    PanelKind::Timeline => timeline::render(workspace, window, cx).into_any_element(),
                    PanelKind::Properties => properties::render(workspace, cx).into_any_element(),
                    PanelKind::Effects => effects::render(workspace, cx).into_any_element(),
                    PanelKind::Libraries => libraries::render(workspace, cx).into_any_element(),
                    PanelKind::Align => align::render(workspace, cx).into_any_element(),
                    PanelKind::Templates => templates::render(workspace, cx).into_any_element(),
                    PanelKind::Captions => captions::render(workspace, cx).into_any_element(),
                }
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// Drives playback. It renders nothing; while playing it advances the playhead from the
/// clock each frame and refreshes only the timed panels, leaving the rest cached.
pub struct PlaybackClock {
    workspace: WeakEntity<Workspace>,
    timed: Vec<EntityId>,
    /// The Project panel, refreshed while its preview plays.
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
