//! The editor window: owns the editor state, wires keyboard shortcuts, playback
//! and in-place value editing, and lays out the panels like After Effects.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::dock::{DockArea, DockAreaState, DockEvent};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, DragMoveEvent, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, StyleRefinement, Styled, Subscription, Window, actions, div,
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
    RenderAndPlay,
    NextFrame,
    PreviousFrame,
    NextTenFrames,
    PreviousTenFrames,
    NextFiveFrames,
    PreviousFiveFrames,
    ZoomViewerIn,
    ZoomViewerOut,
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
    EyedropperTool,
    RateStretchTool,
    ToggleMarker,
    NewSolid,
    NewAdjustmentLayer,
    NewNull,
    SequenceLayers,
    StaggerLayers,
    ToggleGuides,
    ToggleLayoutGrid,
    ToggleSnapToGrid,
    ToggleSnapToCenter,
    ToggleSnapToDeadCenter,
    ToggleLayerControls,
    ShowPositionKeyframes,
    ShowRotationKeyframes,
    ShowScaleKeyframes,
    ShowAllKeyframes,
    Copy,
    Paste,
    PasteAttributes,
    CloseGaps,
    Duplicate,
    OpenSettings,
    Precompose,
    UngroupComposition,
    SearchEffects,
    MorphTo,
    FlipHorizontal,
    FlipVertical,
    ResetTransform,
    CompositionSettings,
    MagicResize,
    MagicMove,
    NewComposition,
    NewProject,
    Import,
    Undo,
    Redo,
    Save,
    SaveAs,
    Open,
    Export,
    SaveFrame,
    Package,
    ExportQueue,
    TrimInToPlayhead,
    TrimOutToPlayhead,
    SplitLayer,
    RippleDelete,
    ToggleSnapping,
    Dismiss,
    Quit,
    Cut,
    SelectAll,
    CloseProject,
    ShowStartScreen,
    BrowseVersions,
    ClearRecentProjects,
    HideStudio,
    HideOtherApps,
    ShowAllApps,
    MinimizeWindow,
    ZoomWindow,
    ToggleFullScreen,
    AboutStudio,
    CheckForUpdates,
    ResetWorkspace,
    SaveWorkspaceAs,
    UpdateWorkspace,
    DeleteWorkspace,
    CopyFrame,
    SyncBySound,
    ToggleMulticam,
    ToggleVoiceover,
    MulticamAngle1,
    MulticamAngle2,
    MulticamAngle3,
    MulticamAngle4,
    MulticamAngle5,
    MulticamAngle6,
    MulticamAngle7,
    MulticamAngle8,
    MulticamAngle9
]);

/// Window ▸ Workspace: switches to the workspace named `name` (`None` for Standard).
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = studio, no_json)]
pub struct SwitchWorkspace {
    pub name: Option<String>,
}

/// File ▸ Open Recent: opens the project at `path`.
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = studio, no_json)]
pub struct OpenRecent {
    pub path: std::path::PathBuf,
}

/// Key context the editor's shortcuts are bound in.
pub(crate) const KEY_CONTEXT: &str = "Studio";
/// Where shortcuts that are plain keys (Space, letters, Delete, Alt+key) apply: anywhere in the
/// editor except while a text field has focus. On macOS a key binding wins over typing, so
/// without this, typing "s" or a space into a field would toggle snapping or start playback.
pub(crate) const NOT_TYPING: &str = "Studio && !Input";

pub fn bind_keys(cx: &mut App) {
    // The shortcuts in use (Settings ▸ Keyboard Shortcuts), on top of the components' own.
    crate::keymap::init(&crate::app_settings::load(), cx);
    // The editor window handles Quit itself, to offer saving first. This only runs when no
    // window does (none is open).
    cx.on_action(|_: &Quit, cx| cx.quit());
    // Hiding is the app's, not the window's, so it works whatever has focus.
    cx.on_action(|_: &HideStudio, cx| cx.hide());
    cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApps, cx| cx.unhide_other_apps());
}

/// Rebuilds the menu bar, so File ▸ Open Recent lists the projects used lately.
pub fn refresh_menus(cx: &mut App) {
    cx.set_menus(menus());
}

/// Turns one of the View menu's on-or-off choices over, saves it, and updates the menu's
/// check marks. `field` picks the choice out of the settings.
pub fn toggle_setting(this: &mut Workspace, cx: &mut Context<Workspace>, field: fn(&mut crate::app_settings::AppSettings) -> &mut bool) {
    let on = !*field(&mut this.ui.settings);
    *field(&mut this.ui.settings) = on;
    crate::app_settings::update(|settings| *field(settings) = on);
    refresh_menus(cx);
    cx.notify();
}

/// The menu bar. Shortcuts shown in it come from the key bindings above. Items for features
/// turned off in Settings ▸ Features are left out.
pub fn menus() -> Vec<gpui_kit::Menu> {
    use crate::features::Feature;
    use gpui_kit::{Menu, MenuItem, OsAction, SystemMenuType};
    let settings = crate::app_settings::load();
    // An item that is only there while its feature is on.
    let gated = |feature: Feature, item: MenuItem| settings.feature_on(feature).then_some(item);
    // The projects used lately that are still on disk, newest first.
    let recent: Vec<MenuItem> = settings
        .recent_projects
        .iter()
        .filter(|project| project.path.exists())
        .take(10)
        .map(|project| {
            let name = project.path.file_stem().map_or_else(|| project.path.display().to_string(), |name| name.to_string_lossy().into_owned());
            MenuItem::action(name, OpenRecent { path: project.path.clone() })
        })
        .collect();
    let mut open_recent = Menu::new("Open Recent").items(recent);
    open_recent.items.extend([MenuItem::separator(), MenuItem::action("Clear Menu", ClearRecentProjects)]);
    vec![
        Menu::new("Omina Motion").items([
            MenuItem::action("About Omina Motion", AboutStudio),
            // Off in builds made without an update address (see `updates`).
            MenuItem::action("Check for Updates…", CheckForUpdates).disabled(crate::updates::FEED.is_none()),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Omina Motion", HideStudio),
            MenuItem::action("Hide Others", HideOtherApps),
            MenuItem::action("Show All", ShowAllApps),
            MenuItem::separator(),
            MenuItem::action("Quit Omina Motion", Quit),
        ]),
        Menu::new("File").items(tidy_menu([
            Some(MenuItem::action("New Project", NewProject)),
            Some(MenuItem::action("Open…", Open)),
            Some(MenuItem::submenu(open_recent)),
            Some(MenuItem::action("Close Project", CloseProject)),
            Some(MenuItem::action("Start Screen", ShowStartScreen)),
            Some(MenuItem::separator()),
            Some(MenuItem::action("Save", Save)),
            Some(MenuItem::action("Save As…", SaveAs)),
            Some(MenuItem::action("Browse Versions…", BrowseVersions)),
            Some(MenuItem::separator()),
            gated(Feature::Export, MenuItem::action("Export…", Export)),
            gated(Feature::ExportQueue, MenuItem::action("Export Queue…", ExportQueue)),
            gated(Feature::Package, MenuItem::action("Package…", Package)),
            Some(MenuItem::separator()),
            Some(MenuItem::action("Import…", Import)),
            Some(MenuItem::action("New Composition…", NewComposition)),
        ])),
        Menu::new("Edit").items(tidy_menu([
            Some(MenuItem::os_action("Undo", Undo, OsAction::Undo)),
            Some(MenuItem::os_action("Redo", Redo, OsAction::Redo)),
            Some(MenuItem::separator()),
            Some(MenuItem::os_action("Cut", Cut, OsAction::Cut)),
            Some(MenuItem::os_action("Copy", Copy, OsAction::Copy)),
            Some(MenuItem::os_action("Paste", Paste, OsAction::Paste)),
            gated(Feature::PasteAttributes, MenuItem::action("Paste Attributes…", PasteAttributes)),
            Some(MenuItem::action("Duplicate", Duplicate)),
            Some(MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll)),
            gated(Feature::Precompose, MenuItem::action("Pre-compose…", Precompose)),
            Some(MenuItem::separator()),
            Some(MenuItem::action("Trim In Point to Playhead", TrimInToPlayhead)),
            Some(MenuItem::action("Trim Out Point to Playhead", TrimOutToPlayhead)),
            Some(MenuItem::action("Split Layer", SplitLayer)),
            Some(MenuItem::action("Ripple Delete", RippleDelete)),
            gated(Feature::CloseGaps, MenuItem::action("Close Gaps", CloseGaps)),
            Some(MenuItem::separator()),
            Some(MenuItem::action("Snapping", ToggleSnapping)),
        ])),
        Menu::new("Composition").items(tidy_menu([
            Some(MenuItem::action("Composition Settings…", CompositionSettings)),
            gated(Feature::MagicResize, MenuItem::action("Magic Resize…", MagicResize)),
            gated(Feature::MagicMove, MenuItem::action("Magic Move…", MagicMove)),
        ])),
        Menu::new("Layer").items(tidy_menu([
            gated(Feature::NewLayer, MenuItem::action("New Solid", NewSolid)),
            gated(Feature::NewLayer, MenuItem::action("New Adjustment Layer", NewAdjustmentLayer)),
            gated(Feature::NewLayer, MenuItem::action("New Null Object", NewNull)),
            Some(MenuItem::separator()),
            gated(Feature::Precompose, MenuItem::action("Pre-compose…", Precompose)),
            Some(MenuItem::action("Ungroup Composition", UngroupComposition)),
            gated(Feature::Morph, MenuItem::action("Morph To…", MorphTo)),
            Some(MenuItem::action("Flip Horizontal", FlipHorizontal)),
            Some(MenuItem::action("Flip Vertical", FlipVertical)),
            Some(MenuItem::action("Reset Transform", ResetTransform)),
            Some(MenuItem::separator()),
            gated(Feature::Markers, MenuItem::action("Add or Remove Marker", ToggleMarker)),
            Some(MenuItem::separator()),
            gated(Feature::SequenceLayers, MenuItem::action("Sequence Layers", SequenceLayers)),
            gated(Feature::SequenceLayers, MenuItem::action("Stagger Layers by 5 Frames", StaggerLayers)),
        ])),
        Menu::new("View").items(tidy_menu([
            Some(MenuItem::action("Zoom In", ZoomViewerIn)),
            Some(MenuItem::action("Zoom Out", ZoomViewerOut)),
            Some(MenuItem::separator()),
            gated(Feature::Guides, MenuItem::action("Show Guides", ToggleGuides)),
            Some(MenuItem::action("Show or Hide Layer Outlines", ToggleLayerControls)),
            Some(MenuItem::action("Layout Grid", ToggleLayoutGrid).checked(settings.show_grid)),
            Some(MenuItem::separator()),
            Some(MenuItem::action("Snap to Grid", ToggleSnapToGrid).checked(settings.snap_to_grid)),
            Some(MenuItem::action("Snap to Center Lines", ToggleSnapToCenter).checked(settings.snap_to_center)),
            Some(MenuItem::action("Snap to Dead Center", ToggleSnapToDeadCenter).checked(settings.snap_to_dead_center)),
        ])),
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::separator(),
            MenuItem::action("Enter Full Screen", ToggleFullScreen),
            MenuItem::separator(),
            MenuItem::submenu(workspace_menu(&settings)),
        ]),
    ]
}

/// Window ▸ Workspace: the built-in workspaces and then the saved ones to switch between (the
/// current one checked), then saving, updating, deleting and resetting them. Built-in ones
/// can't be updated or deleted.
fn workspace_menu(settings: &crate::app_settings::AppSettings) -> gpui_kit::Menu {
    use gpui_kit::{Menu, MenuItem};
    let current = settings.current_workspace.as_deref();
    let is_saved = panels::workspaces::is_saved(current, &settings.saved_workspaces);
    let mut items = vec![MenuItem::action(panels::workspaces::STANDARD, SwitchWorkspace { name: None }).checked(current.is_none())];
    items.extend(panels::workspaces::BuiltIn::ALL.map(|built_in| {
        MenuItem::action(built_in.name(), SwitchWorkspace { name: Some(built_in.name().to_owned()) }).checked(current == Some(built_in.name()))
    }));
    items.extend(settings.saved_workspaces.iter().map(|saved| {
        MenuItem::action(saved.name.clone(), SwitchWorkspace { name: Some(saved.name.clone()) }).checked(current == Some(saved.name.as_str()))
    }));
    items.extend([
        MenuItem::separator(),
        MenuItem::action("Save as New Workspace…", SaveWorkspaceAs),
        MenuItem::action("Update Current Workspace", UpdateWorkspace).disabled(!is_saved),
        MenuItem::action("Delete Workspace…", DeleteWorkspace).disabled(!is_saved),
        MenuItem::separator(),
        MenuItem::action("Reset Workspace", ResetWorkspace),
    ]);
    Menu::new("Workspace").items(items)
}

/// A menu's items without the ones left out (`None`), and without the separators that would
/// leave behind: none at either end, and never two in a row.
fn tidy_menu(items: impl IntoIterator<Item = Option<gpui_kit::MenuItem>>) -> Vec<gpui_kit::MenuItem> {
    use gpui_kit::MenuItem;
    let mut kept: Vec<MenuItem> = Vec::new();
    for item in items.into_iter().flatten() {
        let is_separator = matches!(item, MenuItem::Separator);
        if is_separator && kept.last().is_none_or(|last| matches!(last, MenuItem::Separator)) {
            continue;
        }
        kept.push(item);
    }
    if kept.last().is_some_and(|last| matches!(last, MenuItem::Separator)) {
        kept.pop();
    }
    kept
}

/// What the side panel (the one beside the rail) shows, picked with the rail's buttons.
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
    /// Ready-made graphics: shapes, arrows, badges, frames and stickers.
    Elements,
    /// Device mockups: iPhone, iPhone Duo, iPad and Mac.
    Mockups,
    /// Free stock video, pictures and sound, searched online.
    Stock,
    /// The undo history as a list of steps to click back to.
    History,
    /// The Motion Assistant: suggestions about the composition's keyframes, with fixes.
    Assistant,
}

/// A voiceover being recorded: the microphone recording, the file it is written to, and the
/// frame the playhead was on when it began (where the clip will go).
pub struct Voiceover {
    pub recording: studio_media::Recording,
    pub path: std::path::PathBuf,
    pub start_frame: i64,
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
    Elements,
    Mockups,
    Stock,
    /// The node graph of the selected layer's effects.
    Nodes,
    /// The audio mixer: a level meter, voiceover recording, and each sounding clip's volume
    /// and pan.
    Mixer,
    /// The side panel beside the rail, as a whole: it shows one of Project, Effects, Libraries,
    /// Templates, Captions, Elements, Mockups or Stock (see `UiState::left_panel`), and is moved as one.
    Side,
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
    /// Drag a clip's end to change its speed so the same footage fills the new length, like
    /// Premiere's Rate Stretch tool.
    RateStretch,
    /// Drag a clip to change which part of its source plays.
    Slip,
    /// Drag a clip between its neighbors, which stretch to meet it.
    Slide,
    /// Click a clip to pick it and everything after it, then drag them together.
    TrackSelect,
    /// Click the viewer to take the color there as the current color (Shift-click also puts it
    /// on the selected shapes, text and solids).
    Eyedropper,
}

/// The kinds of layer the Layer menu makes from nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewLayer {
    Solid,
    Adjustment,
    Null,
}

/// How many frames apart Stagger Layers starts each layer.
pub const STAGGER_FRAMES: i64 = 5;

/// Drag payloads. GPUI routes drops by type, so each kind of drag has its own type.
#[derive(Clone, Debug)]
pub struct ProjectItemDrag {
    pub item: Id,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct EffectDrag {
    /// The catalog effect, or for a preset, its name (shown while dragging).
    pub name: String,
    /// For Apply LUT dragged from the panel's LUTs: the table it uses.
    pub lut: Option<studio_core::model::LutSource>,
    /// For one of the user's presets (My Presets): which one.
    pub preset: Option<studio_core::model::LinkId>,
}

impl EffectDrag {
    /// Adds the dragged effect, or the preset's effects, to `layer`.
    pub fn apply(&self, state: &mut EditorState, layer: Id) {
        match (&self.preset, &self.lut) {
            (Some(preset), _) => {
                state.apply_effect_preset(*preset, &[layer]);
            }
            (None, Some(lut)) => {
                state.add_lut(layer, lut.clone());
            }
            (None, None) => state.add_effect(layer, &self.name),
        }
    }

    /// Whether dropping it on a layer showing `content` does anything. A preset goes on any
    /// layer (what doesn't suit it is left out); an effect only on layers it suits.
    pub fn suits(&self, content: &studio_core::model::LayerContent) -> bool {
        self.preset.is_some() || studio_core::effects::suits_layer(&self.name, content)
    }
}

/// A transition dragged from the Effects panel onto a cut in the timeline.
#[derive(Clone, Debug)]
pub struct TransitionDrag {
    pub transition: studio_core::transitions::Transition,
}

/// How long the caret drawn over text being typed in the viewer stays on, then off.
pub const CARET_BLINK: std::time::Duration = std::time::Duration::from_millis(530);

/// What an in-place text input is editing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditTarget {
    Number(FieldTarget),
    SourceText(Id),
    /// The name of a library brand, color or text style.
    LibraryName(studio_core::model::LinkId),
    /// The text of a caption layer's words in a range (one phrase).
    CaptionText(Id, usize, usize),
    /// A layer's name (Rename in a clip's right-click menu).
    LayerName(Id),
    /// A Project panel item's name (Rename in its right-click menu).
    ItemName(Id),
}

/// The field an in-place edit is typed into: one line for names and numbers, several for a
/// text layer's text (Return starts a new line there, as in After Effects).
pub enum EditInput {
    Line(Entity<InputState>),
    Lines(Entity<TextareaState>),
}

impl EditInput {
    pub fn value(&self, cx: &App) -> gpui_kit::SharedString {
        match self {
            Self::Line(input) => input.read(cx).value(),
            Self::Lines(input) => input.read(cx).value(),
        }
    }

    /// The selected bytes of the text; empty at the caret when nothing is selected.
    pub fn selected_range(&self, cx: &App) -> std::ops::Range<usize> {
        match self {
            Self::Line(input) => input.read(cx).selected_range(),
            Self::Lines(input) => input.read(cx).selected_range(),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Line(input) => input.update(cx, |input, cx| input.focus(window, cx)),
            Self::Lines(input) => input.update(cx, |input, cx| input.focus(window, cx)),
        }
    }

    pub fn set_selected_range(&self, range: std::ops::Range<usize>, cx: &mut App) {
        match self {
            Self::Line(input) => input.update(cx, |input, cx| input.set_selected_range(range, cx)),
            Self::Lines(input) => input.update(cx, |input, cx| input.set_selected_range(range, cx)),
        }
    }

    /// The one-line field, for every edit but a text layer's text.
    pub fn line(&self) -> Option<&Entity<InputState>> {
        match self {
            Self::Line(input) => Some(input),
            Self::Lines(_) => None,
        }
    }

    /// The field as an element, unstyled.
    pub fn element(&self) -> AnyElement {
        match self {
            Self::Line(input) => gpui_kit::component::input::Input::new(input).into_any_element(),
            Self::Lines(input) => gpui_kit::component::input::Textarea::new(input).into_any_element(),
        }
    }
}

impl EditTarget {
    /// Whether the text can run over several lines: only a text layer's text.
    pub fn is_multi_line(self) -> bool {
        matches!(self, Self::SourceText(_))
    }
}

/// The text input currently open over a value, and the subscription that commits it.
pub struct EditingField {
    pub target: EditTarget,
    pub input: EditInput,
    /// The text the input opened with. Left as it was, nothing is written back: a number shown
    /// rounded would otherwise replace the exact value (and could add a keyframe).
    initial: String,
    /// Typed straight onto the layer in the viewer (the Type tool, or a double-click there)
    /// rather than in its panel.
    pub in_viewer: bool,
    _events: Subscription,
    /// Redraws the editor whenever the field changes (text, caret or selection), so text typed
    /// in the viewer and its caret stay current.
    _changes: Subscription,
    /// Redraws now and then while text is typed in the viewer, to blink its caret.
    _blink: gpui_kit::Task<()>,
}

/// UI-only state that is not part of the project or the editing model.
pub struct UiState {
    pub active_panel: PanelId,
    /// What the side panel shows, picked on the rail.
    pub left_panel: LeftPanel,
    /// Panels to bring to the front of their tab group on the next draw (a rail button asks for
    /// one, say). The dock needs the window to change, which only the draw has to hand.
    pub reveal_panels: Vec<PanelId>,
    /// The panels on screen as of this draw: each the front tab of its group, and turned on.
    pub showing_panels: HashSet<PanelId>,
    /// Set while the standard arrangement is being put back, so the dock's report of that
    /// change doesn't save it as a custom arrangement (with sizes not yet measured).
    pub layout_resetting: bool,
    /// How many more draws until a changed panel arrangement is saved again (0 when none is
    /// due). The dock only measures a newly made split when it is first drawn; saved sooner,
    /// the split's panels would be recorded at a placeholder size.
    pub layout_save_countdown: u8,
    /// The menu opened from a panel tab's grip, while it is open.
    pub panel_menu: Option<panels::dock::PanelMenu>,
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
    /// Project panel folders and Properties panel sections start closed. These hold the ones the
    /// user turned the other way; see `folder_open` and `section_open`.
    pub toggled_folders: HashSet<Id>,
    /// Where a project item being dragged in the Project panel would land.
    pub project_drop: Option<panels::project::ProjectDrop>,
    pub toggled_sections: HashSet<&'static str>,
    /// Whether folders and sections start open instead. Only the UI tests turn this on, so they
    /// can reach controls without unfolding everything first.
    pub sections_start_open: bool,
    /// Library brands the user folded away (all are open to begin with).
    pub closed_brands: HashSet<studio_core::model::LinkId>,
    /// Effect categories the user unfolded (all are closed to begin with, so the panel opens
    /// as a short list of categories).
    pub open_effect_categories: HashSet<&'static str>,
    pub viewer_bounds: panels::chrome::MeasuredBounds,
    /// Window-space bounds of the composition frame inside the viewer.
    pub frame_bounds: panels::chrome::MeasuredBounds,
    /// Pointer position where the current viewer drag started.
    pub viewer_anchor: Option<(f32, f32)>,
    /// The shape layer whose outline is being edited in the viewer: its points show and can be
    /// dragged. Started by double-clicking the shape (or drawing it with the Pen); a click
    /// elsewhere or Escape ends it. A shape that is only selected moves as a whole.
    pub path_editing: Option<Id>,
    /// The rail button the pointer is over, by its id, with the number of that hover (see
    /// `rail_hovers`), and the button whose name is showing beside it: the hovered one, once
    /// the pointer has rested on it for a moment.
    pub rail_hover: Option<(&'static str, u64)>,
    pub rail_label: Option<&'static str>,
    /// Counts hovers over rail buttons, to tell one from the next on the same button.
    pub rail_hovers: u64,
    /// The voiceover being recorded from the microphone, if one is.
    pub voiceover: Option<Voiceover>,
    /// While cutting between camera angles (multicam): the angles, as the video file behind
    /// each, in the order the number keys pick them. Empty when multicam is off.
    pub multicam: Vec<Id>,
    /// The Mixer panel's level meter for the mix being played.
    pub meter: panels::mixer::Meter,
    /// The Mixer panel's level meter for the microphone while a voiceover is recorded.
    pub mic_meter: panels::mixer::Meter,
    /// The words picked in a caption layer's transcript, to cut from the video: the layer, the
    /// word clicked first and the word clicked last (shift-click picks a run of words).
    pub transcript_selection: Option<(Id, usize, usize)>,
    /// The viewer's magnification (screen points per composition pixel); `None` fits the frame.
    pub viewer_zoom: Option<f32>,
    /// How far the frame is moved from the middle of the viewer, in screen points (Hand tool).
    pub viewer_pan: (f32, f32),
    /// The pan when a Hand drag began.
    pub pan_start: (f32, f32),
    /// While the middle mouse button is held over the viewer (a temporary Hand tool): where the
    /// pointer was pressed, and the pan then.
    pub middle_pan: Option<((f32, f32), (f32, f32))>,
    /// While dragging across text being typed in the viewer: the layer, and the place in its
    /// text where the drag started (the selection runs from there to the pointer).
    pub text_select: Option<(Id, usize)>,
    /// When the text field last changed, so the caret drawn in the viewer stays solid while
    /// typing and blinks once it rests.
    pub caret_epoch: Instant,
    /// Pointer x and starting value of the field being scrubbed.
    pub field_anchor: Option<(f32, f32)>,
    pub editing: Option<EditingField>,
    /// The last curve used in the graph editor, shown when the selection has none of its own.
    pub flow_curve: CubicBezier,
    /// The segment (or All) picked in the graph editor, with the keyframe selection it was
    /// picked for; it no longer applies once the selection changes.
    pub segment_focus: Option<(Vec<crate::state::KeyframeRef>, crate::panels::graph_editor::SegmentFocus)>,
    /// Font families offered in the Text section, looked up once at startup.
    pub font_families: Vec<String>,
    /// The Templates panel's chosen shape and category, and the pictures made of templates.
    pub template_format: studio_core::templates::Format,
    pub template_category: Option<studio_core::templates::Category>,
    /// The colors templates are made in: their own, another palette, or a brand's look.
    pub template_look: crate::state::TemplateLook,
    pub template_pictures: HashMap<(usize, studio_core::templates::Format), Arc<gpui_kit::RenderImage>>,
    /// The template playing on its card while the pointer is over it.
    pub template_preview: Option<panels::templates::TemplatePreview>,
    /// Whether the View All gallery of every template is open.
    pub template_gallery: bool,
    /// Whether an update check is running, so a second one waits for it.
    pub checking_for_updates: bool,
    /// The start screen, while it shows over the editor.
    pub start_screen: Option<panels::start_screen::StartScreen>,
    /// The welcome setup's page, while it shows (see `panels::onboarding`).
    pub onboarding: Option<panels::onboarding::Step>,
    /// Whether the last recovery copy failed (said once, until it works again).
    pub recovery_failing: bool,
    /// Whether the launch is still asking whether to restore the recovery copy. Until it is
    /// answered, the copy is left alone, so quitting first doesn't lose it.
    pub recovery_offered: bool,
    /// When the last version of the project was kept (see `versions`).
    pub last_version_at: Option<std::time::Instant>,
    /// Render & Play is rendering the work area, to play it once every frame is ready.
    pub render_then_play: bool,
    /// A library save that failed, to try again (with whether to back up first).
    pub library_save_failed: Option<bool>,
    /// The Mockups panel's choices and its device pictures.
    pub mockups: panels::mockups::MockupsUi,
    /// The Stock panel's search, results and downloads.
    pub stock: panels::stock::StockUi,
    /// Pictures made of the Elements panel's elements, by index.
    pub element_pictures: HashMap<usize, Arc<gpui_kit::RenderImage>>,
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
    /// The Export dialog's field for naming a preset of its choices, made when it first opens.
    pub export_preset_name: Option<Entity<gpui_kit::component::input::InputState>>,
    pub export_job: Option<panels::export::ExportJob>,
    /// Exports saved to run later, one after another (File ▸ Export Queue…).
    pub export_queue: panels::export_queue::ExportQueue,
    /// The package File ▸ Package… is making now, if any.
    pub package_job: Option<panels::package::PackageJob>,
    /// The Export dialog's preview: the frame picked and the picture drawn for it.
    pub export_preview: panels::export_dialog::ExportPreview,
    /// While an effect is dragged over the viewer, the layer it would be dropped on.
    pub effect_drop_layer: Option<Id>,
    /// While a photo or video is dragged over the viewer, the outline (in composition pixels)
    /// of the media slot it would fill (see `studio_core::slots`).
    pub slot_drop: Option<[studio_core::animation::Vec2; 4]>,
    /// Captions being made in the background, if any.
    pub caption_job: Option<panels::captions::CaptionJob>,
    /// Silence removal, beat finding, tracking or reframing running in the background, if any.
    pub analysis_job: Option<panels::analysis::AnalysisJob>,
    /// The import running in the background, shown in a popup.
    pub import_job: Option<panels::importing::ImportJob>,
    /// An import waiting to be told how its SVG files come in, shown as a popup.
    pub svg_question: Option<panels::importing::SvgQuestion>,
    /// The Motion Assistant's dismissed suggestions and whether its settings are showing.
    pub assistant: panels::assistant::AssistantUi,
    /// The layer waiting for a click on a video to track (Track Motion).
    pub track_pick: Option<Id>,
    /// Background removal running in the background, if any.
    pub cutout_job: Option<panels::cutout::CutoutJob>,
    /// The Enhance Speech copy being made, and the layers (with the settings they had) whose
    /// copy failed or was cancelled, which aren't tried again until their settings change.
    pub enhance_job: Option<panels::enhance::EnhanceJob>,
    pub enhance_skipped: Vec<(Id, studio_core::model::SpeechEnhance)>,
    /// The quick effect search (⌘Return), while it is open.
    pub effect_search: Option<panels::effect_search::EffectSearch>,
    /// The layer whose object matte is being painted: strokes in the viewer go to it.
    pub cutout_pick: Option<Id>,
    /// The stroke being painted in the viewer, if a press is under way.
    pub matte_paint: Option<panels::viewer::MattePaint>,
    /// The object matte brush's radius, in screen points.
    pub matte_brush: f32,
    /// Where the pointer is over the picture (composition pixels), for the brush outline.
    pub matte_pointer: Option<studio_core::animation::Vec2>,
    /// How the viewer shows a keyed layer (one with an Object Matte or a green screen key):
    /// its plain result (`None`), or what the key keeps and removes (see `key_view`).
    pub key_view: Option<studio_core::key_view::KeyView>,
    /// The color picker, while it is open.
    pub color_picker: Option<panels::color_picker::Picker>,
    /// Whether the eyedropper's color menu (current color, recent picks, swatches) is open, and
    /// whether its rail button is held down (a long press opens the menu).
    pub color_well: bool,
    /// Whether the viewer shows its guides (') and which ones: the grid, title safe, or the
    /// social apps' safe zones.
    pub guides: bool,
    /// Whether the viewer hides the outlines, handles and anchor points of selected layers
    /// (Cmd+Shift+H), to see the picture without them. The selection itself is unchanged.
    pub layer_controls_hidden: bool,
    pub guide_choice: panels::guides::GuideChoice,
    pub eyedropper_press: bool,
    /// The Basic Correction (layer, effect) waiting for a click on something gray in the viewer
    /// to set its white balance.
    pub white_balance_pick: Option<(Id, Id)>,
    /// The Keyer (layer, effect) waiting for a click in the viewer to pick its key color.
    pub key_color_pick: Option<(Id, Id)>,
    /// The Nodes panel: whether it is open, where its canvas is looking, and what is picked.
    pub nodes: panels::nodes::NodesUi,
    /// Where the Color panel's three color wheels and its sliders are on screen, for dragging,
    /// and whether one is pressed (so the press and the drag after it are one undo step).
    pub wheel_bounds: [panels::chrome::MeasuredBounds; 3],
    pub slider_bounds: std::cell::RefCell<HashMap<(Id, usize), panels::chrome::MeasuredBounds>>,
    pub grade_pressed: bool,
    /// Where the Color panel is drawn, so its color wheels can fit its width.
    pub grade_bounds: panels::chrome::MeasuredBounds,
    /// Which video scope the Color tab shows.
    pub scope_kind: crate::scopes::ScopeKind,
    /// The scope as last drawn, with the viewer picture and kind it was drawn from, so an
    /// unchanged frame isn't measured again. Filled in while the panel is drawn.
    pub scope_cache: std::cell::RefCell<Option<(gpui_kit::ImageId, crate::scopes::ScopeKind, Arc<gpui_kit::RenderImage>)>>,
    /// The gradient stop picked in Properties (its shape, fill or stroke, and index), whose
    /// location and Delete button show under the gradient bar.
    pub gradient_stop: Option<(Id, crate::state::PaintSlot, usize)>,
    /// Whether the picked gradient stop was dragged since it was pressed (a press without a drag
    /// opens its color instead).
    pub gradient_stop_dragged: bool,
    /// Where the fill's and the stroke's gradient bars were drawn, to turn a click into a location.
    pub gradient_bars: [panels::chrome::MeasuredBounds; 2],
    /// The LUTs imported into Studio's LUT library, read when it changes.
    pub user_luts: Vec<std::path::PathBuf>,
    /// Pictures of what each LUT does to the current frame.
    pub lut_previews: panels::lut_previews::LutPreviews,
    /// The Apply LUT effect whose LUT browser is open in the Color panel.
    pub lut_browser: Option<Id>,
    /// The LUT under the pointer in the Effects panel, which the viewer shows on the picture
    /// until the pointer leaves it (see `viewer::lut_preview`).
    pub lut_hover: Option<studio_core::model::LutSource>,
    /// Keeps the open dialog's field links alive (Composition Settings' locked aspect ratio).
    pub dialog_subscriptions: Vec<Subscription>,
    /// Set when the user chose to quit while exporting: quit once the export has stopped.
    pub quit_after_export: bool,
}

impl UiState {
    pub fn tab(&self, panel: PanelId) -> usize {
        self.tabs.get(&panel).copied().unwrap_or(0)
    }

    /// Whether the Project panel shows folder `id`'s contents.
    pub fn folder_open(&self, id: Id) -> bool {
        self.sections_start_open != self.toggled_folders.contains(&id)
    }

    pub fn toggle_folder(&mut self, id: Id) {
        if !self.toggled_folders.remove(&id) {
            self.toggled_folders.insert(id);
        }
    }

    /// Opens every folder `item` is inside, so a newly imported file can be seen.
    pub fn reveal_item(&mut self, project: &studio_core::model::Project, item: Id) {
        let mut parent = project.item(item).and_then(|item| item.parent);
        while let Some(folder) = parent {
            if !self.folder_open(folder) {
                self.toggle_folder(folder);
            }
            parent = project.item(folder).and_then(|folder| folder.parent);
        }
    }

    /// Whether the Properties panel section named `key` is unfolded.
    pub fn section_open(&self, key: &str) -> bool {
        self.sections_start_open != self.toggled_sections.contains(key)
    }

    pub fn toggle_section(&mut self, key: &'static str) {
        if !self.toggled_sections.remove(key) {
            self.toggled_sections.insert(key);
        }
    }

    /// Unfolds the section named `key`, if it is folded.
    pub fn open_section(&mut self, key: &'static str) {
        if !self.section_open(key) {
            self.toggle_section(key);
        }
    }
}

pub struct Workspace {
    pub state: EditorState,
    pub ui: UiState,
    /// The GPU renderer and the frame the viewer shows.
    pub render: panels::viewer::ViewerRender,
    /// The composition's sound, kept in step with playback.
    sound: Sound,
    pub(crate) focus: FocusHandle,
    panels: Panels,
    /// Where the panels sit: the dock that lets them be moved, and its saved arrangement.
    dock: Entity<DockArea>,
    _dock_events: Subscription,
}

/// The cached panel views and the playback clock (see `panels::views`).
struct Panels {
    toolbar: Entity<PanelView>,
    /// The side panel, which shows the Project panel or another picked on the rail.
    side: Entity<PanelView>,
    viewer: Entity<PanelView>,
    timeline: Entity<PanelView>,
    properties: Entity<PanelView>,
    align: Entity<PanelView>,
    nodes: Entity<PanelView>,
    mixer: Entity<PanelView>,
    clock: Entity<PlaybackClock>,
}

impl Panels {
    fn new(focus: &FocusHandle, cx: &mut Context<Workspace>) -> Self {
        let workspace = cx.entity();
        let mut view = |kind| cx.new(|cx| PanelView::new(&workspace, kind, focus.clone(), cx));
        let (toolbar, side, viewer, timeline) =
            (view(PanelKind::Toolbar), view(PanelKind::Side), view(PanelKind::Viewer), view(PanelKind::Timeline));
        let (properties, align, nodes, mixer) = (view(PanelKind::Properties), view(PanelKind::Align), view(PanelKind::Nodes), view(PanelKind::Mixer));
        // The Mixer's meters move with the sound, so it is redrawn on every playing frame too.
        let timed = vec![viewer.entity_id(), timeline.entity_id(), properties.entity_id(), mixer.entity_id()];
        let clock = cx.new(|_| PlaybackClock::new(workspace.downgrade(), timed, side.entity_id()));
        Self { toolbar, side, viewer, timeline, properties, align, nodes, mixer, clock }
    }

    /// The view that draws dockable panel `id`.
    fn view(&self, id: PanelId) -> &Entity<PanelView> {
        match id {
            PanelId::Composition => &self.viewer,
            PanelId::Timeline => &self.timeline,
            PanelId::Properties => &self.properties,
            PanelId::Align => &self.align,
            PanelId::Nodes => &self.nodes,
            PanelId::Mixer => &self.mixer,
            // The panels the side panel shows are all drawn by it.
            PanelId::Side
            | PanelId::Project
            | PanelId::Effects
            | PanelId::Libraries
            | PanelId::Templates
            | PanelId::Captions
            | PanelId::Elements
            | PanelId::Mockups
            | PanelId::Stock => &self.side,
        }
    }
}

#[cfg(test)]
impl Workspace {
    /// How many times each panel has rendered, in `PanelKind` order: toolbar, project, viewer,
    /// timeline, properties, effects, libraries, align.
    /// How long each panel has spent rendering, in the same order.
    pub fn panel_render_times(&self, cx: &App) -> [std::time::Duration; 7] {
        let p = &self.panels;
        [&p.toolbar, &p.side, &p.viewer, &p.timeline, &p.properties, &p.align, &p.nodes].map(|view| view.read(cx).render_time)
    }

    pub fn panel_render_counts(&self, cx: &App) -> [usize; 7] {
        let p = &self.panels;
        [&p.toolbar, &p.side, &p.viewer, &p.timeline, &p.properties, &p.align, &p.nodes].map(|view| view.read(cx).renders)
    }
}

#[cfg(test)]
impl Workspace {
    /// The dock entry for panel `id`.
    fn dock_id(&self, id: PanelId) -> gpui_kit::component::dock::PanelId {
        gpui_kit::component::dock::PanelId::from(self.panels.view(id).entity_id())
    }

    /// The panels sharing panel `id`'s tab group, in tab order (`id` among them).
    pub fn dock_group_of(&self, id: PanelId, cx: &App) -> Vec<PanelId> {
        let dock = self.dock.read(cx);
        let Some(tree) = dock.layout(gpui_kit::component::dock::DockPlacement::Center) else { return Vec::new() };
        let Some(node) = tree.find_panel_node(self.dock_id(id)).and_then(|node| tree.find_node(node)) else { return Vec::new() };
        let gpui_kit::component::dock::PaneRef::Tabs { panels, .. } = node.kind() else { return Vec::new() };
        panels.iter().filter_map(|panel| panels::dock::DOCKABLE.into_iter().find(|id| self.dock_id(*id) == *panel)).collect()
    }

    /// The dock's arrangement, as it would be saved.
    pub fn dock_state(&self, cx: &App) -> DockAreaState {
        self.dock.read(cx).dump(cx)
    }

    /// Moves panel `id` into `target`'s tab group, behind the panel showing there.
    pub fn move_panel_behind(&mut self, id: PanelId, target: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        let (panel, target) = (self.dock_id(id), self.dock_id(target));
        self.dock.update(cx, |dock, cx| {
            let Some(node) = dock.layout(gpui_kit::component::dock::DockPlacement::Center).and_then(|tree| tree.find_panel_node(target)) else { return };
            dock.move_panel(panel, gpui_kit::component::dock::InsertTarget::Tabs { node, ix: None, activate: false }, window, cx);
        });
    }
}

impl Workspace {
    /// Redraws just the timeline (scrolling it, say), not every panel.
    pub fn notify_timeline(&self, cx: &mut App) {
        cx.notify(self.panels.timeline.entity_id());
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
        let panels = Panels::new(&focus, cx);
        let (dock, dock_events) = Self::make_dock(&panels, window, cx);
        // Background decoders signal here when a frame is ready; redraw only the viewer.
        let (frame_ready, frames_ready) = async_channel::bounded::<()>(1);
        render.media.set_on_ready(move || {
            // A full channel already means "redraw pending", so extra signals can be dropped.
            let _ = frame_ready.try_send(());
        });
        let viewer = panels.viewer.entity_id();
        let project_panel = panels.side.entity_id();
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
            let project_panel = panels.side.entity_id();
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
        // Frames rendered ahead redraw the timeline's green line as they arrive. (Headless tests
        // leave this out: their scheduler refuses wake-ups from other threads.)
        #[cfg(not(test))]
        {
            let (ahead_ready, ahead_arrived) = async_channel::bounded::<()>(1);
            render.preview.set_on_change(move || {
                let _ = ahead_ready.try_send(());
            });
            let timeline = panels.timeline.entity_id();
            cx.spawn(async move |this, cx| {
                while ahead_arrived.recv().await.is_ok() {
                    // At most a few redraws a second: the line needn't follow every frame.
                    cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
                    cx.update(|cx| cx.notify(timeline));
                    // Render & Play starts playing once its frames are all in.
                    if this.update(cx, |this, cx| this.check_render_then_play(cx)).is_err() {
                        break;
                    }
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
            let (timeline, project_panel) = (panels.timeline.entity_id(), panels.side.entity_id());
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
        // Iris's recipes' shortcuts work from the start, before her chat is ever opened.
        cx.defer_in(window, |this, _, cx| panels::assistant_automation::start(this, cx));
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
            // Canvas shares the libraries file: what it changes shows up here within moments.
            // (Not while the color picker is being dragged, which is changing them here.)
            // Compositions Canvas sends (Bring to Motion) are imported as they arrive, once
            // nothing else is being imported.
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
                    let alive = this.update_in(cx, |this, window, cx| {
                        let picking = this.ui.color_picker.as_ref().is_some_and(|picker| picker.pressed);
                        if !picking && let Some(libraries) = crate::library_store::outside_changes(&this.state.libraries) {
                            this.state.set_libraries(libraries);
                            cx.notify();
                        }
                        if this.ui.import_job.is_none() && this.ui.svg_question.is_none() {
                            let arrived = crate::canvas_inbox::take_ready(&crate::canvas_inbox::folder());
                            if !arrived.is_empty() {
                                panels::importing::start(this, arrived, None, window, cx);
                            }
                        }
                        // What Iris's other apps hand over ("put this photo in Motion") comes in
                        // the same way, onto the open composition.
                        panels::assistant_automation::bring_in_handoffs(this, window, cx);
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            })
            .detach();
            // The Motion Assistant's face blinks every few seconds while it is on screen.
            cx.spawn_in(window, async move |this, cx| {
                for pause in panels::assistant::BLINK_PAUSES.into_iter().cycle() {
                    cx.background_executor().timer(pause).await;
                    if this.update_in(cx, |this, window, cx| panels::assistant::blink(this, window, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
            cx.defer_in(window, |this, window, cx| this.restore_session(window, cx));
            // A crash last time is mentioned once, with its report.
            cx.defer_in(window, |_, window, cx| crate::crash_log::mention_last_crash(window, cx));
            // The daily update check, quietly.
            cx.defer_in(window, |this, window, cx| crate::updates::check(this, false, window, cx));
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    // As often as Settings says; checked again each time, so a change applies.
                    // With the recovery copy off, timed versions and library retries still run.
                    let seconds = crate::app_settings::load().recovery_seconds;
                    let wait = if seconds == 0 { crate::project_files::RECOVERY_INTERVAL } else { std::time::Duration::from_secs(u64::from(seconds)) };
                    cx.background_executor().timer(wait).await;
                    if this.update_in(cx, |this, window, cx| this.keep_work_safe(window, cx)).is_err() {
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
        // The app starts on an empty project (tests start on the sample project, with something
        // in it to work on), so the sample never shows, even for a frame.
        #[cfg(not(test))]
        let state = {
            let settings = crate::app_settings::load();
            let mut state = EditorState::new();
            state.new_project(&settings.default_folders, settings.new_composition);
            state
        };
        #[cfg(test)]
        let state = EditorState::new();
        Self {
            state,
            ui: UiState {
                active_panel: PanelId::Timeline,
                left_panel: LeftPanel::Media,
                reveal_panels: Vec::new(),
                showing_panels: HashSet::new(),
                layout_resetting: false,
                layout_save_countdown: 0,
                panel_menu: None,
                tool: Tool::Selection,
                shape_kind: studio_core::model::ShapeKind::Rectangle,
                shape_menu: false,
                tool_menu: None,
                tool_press: None,
                group_tools: HashMap::new(),
                shape_press: false,
                tabs: HashMap::new(),
                toggled_folders: HashSet::new(),
                project_drop: None,
                toggled_sections: HashSet::new(),
                sections_start_open: cfg!(test),
                open_effect_categories: HashSet::new(),
                closed_brands: HashSet::new(),
                viewer_bounds: Default::default(),
                frame_bounds: Default::default(),
                viewer_anchor: None,
                path_editing: None,
                rail_hover: None,
                rail_label: None,
                rail_hovers: 0,
                multicam: Vec::new(),
                voiceover: None,
                meter: Default::default(),
                mic_meter: Default::default(),
                transcript_selection: None,
                viewer_zoom: None,
                viewer_pan: (0.0, 0.0),
                pan_start: (0.0, 0.0),
                middle_pan: None,
                text_select: None,
                caret_epoch: Instant::now(),
                field_anchor: None,
                editing: None,
                flow_curve: studio_core::animation::EASING_PRESETS[0].1,
                segment_focus: None,
                font_families: panels::properties::installed_font_families(cx),
                project_columns: crate::app_settings::load().project_columns,
                settings: crate::app_settings::load(),
                preview: Default::default(),
                template_format: studio_core::templates::Format::Vertical,
                template_category: None,
                template_look: crate::state::TemplateLook::Own,
                template_pictures: HashMap::new(),
                template_preview: None,
                template_gallery: false,
                checking_for_updates: false,
                start_screen: None,
                onboarding: None,
                recovery_failing: false,
                recovery_offered: false,
                last_version_at: None,
                render_then_play: false,
                library_save_failed: None,
                mockups: Default::default(),
                element_pictures: HashMap::new(),
                stock: panels::stock::StockUi::default(),
                preview_bounds: Default::default(),
                timeline: timeline_ui,
                window_title: String::new(),
                export_choice: Default::default(),
                export_preset_name: None,
                export_job: None,
                export_queue: Default::default(),
                package_job: None,
                export_preview: Default::default(),
                dialog_subscriptions: Vec::new(),
                color_picker: None,
                caption_job: None,
                cutout_job: None,
                enhance_job: None,
                enhance_skipped: Vec::new(),
                analysis_job: None,
                import_job: None,
                svg_question: None,
                assistant: panels::assistant::AssistantUi::default(),
                track_pick: None,
                cutout_pick: None,
                effect_search: None,
                matte_paint: None,
                matte_brush: panels::viewer::DEFAULT_BRUSH,
                matte_pointer: None,
                key_view: None,
                effect_drop_layer: None,
                slot_drop: None,
                color_well: false,
                guides: false,
                layer_controls_hidden: false,
                guide_choice: panels::guides::GuideChoice::default(),
                eyedropper_press: false,
                white_balance_pick: None,
                key_color_pick: None,
                nodes: Default::default(),
                wheel_bounds: Default::default(),
                grade_pressed: false,
                slider_bounds: Default::default(),
                grade_bounds: Default::default(),
                scope_kind: Default::default(),
                scope_cache: Default::default(),
                gradient_stop: None,
                gradient_stop_dragged: false,
                gradient_bars: Default::default(),
                user_luts: crate::lut_library::list(),
                lut_previews: Default::default(),
                lut_browser: None,
                lut_hover: None,
                quit_after_export: false,
            },
            focus,
            render,
            sound,
            panels,
            dock,
            _dock_events: dock_events,
        }
    }

    /// Shows the project's name in the title bar, with a dot (and macOS's edited marker on the
    /// close button) while there are unsaved changes.
    fn update_window_title(&mut self, window: &mut Window) {
        let unsaved = self.state.has_unsaved_changes();
        let title = format!("{}{} — Omina Motion", self.state.document_name(), if unsaved { " •" } else { "" });
        if title != self.ui.window_title {
            window.set_window_title(&title);
            window.set_window_edited(unsaved);
            self.ui.window_title = title;
        }
    }

    /// Splits layers at the playhead, explaining any that had to be left whole.
    /// Ungroups the selected nested composition (see `state.ungroup_layer`), saying what
    /// couldn't come along, or why it couldn't be done.
    pub(crate) fn ungroup_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let [layer] = self.state.selected_layers[..] else {
            window.push_notification("Select one nested composition's layer to ungroup.", cx);
            return;
        };
        match self.state.ungroup_layer(layer) {
            Ok(result) if !result.dropped.is_empty() => {
                window.push_notification(format!("Ungrouped. The composition layer's {} don't carry over to its layers.", result.dropped.join(", ")).as_str(), cx);
            }
            Ok(_) => {}
            Err(error) => window.push_notification(error.to_string().as_str(), cx),
        }
        cx.notify();
    }

    pub(crate) fn split_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.ui.render_then_play = false;
        self.state.toggle_playback(Instant::now());
        self.sound.sync(&mut self.state);
        cx.notify();
    }

    /// Render & Play, as After Effects' preview does: renders every frame of the work area
    /// ahead (from its start), then plays it in a loop at full speed. The timeline shows how
    /// far it has got, with Cancel. With rendering ahead switched off, it just plays.
    pub fn render_and_play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.settings.render_ahead_mb == 0 {
            window.push_notification("Rendering ahead is off (Settings ▸ Playback), so this plays without rendering first.", cx);
            if !self.state.is_playing() {
                self.toggle_playback(cx);
            }
            return;
        }
        if self.state.is_playing() {
            self.toggle_playback(cx);
        }
        self.state.current_frame = self.state.work_area.start as i64;
        self.ui.render_then_play = true;
        self.notify_timeline(cx);
        cx.notify();
    }

    /// While Render & Play waits: the frames done and the frames to do, if the cache is on it.
    pub fn render_then_play_progress(&self) -> Option<(usize, usize)> {
        if !self.ui.render_then_play {
            return None;
        }
        self.render.preview.readiness(&self.state.project, self.state.active_comp, self.state.work_area).or(Some((0, 1)))
    }

    /// Starts playing once Render & Play's frames are all ready. Called as frames arrive.
    pub fn check_render_then_play(&mut self, cx: &mut Context<Self>) {
        let Some((done, wanted)) = self.render_then_play_progress() else { return };
        if done < wanted || wanted == 0 {
            return;
        }
        self.ui.render_then_play = false;
        self.state.loop_playback = true;
        self.state.current_frame = self.state.work_area.start as i64;
        if !self.state.is_playing() {
            self.state.toggle_playback(Instant::now());
            self.sound.sync(&mut self.state);
        }
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

    /// Starts recording a voiceover from the microphone at the playhead, playing the
    /// composition so it can be spoken over, or stops the one being recorded and adds it to the
    /// timeline as a clip where it began. Recordings are saved in a Voiceover folder beside the
    /// project (or in Movies for a project not saved yet).
    pub fn toggle_voiceover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(voiceover) = self.ui.voiceover.take() {
            if self.state.is_playing() {
                self.toggle_playback(cx);
            }
            match voiceover.recording.finish() {
                Ok(seconds) if seconds > 0.0 => {
                    // Import the recording, then put it on the timeline where it began.
                    let start_frame = voiceover.start_frame;
                    let place: panels::importing::Then = Box::new(move |this, items, window, cx| {
                        let comp = this.state.active_comp;
                        match items.first().and_then(|&item| studio_core::edit::add_layer_from_item(&mut this.state.project, comp, item, 0, start_frame)) {
                            Some(clip) => {
                                this.state.select_layer(clip, false);
                                window.push_notification(format!("Recorded {seconds:.1} seconds of voiceover.").as_str(), cx);
                            }
                            None => window.push_notification("The voiceover was recorded but could not be added to the timeline. It is in the Project panel.", cx),
                        }
                        cx.notify();
                    });
                    self.import_paths(vec![voiceover.path], Some(place), window, cx);
                }
                Ok(_) => window.push_notification("Nothing was recorded.", cx),
                Err(error) => window.push_notification(format!("The voiceover could not be saved: {error}").as_str(), cx),
            }
            cx.notify();
            return;
        }
        let folder = match self.state.file().and_then(std::path::Path::parent) {
            Some(folder) => folder.join("Voiceover"),
            None => panels::export::starting_folder(None).join("Studio Voiceover"),
        };
        if let Err(error) = std::fs::create_dir_all(&folder) {
            window.push_notification(format!("{} could not be created ({error}).", folder.display()).as_str(), cx);
            return;
        }
        let path = panels::export::unused_path(&folder, "Voiceover", "wav");
        match studio_media::Recording::start(&path) {
            Ok(recording) => {
                self.ui.voiceover = Some(Voiceover { recording, path, start_frame: self.state.current_frame });
                if !self.state.is_playing() {
                    self.toggle_playback(cx);
                }
                window.push_notification("Recording a voiceover. Use headphones, so the microphone doesn't pick up the playback.", cx);
            }
            Err(error) => window.push_notification(format!("Recording could not start: {error}. Check that Omina Motion may use the microphone in System Settings ▸ Privacy & Security.").as_str(), cx),
        }
        cx.notify();
    }

    /// Starts cutting between the selected clips as camera angles, or ends it. While it is on,
    /// the number keys cut to an angle at the playhead (see `cut_to_angle`).
    pub fn toggle_multicam(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ui.multicam.is_empty() {
            self.ui.multicam.clear();
            window.push_notification("Multicam ended. The cuts stay as ordinary clips.", cx);
            cx.notify();
            return;
        }
        let angles = studio_core::multicam::angles_of(self.state.composition(), &self.state.selected_layers);
        if angles.len() < 2 {
            window.push_notification("Select the video clips to cut between first: two or more, stacked on separate tracks.", cx);
            return;
        }
        let count = angles.len().min(9);
        self.ui.multicam = angles;
        window.push_notification(format!("Multicam is on: press 1 to {count} to cut to an angle at the playhead, also while playing. The same command ends it.").as_str(), cx);
        cx.notify();
    }

    /// Cuts to camera angle `angle` (0 for the first) at the playhead, while multicam is on.
    /// The keys do nothing otherwise.
    pub fn cut_to_angle(&mut self, angle: usize, cx: &mut Context<Self>) {
        if self.ui.multicam.is_empty() {
            return;
        }
        self.state.settle_drags();
        let (comp, frame) = (self.state.active_comp, self.state.current_frame);
        if studio_core::multicam::cut_to_angle(&mut self.state.project, comp, &self.ui.multicam, angle, frame) {
            cx.notify();
        }
    }

    /// Plays a sound file from its start, apart from the composition, for the Stock panel's
    /// previews. The composition and the Project panel's preview stop first.
    pub fn play_sound_file(&mut self, path: &std::path::Path, length: f64) {
        if self.state.is_playing() {
            self.state.toggle_playback(Instant::now());
            self.sound.sync(&mut self.state);
        }
        self.stop_preview();
        self.sound.play_preview(path, 0.0, length);
    }

    /// Stops a sound started with `play_sound_file`.
    pub fn stop_sound_file(&mut self) {
        self.sound.stop_preview();
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
        // The Mixer's meter follows the mix while it plays and rests when it stops.
        match self.sound.take_levels().filter(|_| playing) {
            Some(levels) => self.ui.meter.take(levels),
            None => self.ui.meter.rest(),
        }
        // The microphone's meter likewise, while a voiceover is being recorded: it rises at
        // once and falls back gradually, so it doesn't drop to nothing between blocks of sound.
        match &self.ui.voiceover {
            Some(voiceover) => {
                let peak = voiceover.recording.take_peak();
                self.ui.mic_meter.take([peak, peak]);
            }
            None => self.ui.mic_meter.rest(),
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

    /// Switches tools. A pen path still being drawn is finished as an open line first, so
    /// changing tools never throws away what was drawn.
    /// Adds a new solid, adjustment layer or null object (the Layer menu and the top bar's New
    /// Layer button), and shows its properties.
    pub fn new_layer(&mut self, kind: NewLayer, cx: &mut Context<Self>) {
        let added = match kind {
            NewLayer::Solid => self.state.add_solid(),
            NewLayer::Adjustment => self.state.add_adjustment_layer(),
            NewLayer::Null => self.state.add_null(),
        };
        if added.is_some() {
            // An adjustment layer is for effects; the others are set up in Properties.
            self.ui.tabs.insert(PanelId::Properties, if kind == NewLayer::Adjustment { 1 } else { 0 });
            cx.notify();
        }
    }

    /// Sequence Layers (`None`) or Stagger Layers (`Some(frames)`) on the selection, saying
    /// why nothing happened when fewer than two layers are selected.
    pub fn stagger(&mut self, step: Option<i64>, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.selected_layers.len() < 2 {
            window.push_notification("Select two or more layers, in the order they should play.", cx);
        } else if self.state.stagger_selection(step) {
            cx.notify();
        }
    }

    /// Takes changed settings into the editor, whose panels read them as they draw. Features
    /// turned off let go of anything of theirs that is open: their panel, their tool, their
    /// overlay, a background-removal pick.
    pub fn use_settings(&mut self, settings: crate::app_settings::AppSettings) {
        self.ui.project_columns = settings.project_columns;
        self.state.import_folders = settings.import_folders.clone();
        if !settings.panel_on(self.ui.left_panel) {
            self.ui.left_panel = LeftPanel::Media;
        }
        self.ui.settings = settings;
        if !self.ui.settings.tool_on(self.ui.tool) {
            self.set_tool(Tool::Selection);
        }
        if !self.has(crate::features::Feature::Guides) {
            self.ui.guides = false;
        }
        if !self.ui.settings.ai.remove_background {
            self.ui.cutout_pick = None;
        }
    }

    /// Whether a feature is turned on in Settings ▸ Features.
    pub fn has(&self, feature: crate::features::Feature) -> bool {
        self.ui.settings.feature_on(feature)
    }

    pub fn set_tool(&mut self, tool: Tool) {
        // A tool turned off in Settings ▸ Features can't be picked, by key or otherwise.
        if !self.ui.settings.tool_on(tool) {
            return;
        }
        if tool != Tool::Pen {
            self.state.finish_pen(false);
        }
        self.ui.tool = tool;
    }

    /// Opens a text input over a value. Enter or clicking elsewhere commits it.
    pub fn begin_edit(&mut self, target: EditTarget, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_edit(window, cx);
        // Return commits a one-line field. A text layer's field takes Return as a new line, and
        // is committed with ⌘Return, Escape or a click elsewhere, as in After Effects.
        let commits = |event: &InputEvent, multi_line: bool| match event {
            InputEvent::Blur => true,
            InputEvent::PressEnter { secondary, .. } => !multi_line || *secondary,
            _ => false,
        };
        let (input, events, changes) = if target.is_multi_line() {
            // The field reports Return rather than acting on it (otherwise ⌘Return would add a
            // line too); a plain Return is turned into a new line here.
            let input = cx.new(|cx| TextareaState::new(window, cx).submit_on_enter(true).default_value(text.clone()));
            input.update(cx, |state, cx| {
                state.focus(window, cx);
                state.select_all(window, cx);
            });
            let events = cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, window, cx| {
                if commits(event, true) {
                    this.commit_edit(window, cx);
                } else if matches!(event, InputEvent::PressEnter { secondary: false, shift: false }) {
                    input.update(cx, |input, cx| input.replace("\n", window, cx));
                }
            });
            let changes = cx.observe(&input, |this, _, cx| {
                this.ui.caret_epoch = Instant::now();
                cx.notify();
            });
            (EditInput::Lines(input), events, changes)
        } else {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
            input.update(cx, |state, cx| {
                state.focus(window, cx);
                state.select_all(window, cx);
            });
            let events = cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, window, cx| {
                if commits(event, false) {
                    this.commit_edit(window, cx);
                }
            });
            let changes = cx.observe(&input, |this, _, cx| {
                this.ui.caret_epoch = Instant::now();
                cx.notify();
            });
            (EditInput::Line(input), events, changes)
        };
        // The caret drawn in the viewer blinks: redraw at each half beat while typing there.
        let blink = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CARET_BLINK).await;
                let typing = this.update(cx, |this, cx| {
                    let in_viewer = this.ui.editing.as_ref().is_some_and(|editing| editing.in_viewer);
                    if in_viewer {
                        cx.notify();
                    }
                    this.ui.editing.is_some()
                });
                if !matches!(typing, Ok(true)) {
                    break;
                }
            }
        });
        self.ui.caret_epoch = Instant::now();
        self.ui.editing = Some(EditingField { target, input, initial: text, in_viewer: false, _events: events, _changes: changes, _blink: blink });
        cx.notify();
    }

    /// Applies and closes the open text input, if any. Invalid numbers, and text left as it
    /// was, are ignored.
    pub fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.ui.editing.take() else { return };
        let text = editing.input.value(cx).to_string();
        let target = if text == editing.initial { None } else { Some(editing.target) };
        match target {
            None => {}
            Some(EditTarget::Number(target)) => {
                if let Some(value) = parse_value(&text) {
                    target.write(&mut self.state, value);
                }
            }
            Some(EditTarget::SourceText(layer)) => self.state.set_text(layer, &text),
            Some(EditTarget::LibraryName(id)) => self.state.rename_library_item(id, &text),
            Some(EditTarget::CaptionText(layer, start, end)) => self.state.set_caption_text(layer, start..end, &text),
            Some(EditTarget::LayerName(layer)) => self.state.rename_layer(layer, &text),
            Some(EditTarget::ItemName(item)) => self.state.rename_item(item, &text),
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
            this.update_in(cx, |this, window, cx| this.import_paths(paths, None, window, cx)).ok();
        })
        .detach();
    }

    /// Imports files (from the Import dialog, or dropped from the Finder) in the background,
    /// with a popup while it runs, then runs `then` with the new items. Media is linked where it
    /// is on disk, and proxies start building.
    /// The P, R, S and U keys: shows one transform property's keyframe row (or, with `None`,
    /// every animated property's) under the selected clips in the timeline.
    fn show_keyframe_rows(&mut self, only: Option<studio_core::property::TransformProperty>, cx: &mut Context<Self>) {
        panels::timeline::show_keyframe_rows(self, only.map(studio_core::property::PropertyPath::Transform));
        cx.notify();
    }

    pub fn import_paths(&mut self, paths: Vec<std::path::PathBuf>, then: Option<panels::importing::Then>, window: &mut Window, cx: &mut Context<Self>) {
        panels::importing::start(self, paths, then, window, cx);
    }

    /// Where images embedded in imported project files (Lottie animations) are kept, in the cache.
    pub fn imported_folder(&self) -> std::path::PathBuf {
        self.render.proxies.cache_folder().parent().map_or_else(std::env::temp_dir, std::path::Path::to_path_buf).join("Imported")
    }

    /// Where images embedded in an imported project file (a Lottie animation's) are written:
    /// a folder of its own in the cache, so importing the same file again reuses it.
    pub fn imported_assets_folder(&self, path: &std::path::Path) -> std::path::PathBuf {
        self.imported_folder().join(format!("{:x}", {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            path.hash(&mut hasher);
            hasher.finish()
        }))
    }

    /// Adds what an import read to the project and says what came in and what was left out.
    /// Returns the new media items (compositions from project files open instead).
    pub fn add_imported(&mut self, projects: Vec<(std::path::PathBuf, Result<crate::state::ReadProject, String>)>, files: Vec<crate::state::ReadFile>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Id> {
        for (path, read) in projects {
            self.add_imported_project(&path, read, window, cx);
        }
        if files.is_empty() {
            self.request_proxies();
            return Vec::new();
        }
        let (imported, skipped) = self.state.add_read_files(files);
        for &item in &imported {
            self.ui.reveal_item(&self.state.project, item);
        }
        self.request_proxies();
        if !skipped.is_empty() {
            let names: Vec<String> = skipped.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
            let message = format!("Not imported: {}. These are not images, video or sound that can be read.", names.join(", "));
            window.push_notification(message.as_str(), cx);
        }
        cx.notify();
        imported
    }

    /// Adds an After Effects project or a Lottie animation, then says what came in and what
    /// was left out.
    fn add_imported_project(&mut self, path: &std::path::Path, read: Result<crate::state::ReadProject, String>, window: &mut Window, cx: &mut Context<Self>) {
        let file = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        match read.and_then(|read| self.state.add_read_project(&read)) {
            Ok(imported) => {
                if let Some(folder) = imported.folder {
                    self.ui.reveal_item(&self.state.project, folder);
                    if !self.ui.folder_open(folder) {
                        self.ui.toggle_folder(folder);
                    }
                }
                let count = imported.compositions.len();
                let heading = if crate::state::is_extension(path, "svg") {
                    format!("Imported “{file}” as a composition of its shapes. Drag it into a composition; right-click it there and choose Break into Shapes to animate each one.")
                } else {
                    format!("Imported “{file}”: {count} composition{}.", if count == 1 { "" } else { "s" })
                };
                if imported.notes.is_empty() {
                    window.push_notification(heading.as_str(), cx);
                } else {
                    let notes = imported.notes.join("\n\n");
                    crate::panels::chrome::open_alert_dialog(window, cx, move |dialog, _, _| dialog.title(heading.clone()).description(notes.clone()));
                }
            }
            Err(message) => window.push_notification(format!("“{file}” couldn't be imported. {message}").as_str(), cx),
        }
        cx.notify();
    }

    /// Delete key. In the Project panel it asks before deleting the selected item (and the layers
    /// that use it); elsewhere it deletes selected keyframes, or else the selected layers.
    pub(crate) fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // In the Nodes panel, Delete removes the picked node.
        if self.ui.active_panel == PanelId::Nodes {
            if panels::nodes::delete_selected(self) {
                cx.notify();
            }
            return;
        }
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
        crate::panels::chrome::open_alert_dialog(window, cx, move |dialog, _, _| {
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
        // A changed panel arrangement is saved again once a draw has measured it.
        if self.ui.layout_save_countdown > 0 {
            self.ui.layout_save_countdown -= 1;
            if self.ui.layout_save_countdown > 0 {
                // Draw once more, after this draw has measured the panels.
                cx.notify();
            } else if let Some(layout) = self.current_layout(cx) {
                crate::app_settings::update(|settings| settings.workspace_layout = Some(layout));
            }
        }
        // Every edit redraws the workspace, so this is where edits become undo steps, and where
        // manual stops (moving the playhead) and changes such as muting a layer reach the sound.
        // A different item selected in the Project panel starts its preview from the beginning.
        if self.ui.preview.item != self.state.selected_item {
            self.stop_preview();
            self.ui.preview.item = self.state.selected_item;
            self.ui.preview.seconds = 0.0;
        }
        // A file dragged back out of the window ends its drag without a drop or a mouse release,
        // so a drop outline left from it is cleared here.
        if self.ui.slot_drop.is_some() && !cx.has_active_drag() {
            self.ui.slot_drop = None;
        }
        // The Templates panel's pictures are made here, a few per frame, as it needs them.
        // Both start screen pages show templates (Home a strip of them).
        // First, bring the dock up to date: which panels are turned on, and any asked to the front.
        self.sync_dock(window, cx);
        // Editing a shape's points ends once it is no longer the one selected layer.
        if self.ui.path_editing.is_some_and(|layer| self.state.selected_layers != [layer]) {
            self.ui.path_editing = None;
        }
        let start_templates = self.ui.start_screen.is_some();
        if self.showing(PanelId::Templates) || self.ui.template_gallery || start_templates {
            panels::templates::make_pictures(self, window, cx);
        }
        if self.showing(PanelId::Mockups) {
            panels::mockups::keep_up(self, window, cx);
        }
        if self.showing(PanelId::Elements) {
            panels::elements::make_pictures(self, window, cx);
        }
        if self.showing(PanelId::Stock) {
            panels::stock::keep_up(self, window, cx);
        }
        // Stock that finished downloading during another import goes in once that one ends.
        panels::stock::import_downloaded(self, window, cx);
        // A LUT preview ends when its row goes away (another panel opened) as well as when the
        // pointer leaves it.
        if !self.showing(PanelId::Effects) {
            self.ui.lut_hover = None;
        }
        // The LUT previews follow the current frame while the Effects panel or the Color
        // panel's LUT is showing.
        panels::lut_previews::keep_up(self, window, cx);
        panels::assistant::keep_up(self, window, cx);
        // Layers set to Enhance Speech get their cleaned copies made, one at a time.
        panels::enhance::keep_going(self, window, cx);
        let picking = self.ui.color_picker.as_ref().is_some_and(|picker| picker.pressed);
        let dragging = cx.has_active_drag() || picking || self.ui.grade_pressed;
        self.state.record_history(dragging);
        // Library edits are saved once a drag (on the color wheel, say) is over.
        if !dragging && let Some(back_up) = self.state.take_library_save() {
            #[cfg(not(test))]
            if let Err(problem) = crate::library_store::save(&self.state.libraries, back_up) {
                window.push_notification(problem.as_str(), cx);
                // Tried again with the recovery copy, and with the next library change.
                self.ui.library_save_failed = Some(back_up);
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
        // Every panel sits in the dock, where it can be dragged to a new place.
        let workspace = div().id("workspace-dock").size_full().child(self.dock.clone());

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(color::BACKGROUND))
            .text_color(hsla(color::TEXT))
            .text_size(px(crate::theme::size::TEXT))
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            // Escape closes the start screen or the template gallery.
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                // Escape closes the effect search first.
                if this.ui.effect_search.is_some() {
                    panels::effect_search::close(this, window, cx);
                    return;
                }
                // Escape on the SVG question cancels the import it is holding.
                if this.ui.svg_question.is_some() {
                    panels::importing::answer_svg_question(this, None, window, cx);
                    return;
                }
                // The welcome setup is left with its own buttons, so Escape can't skip it by accident.
                if this.ui.onboarding.is_some() {
                    return;
                }
                if this.ui.start_screen.is_some() {
                    panels::start_screen::hide(this);
                    cx.notify();
                    return;
                }
                if this.ui.template_gallery {
                    this.ui.template_gallery = false;
                    cx.notify();
                }
                // Escape also stops editing a shape's points; the shape stays selected.
                if this.ui.path_editing.take().is_some() {
                    cx.notify();
                }
                // Escape also gives up picking a white balance or key color and closes the color menu.
                if this.ui.white_balance_pick.is_some() || this.ui.key_color_pick.is_some() || this.ui.color_well {
                    this.ui.white_balance_pick = None;
                    this.ui.key_color_pick = None;
                    this.ui.color_well = false;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| panels::settings::open(this, window, cx)))
            .on_action(cx.listener(|this, _: &CloseProject, window, cx| this.close_project(window, cx)))
            .on_action(cx.listener(|this, _: &ShowStartScreen, window, cx| panels::start_screen::go_to(this, window, cx)))
            .on_action(cx.listener(|this, _: &BrowseVersions, window, cx| panels::versions::open(this, window, cx)))
            .on_action(cx.listener(|this, action: &OpenRecent, window, cx| panels::start_screen::open_recent(this, action.path.clone(), window, cx)))
            .on_action(cx.listener(|_, _: &ClearRecentProjects, _, cx| {
                crate::app_settings::update(|settings| settings.recent_projects.clear());
                refresh_menus(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &MinimizeWindow, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &ZoomWindow, window, _| window.zoom_window()))
            .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|this, _: &ResetWorkspace, window, cx| this.reset_workspace(window, cx)))
            .on_action(cx.listener(|this, action: &SwitchWorkspace, window, cx| this.switch_workspace(action.name.clone(), window, cx)))
            .on_action(cx.listener(|this, _: &SaveWorkspaceAs, window, cx| panels::workspaces::save_as_new(this, window, cx)))
            .on_action(cx.listener(|this, _: &UpdateWorkspace, window, cx| panels::workspaces::update_current(this, window, cx)))
            .on_action(cx.listener(|this, _: &DeleteWorkspace, window, cx| panels::workspaces::delete_current(this, window, cx)))
            .on_action(cx.listener(|this, _: &AboutStudio, window, cx| panels::settings::open_about(this, window, cx)))
            .on_action(cx.listener(|this, _: &CheckForUpdates, window, cx| crate::updates::check(this, true, window, cx)))
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open(window, cx)))
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.quit(window, cx)))
            .on_action(cx.listener(|this, _: &NewProject, window, cx| this.new_project(window, cx)))
            // The editor's own commands are only there while the editor shows. Behind the start
            // screen, shortcuts and menu commands would otherwise change a project nobody can see;
            // without a handler, their menu items are greyed out too.
            .when(self.ui.start_screen.is_none() && self.ui.onboarding.is_none(), |root| {
                root
                .on_action(cx.listener(|this, _: &TogglePlayback, _, cx| this.toggle_playback(cx)))
                // A shortcut given to one of Iris's recipes runs it, with what it did in her chat.
                .on_action(cx.listener(|this, action: &crate::keymap::RunRecipe, window, cx| panels::assistant_automation::run_from_key(this, &action.name, window, cx)))
                .on_action(cx.listener(|this, _: &RenderAndPlay, window, cx| this.render_and_play(window, cx)))
                .on_action(cx.listener(|this, _: &NextFrame, _, cx| this.step(1, cx)))
                .on_action(cx.listener(|this, _: &PreviousFrame, _, cx| this.step(-1, cx)))
                .on_action(cx.listener(|this, _: &NextTenFrames, _, cx| this.step(10, cx)))
                .on_action(cx.listener(|this, _: &PreviousTenFrames, _, cx| this.step(-10, cx)))
                .on_action(cx.listener(|this, _: &NextFiveFrames, _, cx| this.step(5, cx)))
                .on_action(cx.listener(|this, _: &PreviousFiveFrames, _, cx| this.step(-5, cx)))
                .on_action(cx.listener(|this, _: &ZoomViewerIn, _, cx| {
                    panels::viewer::zoom_step(this, true);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ZoomViewerOut, _, cx| {
                    panels::viewer::zoom_step(this, false);
                    cx.notify();
                }))
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
                .on_action(cx.listener(|this, _: &EyedropperTool, _, cx| {
                    this.set_tool(Tool::Eyedropper);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &RateStretchTool, _, cx| {
                    this.set_tool(Tool::RateStretch);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ToggleMarker, _, cx| {
                    this.state.toggle_marker_at_playhead();
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &NewSolid, _, cx| this.new_layer(NewLayer::Solid, cx)))
                .on_action(cx.listener(|this, _: &NewAdjustmentLayer, _, cx| this.new_layer(NewLayer::Adjustment, cx)))
                .on_action(cx.listener(|this, _: &NewNull, _, cx| this.new_layer(NewLayer::Null, cx)))
                .on_action(cx.listener(|this, _: &SequenceLayers, window, cx| this.stagger(None, window, cx)))
                .on_action(cx.listener(|this, _: &StaggerLayers, window, cx| this.stagger(Some(STAGGER_FRAMES), window, cx)))
                .on_action(cx.listener(|this, _: &ToggleGuides, _, cx| {
                    this.ui.guides = !this.ui.guides;
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ToggleLayoutGrid, _, cx| toggle_setting(this, cx, |settings| &mut settings.show_grid)))
                .on_action(cx.listener(|this, _: &ToggleSnapToGrid, _, cx| toggle_setting(this, cx, |settings| &mut settings.snap_to_grid)))
                .on_action(cx.listener(|this, _: &ToggleSnapToCenter, _, cx| toggle_setting(this, cx, |settings| &mut settings.snap_to_center)))
                .on_action(cx.listener(|this, _: &ToggleSnapToDeadCenter, _, cx| toggle_setting(this, cx, |settings| &mut settings.snap_to_dead_center)))
                .on_action(cx.listener(|this, _: &ShowPositionKeyframes, _, cx| this.show_keyframe_rows(Some(studio_core::property::TransformProperty::Position), cx)))
                .on_action(cx.listener(|this, _: &ShowRotationKeyframes, _, cx| this.show_keyframe_rows(Some(studio_core::property::TransformProperty::Rotation), cx)))
                .on_action(cx.listener(|this, _: &ShowScaleKeyframes, _, cx| this.show_keyframe_rows(Some(studio_core::property::TransformProperty::Scale), cx)))
                .on_action(cx.listener(|this, _: &ShowAllKeyframes, _, cx| this.show_keyframe_rows(None, cx)))
                .on_action(cx.listener(|this, _: &ToggleLayerControls, _, cx| {
                    this.ui.layer_controls_hidden = !this.ui.layer_controls_hidden;
                    cx.notify();
                }))
                // Enter ends a pen path as an open line (a path closes by clicking its first point).
                .on_action(cx.listener(|this, _: &FinishPath, _, cx| {
                    let finished = this.state.finish_pen(false);
                    if finished.is_some() {
                        this.ui.tabs.insert(PanelId::Properties, 0);
                        // A path just drawn keeps its points up, to adjust them straight away.
                        this.ui.path_editing = finished;
                    }
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Precompose, window, cx| panels::precompose::open(this, window, cx)))
                .on_action(cx.listener(|this, _: &ExportQueue, window, cx| panels::export_queue::open(this, window, cx)))
                .on_action(cx.listener(|this, _: &UngroupComposition, window, cx| this.ungroup_selected(window, cx)))
                .on_action(cx.listener(|this, _: &SearchEffects, window, cx| panels::effect_search::open(this, window, cx)))
                .on_action(cx.listener(|this, _: &MorphTo, window, cx| panels::morph::open(this, window, cx)))
                .on_action(cx.listener(|this, _: &FlipHorizontal, _, cx| {
                    if this.state.flip_selection(true) {
                        cx.notify();
                    }
                }))
                .on_action(cx.listener(|this, _: &FlipVertical, _, cx| {
                    if this.state.flip_selection(false) {
                        cx.notify();
                    }
                }))
                .on_action(cx.listener(|this, _: &ResetTransform, _, cx| {
                    this.state.reset_selection_transform();
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Cut, _, cx| {
                    if this.state.cut_selection() > 0 {
                        cx.notify();
                    }
                }))
                .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                    this.state.select_all_layers();
                    cx.notify();
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
                // In the Project panel, Duplicate copies the selected item; elsewhere, the selected layers.
                .on_action(cx.listener(|this, _: &Duplicate, window, cx| {
                    if this.ui.active_panel == PanelId::Project {
                        let Some(item) = this.state.selected_item else { return };
                        if this.state.duplicate_item(item).is_none() {
                            window.push_notification("Folders can't be duplicated. Select a composition or media item.", cx);
                        }
                        cx.notify();
                    } else if !this.state.duplicate_selection().is_empty() {
                        cx.notify();
                    }
                }))
                .on_action(cx.listener(|this, _: &CompositionSettings, window, cx| {
                    panels::composition_settings::open(this, false, window, cx);
                }))
                .on_action(cx.listener(|this, _: &MagicResize, window, cx| panels::resize::open(this, window, cx)))
                .on_action(cx.listener(|this, _: &MagicMove, window, cx| panels::magic_move::open(this, window, cx)))
                .on_action(cx.listener(|this, _: &Import, window, cx| this.import(window, cx)))
                // Offered only when there is something to undo or redo, so the Edit menu greys them out otherwise.
                .when(self.state.can_undo(), |root| root.on_action(cx.listener(|this, _: &Undo, _, cx| this.undo_or_redo(false, cx))))
                .when(self.state.can_redo(), |root| root.on_action(cx.listener(|this, _: &Redo, _, cx| this.undo_or_redo(true, cx))))
                .on_action(cx.listener(|this, _: &Save, window, cx| this.save(window, cx)))
                .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.save_as(window, cx)))
                .on_action(cx.listener(|this, _: &Export, window, cx| panels::export_dialog::open(this, window, cx)))
                .on_action(cx.listener(|this, _: &SaveFrame, window, cx| panels::export::save_frame(this, window, cx)))
                .on_action(cx.listener(|this, _: &CopyFrame, window, cx| panels::export::copy_frame(this, window, cx)))
                .on_action(cx.listener(|this, _: &SyncBySound, window, cx| panels::analysis::sync_by_sound(this, window, cx)))
                .on_action(cx.listener(|this, _: &ToggleMulticam, window, cx| this.toggle_multicam(window, cx)))
                .on_action(cx.listener(|this, _: &ToggleVoiceover, window, cx| this.toggle_voiceover(window, cx)))
                .on_action(cx.listener(|this, _: &MulticamAngle1, _, cx| this.cut_to_angle(0, cx)))
                .on_action(cx.listener(|this, _: &MulticamAngle2, _, cx| this.cut_to_angle(1, cx)))
                .on_action(cx.listener(|this, _: &MulticamAngle3, _, cx| this.cut_to_angle(2, cx)))
                .on_action(cx.listener(|this, _: &MulticamAngle4, _, cx| this.cut_to_angle(3, cx)))
                .on_action(cx.listener(|this, _: &MulticamAngle5, _, cx| this.cut_to_angle(4, cx)))
                .on_action(cx.listener(|this, _: &MulticamAngle6, _, cx| this.cut_to_angle(5, cx)))
                .on_action(cx.listener(|this, _: &MulticamAngle7, _, cx| this.cut_to_angle(6, cx)))
                .on_action(cx.listener(|this, _: &MulticamAngle8, _, cx| this.cut_to_angle(7, cx)))
                .on_action(cx.listener(|this, _: &MulticamAngle9, _, cx| this.cut_to_angle(8, cx)))
                // Greyed out in the File menu while a package is being made (the top bar shows its progress).
                .when(self.ui.package_job.is_none(), |root| root.on_action(cx.listener(|this, _: &Package, window, cx| panels::package::open(this, window, cx))))
                .on_action(cx.listener(|this, _: &SplitLayer, window, cx| this.split_layer(window, cx)))
                .on_action(cx.listener(|this, _: &ToggleSnapping, _, cx| {
                    this.state.snapping = !this.state.snapping;
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &PasteAttributes, window, cx| panels::paste_attributes::open(this, window, cx)))
                .on_action(cx.listener(|this, _: &CloseGaps, window, cx| {
                    if this.state.close_gaps() == 0 {
                        window.push_notification("There are no gaps to close.", cx);
                    }
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
                .on_action(cx.listener(|this, _: &NewComposition, window, cx| {
                    panels::composition_settings::open(this, true, window, cx);
                }))
            })
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
                this.ui.eyedropper_press = false;
                this.ui.grade_pressed = false;
                this.ui.effect_drop_layer = None;
                this.ui.slot_drop = None;
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
            .children(panels::dock::panel_menu(self))
            .children(panels::templates::gallery(self, cx))
            .children(panels::start_screen::render(self, cx))
            .children(panels::onboarding::render(self, cx))
            .children(panels::importing::popup(self, cx))
            .children(panels::importing::svg_popup(self, cx))
            .children(panels::effect_search::render(self, cx))
    }
}

impl Workspace {
    /// Builds the dock with the arrangement saved last time (or the standard one), and listens
    /// for changes to save them.
    fn make_dock(panels: &Panels, window: &mut Window, cx: &mut Context<Self>) -> (Entity<DockArea>, Subscription) {
        let skin = panels::dock::StudioSkin::new(cx.weak_entity());
        let dock = cx.new(|cx| DockArea::new("studio-workspace", Some(panels::dock::LAYOUT_VERSION), window, cx).with_renderer(skin));
        let view = |id| panels.view(id).clone();
        let settings = crate::app_settings::load();
        let saved = settings.workspace_layout.and_then(|saved| serde_json::from_value::<DockAreaState>(saved).ok());
        // With no arrangement of the user's own saved, the current workspace's built-in one.
        let built_in = settings.current_workspace.as_deref().and_then(panels::workspaces::BuiltIn::named);
        let layout = saved.and_then(|saved| panels::dock::restore_layout(&saved, view)).unwrap_or_else(|| panels::dock::built_in_layout(built_in, view));
        dock.update(cx, |dock, cx| dock.set_center(layout, window, cx));
        let events = cx.subscribe(&dock, |this, dock, event: &DockEvent, cx| {
            if let DockEvent::LayoutChanged = event {
                if std::mem::take(&mut this.ui.layout_resetting) {
                    this.ui.layout_save_countdown = 0;
                    crate::app_settings::update(|settings| settings.workspace_layout = None);
                } else {
                    // Saved now, and again once it has been drawn, with the sizes as measured:
                    // the first draw from here measures the new arrangement, the second saves it.
                    this.ui.layout_save_countdown = 2;
                    match serde_json::to_value(dock.read(cx).dump(cx)) {
                        Ok(layout) => crate::app_settings::update(|settings| settings.workspace_layout = Some(layout)),
                        Err(error) => eprintln!("could not save the panel layout: {error}"),
                    }
                }
                // The rail and other panels show which panels are in front.
                this.refresh_showing(cx);
                cx.notify();
            }
        });
        (dock, events)
    }

    /// Window ▸ Reset Workspace: the current workspace back as it was saved (every panel in
    /// its standard place, for Standard), undoing any moves since.
    pub fn reset_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = crate::app_settings::load().current_workspace;
        self.switch_workspace(current, window, cx);
    }

    /// Switches to the workspace named `name` (`None` for Standard) and remembers the choice: a
    /// built-in one by its name, or else a saved one. A saved workspace that can't be read (one
    /// written by a newer Studio, or damaged) is said so, and Standard is used instead.
    pub fn switch_workspace(&mut self, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let saved = name.as_ref().and_then(|name| crate::app_settings::load().saved_workspaces.into_iter().find(|saved| &saved.name == name));
        let built_in = name.as_deref().and_then(panels::workspaces::BuiltIn::named);
        let shown = match (&name, saved) {
            (None, _) => None,
            (Some(name), None) if built_in.is_some() => {
                self.show_built_in(built_in, window, cx);
                Some(name.clone())
            }
            (Some(name), Some(saved)) if self.show_saved_layout(&saved.layout, window, cx) => Some(name.clone()),
            (Some(name), _) => {
                window.push_notification(format!("The “{name}” workspace couldn't be read, so Standard is shown instead.").as_str(), cx);
                None
            }
        };
        if shown.is_none() {
            self.show_built_in(None, window, cx);
        }
        crate::app_settings::update(|settings| settings.current_workspace = shown);
        refresh_menus(cx);
        cx.notify();
    }

    /// Puts a built-in arrangement on screen (`None` for Standard). It isn't saved as a custom
    /// arrangement: the next launch builds it afresh from the workspace's name.
    fn show_built_in(&mut self, built_in: Option<panels::workspaces::BuiltIn>, window: &mut Window, cx: &mut Context<Self>) {
        let layout = panels::dock::built_in_layout(built_in, |id| self.panels.view(id).clone());
        if built_in == Some(panels::workspaces::BuiltIn::ColorCorrection) {
            // Properties opens on its Color tab, which is what this workspace is for.
            self.ui.tabs.insert(PanelId::Properties, 2);
        }
        self.ui.layout_resetting = true;
        self.dock.update(cx, |dock, cx| dock.set_center(layout, window, cx));
    }

    /// Puts a saved layout on screen. False if it can't be read, leaving the screen as it was.
    fn show_saved_layout(&mut self, layout: &serde_json::Value, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Ok(state) = serde_json::from_value::<DockAreaState>(layout.clone()) else { return false };
        let Some(layout) = panels::dock::restore_layout(&state, |id| self.panels.view(id).clone()) else { return false };
        self.dock.update(cx, |dock, cx| dock.set_center(layout, window, cx));
        true
    }

    /// The panel layout on screen, in its saved form.
    pub fn current_layout(&self, cx: &App) -> Option<serde_json::Value> {
        serde_json::to_value(self.dock.read(cx).dump(cx)).inspect_err(|error| eprintln!("could not save the panel layout: {error}")).ok()
    }

    /// Asks for `panel` to be brought to the front of its tab group, wherever it has been
    /// moved to. Done on the next draw. One of the side panel's own (Effects, say) is shown in
    /// the side panel, which comes to the front.
    pub fn reveal(&mut self, panel: PanelId) {
        if let Some(side) = side_choice(panel) {
            self.ui.left_panel = side;
            self.ui.reveal_panels.push(PanelId::Side);
        } else {
            self.ui.reveal_panels.push(panel);
        }
    }

    /// Whether `panel` is on screen: turned on, and the front tab of its group. One of the side
    /// panel's own is on screen while the side panel is and shows it.
    pub fn showing(&self, panel: PanelId) -> bool {
        match side_choice(panel) {
            Some(side) => self.ui.left_panel == side && self.ui.showing_panels.contains(&PanelId::Side),
            None => self.ui.showing_panels.contains(&panel),
        }
    }

    /// Whether panel `id` is turned on: the Nodes panel while it is open, the Mixer unless it
    /// is turned off in Settings ▸ Tools & Features, every other panel always.
    fn panel_turned_on(&self, id: PanelId) -> bool {
        match id {
            PanelId::Nodes => self.ui.nodes.open,
            PanelId::Mixer => self.has(crate::features::Feature::Mixer),
            _ => true,
        }
    }

    /// Before each draw: tells each panel view whether it is turned on (the dock hides those
    /// that aren't), brings forward any panels asked for, and notes which are now on screen.
    fn sync_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for id in panels::dock::DOCKABLE {
            let on = self.panel_turned_on(id);
            self.panels.view(id).read(cx).shown.set(on);
        }
        for id in std::mem::take(&mut self.ui.reveal_panels) {
            let panel = gpui_kit::component::dock::PanelId::from(self.panels.view(id).entity_id());
            self.dock.update(cx, |dock, cx| dock.select_panel(panel, window, cx));
        }
        self.refresh_showing(cx);
    }

    /// Notes which panels are on screen, from the dock's current arrangement.
    fn refresh_showing(&mut self, cx: &App) {
        let dock = self.dock.read(cx);
        let front: HashSet<gpui_kit::component::dock::PanelId> = dock
            .layout(gpui_kit::component::dock::DockPlacement::Center)
            .map(|tree| {
                let mut front = HashSet::new();
                tree.root().walk(&mut |node| {
                    if let gpui_kit::component::dock::PaneRef::Tabs { panels, active_ix } = node.kind() {
                        // The front tab, or the first turned-on one if the front one is off.
                        let shown = |index: &usize| panels.get(*index).is_some_and(|panel| self.dock_panel_on(*panel, cx));
                        if let Some(index) = std::iter::once(active_ix).chain(0..panels.len()).find(shown) {
                            front.insert(panels[index]);
                        }
                    }
                });
                front
            })
            .unwrap_or_default();
        self.ui.showing_panels = panels::dock::DOCKABLE
            .into_iter()
            .filter(|id| front.contains(&gpui_kit::component::dock::PanelId::from(self.panels.view(*id).entity_id())))
            .filter(|id| self.panel_turned_on(*id))
            .collect();
    }

    /// Whether the dock entry `panel` is one of the panels turned on.
    fn dock_panel_on(&self, panel: gpui_kit::component::dock::PanelId, cx: &App) -> bool {
        panels::dock::DOCKABLE
            .into_iter()
            .find(|id| gpui_kit::component::dock::PanelId::from(self.panels.view(*id).entity_id()) == panel)
            .is_some_and(|id| self.panels.view(id).read(cx).shown.get())
    }
}

/// Which rail choice shows `panel` in the side panel, if it is one of the side panel's own.
fn side_choice(panel: PanelId) -> Option<LeftPanel> {
    Some(match panel {
        PanelId::Project => LeftPanel::Media,
        PanelId::Effects => LeftPanel::Effects,
        PanelId::Libraries => LeftPanel::Libraries,
        PanelId::Templates => LeftPanel::Templates,
        PanelId::Captions => LeftPanel::Captions,
        PanelId::Elements => LeftPanel::Elements,
        PanelId::Mockups => LeftPanel::Mockups,
        PanelId::Stock => LeftPanel::Stock,
        _ => return None,
    })
}

/// "1 layer is" or "3 layers are", for messages.
fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
}
