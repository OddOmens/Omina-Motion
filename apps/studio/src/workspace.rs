//! The editor window: owns the editor state, wires keyboard shortcuts, playback
//! and in-place value editing, and lays out the panels like After Effects.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    App, AppContext as _, Context, DragMoveEvent, Entity, FocusHandle, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, StyleRefinement, Styled, Subscription, Window, actions, div,
    prelude::FluentBuilder as _, px,
};
use studio_core::animation::CubicBezier;
use studio_core::model::Id;

use crate::panels;
use crate::panels::fields::{FieldDrag, FieldTarget, parse_value};
use crate::panels::views::{PanelKind, PanelView, PlaybackClock};
use crate::sound::Sound;
use crate::state::EditorState;
use crate::theme::{color, hsla};

actions!(studio, [
    TogglePlayback,
    NextFrame,
    PreviousFrame,
    NextTenFrames,
    PreviousTenFrames,
    FirstFrame,
    LastFrame,
    Delete,
    EasyEase,
    SelectionTool,
    TypeTool,
    ShapeTool,
    CutTool,
    PenTool,
    FinishPath,
    HandTool,
    ZoomTool,
    RotateTool,
    RippleTool,
    RollTool,
    SlipTool,
    SlideTool,
    TrackSelectTool,
    Copy,
    Paste,
    Duplicate,
    OpenSettings,
    Precompose,
    CompositionSettings,
    NewComposition,
    NewProject,
    Import,
    Undo,
    Redo,
    Save,
    SaveAs,
    Open,
    Export,
    TrimInToPlayhead,
    TrimOutToPlayhead,
    SplitLayer,
    RippleDelete,
    ToggleSnapping,
    Quit
]);

/// Key context the editor's shortcuts are bound in.
const KEY_CONTEXT: &str = "Studio";
/// Where shortcuts that are plain keys (Space, letters, Delete, Alt+key) apply: anywhere in the
/// editor except while a text field has focus. On macOS a key binding wins over typing, so
/// without this, typing "s" or a space into a field would toggle snapping or start playback.
const NOT_TYPING: &str = "Studio && !Input";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("space", TogglePlayback, Some(NOT_TYPING)),
        KeyBinding::new("pagedown", NextFrame, Some(NOT_TYPING)),
        KeyBinding::new("pageup", PreviousFrame, Some(NOT_TYPING)),
        KeyBinding::new("shift-pagedown", NextTenFrames, Some(NOT_TYPING)),
        KeyBinding::new("shift-pageup", PreviousTenFrames, Some(NOT_TYPING)),
        KeyBinding::new("home", FirstFrame, Some(NOT_TYPING)),
        KeyBinding::new("end", LastFrame, Some(NOT_TYPING)),
        KeyBinding::new("delete", Delete, Some(NOT_TYPING)),
        KeyBinding::new("backspace", Delete, Some(NOT_TYPING)),
        KeyBinding::new("f9", EasyEase, Some(NOT_TYPING)),
        KeyBinding::new("v", SelectionTool, Some(NOT_TYPING)),
        KeyBinding::new("cmd-t", TypeTool, Some(KEY_CONTEXT)),
        KeyBinding::new("q", ShapeTool, Some(NOT_TYPING)),
        KeyBinding::new("c", CutTool, Some(NOT_TYPING)),
        KeyBinding::new("g", PenTool, Some(NOT_TYPING)),
        // The rest of the tools, on After Effects' and Premiere's keys.
        KeyBinding::new("h", HandTool, Some(NOT_TYPING)),
        KeyBinding::new("z", ZoomTool, Some(NOT_TYPING)),
        KeyBinding::new("w", RotateTool, Some(NOT_TYPING)),
        KeyBinding::new("b", RippleTool, Some(NOT_TYPING)),
        KeyBinding::new("n", RollTool, Some(NOT_TYPING)),
        KeyBinding::new("y", SlipTool, Some(NOT_TYPING)),
        KeyBinding::new("u", SlideTool, Some(NOT_TYPING)),
        KeyBinding::new("a", TrackSelectTool, Some(NOT_TYPING)),
        KeyBinding::new("enter", FinishPath, Some(NOT_TYPING)),
        // Outside text fields, so copying and pasting text in a field still works.
        KeyBinding::new("cmd-c", Copy, Some(NOT_TYPING)),
        KeyBinding::new("cmd-v", Paste, Some(NOT_TYPING)),
        KeyBinding::new("cmd-d", Duplicate, Some(NOT_TYPING)),
        KeyBinding::new("cmd-shift-c", Precompose, Some(NOT_TYPING)),
        KeyBinding::new("cmd-k", CompositionSettings, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-,", OpenSettings, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-n", NewComposition, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-alt-n", NewProject, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-i", Import, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-z", Undo, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-shift-z", Redo, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-s", Save, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-shift-s", SaveAs, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-o", Open, Some(KEY_CONTEXT)),
        // ⌘M, as After Effects' Add to Render Queue.
        KeyBinding::new("cmd-m", Export, Some(KEY_CONTEXT)),
        // As in After Effects: trim the selected layers' in or out point to the playhead.
        KeyBinding::new("alt-[", TrimInToPlayhead, Some(NOT_TYPING)),
        KeyBinding::new("alt-]", TrimOutToPlayhead, Some(NOT_TYPING)),
        // After Effects' Split Layer.
        KeyBinding::new("cmd-shift-d", SplitLayer, Some(KEY_CONTEXT)),
        // Premiere's Ripple Delete.
        KeyBinding::new("shift-delete", RippleDelete, Some(NOT_TYPING)),
        KeyBinding::new("shift-backspace", RippleDelete, Some(NOT_TYPING)),
        // Premiere's snapping toggle.
        KeyBinding::new("s", ToggleSnapping, Some(NOT_TYPING)),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    // The editor window handles Quit itself, to offer saving first. This only runs when no
    // window does (none is open).
    cx.on_action(|_: &Quit, cx| cx.quit());
}

/// The menu bar. Shortcuts shown in it come from the key bindings above.
pub fn menus() -> Vec<gpui_kit::Menu> {
    use gpui_kit::{Menu, MenuItem};
    vec![
        Menu::new("Studio").items([MenuItem::action("Settings…", OpenSettings), MenuItem::separator(), MenuItem::action("Quit Studio", Quit)]),
        Menu::new("File").items([
            MenuItem::action("New Project", NewProject),
            MenuItem::action("Open…", Open),
            MenuItem::action("Save", Save),
            MenuItem::action("Save As…", SaveAs),
            MenuItem::separator(),
            MenuItem::action("Export…", Export),
            MenuItem::separator(),
            MenuItem::action("Import…", Import),
            MenuItem::action("New Composition…", NewComposition),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", Undo),
            MenuItem::action("Redo", Redo),
            MenuItem::separator(),
            MenuItem::action("Copy", Copy),
            MenuItem::action("Paste", Paste),
            MenuItem::action("Duplicate", Duplicate),
            MenuItem::action("Pre-compose", Precompose),
            MenuItem::separator(),
            MenuItem::action("Trim In Point to Playhead", TrimInToPlayhead),
            MenuItem::action("Trim Out Point to Playhead", TrimOutToPlayhead),
            MenuItem::action("Split Layer", SplitLayer),
            MenuItem::action("Ripple Delete", RippleDelete),
            MenuItem::separator(),
            MenuItem::action("Snapping", ToggleSnapping),
        ]),
        Menu::new("Composition").items([MenuItem::action("Composition Settings…", CompositionSettings)]),
    ]
}

/// What the panel beside the rail shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftPanel {
    /// The project's media and compositions.
    Media,
    /// Effects & Presets, to drag onto clips.
    Effects,
    /// Brands' shared colors and text styles.
    Libraries,
    /// Ready-made animated compositions.
    Templates,
    /// Making captions from speech, and caption looks.
    Captions,
}

/// Panels that can hold focus; the focused one gets a blue outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelId {
    Project,
    Composition,
    Timeline,
    /// Holds the Properties and Effect Controls tabs.
    Properties,
    /// Holds the Effects & Presets and Flow tabs.
    Effects,
    Libraries,
    Align,
    Templates,
    Captions,
}

/// The editing tools on the rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Selection,
    /// Click in the viewer to type a new text layer.
    Type,
    /// Drag in the viewer to draw a shape layer (the kind is `UiState::shape_kind`).
    Shape,
    /// Click a clip in the timeline to split it there, like Premiere's Razor tool.
    Cut,
    /// Click (or drag, for curves) point by point in the viewer to draw a custom shape.
    Pen,
    /// Drag the viewer to look around a zoomed-in picture.
    Hand,
    /// Click the viewer to zoom in there; Option-click to zoom out.
    Zoom,
    /// Drag in the viewer to turn the selected layers about their anchor points.
    Rotate,
    /// Drag a clip's end to trim it, moving later clips on its track with it.
    Ripple,
    /// Drag the cut between two touching clips to move it.
    Roll,
    /// Drag a clip to change which part of its source plays.
    Slip,
    /// Drag a clip between its neighbors, which stretch to meet it.
    Slide,
    /// Click a clip to pick it and everything after it, then drag them together.
    TrackSelect,
}

/// Drag payloads. GPUI routes drops by type, so each kind of drag has its own type.
#[derive(Clone, Debug)]
pub struct ProjectItemDrag {
    pub item: Id,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct EffectDrag {
    pub name: String,
}

/// What an in-place text input is editing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditTarget {
    Number(FieldTarget),
    SourceText(Id),
    /// The name of a library brand, color or text style.
    LibraryName(studio_core::model::LinkId),
    /// The text of a caption layer's words in a range (one phrase).
    CaptionText(Id, usize, usize),
}

/// The text input currently open over a value, and the subscription that commits it.
pub struct EditingField {
    pub target: EditTarget,
    pub input: Entity<InputState>,
    /// Shown over the layer in the viewer (a double-click there) rather than in its panel.
    pub in_viewer: bool,
    _events: Subscription,
}

/// UI-only state that is not part of the project or the editing model.
pub struct UiState {
    pub active_panel: PanelId,
    /// What the panel beside the rail shows.
    pub left_panel: LeftPanel,
    pub tool: Tool,
    /// The shape the Shape tool draws, picked from the rail's shape menu.
    pub shape_kind: studio_core::model::ShapeKind,
    /// Whether the rail's shape menu is open (press and hold the Shape button).
    pub shape_menu: bool,
    /// Which rail tool group's menu is open (an index into `toolbar::TOOL_GROUPS`).
    pub tool_menu: Option<usize>,
    /// The tool group button being held, to open its menu after a moment.
    pub tool_press: Option<usize>,
    /// The tool each group's button last picked, which it shows and picks on a click.
    pub group_tools: HashMap<usize, Tool>,
    /// Whether the Shape button is held down, so a long press can open the menu.
    pub shape_press: bool,
    /// Active tab index for panels with tabs.
    pub tabs: HashMap<PanelId, usize>,
    pub collapsed_folders: HashSet<Id>,
    /// Panels in the right column collapsed to their header.
    pub collapsed_panels: HashSet<PanelId>,
    /// Library brands the user folded away (all are open to begin with).
    pub closed_brands: HashSet<studio_core::model::LinkId>,
    /// Effect categories the user folded away (all are open to begin with).
    pub closed_effect_categories: HashSet<&'static str>,
    pub viewer_bounds: panels::chrome::MeasuredBounds,
    /// Window-space bounds of the composition frame inside the viewer.
    pub frame_bounds: panels::chrome::MeasuredBounds,
    /// Pointer position where the current viewer drag started.
    pub viewer_anchor: Option<(f32, f32)>,
    /// The viewer's magnification (screen points per composition pixel); `None` fits the frame.
    pub viewer_zoom: Option<f32>,
    /// How far the frame is moved from the middle of the viewer, in screen points (Hand tool).
    pub viewer_pan: (f32, f32),
    /// The pan when a Hand drag began.
    pub pan_start: (f32, f32),
    /// Pointer x and starting value of the field being scrubbed.
    pub field_anchor: Option<(f32, f32)>,
    pub editing: Option<EditingField>,
    /// The last curve used in the graph editor, shown when the selection has none of its own.
    pub flow_curve: CubicBezier,
    /// Where the graph editor popover is open (below the Easings button), if it is.
    pub graph_editor: Option<gpui_kit::Point<gpui_kit::Pixels>>,
    /// Font families offered in the Text section, looked up once at startup.
    pub font_families: Vec<&'static str>,
    /// The Templates panel's chosen shape and category, and the pictures made of templates.
    pub template_format: studio_core::templates::Format,
    pub template_category: Option<studio_core::templates::Category>,
    /// The brand whose colors and font templates take, if one is chosen.
    pub template_brand: Option<studio_core::model::LinkId>,
    pub template_pictures: HashMap<(usize, studio_core::templates::Format), Arc<gpui_kit::RenderImage>>,
    /// The Project panel's preview of the selected item.
    pub preview: panels::project::Preview,
    /// Where the preview box is on screen, for scrubbing.
    pub preview_bounds: panels::chrome::MeasuredBounds,
    /// Settings the panels read as they draw (kept in step by the Settings dialog).
    pub settings: crate::app_settings::AppSettings,
    /// Which columns the Project panel shows (from Settings).
    pub project_columns: crate::app_settings::ProjectColumns,
    pub timeline: panels::timeline::TimelineUi,
    /// The window title last set, so it is only changed when it needs to be.
    pub window_title: String,
    /// The Export dialog's choices, and the export running now, if any.
    pub export_choice: panels::export::ExportChoice,
    pub export_job: Option<panels::export::ExportJob>,
    /// While an effect is dragged over the viewer, the layer it would be dropped on.
    pub effect_drop_layer: Option<Id>,
    /// Captions being made in the background, if any.
    pub caption_job: Option<panels::captions::CaptionJob>,
    /// Silence removal, beat finding, tracking or reframing running in the background, if any.
    pub analysis_job: Option<panels::analysis::AnalysisJob>,
    /// The layer waiting for a click on a video to track (Track Motion).
    pub track_pick: Option<Id>,
    /// Background removal running in the background, if any.
    pub cutout_job: Option<panels::cutout::CutoutJob>,
    /// The layer whose subject is being picked: clicks in the viewer go to its cut-out.
    pub cutout_pick: Option<Id>,
    /// The color picker, while it is open.
    pub color_picker: Option<panels::color_picker::Picker>,
    /// Keeps the open dialog's field links alive (Composition Settings' locked aspect ratio).
    pub dialog_subscriptions: Vec<Subscription>,
    /// Set when the user chose to quit while exporting: quit once the export has stopped.
    pub quit_after_export: bool,
}

impl UiState {
    pub fn tab(&self, panel: PanelId) -> usize {
        self.tabs.get(&panel).copied().unwrap_or(0)
    }
}

pub struct Workspace {
    pub state: EditorState,
    pub ui: UiState,
    /// The GPU renderer and the frame the viewer shows.
    pub render: panels::viewer::ViewerRender,
    /// The composition's sound, kept in step with playback.
    sound: Sound,
    focus: FocusHandle,
    panels: Panels,
}

/// The cached panel views and the playback clock (see `panels::views`).
struct Panels {
    toolbar: Entity<PanelView>,
    project: Entity<PanelView>,
    viewer: Entity<PanelView>,
    timeline: Entity<PanelView>,
    properties: Entity<PanelView>,
    effects: Entity<PanelView>,
    libraries: Entity<PanelView>,
    align: Entity<PanelView>,
    templates: Entity<PanelView>,
    captions: Entity<PanelView>,
    clock: Entity<PlaybackClock>,
}

impl Panels {
    fn new(cx: &mut Context<Workspace>) -> Self {
        let workspace = cx.entity();
        let mut view = |kind| cx.new(|cx| PanelView::new(&workspace, kind, cx));
        let (toolbar, project, viewer, timeline) =
            (view(PanelKind::Toolbar), view(PanelKind::Project), view(PanelKind::Viewer), view(PanelKind::Timeline));
        let (properties, effects, align) = (view(PanelKind::Properties), view(PanelKind::Effects), view(PanelKind::Align));
        let libraries = view(PanelKind::Libraries);
        let (templates, captions) = (view(PanelKind::Templates), view(PanelKind::Captions));
        let timed = vec![viewer.entity_id(), timeline.entity_id(), properties.entity_id()];
        let clock = cx.new(|_| PlaybackClock::new(workspace.downgrade(), timed, project.entity_id()));
        Self { toolbar, project, viewer, timeline, properties, effects, libraries, align, templates, captions, clock }
    }
}

#[cfg(test)]
impl Workspace {
    /// How many times each panel has rendered, in `PanelKind` order: toolbar, project, viewer,
    /// timeline, properties, effects, libraries, align.
    pub fn panel_render_counts(&self, cx: &App) -> [usize; 10] {
        let p = &self.panels;
        [&p.toolbar, &p.project, &p.viewer, &p.timeline, &p.properties, &p.effects, &p.libraries, &p.align, &p.templates, &p.captions].map(|view| view.read(cx).renders)
    }
}

/// Embeds a panel view, reusing last frame's rendering unless the panel was notified.
fn cached(view: &Entity<PanelView>) -> impl IntoElement + use<> {
    view.clone().cached(StyleRefinement::default().size_full())
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // Closing the window with unsaved changes, or during an export, asks first (and quits
        // once answered).
        let workspace = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            workspace
                .update(cx, |this, cx| {
                    let busy = this.state.has_unsaved_changes() || this.ui.export_job.is_some();
                    if busy {
                        this.quit(window, cx);
                    }
                    !busy
                })
                .unwrap_or(true)
        });
        let render = panels::viewer::ViewerRender::new();
        let timeline_ui = panels::timeline::TimelineUi::default();
        let panels = Panels::new(cx);
        // Background decoders signal here when a frame is ready; redraw only the viewer.
        let (frame_ready, frames_ready) = async_channel::bounded::<()>(1);
        render.media.set_on_ready(move || {
            // A full channel already means "redraw pending", so extra signals can be dropped.
            let _ = frame_ready.try_send(());
        });
        let viewer = panels.viewer.entity_id();
        let project_panel = panels.project.entity_id();
        // The loop ends when the frame server (which holds the sender) is dropped. The Project
        // panel's preview shows frames from it too.
        cx.spawn(async move |_, cx| {
            while frames_ready.recv().await.is_ok() {
                cx.update(|cx| {
                    cx.notify(viewer);
                    cx.notify(project_panel);
                });
            }
        })
        .detach();
        // The proxy builder signals here as a proxy's status changes (queued, progress, ready,
        // failed): refresh the Project panel, which shows it, and the viewer, which can then
        // switch to the finished proxy. (Headless tests leave this out: their scheduler refuses
        // wake-ups from the proxy builder's thread.)
        #[cfg(not(test))]
        {
            let (proxy_changed, proxies_changed) = async_channel::bounded::<()>(1);
            render.proxies.set_on_change(move || {
                let _ = proxy_changed.try_send(());
            });
            let project_panel = panels.project.entity_id();
            cx.spawn(async move |_, cx| {
                while proxies_changed.recv().await.is_ok() {
                    cx.update(|cx| {
                        cx.notify(project_panel);
                        cx.notify(viewer);
                    });
                }
            })
            .detach();
        }
        // Thumbnails and waveforms decoded in the background redraw the timeline as they arrive.
        // (Headless tests leave this out: their scheduler refuses wake-ups from other threads.)
        #[cfg(not(test))]
        {
            let (media_ready, media_arrived) = async_channel::bounded::<()>(1);
            let (thumbnail_ready, peaks_ready) = (media_ready.clone(), media_ready);
            let ui = &timeline_ui;
            ui.media.thumbnails.set_on_ready(move || {
                let _ = thumbnail_ready.try_send(());
            });
            ui.media.peaks.set_on_ready(move || {
                let _ = peaks_ready.try_send(());
            });
            let (timeline, project_panel) = (panels.timeline.entity_id(), panels.project.entity_id());
            // Waveforms show in the Project panel's preview too.
            cx.spawn(async move |_, cx| {
                while media_arrived.recv().await.is_ok() {
                    cx.update(|cx| {
                        cx.notify(timeline);
                        cx.notify(project_panel);
                    });
                }
            })
            .detach();
        }
        // Pick up where the last session left off, and keep a recovery copy of unsaved work.
        // (Not in headless tests, which start fresh and must not touch real settings.)
        #[cfg(not(test))]
        {
            // Libraries first, so the restored project's linked colors come up to date.
            let loaded = crate::library_store::load();
            cx.defer_in(window, move |this, window, cx| {
                this.state.set_libraries(loaded.libraries);
                if let Some(notice) = loaded.notice {
                    window.push_notification(notice.as_str(), cx);
                }
            });
            cx.defer_in(window, |this, window, cx| this.restore_session(window, cx));
            cx.spawn(async move |this, cx| {
                loop {
                    // As often as Settings says; checked again each time, so a change applies.
                    let seconds = crate::app_settings::load().recovery_seconds;
                    let wait = if seconds == 0 { crate::project_files::RECOVERY_INTERVAL } else { std::time::Duration::from_secs(u64::from(seconds)) };
                    cx.background_executor().timer(wait).await;
                    if seconds == 0 {
                        continue;
                    }
                    if this.update(cx, |this, _| this.keep_recovery_copy()).is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        // Headless tests stay silent so they never depend on (or use) a sound device.
        #[cfg(not(test))]
        let sound = Sound::new();
        #[cfg(test)]
        let sound = Sound::silent();
        Self {
            state: EditorState::new(),
            ui: UiState {
                active_panel: PanelId::Timeline,
                left_panel: LeftPanel::Media,
                tool: Tool::Selection,
                shape_kind: studio_core::model::ShapeKind::Rectangle,
                shape_menu: false,
                tool_menu: None,
                tool_press: None,
                group_tools: HashMap::new(),
                shape_press: false,
                tabs: HashMap::new(),
                collapsed_folders: HashSet::new(),
                closed_effect_categories: HashSet::new(),
                closed_brands: HashSet::new(),
                collapsed_panels: HashSet::new(),
                viewer_bounds: Default::default(),
                frame_bounds: Default::default(),
                viewer_anchor: None,
                viewer_zoom: None,
                viewer_pan: (0.0, 0.0),
                pan_start: (0.0, 0.0),
                field_anchor: None,
                editing: None,
                flow_curve: studio_core::animation::EASING_PRESETS[0].1,
                graph_editor: None,
                font_families: panels::properties::installed_font_families(cx),
                project_columns: crate::app_settings::load().project_columns,
                settings: crate::app_settings::load(),
                preview: Default::default(),
                template_format: studio_core::templates::Format::Vertical,
                template_category: None,
                template_brand: None,
                template_pictures: HashMap::new(),
                preview_bounds: Default::default(),
                timeline: timeline_ui,
                window_title: String::new(),
                export_choice: Default::default(),
                export_job: None,
                dialog_subscriptions: Vec::new(),
                color_picker: None,
                caption_job: None,
                cutout_job: None,
                analysis_job: None,
                track_pick: None,
                cutout_pick: None,
                effect_drop_layer: None,
                quit_after_export: false,
            },
            focus,
            render,
            sound,
            panels,
        }
    }

    /// Shows the project's name in the title bar, with a dot (and macOS's edited marker on the
    /// close button) while there are unsaved changes.
    fn update_window_title(&mut self, window: &mut Window) {
        let unsaved = self.state.has_unsaved_changes();
        let title = format!("{}{} — Studio", self.state.document_name(), if unsaved { " •" } else { "" });
        if title != self.ui.window_title {
            window.set_window_title(&title);
            window.set_window_edited(unsaved);
            self.ui.window_title = title;
        }
    }

    /// Splits layers at the playhead, explaining any that had to be left whole.
    fn split_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let report = self.state.split_at_playhead();
        let mut reasons = Vec::new();
        if report.locked > 0 {
            reasons.push(format!("{} locked", plural(report.locked, "layer is", "layers are")));
        }
        if report.matte_not_spanning > 0 {
            reasons.push(format!(
                "{} a track matte that does not reach the playhead",
                plural(report.matte_not_spanning, "layer shares", "layers share")
            ));
        }
        if !reasons.is_empty() {
            window.push_notification(format!("Not split: {}.", reasons.join("; ")).as_str(), cx);
        }
        if !report.created.is_empty() {
            cx.notify();
        }
    }

    /// Undo or redo, then refresh every panel.
    fn undo_or_redo(&mut self, redo: bool, cx: &mut Context<Self>) {
        let changed = if redo { self.state.redo() } else { self.state.undo() };
        if changed {
            self.request_proxies(); // An undone deletion brings its media back.
            cx.notify();
        }
    }

    pub fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        self.state.toggle_playback(Instant::now());
        self.sound.sync(&mut self.state);
        cx.notify();
    }

    /// Plays or pauses the Project panel's preview of the selected item: video (with its sound)
    /// or sound. The composition stops playing first.
    pub fn toggle_preview(&mut self) {
        if self.ui.preview.playing.is_some() {
            self.stop_preview();
            return;
        }
        let Some(item) = self.state.selected_item.and_then(|id| self.state.project.item(id)).cloned() else { return };
        let (Some(path), Some(length)) = (item.path.clone(), item.duration_seconds) else { return };
        if self.state.is_playing() {
            self.state.toggle_playback(Instant::now());
            self.sound.sync(&mut self.state);
        }
        // From the start again once it has played to the end.
        if self.ui.preview.seconds >= length - 0.05 {
            self.ui.preview.seconds = 0.0;
        }
        let from = self.ui.preview.seconds;
        if item.has_audio {
            self.sound.play_preview(&path, from, length);
        }
        self.ui.preview.playing = Some((Instant::now(), from));
    }

    pub fn stop_preview(&mut self) {
        self.sound.stop_preview();
        self.ui.preview.playing = None;
    }

    /// Moves the playing preview on to now: by the sound heard, or by the clock for video with
    /// no sound. Returns whether it is still playing.
    pub fn tick_preview(&mut self) -> bool {
        let Some((started, from)) = self.ui.preview.playing else { return false };
        let length = self.ui.preview.item.and_then(|id| self.state.project.item(id)).and_then(|item| item.duration_seconds).unwrap_or(0.0);
        let elapsed = self.sound.preview_heard().unwrap_or_else(|| started.elapsed()).as_secs_f64();
        self.ui.preview.seconds = (from + elapsed).min(length);
        if self.ui.preview.seconds >= length {
            self.stop_preview();
            return false;
        }
        true
    }

    /// Advances the playhead for one display frame. With sound, it follows the time actually
    /// heard; otherwise the wall clock. Returns whether playback is still running.
    pub fn tick_playback(&mut self) -> bool {
        let playing = match self.sound.sync(&mut self.state) {
            Some(heard) => self.state.advance_playback(heard),
            None => self.state.tick_playback(Instant::now()),
        };
        if !playing {
            self.sound.sync(&mut self.state); // Reached the end: stop the sound too.
        }
        playing
    }

    fn step(&mut self, frames: i64, cx: &mut Context<Self>) {
        self.state.set_current_frame(self.state.current_frame + frames);
        cx.notify();
    }

    pub fn go_to(&mut self, frame: i64, cx: &mut Context<Self>) {
        self.state.set_current_frame(frame);
        cx.notify();
    }

    /// Opens a text input over a value. Enter or clicking elsewhere commits it.
    /// Switches tools. A pen path still being drawn is finished as an open line first, so
    /// changing tools never throws away what was drawn.
    pub fn set_tool(&mut self, tool: Tool) {
        if tool != Tool::Pen {
            self.state.finish_pen(false);
        }
        self.ui.tool = tool;
    }

    pub fn begin_edit(&mut self, target: EditTarget, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_edit(window, cx);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        let events = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit_edit(window, cx);
            }
        });
        self.ui.editing = Some(EditingField { target, input, in_viewer: false, _events: events });
        cx.notify();
    }

    /// Applies and closes the open text input, if any. Invalid numbers are ignored.
    pub fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.ui.editing.take() else { return };
        let text = editing.input.read(cx).value().to_string();
        match editing.target {
            EditTarget::Number(target) => {
                if let Some(value) = parse_value(&text) {
                    target.write(&mut self.state, value);
                }
            }
            EditTarget::SourceText(layer) => self.state.set_text(layer, &text),
            EditTarget::LibraryName(id) => self.state.rename_library_item(id, &text),
            EditTarget::CaptionText(layer, start, end) => self.state.set_caption_text(layer, start..end, &text),
        }
        // Hand keyboard focus back to the editor so shortcuts work again.
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Starts building a proxy for every footage item that does not have one yet.
    pub fn request_proxies(&self) {
        // Proxies can be turned off in Settings, to save disk space.
        if !crate::app_settings::load().build_proxies {
            return;
        }
        for item in &self.state.project.items {
            if let (studio_core::model::ProjectItemKind::Footage, Some(path)) = (item.kind, &item.path) {
                self.render.proxies.request(path);
            }
        }
    }

    /// Opens the system file picker and adds the chosen images and video to the project.
    pub fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            this.update_in(cx, |this, window, cx| this.import_paths(paths, window, cx)).ok();
        })
        .detach();
    }

    /// Imports files (from the Import dialog, or dropped from the Finder) and returns the new
    /// items. Media is linked where it is on disk, and proxies start building.
    pub fn import_paths(&mut self, paths: Vec<std::path::PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Id> {
        let (imported, skipped) = self.state.import_paths(paths);
        self.request_proxies();
        if !skipped.is_empty() {
            let names: Vec<String> = skipped.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
            let message = format!("Not imported: {}. These are not images, video or sound that can be read.", names.join(", "));
            window.push_notification(message.as_str(), cx);
        }
        cx.notify();
        imported
    }

    /// Delete key. In the Project panel it asks before deleting the selected item (and the layers
    /// that use it); elsewhere it deletes selected keyframes, or else the selected layers.
    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.active_panel != PanelId::Project {
            self.state.delete_selection();
            cx.notify();
            return;
        }
        let Some(plan) = self.state.plan_item_deletion() else { return };
        if plan.includes_open_composition {
            window.push_notification("The open composition can’t be deleted.", cx);
            return;
        }
        let name = self.state.selected_item.and_then(|id| self.state.project.item(id)).map_or_else(String::new, |item| item.name.clone());
        let mut details = match plan.items.len() {
            1 => String::new(),
            count => format!("This deletes {count} items, including everything in the folder. "),
        };
        if plan.layers > 0 {
            details.push_str(&format!("{} layer{} using it will also be removed.", plan.layers, if plan.layers == 1 { "" } else { "s" }));
        }
        let workspace = cx.weak_entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let (workspace, plan) = (workspace.clone(), plan.clone());
            dialog
                .title(format!("Delete “{name}”?"))
                .description(format!("{details} You can undo this with ⌘Z.").trim_start().to_owned())
                .ok_text("Delete")
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    workspace
                        .update(cx, |this, cx| {
                            this.state.delete_items(&plan);
                            cx.notify();
                        })
                        .ok();
                    true
                })
        });
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every edit redraws the workspace, so this is where edits become undo steps, and where
        // manual stops (moving the playhead) and changes such as muting a layer reach the sound.
        // A different item selected in the Project panel starts its preview from the beginning.
        if self.ui.preview.item != self.state.selected_item {
            self.stop_preview();
            self.ui.preview.item = self.state.selected_item;
            self.ui.preview.seconds = 0.0;
        }
        // The Templates panel's pictures are made here, a few per frame, as it needs them.
        if self.ui.left_panel == LeftPanel::Templates {
            panels::templates::make_pictures(self, window, cx);
        }
        let picking = self.ui.color_picker.as_ref().is_some_and(|picker| picker.pressed);
        let dragging = cx.has_active_drag() || picking;
        self.state.record_history(dragging);
        // Library edits are saved once a drag (on the color wheel, say) is over.
        if !dragging && let Some(back_up) = self.state.take_library_save() {
            #[cfg(not(test))]
            if let Err(problem) = crate::library_store::save(&self.state.libraries, back_up) {
                window.push_notification(problem.as_str(), cx);
            }
            #[cfg(test)]
            let _ = back_up;
        }
        // A dialog closed with OK leaves nothing focused, which would silence every shortcut
        // until the next click; give focus back to the editor.
        if window.focused(cx).is_none() && !window.has_active_dialog(cx) {
            window.focus(&self.focus, cx);
        }
        self.sound.sync(&mut self.state);
        self.update_window_title(window);
        let panels = &self.panels;
        let gutter = px(crate::theme::size::GUTTER);
        let workspace = h_resizable("workspace-root")
            .child(
                resizable_panel().child(
                    v_resizable("workspace-main")
                        .child(
                            resizable_panel().size(px(520.0)).child(
                                h_resizable("workspace-top")
                                    .child(
                                        resizable_panel()
                                            .size(px(300.0))
                                            .size_range(px(220.0)..px(700.0))
                                            // Media or Effects, picked on the rail.
                                            .child(match self.ui.left_panel {
                                                LeftPanel::Media => cached(&panels.project),
                                                LeftPanel::Effects => cached(&panels.effects),
                                                LeftPanel::Libraries => cached(&panels.libraries),
                                                LeftPanel::Templates => cached(&panels.templates),
                                                LeftPanel::Captions => cached(&panels.captions),
                                            }),
                                    )
                                    .child(resizable_panel().child(cached(&panels.viewer))),
                            ),
                        )
                        .child(resizable_panel().size_range(px(180.0)..px(2000.0)).child(cached(&panels.timeline))),
                ),
            )
            .child(
                resizable_panel().size(px(430.0)).size_range(px(300.0)..px(800.0)).child(self.utility_column(window, cx)),
            );

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(color::BACKGROUND))
            .text_color(hsla(color::TEXT))
            .text_size(px(crate::theme::size::TEXT))
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|this, _: &TogglePlayback, _, cx| this.toggle_playback(cx)))
            .on_action(cx.listener(|this, _: &NextFrame, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousFrame, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &NextTenFrames, _, cx| this.step(10, cx)))
            .on_action(cx.listener(|this, _: &PreviousTenFrames, _, cx| this.step(-10, cx)))
            .on_action(cx.listener(|this, _: &FirstFrame, _, cx| this.go_to(0, cx)))
            .on_action(cx.listener(|this, _: &LastFrame, _, cx| {
                let last = this.state.composition().duration_frames - 1;
                this.go_to(last, cx);
            }))
            .on_action(cx.listener(|this, _: &Delete, window, cx| this.delete(window, cx)))
            .on_action(cx.listener(|this, _: &EasyEase, _, cx| {
                this.state.easy_ease_selected();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectionTool, _, cx| {
                this.set_tool(Tool::Selection);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TypeTool, _, cx| {
                this.set_tool(Tool::Type);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShapeTool, _, cx| {
                this.set_tool(Tool::Shape);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CutTool, _, cx| {
                this.set_tool(Tool::Cut);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PenTool, _, cx| {
                this.set_tool(Tool::Pen);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &HandTool, _, cx| {
                this.set_tool(Tool::Hand);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomTool, _, cx| {
                this.set_tool(Tool::Zoom);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RotateTool, _, cx| {
                this.set_tool(Tool::Rotate);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RippleTool, _, cx| {
                this.set_tool(Tool::Ripple);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RollTool, _, cx| {
                this.set_tool(Tool::Roll);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SlipTool, _, cx| {
                this.set_tool(Tool::Slip);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SlideTool, _, cx| {
                this.set_tool(Tool::Slide);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TrackSelectTool, _, cx| {
                this.set_tool(Tool::TrackSelect);
                cx.notify();
            }))
            // Enter ends a pen path as an open line (a path closes by clicking its first point).
            .on_action(cx.listener(|this, _: &FinishPath, _, cx| {
                if this.state.finish_pen(false).is_some() {
                    this.ui.tabs.insert(PanelId::Properties, 0);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| panels::settings::open(this, window, cx)))
            .on_action(cx.listener(|this, _: &Precompose, _, cx| {
                if this.state.precompose_selection().is_some() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                this.state.copy_selection();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if !this.state.paste().is_empty() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Duplicate, _, cx| {
                if !this.state.duplicate_selection().is_empty() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &CompositionSettings, window, cx| {
                panels::composition_settings::open(this, false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Import, window, cx| this.import(window, cx)))
            // Offered only when there is something to undo or redo, so the Edit menu greys them out otherwise.
            .when(self.state.can_undo(), |root| root.on_action(cx.listener(|this, _: &Undo, _, cx| this.undo_or_redo(false, cx))))
            .when(self.state.can_redo(), |root| root.on_action(cx.listener(|this, _: &Redo, _, cx| this.undo_or_redo(true, cx))))
            .on_action(cx.listener(|this, _: &Save, window, cx| this.save(window, cx)))
            .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.save_as(window, cx)))
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open(window, cx)))
            .on_action(cx.listener(|this, _: &Export, window, cx| panels::export::open(this, window, cx)))
            .on_action(cx.listener(|this, _: &SplitLayer, window, cx| this.split_layer(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSnapping, _, cx| {
                this.state.snapping = !this.state.snapping;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &RippleDelete, _, cx| {
                if this.state.ripple_delete_selection() > 0 {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &TrimInToPlayhead, _, cx| {
                if this.state.trim_selected_in_to_playhead() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &TrimOutToPlayhead, _, cx| {
                if this.state.trim_selected_out_to_playhead() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.quit(window, cx)))
            .on_action(cx.listener(|this, _: &NewProject, window, cx| this.new_project(window, cx)))
            .on_action(cx.listener(|this, _: &NewComposition, window, cx| {
                panels::composition_settings::open(this, true, window, cx);
            }))
            // Releasing the mouse anywhere ends whatever drag was in progress.
            .capture_any_mouse_up(cx.listener(|this, _, _, cx| {
                this.state.end_keyframe_drag();
                this.state.end_viewer_drag();
                this.state.end_layer_time_drag();
                this.state.end_clip_drag();
                this.state.clear_snap_line();
                this.ui.field_anchor = None;
                this.ui.viewer_anchor = None;
                this.ui.shape_press = false;
                this.ui.tool_press = None;
                this.ui.effect_drop_layer = None;
                if let Some(picker) = this.ui.color_picker.as_mut() {
                    picker.pressed = false;
                }
                // Redraw so the finished drag is recorded as one undo step right away, rather
                // than merging with whatever edit comes next.
                cx.notify();
            }))
            // Scrubbing a value field: the drag can wander anywhere in the window.
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<FieldDrag>, _, cx| {
                let target = event.drag(cx).0;
                let modifiers = event.event.modifiers;
                this.scrub_field(target, f32::from(event.event.position.x), modifiers.shift, modifiers.alt);
                cx.notify();
            }))
            // The clock renders first so the panels it refreshes see the new playhead this frame.
            .child(panels.clock.clone())
            .child(div().h(px(crate::theme::size::TOOLBAR_HEIGHT)).flex_none().child(cached(&panels.toolbar)))
            // The tool rail on the left, then the panels.
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(panels::toolbar::rail(self, cx))
                    .child(div().flex_1().min_w_0().pr(gutter).pb(gutter).child(workspace)),
            )
            .children(panels::color_picker::popover(self, cx))
    }
}

/// Height of the Align panel when Properties is open above it.
const ALIGN_HEIGHT: f32 = 230.0;

impl Workspace {
    /// The right column: Properties above Align. Either can be collapsed to its header (its
    /// chevron); whichever is open fills the space the other leaves.
    fn utility_column(&self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let collapsed = |id| self.ui.collapsed_panels.contains(&id);
        let header_only = px(crate::theme::size::PANEL_HEADER_HEIGHT + 2.0);
        let (properties, align) = (collapsed(PanelId::Properties), collapsed(PanelId::Align));
        let properties_box = div().min_h_0().map(|panel| if properties { panel.flex_none().h(header_only) } else { panel.flex_1() });
        let align_box = div().min_h_0().map(|panel| match (properties, align) {
            (_, true) => panel.flex_none().h(header_only),
            (true, false) => panel.flex_1(),
            (false, false) => panel.flex_none().h(px(ALIGN_HEIGHT)),
        });
        div()
            .id("workspace-utilities")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(crate::theme::size::GUTTER))
            .child(properties_box.child(cached(&self.panels.properties)))
            .child(align_box.child(cached(&self.panels.align)))
    }
}

/// "1 layer is" or "3 layers are", for messages.
fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
}
