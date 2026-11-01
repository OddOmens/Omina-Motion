//! Editor state and every action the UI can take on it. This module has no
//! GPUI dependency, so the editing behavior is unit-tested directly.
//!
//! Split by topic: this file holds playback, selection, layer edits, undo and
//! project files; `keyframes` holds keyframe selection, dragging and easing;
//! `viewer` holds moving and scaling layers by dragging them in the composition
//! viewer; `layer_time` holds moving, trimming and slipping layers in time; `snap`
//! holds snapping while dragging in the timeline; `clips` holds editing clips on
//! tracks (the Tracks view) and the magnetic main track;
//! `history` holds the undo steps.

mod captions;
mod clips;
mod copy;
mod history;
mod keyframes;
mod layer_time;
mod library;
mod snap;
mod viewer;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use studio_core::align::{AlignTarget, Alignment, Distribution, align_layers, distribute_layers};
use studio_core::animation::Vec2;
use studio_core::edit::{self, CompositionSettings, ItemDeletion};
use studio_core::model::{
    BlendMode, Composition, Id, LayerContent, LayerSwitches, Paint, Project, Rgba, ShapeKind, ShapeStyle, Stroke, TextAlign, TextStyle,
    TrackMatte,
};
use studio_core::playback::{playback_position, playback_start_frame};
use studio_core::project_file::{self, EditorSettings, EditorView};
use studio_core::property::{PropertyPath, PropertyValue, TransformProperty};
use studio_core::sample::{ids, sample_project};
use studio_core::time::FrameRange;

pub use captions::HeardWord;
pub use keyframes::KeyframeRef;
pub use layer_time::LayerTimeEdit;
pub use viewer::{ANCHOR_POINTS, ScaleHandle};

/// The view range never shrinks below this many frames, so zooming in stops at a readable level.
pub const MIN_VIEW_FRAMES: f64 = 10.0;

/// Which layer switch a toggle controls (on clips, and in the Properties panel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Visible,
    Audio,
    Solo,
    Locked,
    EffectsEnabled,
}

impl Switch {
    fn slot(self, switches: &mut LayerSwitches) -> &mut bool {
        match self {
            Self::Visible => &mut switches.visible,
            Self::Audio => &mut switches.audio,
            Self::Solo => &mut switches.solo,
            Self::Locked => &mut switches.locked,
            Self::EffectsEnabled => &mut switches.effects_enabled,
        }
    }
}

/// A layer's sound settings that the Properties panel edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioSetting {
    /// In decibels.
    Volume,
    /// In seconds.
    FadeIn,
    FadeOut,
}

/// Numeric shape settings that the Properties panel edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeSetting {
    Width,
    Height,
    /// A polygon's or star's outer radius: half its box (which it keeps square).
    OuterRadius,
    /// A star's inner radius, in pixels (stored as a fraction of the outer radius).
    InnerRadius,
    CornerRadius,
    /// Sides of a polygon, or points of a star.
    Points,
    StrokeWidth,
    /// The direction of a linear gradient, in degrees.
    FillAngle,
    StrokeAngle,
}

/// Which of a shape's paints: its fill or its stroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaintSlot {
    Fill,
    Stroke,
}

/// A color the color picker edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorTarget {
    /// Color `stop` of a shape's fill or stroke (0 for a solid; 0 or 1 along a gradient).
    Shape { layer: Id, slot: PaintSlot, stop: usize },
    TextFill(Id),
    TextStroke(Id),
    /// A color in a library; changing it changes everything linked to it.
    Library(studio_core::model::LinkId),
    /// A caption layer's spoken-word color, and its background box.
    CaptionHighlight(Id),
    CaptionBackground(Id),
}

/// A paint's angle, if it is a linear gradient.
fn paint_angle(paint: Option<Paint>) -> f32 {
    match paint {
        Some(Paint::Linear { angle, .. }) => angle,
        _ => 0.0,
    }
}

/// Sets a linear gradient's angle (other paints have none).
fn set_angle(paint: &mut Paint, value: f32) {
    if let Paint::Linear { angle, .. } = paint {
        *angle = value;
    }
}

impl ShapeSetting {
    /// The setting's value for a shape of `style` in a box of `size`.
    pub fn get(self, style: &ShapeStyle, size: Vec2) -> f32 {
        let outer = size.x.min(size.y) * 0.5;
        match self {
            Self::Width => size.x,
            Self::Height => size.y,
            Self::OuterRadius => outer,
            Self::InnerRadius => style.inner_radius * outer,
            Self::CornerRadius => style.corner_radius,
            Self::Points => style.points as f32,
            Self::StrokeWidth => style.stroke.map_or(0.0, |stroke| stroke.width),
            Self::FillAngle => paint_angle(style.fill),
            Self::StrokeAngle => paint_angle(style.stroke.map(|stroke| stroke.paint)),
        }
    }
}

/// Numeric text settings that the Properties panel edits./// Numeric text settings that the Properties panel edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSetting {
    FontSize,
    Leading,
    Tracking,
    StrokeWidth,
}

impl TextSetting {
    pub fn get(self, style: &TextStyle) -> f32 {
        match self {
            Self::FontSize => style.font_size,
            Self::Leading => style.line_height,
            Self::Tracking => style.tracking,
            Self::StrokeWidth => style.stroke.map_or(0.0, |(_, width)| width),
        }
    }

    fn set(self, style: &mut TextStyle, value: f32) {
        match self {
            Self::FontSize => {
                // Leading follows the font size, as After Effects does with auto leading.
                let size = value.clamp(1.0, 2000.0);
                style.line_height *= size / style.font_size.max(1.0);
                style.font_size = size;
            }
            Self::Leading => style.line_height = value.clamp(1.0, 4000.0),
            Self::Tracking => style.tracking = value.clamp(-1000.0, 10000.0),
            Self::StrokeWidth => {
                if let Some((_, width)) = style.stroke.as_mut() {
                    *width = value.clamp(0.0, 1000.0);
                }
            }
        }
    }
}

/// What the Delete key removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Deleted {
    Nothing,
    Keyframes(usize),
    Layers(usize),
}

/// When playback started, so the playhead can be derived from elapsed wall-clock time.
#[derive(Clone, Copy, Debug)]
struct PlaybackClock {
    started_at: Instant,
    start_frame: i64,
}

pub struct EditorState {
    pub project: Project,
    pub active_comp: Id,
    pub selected_item: Option<Id>,
    pub current_frame: i64,
    pub work_area: FrameRange,
    pub view_range: FrameRange,
    pub loop_playback: bool,
    pub selected_layers: Vec<Id>,
    pub selected_keyframes: Vec<KeyframeRef>,
    /// Panel sections the user opened or closed, by key (effects in Effect Controls).
    pub expanded_rows: HashSet<String>,
    pub align_target: AlignTarget,
    /// When on, editing one scale component changes the other proportionally.
    pub scale_linked: bool,
    /// When on, a shape's width and height change together.
    pub shape_size_linked: bool,
    /// Whether anchor points snap to their layer's corners, edge middles and center in the viewer.
    pub viewer_snapping: bool,
    playback: Option<PlaybackClock>,
    keyframe_drag: Option<keyframes::KeyframeDrag>,
    /// Whether the main track keeps its clips packed together from frame 0, as in CapCut.
    pub magnetic_main_track: bool,
    /// A clip being dragged in the timeline's Tracks view.
    clip_drag: Option<clips::ClipDrag>,
    /// Whether timeline drags snap (S toggles it), how far they reach in frames (set by the
    /// timeline from its zoom), and the frame the current drag snapped to.
    pub snapping: bool,
    pub snap_threshold: i64,
    snap_line: Option<i64>,
    /// The layer bar (or bar end) being dragged in time.
    layer_time_drag: Option<layer_time::LayerTimeDrag>,
    viewer_drag: Option<viewer::ViewerDrag>,
    history: history::History,
    /// The brands, colors and text styles shared by every project (see `library_store`).
    pub libraries: studio_core::library::Libraries,
    library_save: library::LibrarySave,
    /// Layers copied with Cmd+C, ready to paste.
    clipboard: Vec<studio_core::model::Layer>,
    /// The project file being edited; `None` until the project is first saved.
    file: Option<PathBuf>,
}

impl EditorState {
    pub fn new() -> Self {
        let project = sample_project();
        let duration = project.composition(ids::MAIN_COMP).map_or(240, |comp| comp.duration_frames) as f64;
        Self {
            active_comp: ids::MAIN_COMP,
            selected_item: Some(ids::MAIN_COMP),
            current_frame: 0,
            work_area: FrameRange::new(0.0, duration),
            view_range: FrameRange::new(0.0, duration),
            loop_playback: true,
            selected_layers: Vec::new(),
            selected_keyframes: Vec::new(),
            expanded_rows: HashSet::new(),
            align_target: AlignTarget::Composition,
            scale_linked: true,
            shape_size_linked: false,
            viewer_snapping: true,
            playback: None,
            keyframe_drag: None,
            layer_time_drag: None,
            magnetic_main_track: true,
            clip_drag: None,
            snapping: true,
            snap_threshold: 0,
            snap_line: None,
            viewer_drag: None,
            history: history::History::new(&project),
            clipboard: Vec::new(),
            libraries: studio_core::library::Libraries::default(),
            library_save: library::LibrarySave::Saved,
            file: None,
            project,
        }
    }

    pub fn composition(&self) -> &Composition {
        self.project.composition(self.active_comp).expect("the active composition exists")
    }

    /// Direct access for UI tests that need to set up a particular composition.
    #[cfg(test)]
    pub fn composition_mut_for_test(&mut self) -> &mut Composition {
        self.composition_mut()
    }

    fn composition_mut(&mut self) -> &mut Composition {
        self.project.composition_mut(self.active_comp).expect("the active composition exists")
    }

    fn duration(&self) -> i64 {
        self.composition().duration_frames
    }

    // Playback ---------------------------------------------------------------

    pub fn is_playing(&self) -> bool {
        self.playback.is_some()
    }

    pub fn toggle_playback(&mut self, now: Instant) {
        if self.playback.take().is_none() {
            let start_frame = playback_start_frame(self.current_frame, self.work_area);
            self.current_frame = start_frame;
            self.playback = Some(PlaybackClock { started_at: now, start_frame });
        }
    }

    /// When the current playback started and from which frame, if playing. A new pair means a new playback.
    pub fn playback_started(&self) -> Option<(Instant, i64)> {
        self.playback.map(|clock| (clock.started_at, clock.start_frame))
    }

    /// Starts the current playback over from the playhead, for example after the work area changed.
    pub fn restart_playback(&mut self, now: Instant) {
        if self.playback.is_some() {
            let start_frame = playback_start_frame(self.current_frame, self.work_area);
            self.current_frame = start_frame;
            self.playback = Some(PlaybackClock { started_at: now, start_frame });
        }
    }

    /// Advances the playhead to where it should be at `now`, by the wall clock. Returns whether
    /// playback is still running.
    pub fn tick_playback(&mut self, now: Instant) -> bool {
        let Some(clock) = self.playback else { return false };
        self.advance_playback(now.saturating_duration_since(clock.started_at))
    }

    /// Advances the playhead to `elapsed` into the current playback. With sound, `elapsed` is how
    /// much has been heard, so the picture stays in step with it. Returns whether playback is still running.
    pub fn advance_playback(&mut self, elapsed: Duration) -> bool {
        let Some(clock) = self.playback else { return false };
        let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
        let rate = self.composition().frame_rate;
        let position = playback_position(clock.start_frame, elapsed_ms, rate, self.work_area, self.loop_playback);
        self.current_frame = position.frame;
        if position.ended {
            self.playback = None;
        }
        !position.ended
    }

    /// Moves the playhead. Any manual move stops playback, as in After Effects.
    pub fn set_current_frame(&mut self, frame: i64) {
        self.playback = None;
        self.current_frame = frame.clamp(0, self.duration() - 1);
    }

    // Time ranges ------------------------------------------------------------

    pub fn set_view_range(&mut self, range: FrameRange) {
        let duration = self.duration() as f64;
        self.view_range = range.clamped(duration, MIN_VIEW_FRAMES);
    }

    pub fn set_work_area(&mut self, range: FrameRange) {
        let duration = self.duration() as f64;
        let rounded = FrameRange::new(range.start.round(), range.end.round());
        self.work_area = rounded.clamped(duration, 1.0);
    }

    // Selection and rows -----------------------------------------------------

    /// Selects a layer. Selecting layers clears any keyframe selection, as in After Effects.
    /// Locked layers cannot be selected, so nothing can move, scale or delete them.
    pub fn select_layer(&mut self, id: Id, additive: bool) {
        if self.composition().layer(id).is_some_and(|layer| layer.switches.locked) {
            return;
        }
        self.selected_keyframes.clear();
        if !additive {
            self.selected_layers = vec![id];
        } else if let Some(index) = self.selected_layers.iter().position(|selected| *selected == id) {
            self.selected_layers.remove(index);
        } else {
            self.selected_layers.push(id);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected_layers.clear();
        self.selected_keyframes.clear();
    }

    pub fn toggle_row(&mut self, key: &str) {
        if !self.expanded_rows.remove(key) {
            self.expanded_rows.insert(key.to_owned());
        }
    }

    // Property values --------------------------------------------------------

    /// Sets one component of a property at the playhead (auto-keyframing animated properties).
    /// With scale linked, changing one scale component changes the other in proportion.
    ///
    /// Mass edit: if the playhead is on a selected keyframe of this property, every selected
    /// keyframe of the property gets the new value, as in After Effects.
    pub fn set_property_component(&mut self, layer_id: Id, path: PropertyPath, component: usize, value: f32) {
        let frame = self.current_frame;
        let selected: Vec<i64> =
            self.selected_keyframes.iter().filter(|key| key.layer == layer_id && key.path == path).map(|key| key.frame).collect();
        let frames = if selected.contains(&frame) { selected } else { vec![frame] };
        for frame in frames {
            self.set_component_at(layer_id, path, frame, component, value);
        }
    }

    fn set_component_at(&mut self, layer_id: Id, path: PropertyPath, frame: i64, component: usize, value: f32) {
        let linked = self.scale_linked && path == PropertyPath::Transform(TransformProperty::Scale);
        let Some(layer) = self.composition_mut().layer_mut(layer_id) else { return };
        if layer.switches.locked {
            return;
        }
        let Some(current) = layer.property(path, frame).map(|snapshot| snapshot.value) else { return };
        let updated = match current {
            PropertyValue::Pair(pair) if linked => {
                let old = current.component(component);
                let ratio = if old.abs() > f32::EPSILON { value / old } else { 1.0 };
                let other = 1 - component.min(1);
                PropertyValue::Pair(pair).with_component(component, value).with_component(other, current.component(other) * ratio)
            }
            _ => current.with_component(component, value),
        };
        layer.set_property(path, frame, updated);
    }

    /// Resets a layer's transform to its defaults at the playhead (keyframing animated properties).
    pub fn reset_transform(&mut self, layer_id: Id) {
        let frame = self.current_frame;
        let comp_center = self.composition().size() * 0.5;
        let Some(layer) = self.composition_mut().layer_mut(layer_id) else { return };
        let anchor = layer.size * 0.5;
        let defaults = [
            (TransformProperty::AnchorPoint, PropertyValue::Pair(anchor)),
            (TransformProperty::Position, PropertyValue::Pair(comp_center)),
            (TransformProperty::Scale, PropertyValue::Pair(Vec2::new(100.0, 100.0))),
            (TransformProperty::Rotation, PropertyValue::Scalar(0.0)),
            (TransformProperty::Opacity, PropertyValue::Scalar(100.0)),
        ];
        for (property, value) in defaults {
            layer.set_property(PropertyPath::Transform(property), frame, value);
        }
    }

    // Layer edits ------------------------------------------------------------

    pub fn toggle_switch(&mut self, layer: Id, switch: Switch) {
        if let Some(layer) = self.composition_mut().layer_mut(layer) {
            let slot = switch.slot(&mut layer.switches);
            *slot = !*slot;
        }
        // Locking a layer lets go of it and its keyframes, as in After Effects.
        if switch == Switch::Locked && self.composition().layer(layer).is_some_and(|l| l.switches.locked) {
            self.selected_layers.retain(|id| *id != layer);
            self.selected_keyframes.retain(|key| key.layer != layer);
        }
    }

    pub fn set_blend_mode(&mut self, layer: Id, mode: BlendMode) {
        if let Some(layer) = self.composition_mut().layer_mut(layer) {
            layer.blend_mode = mode;
        }
    }

    pub fn set_track_matte(&mut self, layer: Id, matte: TrackMatte) {
        if let Some(layer) = self.composition_mut().layer_mut(layer) {
            layer.track_matte = matte;
        }
    }

    /// Parents a layer, keeping it where it is on screen at the playhead.
    pub fn set_parent(&mut self, layer: Id, parent: Option<Id>) {
        let frame = self.current_frame;
        edit::set_parent(self.composition_mut(), layer, parent, frame);
    }

    /// Adds a project item as a layer and selects it.
    #[cfg(test)]
    pub fn add_layer_from_item(&mut self, item: Id, stack_index: usize, in_frame: i64) {
        if let Some(layer) = edit::add_layer_from_item(&mut self.project, self.active_comp, item, stack_index, in_frame)
        {
            self.selected_layers = vec![layer];
            self.selected_keyframes.clear();
        }
    }

    /// Adds a catalog effect to a layer (if it suits the layer) and selects the layer.
    pub fn add_effect(&mut self, layer: Id, name: &str) {
        // The layer is selected so its new effect shows in Effect Controls; a selection it is
        // already part of is kept, so one effect can go onto several clips.
        if edit::add_effect(&mut self.project, self.active_comp, layer, name).is_some() && !self.selected_layers.contains(&layer) {
            self.selected_layers = vec![layer];
        }
    }

    pub fn remove_effect(&mut self, layer: Id, effect: Id) {
        edit::remove_effect(self.composition_mut(), layer, effect);
        self.selected_keyframes.retain(|key| !matches!(key.path, PropertyPath::Effect { effect: e, .. } if e == effect));
    }

    pub fn toggle_effect(&mut self, layer: Id, effect: Id) {
        if let Some(effect) = self.composition_mut().layer_mut(layer).and_then(|l| l.effects.iter_mut().find(|e| e.id == effect)) {
            effect.enabled = !effect.enabled;
        }
    }

    /// Delete key in the timeline or viewer: selected keyframes go first; otherwise the selected layers.
    pub fn delete_selection(&mut self) -> Deleted {
        if !self.selected_keyframes.is_empty() {
            let count = self.delete_selected_keyframes();
            return Deleted::Keyframes(count);
        }
        if self.selected_layers.is_empty() {
            return Deleted::Nothing;
        }
        let ids = std::mem::take(&mut self.selected_layers);
        Deleted::Layers(edit::delete_layers(self.composition_mut(), &ids))
    }

    /// What deleting the selected Project panel item would remove, for the confirmation dialog.
    pub fn plan_item_deletion(&self) -> Option<ItemDeletion> {
        let item = self.selected_item?;
        Some(edit::plan_item_deletion(&self.project, &[item], self.active_comp))
    }

    /// Deletes project items after the user confirmed. Layers that used them are removed too.
    pub fn delete_items(&mut self, plan: &ItemDeletion) -> bool {
        if !edit::delete_items(&mut self.project, plan) {
            return false;
        }
        let comp = self.composition();
        let remaining: Vec<Id> = comp.layers.iter().map(|layer| layer.id).collect();
        self.selected_layers.retain(|id| remaining.contains(id));
        self.selected_keyframes.retain(|key| remaining.contains(&key.layer));
        self.selected_item = None;
        true
    }

    /// Adds a project folder and selects it.
    pub fn new_folder(&mut self) {
        let id = self.project.allocate_id();
        let count = self.project.items.iter().filter(|item| item.kind == studio_core::model::ProjectItemKind::Folder).count();
        self.project.items.push(studio_core::model::ProjectItem {
            id,
            name: format!("Folder {}", count + 1),
            kind: studio_core::model::ProjectItemKind::Folder,
            label: studio_core::model::LabelColor::None,
            parent: None,
            size: None,
            duration_seconds: None,
            frame_rate: None,
            has_audio: false,
            color: None,
        path: None,
        });
        self.selected_item = Some(id);
    }

    // Text -------------------------------------------------------------------

    /// Adds a text layer centered at `position` (composition pixels) and selects it.
    pub fn add_text_layer(&mut self, text: &str, position: Vec2) -> Option<Id> {
        let id = edit::add_text_layer(&mut self.project, self.active_comp, text, position, self.current_frame)?;
        self.selected_layers = vec![id];
        self.selected_keyframes.clear();
        Some(id)
    }

    pub fn set_text(&mut self, layer: Id, text: &str) {
        edit::edit_text(self.composition_mut(), layer, |current, _| *current = text.to_owned());
    }

    pub fn set_text_align(&mut self, layer: Id, align: TextAlign) {
        edit::edit_text(self.composition_mut(), layer, |_, style| style.align = align);
    }

    pub fn set_text_setting(&mut self, layer: Id, setting: TextSetting, value: f32) {
        edit::edit_text(self.composition_mut(), layer, |_, style| {
            setting.set(style, value);
            // Changing what a library text style sets unlinks it (stroke width is not part of one).
            if setting != TextSetting::StrokeWidth {
                style.template = None;
            }
        });
    }

    pub fn set_font(&mut self, layer: Id, family: &str, weight: u16) {
        edit::edit_text(self.composition_mut(), layer, |_, style| {
            style.font_family = family.to_owned();
            style.font_weight = weight;
            style.template = None;
        });
    }

    pub fn set_text_fill(&mut self, layer: Id, fill: Option<studio_core::model::Rgba>) {
        edit::edit_text(self.composition_mut(), layer, |_, style| {
            // A different fill from the one the linked text style sets unlinks it.
            if style.fill.map(|old| old.linked(None)) != fill.map(|new| new.linked(None)) {
                style.template = None;
            }
            style.fill = fill;
        });
    }

    pub fn set_text_stroke(&mut self, layer: Id, stroke: Option<(studio_core::model::Rgba, f32)>) {
        edit::edit_text(self.composition_mut(), layer, |_, style| style.stroke = stroke);
    }

    pub fn shape_of(&self, layer: Id) -> Option<&ShapeStyle> {
        match &self.composition().layer(layer)?.content {
            LayerContent::Shape { style } => Some(style),
            _ => None,
        }
    }

    /// Sets a shape setting. Sizes keep the shape centered on its anchor; with
    /// `shape_size_linked`, width and height change together.
    pub fn set_shape_setting(&mut self, layer: Id, setting: ShapeSetting, value: f32) {
        if self.shape_of(layer).is_none() {
            return;
        }
        let Some(size) = self.composition().layer(layer).map(|layer| layer.size) else { return };
        let ratio = size.y / size.x.max(1.0);
        let new_size = match setting {
            ShapeSetting::Width if self.shape_size_linked => Some(Vec2::new(value, value * ratio)),
            ShapeSetting::Height if self.shape_size_linked => Some(Vec2::new(value / ratio.max(f32::EPSILON), value)),
            ShapeSetting::Width => Some(Vec2::new(value, size.y)),
            ShapeSetting::Height => Some(Vec2::new(size.x, value)),
            ShapeSetting::OuterRadius => Some(Vec2::new(value * 2.0, value * 2.0)),
            _ => None,
        };
        if let Some(new_size) = new_size {
            edit::set_shape_size(self.composition_mut(), layer, new_size);
            return;
        }
        let outer = size.x.min(size.y) * 0.5;
        edit::edit_shape(self.composition_mut(), layer, |style| match setting {
            ShapeSetting::InnerRadius => style.inner_radius = value / outer.max(1.0),
            ShapeSetting::CornerRadius => style.corner_radius = value,
            ShapeSetting::Points => style.points = value.round().clamp(0.0, u32::MAX as f32) as u32,
            ShapeSetting::StrokeWidth => {
                if let Some(stroke) = style.stroke.as_mut() {
                    stroke.width = value;
                }
            }
            ShapeSetting::FillAngle => style.fill.iter_mut().for_each(|paint| set_angle(paint, value)),
            ShapeSetting::StrokeAngle => style.stroke.iter_mut().for_each(|stroke| set_angle(&mut stroke.paint, value)),
            ShapeSetting::Width | ShapeSetting::Height | ShapeSetting::OuterRadius => {}
        });
    }

    /// Sets a shape's fill or stroke paint; `None` turns it off. A stroke turned on starts 4 px wide.
    pub fn set_shape_paint(&mut self, layer: Id, slot: PaintSlot, paint: Option<Paint>) {
        edit::edit_shape(self.composition_mut(), layer, |style| match slot {
            PaintSlot::Fill => style.fill = paint,
            PaintSlot::Stroke => {
                let width = style.stroke.map_or(4.0, |stroke| stroke.width);
                style.stroke = paint.map(|paint| Stroke { paint, width });
            }
        });
    }

    /// The color a picker target has now, if it has one.
    pub fn color_of(&self, target: ColorTarget) -> Option<Rgba> {
        match target {
            ColorTarget::Shape { layer, slot, stop } => self.shape_paint(layer, slot)?.colors().get(stop).copied(),
            ColorTarget::TextFill(layer) => self.text_style_of(layer)?.fill,
            ColorTarget::TextStroke(layer) => self.text_style_of(layer)?.stroke.map(|(color, _)| color),
            ColorTarget::CaptionHighlight(layer) => self.caption_style(layer).map(|style| style.highlight),
            ColorTarget::CaptionBackground(layer) => self.caption_style(layer)?.background,
            ColorTarget::Library(id) => self.libraries.color(id).map(|(_, item)| item.color),
        }
    }

    /// Sets a picker target's color, turning the fill or stroke on if it was off.
    pub fn set_color(&mut self, target: ColorTarget, color: Rgba) {
        match target {
            ColorTarget::Shape { layer, slot, stop } => {
                let paint = self.shape_paint(layer, slot).map_or(Paint::Solid(color), |paint| paint.with_color(stop, color));
                self.set_shape_paint(layer, slot, Some(paint));
            }
            ColorTarget::TextFill(layer) => self.set_text_fill(layer, Some(color)),
            ColorTarget::TextStroke(layer) => {
                let width = self.text_style_of(layer).and_then(|style| style.stroke).map_or(2.0, |(_, width)| width);
                self.set_text_stroke(layer, Some((color, width)));
            }
            ColorTarget::Library(id) => self.set_library_color(id, color),
            ColorTarget::CaptionHighlight(layer) => self.edit_captions(layer, |style| style.highlight = color),
            ColorTarget::CaptionBackground(layer) => self.edit_captions(layer, |style| style.background = Some(color)),
        }
    }

    /// A shape's fill or stroke paint.
    pub fn shape_paint(&self, layer: Id, slot: PaintSlot) -> Option<Paint> {
        let style = self.shape_of(layer)?;
        match slot {
            PaintSlot::Fill => style.fill,
            PaintSlot::Stroke => style.stroke.map(|stroke| stroke.paint),
        }
    }

    /// Changes which shape a shape layer draws, keeping its box, colour and corners.
    pub fn set_shape_kind(&mut self, layer: Id, kind: ShapeKind) {
        edit::edit_shape(self.composition_mut(), layer, |style| style.kind = kind);
    }

    /// The text style of a text or caption layer (captions share one style for all their words).
    pub fn text_style_of(&self, layer: Id) -> Option<&TextStyle> {
        match &self.composition().layer(layer)?.content {
            LayerContent::Text { style, .. } => Some(style),
            LayerContent::Captions { style, .. } => Some(&style.text),
            _ => None,
        }
    }

    /// Whether a layer has sound to adjust: audio layers, and footage with a sound track.
    pub fn has_sound(&self, layer: Id) -> bool {
        self.composition().layer(layer).is_some_and(|layer| layer.has_audio && matches!(layer.content, LayerContent::Footage { .. } | LayerContent::Audio { .. }))
    }

    pub fn audio_setting(&self, layer: Id, setting: AudioSetting) -> Option<f32> {
        let audio = self.composition().layer(layer)?.audio;
        Some(match setting {
            AudioSetting::Volume => audio.volume_db,
            AudioSetting::FadeIn => audio.fade_in,
            AudioSetting::FadeOut => audio.fade_out,
        })
    }

    pub fn set_audio_setting(&mut self, layer: Id, setting: AudioSetting, value: f32) {
        edit::set_layer_audio(self.composition_mut(), layer, |audio| match setting {
            AudioSetting::Volume => audio.volume_db = value,
            AudioSetting::FadeIn => audio.fade_in = value,
            AudioSetting::FadeOut => audio.fade_out = value,
        });
    }

    /// Moves a video clip's sound onto its own audio clip, and selects that.
    pub fn separate_audio(&mut self, layer: Id) -> Option<Id> {
        let comp = self.active_comp;
        let audio = edit::separate_audio(&mut self.project, comp, layer)?;
        self.select_layer(audio, false);
        Some(audio)
    }

    pub fn text_of(&self, layer: Id) -> Option<(&str, &TextStyle)> {
        match &self.composition().layer(layer)?.content {
            LayerContent::Text { text, style } => Some((text.as_str(), style)),
            _ => None,
        }
    }

    /// Applies text box sizes measured by the renderer, so text bounds match what is drawn.
    pub fn apply_text_sizes(&mut self, sizes: &[(Id, Vec2)]) -> bool {
        let mut changed = false;
        for &(layer, size) in sizes {
            changed |= edit::set_text_box(self.composition_mut(), layer, size);
        }
        // Fitting a box to its text is automatic, not an edit to undo.
        let active = self.active_comp;
        self.history.absorb(|project| {
            if let Some(composition) = project.composition_mut(active) {
                for &(layer, size) in sizes {
                    edit::set_text_box(composition, layer, size);
                }
            }
        });
        changed
    }

    /// Adds image and video files to the project. Files that are neither readable images nor
    /// decodable video are skipped and returned.
    #[cfg(test)]
    pub fn import_files(&mut self, paths: Vec<std::path::PathBuf>) -> Vec<std::path::PathBuf> {
        self.import_paths(paths).1
    }

    /// Adds files to the project, as links to where they are on disk (nothing is copied).
    /// Folders bring in the media inside them. Returns the new items, in order, and the files
    /// that are not media Studio can read.
    pub fn import_paths(&mut self, paths: Vec<std::path::PathBuf>) -> (Vec<Id>, Vec<std::path::PathBuf>) {
        let (mut imported, mut skipped) = (Vec::new(), Vec::new());
        for path in expand_folders(paths) {
            let before = self.project.items.len();
            if let Ok((width, height)) = image::image_dimensions(&path) {
                self.selected_item = Some(edit::import_image(&mut self.project, path, Vec2::new(width as f32, height as f32)));
                imported.extend(self.selected_item);
                continue;
            }
            if let Ok(info) = studio_media::probe(&path) {
                let footage = edit::FootageInfo {
                    size: Vec2::new(info.width as f32, info.height as f32),
                    duration_seconds: info.duration_seconds,
                    frame_rate: info.frame_rate,
                    has_audio: info.has_audio,
                };
                self.selected_item = Some(edit::import_footage(&mut self.project, path, &footage));
            } else if let Ok(info) = studio_media::probe_audio(&path) {
                // No picture, but sound: music, voice-over and other audio files.
                self.selected_item = Some(edit::import_audio(&mut self.project, path, info.duration_seconds));
            } else {
                skipped.push(path);
            }
            if self.project.items.len() > before {
                imported.extend(self.selected_item);
            }
        }
        (imported, skipped)
    }

    // Alignment --------------------------------------------------------------

    pub fn align_selected(&mut self, alignment: Alignment) {
        let (layers, target, frame) = (self.selected_layers.clone(), self.align_target, self.current_frame);
        align_layers(self.composition_mut(), &layers, alignment, target, frame);
    }

    pub fn distribute_selected(&mut self, distribution: Distribution) {
        let (layers, frame) = (self.selected_layers.clone(), self.current_frame);
        distribute_layers(self.composition_mut(), &layers, distribution, frame);
    }

    // Compositions -----------------------------------------------------------

    pub fn composition_settings(&self) -> CompositionSettings {
        let comp = self.composition();
        CompositionSettings {
            name: comp.name.clone(),
            width: comp.width,
            height: comp.height,
            frame_rate: comp.frame_rate,
            duration_frames: comp.duration_frames,
            background: comp.background,
        }
    }

    /// Creates a composition from the dialog and opens it.
    pub fn create_composition(&mut self, settings: &CompositionSettings) -> Result<(), String> {
        let id = edit::new_composition(&mut self.project, settings)?;
        self.open_composition(id);
        Ok(())
    }

    /// Applies the Composition Settings dialog to the open composition.
    pub fn update_composition(&mut self, settings: &CompositionSettings) -> Result<(), String> {
        edit::apply_composition_settings(&mut self.project, self.active_comp, settings)?;
        let duration = settings.duration_frames as f64;
        self.work_area = self.work_area.clamped(duration, 1.0);
        self.view_range = FrameRange::new(0.0, duration);
        self.current_frame = self.current_frame.min(settings.duration_frames - 1);
        Ok(())
    }

    /// Opens a composition in the viewer and timeline.
    pub fn open_composition(&mut self, id: Id) {
        let Some(comp) = self.project.composition(id) else { return };
        let duration = comp.duration_frames as f64;
        self.active_comp = id;
        self.playback = None;
        self.current_frame = 0;
        self.work_area = FrameRange::new(0.0, duration);
        self.view_range = FrameRange::new(0.0, duration);
        self.clear_selection();
        self.selected_item = Some(id);
    }
}

/// The files to import for `paths`: files as they are, and for folders the files inside them
/// (and their subfolders), sorted by name, skipping hidden ones. At most 1,000 files, so
/// dropping a huge folder by mistake can't stall the editor.
fn expand_folders(paths: Vec<std::path::PathBuf>) -> Vec<std::path::PathBuf> {
    const LIMIT: usize = 1000;
    let mut files = Vec::new();
    let mut pending: std::collections::VecDeque<std::path::PathBuf> = paths.into();
    while let Some(path) = pending.pop_front() {
        if files.len() >= LIMIT {
            break;
        }
        if !path.is_dir() {
            files.push(path);
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&path) else { continue };
        let mut inside: Vec<std::path::PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| !path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')))
            .collect();
        inside.sort();
        pending.extend(inside);
    }
    files
}

/// Undo and redo.
impl EditorState {
    /// Whether a drag that edits the project (keyframes, a layer in time or in the viewer) is
    /// in progress. Its changes become one undo step when it ends.
    pub fn is_dragging(&self) -> bool {
        self.keyframe_drag.is_some() || self.layer_time_drag.is_some() || self.viewer_drag.is_some() || self.clip_drag.is_some()
    }

    /// Records the project's latest changes as an undo step. Call after edits, with `ui_drag`
    /// set while the UI has a drag of its own going (such as scrubbing a value), so the whole
    /// drag becomes one step.
    pub fn record_history(&mut self, ui_drag: bool) {
        if !ui_drag && !self.is_dragging() {
            // The magnetic main track closes gaps as part of the edit that opened them.
            self.apply_magnetism();
            self.history.record(&self.project);
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo(&self.project)
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Undoes the last change. Returns whether anything changed.
    pub fn undo(&mut self) -> bool {
        if self.is_dragging() || !self.history.undo(&mut self.project) {
            return false;
        }
        self.after_project_replaced();
        true
    }

    /// Redoes the last undone change. Returns whether anything changed.
    pub fn redo(&mut self) -> bool {
        if self.is_dragging() || !self.history.redo(&mut self.project) {
            return false;
        }
        self.after_project_replaced();
        true
    }

    /// After undo, redo or opening a file, the project may no longer have what the editor
    /// pointed at: keep only what still exists, and stay inside the composition.
    fn after_project_replaced(&mut self) {
        if self.project.composition(self.active_comp).is_none() {
            let first = self.project.compositions[0].id;
            self.open_composition(first);
        }
        let duration = self.duration();
        self.current_frame = self.current_frame.clamp(0, duration - 1);
        self.work_area = self.work_area.clamped(duration as f64, 1.0);
        self.view_range = self.view_range.clamped(duration as f64, MIN_VIEW_FRAMES);
        let composition = self.project.composition(self.active_comp).expect("checked above");
        self.selected_layers.retain(|id| composition.layer(*id).is_some());
        self.selected_keyframes.retain(|key| {
            composition.layer(key.layer).and_then(|layer| layer.property(key.path, key.frame)).is_some_and(|p| p.keyframes.contains(&key.frame))
        });
        if self.selected_item.is_some_and(|id| self.project.item(id).is_none()) {
            self.selected_item = None;
        }
    }
}

/// Project files.
impl EditorState {
    /// The project file being edited, once saved or opened.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// The name shown in the title bar: the file's name, or "Untitled" before the first save.
    pub fn document_name(&self) -> String {
        self.file.as_ref().and_then(|path| path.file_stem()).map_or_else(|| "Untitled".to_owned(), |name| name.to_string_lossy().into_owned())
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.history.has_unsaved_changes(&self.project)
    }

    /// The project as file text, including where the editor is.
    #[cfg(test)]
    pub fn project_text(&self) -> Result<String, String> {
        self.project_text_with(&self.editor_settings())
    }

    /// The project as file text, with `settings` for how the editor was set up.
    pub fn project_text_with(&self, settings: &EditorSettings) -> Result<String, String> {
        let view = EditorView { active_composition: self.active_comp, current_frame: self.current_frame, work_area: self.work_area };
        project_file::to_text_with(&self.project, view, settings)
    }

    /// How the editor is set up, as far as this state knows (the window adds the timeline view
    /// and its open clips).
    pub fn editor_settings(&self) -> EditorSettings {
        let mut expanded_rows: Vec<String> = self.expanded_rows.iter().cloned().collect();
        expanded_rows.sort();
        EditorSettings {
            view_range: Some(self.view_range),
            snapping: self.snapping,
            magnetic_main_track: self.magnetic_main_track,
            loop_playback: self.loop_playback,
            expanded_rows,
            ..EditorSettings::default()
        }
    }

    /// Notes that the project was written to `path`.
    /// `ui_drag` is whether the UI has a drag of its own going (see `record_history`).
    pub fn mark_saved(&mut self, path: PathBuf, ui_drag: bool) {
        self.file = Some(path);
        // Mid-drag, recording now would split the drag into two undo steps.
        if ui_drag || self.is_dragging() {
            self.history.mark_saved_mid_change();
            return;
        }
        self.history.mark_saved(&self.project);
    }

    /// After restoring a recovery copy: the work belongs to `file` (or is untitled), and is
    /// unsaved there.
    pub fn mark_restored_unsaved(&mut self, file: Option<PathBuf>) {
        self.file = file;
        self.history.mark_saved_mid_change();
    }

    /// Replaces the project with one read from `path`, set up as it was saved. Returns the
    /// media files that could not be found (either where they were or next to the project
    /// file), and the saved settings, for the window to restore its part (the timeline view).
    pub fn open_project(&mut self, text: &str, path: PathBuf) -> Result<(Vec<PathBuf>, EditorSettings), String> {
        let (mut project, view, settings) = project_file::from_text_with(text)?;
        let folder = path.parent().unwrap_or(Path::new("."));
        let missing = project_file::relink_missing_media(&mut project, folder, Path::exists);
        self.end_keyframe_drag();
        self.end_viewer_drag();
        self.end_layer_time_drag();
        self.project = project;
        // Library colors and text styles may have changed since it was saved.
        studio_core::library::sync_project(&mut self.project, &self.libraries);
        self.open_composition(view.active_composition);
        self.current_frame = view.current_frame;
        self.work_area = view.work_area;
        self.expanded_rows = settings.expanded_rows.iter().cloned().collect();
        if let Some(range) = settings.view_range {
            self.set_view_range(range);
        }
        (self.snapping, self.magnetic_main_track, self.loop_playback) = (settings.snapping, settings.magnetic_main_track, settings.loop_playback);
        self.history = history::History::new(&self.project);
        self.file = Some(path);
        Ok((missing, settings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn playback_follows_the_clock_and_stops_on_manual_moves() {
        let mut state = EditorState::new();
        let start = Instant::now();
        state.toggle_playback(start);
        assert!(state.tick_playback(start + Duration::from_millis(1000)));
        assert_eq!(state.current_frame, 24);
        state.set_current_frame(5);
        assert!(!state.is_playing());
    }

    #[test]
    fn non_looping_playback_ends_on_the_last_frame() {
        let mut state = EditorState::new();
        state.loop_playback = false;
        let start = Instant::now();
        state.toggle_playback(start);
        assert!(!state.tick_playback(start + Duration::from_secs(60)));
        assert_eq!(state.current_frame, 239);
    }

    #[test]
    fn selection_adds_and_removes_with_modifier() {
        let mut state = EditorState::new();
        let (a, b) = (state.composition().layers[0].id, state.composition().layers[1].id);
        state.select_layer(a, false);
        state.select_layer(b, true);
        assert_eq!(state.selected_layers, vec![a, b]);
        state.select_layer(a, true);
        assert_eq!(state.selected_layers, vec![b]);
    }

    #[test]
    fn editing_a_value_auto_keyframes_only_animated_properties() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.set_current_frame(100);
        let opacity = PropertyPath::Transform(TransformProperty::Opacity);
        state.set_property_component(title, opacity, 0, 40.0);
        let layer = state.composition().layer(title).unwrap();
        assert!(layer.property(opacity, 100).unwrap().keyframes.contains(&100));

        let rotation = PropertyPath::Transform(TransformProperty::Rotation);
        state.set_property_component(title, rotation, 0, 45.0);
        let layer = state.composition().layer(title).unwrap();
        assert!(layer.property(rotation, 0).unwrap().keyframes.is_empty());
        assert_eq!(layer.property(rotation, 0).unwrap().value, PropertyValue::Scalar(45.0));
    }

    #[test]
    fn editing_on_a_selected_keyframe_changes_every_selected_keyframe() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let opacity = PropertyPath::Transform(TransformProperty::Opacity);
        state.select_keyframe(KeyframeRef { layer: title, path: opacity, frame: 36 }, false);
        state.select_keyframe(KeyframeRef { layer: title, path: opacity, frame: 156 }, true);
        state.set_current_frame(36);
        state.set_property_component(title, opacity, 0, 70.0);
        let layer = state.composition().layer(title).unwrap();
        assert_eq!(layer.property(opacity, 36).unwrap().value, PropertyValue::Scalar(70.0));
        assert_eq!(layer.property(opacity, 156).unwrap().value, PropertyValue::Scalar(70.0));
        assert_eq!(layer.property(opacity, 24).unwrap().value, PropertyValue::Scalar(0.0), "unselected keyframes keep their values");

        // Off a selected keyframe, edits behave normally: one new keyframe at the playhead.
        state.set_current_frame(100);
        state.set_property_component(title, opacity, 0, 50.0);
        let layer = state.composition().layer(title).unwrap();
        assert_eq!(layer.property(opacity, 156).unwrap().value, PropertyValue::Scalar(70.0));
        assert_eq!(layer.property(opacity, 0).unwrap().keyframes.len(), 5);
    }

    #[test]
    fn linked_scale_keeps_proportions() {
        let mut state = EditorState::new();
        let solid = state.composition().layers[5].id;
        let scale = PropertyPath::Transform(TransformProperty::Scale);
        state.set_property_component(solid, scale, 0, 50.0);
        let value = state.composition().layer(solid).unwrap().property(scale, 0).unwrap().value;
        assert_eq!(value, PropertyValue::Pair(Vec2::new(50.0, 50.0)));
        state.scale_linked = false;
        state.set_property_component(solid, scale, 1, 80.0);
        let value = state.composition().layer(solid).unwrap().property(scale, 0).unwrap().value;
        assert_eq!(value, PropertyValue::Pair(Vec2::new(50.0, 80.0)));
    }

    #[test]
    fn locked_layers_ignore_edits() {
        let mut state = EditorState::new();
        let solid = state.composition().layers[5].id;
        state.toggle_switch(solid, Switch::Locked);
        let rotation = PropertyPath::Transform(TransformProperty::Rotation);
        state.set_property_component(solid, rotation, 0, 30.0);
        let value = state.composition().layer(solid).unwrap().property(rotation, 0).unwrap().value;
        assert_eq!(value, PropertyValue::Scalar(0.0));
    }

    #[test]
    fn delete_removes_keyframes_before_layers() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let opacity = PropertyPath::Transform(TransformProperty::Opacity);
        state.select_layer(title, false);
        state.select_keyframe(KeyframeRef { layer: title, path: opacity, frame: 36 }, false);
        assert_eq!(state.delete_selection(), Deleted::Keyframes(1));
        assert!(state.composition().layer(title).is_some(), "the layer survives");
        state.select_layer(title, false);
        assert_eq!(state.delete_selection(), Deleted::Layers(1));
        assert!(state.composition().layer(title).is_none());
        assert_eq!(state.delete_selection(), Deleted::Nothing);
    }

    #[test]
    fn deleting_project_items_clears_stale_selection() {
        let mut state = EditorState::new();
        let a_cam_layer = state.composition().layers[4].id;
        state.select_layer(a_cam_layer, false);
        state.selected_item = Some(ids::A_CAM);
        let plan = state.plan_item_deletion().unwrap();
        assert_eq!(plan.layers, 1);
        assert!(state.delete_items(&plan));
        assert!(state.selected_layers.is_empty());
    }

    #[test]
    fn shape_settings_stay_in_range() {
        let mut state = EditorState::new();
        let layer = state.begin_shape_draw(ShapeKind::Polygon, studio_core::animation::Vec2::new(500.0, 500.0)).unwrap();
        state.end_viewer_drag();
        state.set_shape_setting(layer, ShapeSetting::CornerRadius, 10_000.0);
        assert_eq!(state.shape_of(layer).unwrap().corner_radius, 150.0, "at most half the 300 px box");
        state.set_shape_setting(layer, ShapeSetting::Points, 1.0);
        assert_eq!(state.shape_of(layer).unwrap().points, 3);
        state.set_shape_kind(layer, ShapeKind::Star);
        assert_eq!(state.shape_of(layer).unwrap().kind, ShapeKind::Star);
    }

    #[test]
    fn text_settings_resize_text_layers() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let width = state.composition().layer(title).unwrap().size.x;
        state.set_text_setting(title, TextSetting::FontSize, 240.0);
        assert!(state.composition().layer(title).unwrap().size.x > width);
        state.set_text_align(title, TextAlign::Left);
        assert_eq!(state.text_of(title).unwrap().1.align, TextAlign::Left);
    }

    #[test]
    fn importing_reads_real_video_details_and_skips_other_files() {
        let mut state = EditorState::new();
        let clip = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/colors.mp4");
        let readme = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let skipped = state.import_files(vec![clip, readme.clone()]);
        assert_eq!(skipped, vec![readme]);
        let item = state.project.item(state.selected_item.unwrap()).unwrap();
        assert_eq!(item.kind, studio_core::model::ProjectItemKind::Footage);
        assert_eq!(item.size, Some(Vec2::new(64.0, 36.0)));
        assert!(item.has_audio);
        // Dropping it in the timeline makes a 2-second layer (48 frames at 24 fps).
        let id = item.id;
        state.add_layer_from_item(id, 0, 0);
        assert_eq!(state.composition().layers[0].out_frame, 48);
    }

    #[test]
    fn imported_footage_plays_through_the_decoder_and_renderer() {
        use studio_core::media::VideoSource as _;
        let mut state = EditorState::new();
        let clip = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/colors.mp4");
        state.import_files(vec![clip.clone()]);
        let item = state.selected_item.unwrap();
        // Put the clip on top, full frame, starting at frame 0.
        state.add_layer_from_item(item, 0, 0);
        let layer = state.composition().layers[0].id;
        let scale = PropertyPath::Transform(TransformProperty::Scale);
        state.set_property_component(layer, scale, 0, 3000.0); // 64 px wide → 1920

        let media = studio_media::FrameServer::new(1920, None);
        let mut renderer = studio_render::Renderer::new().unwrap();
        let center_color = |state: &EditorState, renderer: &mut studio_render::Renderer, frame: i64| {
            // Wait until the decoder has the exact frame, as the viewer's redraw would.
            let started = std::time::Instant::now();
            while media.frame(&clip, frame).is_none_or(|f| f.index != frame) {
                assert!(started.elapsed() < std::time::Duration::from_secs(5), "frame {frame} never decoded");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let rendered = renderer.render(&state.project, state.active_comp, frame, 192, 108, &media).unwrap();
            let i = ((54 * 192 + 96) * 4) as usize;
            let (r, g, b) = (rendered.bgra[i + 2], rendered.bgra[i + 1], rendered.bgra[i]);
            match (r > 128, g > 128, b > 128) {
                (true, false, false) => "red",
                (false, true, false) => "green",
                (false, false, true) => "blue",
                (true, true, true) => "white",
                _ => "other",
            }
        };
        assert_eq!(center_color(&state, &mut renderer, 3), "red");
        assert_eq!(center_color(&state, &mut renderer, 15), "green");
        assert_eq!(center_color(&state, &mut renderer, 30), "blue");
        assert_eq!(center_color(&state, &mut renderer, 44), "white");
        assert_eq!(center_color(&state, &mut renderer, 2), "red", "scrubbing backwards works");
    }

    #[test]
    fn audio_files_import_as_sound_only_items() {
        let mut state = EditorState::new();
        let tone = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/tone.wav");
        assert!(state.import_files(vec![tone.clone()]).is_empty(), "nothing skipped");
        let item = state.project.item(state.selected_item.unwrap()).unwrap();
        assert_eq!((item.kind, item.has_audio, item.path.as_ref()), (studio_core::model::ProjectItemKind::Audio, true, Some(&tone)));
        assert!((item.duration_seconds.unwrap() - 2.0).abs() < 0.01);
    }

    #[test]
    fn playback_can_follow_the_heard_sound_and_restart() {
        let mut state = EditorState::new();
        let start = Instant::now();
        state.toggle_playback(start);
        assert_eq!(state.playback_started(), Some((start, 0)));
        // Half a second heard at 24 fps is frame 12, whatever the wall clock says.
        assert!(state.advance_playback(Duration::from_millis(500)));
        assert_eq!(state.current_frame, 12);

        let later = start + Duration::from_secs(5);
        state.restart_playback(later);
        assert_eq!(state.playback_started(), Some((later, 12)), "starts over from the playhead");
        state.set_current_frame(3);
        assert_eq!(state.playback_started(), None);
    }

    #[test]
    fn compositions_open_after_creation() {
        let mut state = EditorState::new();
        let settings = CompositionSettings { name: "Square".into(), width: 1080, height: 1080, ..state.composition_settings() };
        state.create_composition(&settings).unwrap();
        assert_eq!(state.composition().name, "Square");
        assert!(state.composition().layers.is_empty());
        let longer = CompositionSettings { duration_frames: 480, ..state.composition_settings() };
        state.update_composition(&longer).unwrap();
        assert_eq!(state.view_range.end, 480.0);
    }

    #[test]
    fn locked_layers_cannot_be_selected_moved_or_scaled() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let position = PropertyPath::Transform(TransformProperty::Position);
        state.select_layer(title, false);
        state.toggle_switch(title, Switch::Locked);
        assert!(state.selected_layers.is_empty(), "locking lets go of the layer");
        state.select_layer(title, false);
        assert!(state.selected_layers.is_empty(), "a locked layer can't be selected");

        // Even if a stale selection still held it, viewer drags leave it alone.
        state.selected_layers = vec![title];
        let before = state.composition().layer(title).unwrap().property(position, 60).unwrap().value;
        state.begin_move();
        state.drag_in_viewer(Vec2::new(100.0, 0.0), false);
        state.end_viewer_drag();
        state.begin_scale(title, ScaleHandle::BottomRight);
        state.drag_in_viewer(Vec2::new(100.0, 100.0), false);
        state.end_viewer_drag();
        let layer = state.composition().layer(title).unwrap();
        assert_eq!(layer.property(position, 60).unwrap().value, before);
    }

    #[test]
    fn undo_reverts_edits_and_forgets_selection_of_what_is_gone() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let rotation = PropertyPath::Transform(TransformProperty::Rotation);
        state.set_property_component(title, rotation, 0, 45.0);
        state.record_history(false);
        let added = state.add_text_layer("Hello", Vec2::new(100.0, 100.0)).unwrap();
        state.record_history(false);
        state.select_layer(added, false);

        assert!(state.undo());
        assert!(state.composition().layer(added).is_none());
        assert!(state.selected_layers.is_empty(), "the undone layer is no longer selected");
        assert!(state.undo());
        assert_eq!(state.composition().layer(title).unwrap().property(rotation, 0).unwrap().value, PropertyValue::Scalar(0.0));
        assert!(!state.undo(), "back at the start");
        assert!(state.redo() && state.redo());
        assert!(state.composition().layer(added).is_some());
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.select_layer(title, false);
        let before = state.project.clone();
        state.begin_move();
        for step in 1..=5 {
            state.drag_in_viewer(Vec2::new(step as f32 * 10.0, 0.0), false);
            state.record_history(false); // Redraws during the drag record nothing.
        }
        state.end_viewer_drag();
        state.record_history(false);
        assert!(state.undo());
        assert_eq!(state.project, before);
        assert!(!state.can_undo());
    }

    #[test]
    fn saving_in_the_middle_of_a_drag_keeps_it_one_undo_step() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.select_layer(title, false);
        let before = state.project.clone();
        state.begin_move();
        state.drag_in_viewer(Vec2::new(10.0, 0.0), false);
        state.mark_saved(PathBuf::from("/tmp/mid-drag.studio"), false);
        assert!(state.has_unsaved_changes(), "what was saved is unfinished");
        state.drag_in_viewer(Vec2::new(40.0, 0.0), false);
        state.end_viewer_drag();
        state.record_history(false);
        assert!(state.undo());
        assert_eq!(state.project, before, "one undo reverts the whole drag");
        assert!(!state.can_undo());
    }

    #[test]
    fn saved_projects_open_again_with_their_view() {
        let folder = std::env::temp_dir().join(format!("studio-save-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("Promo.studio");

        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.set_property_component(title, PropertyPath::Transform(TransformProperty::Rotation), 0, 12.0);
        state.set_current_frame(77);
        assert!(state.has_unsaved_changes());
        std::fs::write(&path, state.project_text().unwrap()).unwrap();
        state.mark_saved(path.clone(), false);
        assert!(!state.has_unsaved_changes());
        assert_eq!(state.document_name(), "Promo");

        let mut reopened = EditorState::new();
        let (missing, _) = reopened.open_project(&std::fs::read_to_string(&path).unwrap(), path.clone()).unwrap();
        assert!(missing.is_empty(), "the sample's media has no files to miss");
        assert_eq!(reopened.project, state.project);
        assert_eq!((reopened.current_frame, reopened.file()), (77, Some(path.as_path())));
        assert!(!reopened.has_unsaved_changes() && !reopened.can_undo(), "opening is not an edit");
        std::fs::remove_dir_all(&folder).unwrap();
    }

}
