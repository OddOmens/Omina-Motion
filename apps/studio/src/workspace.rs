//! The editor window: owns the editor state, wires keyboard shortcuts, playback
//! and in-place value editing, and lays out the panels like After Effects.

use std::collections::{HashMap, HashSet};
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
    CompositionSettings,
    NewComposition,
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
        KeyBinding::new("cmd-k", CompositionSettings, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-n", NewComposition, Some(KEY_CONTEXT)),
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
        Menu::new("Studio").items([MenuItem::action("Quit Studio", Quit)]),
        Menu::new("File").items([
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
    Align,
}

/// Toolbar tools that work today. The rest of the palette is shown disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Selection,
    Type,
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
}

/// The text input currently open over a value, and the subscription that commits it.
pub struct EditingField {
    pub target: EditTarget,
    pub input: Entity<InputState>,
    _events: Subscription,
}

/// UI-only state that is not part of the project or the editing model.
pub struct UiState {
    pub active_panel: PanelId,
    /// What the panel beside the rail shows.
    pub left_panel: LeftPanel,
    pub tool: Tool,
    /// Active tab index for panels with tabs.
    pub tabs: HashMap<PanelId, usize>,
    pub collapsed_folders: HashSet<Id>,
    /// Effect categories the user folded away (all are open to begin with).
    pub closed_effect_categories: HashSet<&'static str>,
    pub viewer_bounds: panels::chrome::MeasuredBounds,
    /// Window-space bounds of the composition frame inside the viewer.
    pub frame_bounds: panels::chrome::MeasuredBounds,
    /// Pointer position where the current viewer drag started.
    pub viewer_anchor: Option<(f32, f32)>,
    /// Pointer x and starting value of the field being scrubbed.
    pub field_anchor: Option<(f32, f32)>,
    pub editing: Option<EditingField>,
    /// The last curve used in the graph editor, shown when the selection has none of its own.
    pub flow_curve: CubicBezier,
    /// Where the graph editor popover is open (below the Easings button), if it is.
    pub graph_editor: Option<gpui_kit::Point<gpui_kit::Pixels>>,
    /// Font families offered in the Text section, looked up once at startup.
    pub font_families: Vec<&'static str>,
    pub timeline: panels::timeline::TimelineUi,
    /// The window title last set, so it is only changed when it needs to be.
    pub window_title: String,
    /// The Export dialog's choices, and the export running now, if any.
    pub export_choice: panels::export::ExportChoice,
    pub export_job: Option<panels::export::ExportJob>,
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
    align: Entity<PanelView>,
    clock: Entity<PlaybackClock>,
}

impl Panels {
    fn new(cx: &mut Context<Workspace>) -> Self {
        let workspace = cx.entity();
        let mut view = |kind| cx.new(|cx| PanelView::new(&workspace, kind, cx));
        let (toolbar, project, viewer, timeline) =
            (view(PanelKind::Toolbar), view(PanelKind::Project), view(PanelKind::Viewer), view(PanelKind::Timeline));
        let (properties, effects, align) = (view(PanelKind::Properties), view(PanelKind::Effects), view(PanelKind::Align));
        let timed = vec![viewer.entity_id(), timeline.entity_id(), properties.entity_id()];
        let clock = cx.new(|_| PlaybackClock::new(workspace.downgrade(), timed));
        Self { toolbar, project, viewer, timeline, properties, effects, align, clock }
    }
}

#[cfg(test)]
impl Workspace {
    /// How many times each panel has rendered: toolbar, project, viewer, timeline, properties, effects, align.
    pub fn panel_render_counts(&self, cx: &App) -> [usize; 7] {
        let p = &self.panels;
        [&p.toolbar, &p.project, &p.viewer, &p.timeline, &p.properties, &p.effects, &p.align].map(|view| view.read(cx).renders)
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
        // The loop ends when the frame server (which holds the sender) is dropped.
        cx.spawn(async move |_, cx| {
            while frames_ready.recv().await.is_ok() {
                cx.update(|cx| cx.notify(viewer));
            }
        })
        .detach();
        // The proxy builder signals here as a proxy's status changes (queued, progress, ready,
        // failed): refresh the Project panel, which shows it, and the viewer, which can then
        // switch to the finished proxy.
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
            let timeline = panels.timeline.entity_id();
            cx.spawn(async move |_, cx| {
                while media_arrived.recv().await.is_ok() {
                    cx.update(|cx| cx.notify(timeline));
                }
            })
            .detach();
        }
        // Pick up where the last session left off, and keep a recovery copy of unsaved work.
        // (Not in headless tests, which start fresh and must not touch real settings.)
        #[cfg(not(test))]
        {
            cx.defer_in(window, |this, window, cx| this.restore_session(window, cx));
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(crate::project_files::RECOVERY_INTERVAL).await;
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
                tabs: HashMap::new(),
                collapsed_folders: HashSet::new(),
                closed_effect_categories: HashSet::new(),
                viewer_bounds: Default::default(),
                frame_bounds: Default::default(),
                viewer_anchor: None,
                field_anchor: None,
                editing: None,
                flow_curve: studio_core::animation::EASING_PRESETS[0].1,
                graph_editor: None,
                font_families: panels::properties::installed_font_families(cx),
                timeline: timeline_ui,
                window_title: String::new(),
                export_choice: Default::default(),
                export_job: None,
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
        self.ui.editing = Some(EditingField { target, input, _events: events });
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
        }
        // Hand keyboard focus back to the editor so shortcuts work again.
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Starts building a proxy for every footage item that does not have one yet.
    pub fn request_proxies(&self) {
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
        self.state.record_history(cx.has_active_drag());
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
                                            }),
                                    )
                                    .child(resizable_panel().child(cached(&panels.viewer))),
                            ),
                        )
                        .child(resizable_panel().size_range(px(180.0)..px(2000.0)).child(cached(&panels.timeline))),
                ),
            )
            .child(
                resizable_panel().size(px(430.0)).size_range(px(300.0)..px(800.0)).child(
                    v_resizable("workspace-utilities")
                        .child(resizable_panel().size(px(620.0)).child(cached(&panels.properties)))
                        .child(resizable_panel().child(cached(&panels.align))),
                ),
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
                this.ui.tool = Tool::Selection;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TypeTool, _, cx| {
                this.ui.tool = Tool::Type;
                cx.notify();
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
    }
}

/// "1 layer is" or "3 layers are", for messages.
fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
}
