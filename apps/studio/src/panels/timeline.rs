//! The Timeline panel, laid out like After Effects: a layer outline on the left
//! (switches, names, modes) and a time graph on the right (navigator, ruler,
//! layer bars, keyframes, playhead). Both halves render the same row list and
//! share one vertical scroll, so rows always line up.
//!
//! Positions in the time graph are fractions of the visible frame range, so the
//! graph needs no pixel bookkeeping. Pointer math (scrubbing, dropping) uses the
//! time column's bounds from the previous frame.

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, AppContext as _, Context, DismissEvent, Div, Entity, Focusable as _, Pixels, Point, Subscription,
    anchored, deferred, DragMoveEvent, ElementId, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, TestSupportExt as _, ScrollEvent, StatefulInteractiveElement, Styled,
    Window, div, prelude::FluentBuilder as _, px, relative, rgba,
};
use studio_core::model::{BlendMode, Id, Layer, TrackMatte};
use studio_core::time::{FrameRange, format_timecode, ruler_tick_step};

use studio_core::animation::Easing;
use studio_core::property::PropertyPath;

use super::chrome::{EmptyDragView, MeasuredBounds, Rect, icon_button, label_swatch, panel};
use super::fields::FieldTarget;
use crate::state::{KeyframeRef, LayerTimeEdit, Switch};
use crate::theme::{color, hsla, size};
use crate::timeline_rows::{RowKind, TimelineRow, build_rows, drop_target, keyframes_in_box};
use crate::workspace::{EffectDrag, PanelId, ProjectItemDrag, Workspace};

/// Heights of the two fixed bands above the rows: controls/navigator, then column titles/ruler.
const CONTROLS_BAND: f32 = 40.0;
const RULER_BAND: f32 = 28.0;
const SPLITTER: f32 = 4.0;
const MIN_OUTLINE: f32 = 260.0;
const MIN_GRAPH: f32 = 160.0;
/// How close, in screen pixels, a drag has to come to something before it snaps there.
const SNAP_PIXELS: f64 = 8.0;
/// Width of the grips at each end of a layer bar that trim it, in pixels.
const TRIM_HANDLE_WIDTH: f32 = 6.0;
/// How strongly one pixel of Alt + wheel scrolling zooms the time graph.
const WHEEL_ZOOM_SENSITIVITY: f64 = 0.004;

/// Fixed outline column widths. Source Name takes the remaining space.
mod columns {
    pub const AV: f32 = 84.0;
    pub const LABEL: f32 = 22.0;
    pub const INDEX: f32 = 28.0;
    pub const NAME_MIN: f32 = 160.0;
    pub const SWITCHES: f32 = 7.0 * 20.0;
    pub const MODE: f32 = 100.0;
    pub const MATTE: f32 = 100.0;
    pub const PARENT: f32 = 128.0;
    pub const FULL: f32 = AV + LABEL + INDEX + NAME_MIN + SWITCHES + MODE + MATTE + PARENT;
    /// Where nested rows start, so they line up under the layer name.
    pub const NAME_START: f32 = AV + LABEL + INDEX;
}

/// What a drop will do, shown while something is dragged over the rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropPreview {
    pub is_effect: bool,
    /// Visible row index the indicator is drawn at.
    pub row: usize,
    /// Start frame for a new layer dropped over the time graph; `None` means the playhead.
    pub frame: Option<i64>,
    pub layer: Option<Id>,
    pub insert_index: usize,
}

/// Values captured when a drag starts, so moves can be applied relative to them.
#[derive(Clone, Debug)]
struct DragAnchor {
    pointer_x: f32,
    pointer_y: f32,
    /// Keyframes selected when the drag began; a Shift box selection adds to them.
    base_selection: Vec<KeyframeRef>,
    view: FrameRange,
    work: FrameRange,
    outline_width: f32,
    /// Outline plus graph width, so the splitter can keep room for the graph.
    split_width: f32,
}

/// Timeline UI state kept between frames.
pub struct TimelineUi {
    pub outline_width: f32,
    /// Bounds of the rows content (it scrolls, so its top moves with the scroll position).
    rows: MeasuredBounds,
    /// The box selection being drawn: where it started and the pointer, in window pixels.
    marquee: Option<((f32, f32), (f32, f32))>,
    open_menu: Option<OpenMenu>,
    /// Bounds of the time column (navigator, ruler and graph share its x range).
    pub(crate) graph: MeasuredBounds,
    drop_preview: Option<DropPreview>,
    drag_anchor: Option<DragAnchor>,
}

impl Default for TimelineUi {
    fn default() -> Self {
        Self {
            outline_width: 640.0,
            graph: MeasuredBounds::default(),
            rows: MeasuredBounds::default(),
            drop_preview: None,
            drag_anchor: None,
            marquee: None,
            open_menu: None,
        }
    }
}

/// Drag kinds that move something inside the timeline rather than carrying data.
#[derive(Clone, Copy, Debug)]
enum TimelineDrag {
    Scrub,
    ViewStart,
    ViewEnd,
    ViewBody,
    WorkStart,
    WorkEnd,
    Splitter,
    /// Moving the selected keyframes in time.
    Keyframes,
    /// Drawing a box on empty graph space to select the keyframes inside it.
    Marquee,
    /// Moving a layer's bar (and its keyframes) in time.
    LayerBar,
}

/// Fraction of the visible range at which `frame` sits (0 = left edge, 1 = right edge).
fn fraction(frame: f64, view: FrameRange) -> f32 {
    ((frame - view.start) / view.len().max(1.0)) as f32
}

/// The frame under window x position `x`, given the time column's bounds.
fn frame_at(x: f32, graph: Rect, view: FrameRange) -> f64 {
    view.start + f64::from((x - graph.left) / graph.width.max(1.0)) * view.len()
}

impl Workspace {
    /// Layers the timeline shows, in stacking order, honoring the Hide Shy Layers switch.
    fn timeline_layers(&self) -> Vec<&Layer> {
        let hide_shy = self.ui.hide_shy_layers;
        self.state.composition().layers.iter().filter(|layer| !hide_shy || !layer.switches.shy).collect()
    }

    fn timeline_rows(&self) -> Vec<TimelineRow> {
        build_rows(&self.timeline_layers(), &self.state.expanded_rows)
    }

    fn anchor_drag(&mut self, event: &MouseDownEvent) {
        self.ui.timeline.drag_anchor = Some(DragAnchor {
            pointer_x: f32::from(event.position.x),
            pointer_y: f32::from(event.position.y),
            base_selection: if event.modifiers.shift { self.state.selected_keyframes.clone() } else { Vec::new() },
            view: self.state.view_range,
            work: self.state.work_area,
            outline_width: self.ui.timeline.outline_width,
            split_width: self.ui.timeline.outline_width + self.ui.timeline.graph.get().width,
        });
    }

    /// Applies a timeline drag at window x position `x`.
    fn apply_timeline_drag(&mut self, drag: TimelineDrag, x: f32, y: f32) {
        let graph = self.ui.timeline.graph.get();
        let Some(anchor) = self.ui.timeline.drag_anchor.clone() else { return };
        let full = FrameRange::new(0.0, self.state.composition().duration_frames as f64);
        // Frames per pixel in the navigator (whole composition) and in the ruler (visible range).
        let navigator_delta = f64::from(x - anchor.pointer_x) / f64::from(graph.width.max(1.0)) * full.len();
        let ruler_delta = f64::from(x - anchor.pointer_x) / f64::from(graph.width.max(1.0)) * anchor.view.len();
        // Snapping reaches the same distance on screen at any zoom.
        let frames_per_pixel = self.state.view_range.len() / f64::from(graph.width.max(1.0));
        self.state.snap_threshold = (SNAP_PIXELS * frames_per_pixel).round() as i64;
        match drag {
            TimelineDrag::Scrub => {
                let frame = frame_at(x, graph, self.state.view_range).round() as i64;
                self.state.scrub_to(frame);
            }
            TimelineDrag::ViewStart => {
                self.state.set_view_range(FrameRange::new(anchor.view.start + navigator_delta, anchor.view.end));
            }
            TimelineDrag::ViewEnd => {
                self.state.set_view_range(FrameRange::new(anchor.view.start, anchor.view.end + navigator_delta));
            }
            TimelineDrag::ViewBody => self.state.set_view_range(FrameRange::new(
                anchor.view.start + navigator_delta,
                anchor.view.end + navigator_delta,
            )),
            TimelineDrag::WorkStart => {
                self.state.set_work_area(FrameRange::new(anchor.work.start + ruler_delta, anchor.work.end));
            }
            TimelineDrag::WorkEnd => {
                self.state.set_work_area(FrameRange::new(anchor.work.start, anchor.work.end + ruler_delta));
            }
            TimelineDrag::Keyframes => self.state.drag_keyframes(ruler_delta.round() as i64),
            TimelineDrag::Marquee => {
                // Box selection: every keyframe between the press point and the pointer.
                let rows_top = self.ui.timeline.rows.get().top;
                let row_at = |y: f32| ((y - rows_top) / size::ROW_HEIGHT).floor().max(0.0) as usize;
                let view = self.state.view_range;
                let rows = self.timeline_rows();
                let picked = keyframes_in_box(
                    &rows,
                    row_at(anchor.pointer_y),
                    row_at(y),
                    frame_at(anchor.pointer_x, graph, view),
                    frame_at(x, graph, view),
                );
                let mut selection = anchor.base_selection.clone();
                selection.extend(picked.into_iter().filter(|key| !anchor.base_selection.contains(key)));
                self.state.set_keyframe_selection(selection);
                self.ui.timeline.marquee = Some(((anchor.pointer_x, anchor.pointer_y), (x, y)));
            }
            TimelineDrag::LayerBar => self.state.drag_layer_time(ruler_delta.round() as i64),
            TimelineDrag::Splitter => {
                let max = (anchor.split_width - MIN_GRAPH).max(MIN_OUTLINE);
                self.ui.timeline.outline_width = (anchor.outline_width + x - anchor.pointer_x).clamp(MIN_OUTLINE, max);
            }
        }
    }

    /// Updates the drop indicator for a drag over the rows. `content` is the rows' bounds.
    fn preview_drop(&mut self, is_effect: bool, x: f32, y: f32, content: Rect) {
        let inside = x >= content.left && x <= content.left + content.width && y >= content.top;
        let preview = inside.then(|| {
            let rows = self.timeline_rows();
            let row_index = ((y - content.top) / size::ROW_HEIGHT).floor().max(0.0) as usize;
            let (layer, insert_index, row) = drop_target(&rows, &self.state.composition().layers, row_index);
            let graph = self.ui.timeline.graph.get();
            let frame = (!is_effect && x >= graph.left)
                .then(|| frame_at(x, graph, self.state.view_range).round().max(0.0) as i64);
            DropPreview { is_effect, row, frame, layer, insert_index }
        });
        // Effects need a layer to land on.
        self.ui.timeline.drop_preview = preview.filter(|preview| !preview.is_effect || preview.layer.is_some());
    }
}

/// A clickable icon in a switch column. Off switches leave an empty slot, as in After Effects.
fn switch_toggle(
    id: (&'static str, usize),
    icon: IconName,
    on: bool,
    layer: Id,
    switch: Switch,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    div()
        .id(id)
        .size(px(20.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_switch(layer, switch);
            cx.notify();
        }))
        .child(Icon::new(icon).size(px(12.0)).text_color(hsla(if on { color::TEXT } else { color::HAIRLINE })))
}

/// One choice in a column dropdown: its label, whether it is the current value, and what choosing it does.
type MenuOption = (String, bool, Box<dyn Fn(&mut Workspace)>);

/// A Mode, Track Matte or Parent cell: plain text with a caret. Clicking it opens the one shared
/// column menu, which keeps the timeline cheap to render (a dropdown component per cell is not).
fn column_menu(
    id: ElementId,
    label: String,
    options: Vec<MenuOption>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<> {
    let options = std::rc::Rc::new(options);
    div()
        .id(id)
        .w_full()
        .h(px(18.0))
        .px_1p5()
        .flex()
        .items_center()
        .justify_between()
        .rounded(px(3.0))
        .text_size(px(size::SMALL_TEXT))
        .cursor_pointer()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
            this.open_column_menu(event.position(), options.clone(), window, cx);
        }))
        .child(div().truncate().child(label))
        .child(Icon::new(IconName::ChevronDown).size(px(10.0)).text_color(hsla(color::MUTED)))
        .test_support()
}

/// The column menu that is open, with the subscription that closes it when it is dismissed.
pub struct OpenMenu {
    menu: Entity<PopupMenu>,
    position: Point<Pixels>,
    _dismiss: Subscription,
}

impl Workspace {
    /// Opens the shared column menu at `position` with `options`.
    fn open_column_menu(&mut self, position: Point<Pixels>, options: std::rc::Rc<Vec<MenuOption>>, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.weak_entity();
        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            options.iter().enumerate().fold(menu, |menu, (index, (label, checked, _))| {
                let (workspace, options) = (workspace.clone(), options.clone());
                menu.item(PopupMenuItem::new(label.clone()).checked(*checked).on_click(move |_, _, cx| {
                    workspace
                        .update(cx, |this, cx| {
                            (options[index].2)(this);
                            cx.notify();
                        })
                        .ok();
                }))
            })
        });
        let dismiss = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.ui.timeline.open_menu = None;
            cx.notify();
        });
        window.focus(&menu.focus_handle(cx), cx);
        self.ui.timeline.open_menu = Some(OpenMenu { menu, position, _dismiss: dismiss });
        cx.notify();
    }
}

/// A small clickable icon, lighter than a `Button`, for dense timeline rows.
fn row_icon<F: Fn(&mut Workspace, &mut Context<Workspace>) + 'static>(
    id: ElementId,
    icon: IconName,
    active: bool,
    on_click: F,
    cx: &mut Context<Workspace>,
) -> impl IntoElement + use<F> {
    div()
        .id(id)
        .size(px(20.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, _, cx| {
            on_click(this, cx);
            cx.notify();
        }))
        .child(Icon::new(icon).size(px(11.0)).text_color(hsla(if active { color::ACCENT } else { color::MUTED })))
        .test_support()
}

fn twirl(key: String, expanded: bool, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    div()
        .id(ElementId::Name(format!("twirl:{key}").into()))
        .size(px(16.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_row(&key);
            cx.notify();
        }))
        .child(
            Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                .size(px(12.0))
                .text_color(hsla(color::MUTED)),
        )
}

fn cell(width: f32) -> Div {
    div().w(px(width)).h_full().flex_none().flex().items_center()
}

/// The switch, name and column cells of a layer row in the outline.
fn layer_outline(layer: &Layer, index: usize, expanded: bool, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let switches = layer.switches;
    let row_key = crate::timeline_rows::layer_key(id);
    let n = id.0 as usize;

    let av = cell(columns::AV)
        .px_1()
        .child(switch_toggle(("sw-visible", n), IconName::Eye, switches.visible, id, Switch::Visible, cx))
        .child(if layer.has_audio {
            switch_toggle(("sw-audio", n), IconName::Volume2, switches.audio, id, Switch::Audio, cx).into_any_element()
        } else {
            div().size(px(20.0)).into_any_element()
        })
        .child(switch_toggle(("sw-solo", n), IconName::CircleDot, switches.solo, id, Switch::Solo, cx))
        .child(switch_toggle(("sw-lock", n), IconName::Lock, switches.locked, id, Switch::Locked, cx));

    let layer_switches = cell(columns::SWITCHES)
        .child(switch_toggle(("sw-shy", n), IconName::Ghost, switches.shy, id, Switch::Shy, cx))
        // Collapse Transformations only applies to nested compositions; its slot stays empty for now.
        .child(div().size(px(20.0)))
        .child(switch_toggle(("sw-quality", n), IconName::Slash, switches.best_quality, id, Switch::BestQuality, cx))
        .child(switch_toggle(("sw-fx", n), IconName::SquareFunction, switches.effects_enabled, id, Switch::EffectsEnabled, cx))
        .child(switch_toggle(("sw-blend", n), IconName::Film, switches.frame_blending, id, Switch::FrameBlending, cx))
        .child(switch_toggle(("sw-blur", n), IconName::Wind, switches.motion_blur, id, Switch::MotionBlur, cx))
        .child(switch_toggle(("sw-adjust", n), IconName::Contrast, switches.adjustment, id, Switch::Adjustment, cx));

    let modes = BlendMode::ALL
        .iter()
        .map(|&mode| {
            let action: Box<dyn Fn(&mut Workspace)> = Box::new(move |this| this.state.set_blend_mode(id, mode));
            (mode.label().to_owned(), mode == layer.blend_mode, action)
        })
        .collect();
    let mattes = TrackMatte::ALL
        .iter()
        .map(|&matte| {
            let action: Box<dyn Fn(&mut Workspace)> = Box::new(move |this| this.state.set_track_matte(id, matte));
            (matte.label().to_owned(), matte == layer.track_matte, action)
        })
        .collect();
    let layers = &workspace.state.composition().layers;
    let parent_label = layer
        .parent
        .and_then(|parent| layers.iter().position(|candidate| candidate.id == parent))
        .map_or("None".to_owned(), |position| format!("{}. {}", position + 1, layers[position].name));
    let mut parents: Vec<MenuOption> =
        vec![("None".into(), layer.parent.is_none(), Box::new(move |this: &mut Workspace| this.state.set_parent(id, None)))];
    for (position, candidate) in layers.iter().enumerate().filter(|(_, candidate)| candidate.id != id) {
        let parent = candidate.id;
        parents.push((
            format!("{}. {}", position + 1, candidate.name),
            layer.parent == Some(parent),
            Box::new(move |this: &mut Workspace| this.state.set_parent(id, Some(parent))),
        ));
    }

    div()
        .flex()
        .h_full()
        .items_center()
        .child(av)
        .child(cell(columns::LABEL).justify_center().child(label_swatch(layer.label.color().to_rgba_u32())))
        .child(cell(columns::INDEX).justify_center().text_color(hsla(color::MUTED)).child(index.to_string()))
        .child(
            div()
                .flex_1()
                .min_w(px(columns::NAME_MIN))
                .h_full()
                .flex()
                .items_center()
                .gap_1()
                .child(twirl(row_key, expanded, cx))
                .child(div().truncate().child(layer.name.clone())),
        )
        .child(layer_switches)
        .child(cell(columns::MODE).pr_1p5().child(column_menu(("mode", n).into(), layer.blend_mode.label().into(), modes, cx)))
        .child(cell(columns::MATTE).pr_1p5().child(column_menu(("matte", n).into(), layer.track_matte.label().into(), mattes, cx)))
        .child(cell(columns::PARENT).pr_1p5().child(column_menu(("parent", n).into(), parent_label, parents, cx)))
}

/// One row of the outline (left half).
fn outline_row(row: &TimelineRow, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let layer_id = row.layer;
    let selected = workspace.state.selected_layers.contains(&layer_id);
    let expanded = workspace.state.expanded_rows.contains(&row.key);
    let base = div()
        .id(ElementId::Name(format!("outline:{}", row.key).into()))
        .h(px(size::ROW_HEIGHT))
        .min_w(px(columns::FULL))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(hsla(color::BORDER))
        .when(selected && matches!(row.kind, RowKind::Layer { .. }), |row| row.bg(hsla(color::ROW_SELECTED)))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            let additive = event.modifiers.shift || event.modifiers.platform;
            this.state.select_layer(layer_id, additive);
            cx.notify();
        }));

    let nested_padding = |depth: usize| px(columns::NAME_START + depth as f32 * 16.0);
    match &row.kind {
        RowKind::Layer { index } => {
            let layer = workspace.state.composition().layer(layer_id).expect("row layers exist");
            base.child(layer_outline(layer, *index, expanded, workspace, cx)).into_any_element()
        }
        RowKind::Group { name, is_effect } => base
            .pl(nested_padding(row.depth))
            .gap_1()
            .child(twirl(row.key.clone(), expanded, cx))
            .when(*is_effect, |row| row.child(Icon::new(IconName::SquareFunction).size(px(12.0)).text_color(hsla(color::MUTED))))
            .child(name.clone())
            .into_any_element(),
        RowKind::Property { path, name, keyframes } => {
            let path = *path;
            let frame = workspace.state.current_frame;
            let animated = !keyframes.is_empty();
            let on_key = keyframes.iter().any(|(key, _)| *key == frame);
            let row_id = row_key_id(&row.key);
            let nav = |name: &str, icon: IconName, active: bool, act: fn(&mut Workspace, Id, PropertyPath), cx: &mut Context<Workspace>| {
                row_icon(ElementId::Name(format!("{name}:{row_id}").into()), icon, active, move |this, _| act(this, layer_id, path), cx)
            };
            let navigator = cell(columns::AV).px_1().when(animated, |cell| {
                cell.child(nav("kf-prev", IconName::ChevronLeft, false, |this, layer, path| this.state.jump_to_keyframe(layer, path, false), cx))
                    .child(nav("kf-here", IconName::Diamond, on_key, |this, layer, path| this.state.toggle_keyframe_at_playhead(layer, path), cx))
                    .child(nav("kf-next", IconName::ChevronRight, false, |this, layer, path| this.state.jump_to_keyframe(layer, path, true), cx))
            });
            let stopwatch = div()
                .id(ElementId::Name(format!("tl-stopwatch:{}", row.key).into()))
                .size(px(18.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state.toggle_stopwatch(layer_id, path);
                    cx.notify();
                }))
                .child(Icon::new(IconName::Timer).size(px(14.0)).text_color(hsla(if animated { color::ACCENT } else { color::MUTED })));
            let components = workspace.state.composition().layer(layer_id).and_then(|l| l.property(path, frame)).map_or(1, |s| s.value.components());
            let fields: Vec<AnyElement> = (0..components)
                .map(|component| super::fields::value_field(FieldTarget::Property { layer: layer_id, path, component }, workspace, cx))
                .collect();
            base.child(navigator)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .pl(px(columns::LABEL + columns::INDEX + row.depth as f32 * 16.0))
                        .child(stopwatch)
                        .child(
                            div()
                                .id(ElementId::Name(format!("prop-name:{}", row_key_id(&row.key)).into()))
                                .w(px(118.0))
                                .truncate()
                                .cursor_pointer()
                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.state.select_property_keyframes(layer_id, path, event.modifiers.shift);
                                    cx.notify();
                                }))
                                .child(name.clone())
                                .test_support(),
                        )
                        .child(div().flex().gap_2().children(fields)),
                )
                .into_any_element()
        }
    }
}

/// A short id fragment for elements inside a row.
fn row_key_id(key: &str) -> String {
    key.replace('/', ":")
}

/// One row of the time graph (right half): a layer's duration bar, or a property's keyframes.
fn graph_row(row: &TimelineRow, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let view = workspace.state.view_range;
    let layer_id = row.layer;
    let layer = workspace.state.composition().layer(layer_id).expect("row layers exist");
    let selected = workspace.state.selected_layers.contains(&layer_id);

    let content: Vec<AnyElement> = match &row.kind {
        RowKind::Layer { .. } => {
            let left = fraction(layer.in_frame as f64, view);
            let width = fraction(layer.out_frame as f64, view) - left;
            let label = layer.label.color().to_rgba_u32() & 0xffffff00;
            let mut parts = Vec::new();
            // Behind a selected clip, a faint bar shows its whole source: how far it can be
            // trimmed out, and what slipping (Alt-drag) can bring into view.
            if selected && let Some((start, end)) = workspace.state.source_span(layer_id) {
                let ghost_left = fraction(start as f64, view);
                parts.push(
                    div()
                        .absolute()
                        .top(px(6.0))
                        .bottom(px(6.0))
                        .left(relative(ghost_left))
                        .w(relative(fraction(end as f64, view) - ghost_left))
                        .rounded(px(2.0))
                        .bg(rgba(label | 0x30))
                        .into_any_element(),
                );
            }
            // A grip at each end trims that end; the cursor shows it.
            let trim_handle = |edit: LayerTimeEdit, cx: &mut Context<Workspace>| {
                let id = match edit {
                    LayerTimeEdit::TrimIn => format!("trim-in:{}", layer_id.0),
                    _ => format!("trim-out:{}", layer_id.0),
                };
                div()
                    .id(ElementId::Name(id.into()))
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .w(px(TRIM_HANDLE_WIDTH))
                    .cursor_ew_resize()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.state.select_layer(layer_id, event.modifiers.shift || event.modifiers.platform);
                        this.anchor_drag(event);
                        this.state.begin_layer_time_drag(layer_id, edit);
                        cx.notify();
                    }))
                    .on_drag(TimelineDrag::LayerBar, |_, _, _, cx| cx.new(|_| EmptyDragView))
                    .test_support()
            };
            parts.push(
                div()
                    .absolute()
                    .top(px(3.0))
                    .bottom(px(3.0))
                    .left(relative(left))
                    .w(relative(width))
                    .rounded(px(2.0))
                    .bg(rgba(label | if selected { 0xdd } else { 0x99 }))
                    .border_1()
                    .border_color(rgba(if selected { 0xffffff80 } else { 0x00000066 }))
                    .when(!layer.switches.visible, |bar| bar.opacity(0.4))
                    .id(ElementId::Name(format!("bar:{}", layer_id.0).into()))
                    .cursor_grab()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.state.select_layer(layer_id, event.modifiers.shift || event.modifiers.platform);
                        this.anchor_drag(event);
                        // Alt-drag slips the source under the clip, as Premiere's Slip tool does.
                        let edit = if event.modifiers.alt && this.state.can_slip(layer_id) { LayerTimeEdit::Slip } else { LayerTimeEdit::Move };
                        this.state.begin_layer_time_drag(layer_id, edit);
                        cx.notify();
                    }))
                    .on_drag(TimelineDrag::LayerBar, |_, _, _, cx| cx.new(|_| EmptyDragView))
                    .child(trim_handle(LayerTimeEdit::TrimIn, cx).left_0())
                    .child(trim_handle(LayerTimeEdit::TrimOut, cx).right_0())
                    .test_support()
                    .into_any_element(),
            );
            parts
        }
        RowKind::Property { path, keyframes, .. } => keyframes
            .iter()
            .map(|&(frame, easing)| {
                let key = KeyframeRef { layer: layer_id, path: *path, frame };
                let is_selected = workspace.state.is_keyframe_selected(&key);
                // Marker shapes follow After Effects: diamond for linear, hourglass for eased, square for hold.
                let icon = match easing {
                    Easing::Linear => IconName::Diamond,
                    Easing::Curve(_) => IconName::Hourglass,
                    Easing::Hold => IconName::Square,
                };
                div()
                    .id(ElementId::Name(format!("kf:{}:{frame}", row_key_id(&row.key)).into()))
                    .absolute()
                    .top(px(4.0))
                    .left(relative(fraction(frame as f64, view)))
                    .ml(px(-7.0))
                    .size(px(14.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_ew_resize()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let additive = event.modifiers.shift || event.modifiers.platform;
                        if !(this.state.is_keyframe_selected(&key) && !additive) {
                            this.state.select_keyframe(key, additive);
                        }
                        this.anchor_drag(event);
                        this.state.begin_keyframe_drag();
                        cx.notify();
                    }))
                    .on_drag(TimelineDrag::Keyframes, |_, _, _, cx| cx.new(|_| EmptyDragView))
                    .child(Icon::new(icon).size(px(if is_selected { 12.0 } else { 10.0 })).text_color(hsla(if is_selected { color::ACCENT } else { color::KEYFRAME })))
                    .test_support()
                    .into_any_element()
            })
            .collect(),
        RowKind::Group { .. } => Vec::new(),
    };

    div()
        .id(ElementId::Name(format!("graph:{}", row.key).into()))
        .relative()
        .h(px(size::ROW_HEIGHT))
        .overflow_hidden()
        .border_b_1()
        .border_color(hsla(color::BORDER))
        // Pressing empty graph space selects the row's layer; dragging from there draws a selection box.
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            this.anchor_drag(event);
            if !event.modifiers.shift {
                this.state.select_layer(layer_id, event.modifiers.platform);
            }
            cx.notify();
        }))
        .on_drag(TimelineDrag::Marquee, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .children(content)
        .test_support()
        .into_any_element()
}

/// Ruler label for a tick: whole seconds as "05s", in-between ticks as frame counts like "12f".
fn tick_label(frame: i64, base: i64) -> String {
    if frame % base == 0 { format!("{:02}s", frame / base) } else { format!("{}f", frame % base) }
}

/// The work-area bar and the ruler with its ticks and the playhead handle.
fn ruler(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let state = &workspace.state;
    let view = state.view_range;
    let rate = state.composition().frame_rate;
    let graph = workspace.ui.timeline.graph.get();
    let pixels_per_frame = f64::from(graph.width) / view.len().max(1.0);
    let label_step = ruler_tick_step(pixels_per_frame, rate, 70.0);
    let subdivisions = [5, 4, 2].into_iter().find(|count| label_step % count == 0).unwrap_or(1);
    let minor_step = label_step / subdivisions;
    let first = (view.start / minor_step as f64).floor() as i64 * minor_step;
    let base = rate.timecode_base();

    let ticks = (0..)
        .map(move |index| first + index * minor_step)
        .take_while(move |frame| (*frame as f64) <= view.end)
        .map(move |frame| {
            let labeled = frame % label_step == 0;
            div()
                .absolute()
                .bottom_0()
                .left(relative(fraction(frame as f64, view)))
                .child(div().w(px(1.0)).h(px(if labeled { 8.0 } else { 4.0 })).bg(hsla(color::MUTED)))
                .when(labeled, |tick| {
                    tick.child(
                        div()
                            .absolute()
                            .bottom(px(8.0))
                            .left(px(3.0))
                            .text_size(px(10.0))
                            .text_color(hsla(color::MUTED))
                            .whitespace_nowrap()
                            .child(tick_label(frame, base)),
                    )
                })
        });

    let work = state.work_area;
    let (work_left, work_right) = (fraction(work.start, view), fraction(work.end, view));
    let playhead = fraction(state.current_frame as f64, view);

    let edge = |id: &'static str, drag: TimelineDrag, cx: &mut Context<Workspace>| {
        div()
            .id(id)
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(8.0))
            .cursor_ew_resize()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.anchor_drag(event);
            }))
            .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragView))
    };

    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .child(workspace.ui.timeline.graph.probe())
        // Work area: the frames a preview or render covers.
        .child(
            div().absolute().top_0().left_0().right_0().h(px(8.0)).bg(hsla(color::INSET)).child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(work_left))
                    .w(relative(work_right - work_left))
                    .bg(hsla(color::WORK_AREA))
                    .child(edge("work-start", TimelineDrag::WorkStart, cx).left(px(-4.0)))
                    .child(edge("work-end", TimelineDrag::WorkEnd, cx).right(px(-4.0))),
            ),
        )
        // Ticks; pressing or dragging here moves the playhead.
        .child(
            div()
                .id("ruler-scrub")
                .absolute()
                .top(px(8.0))
                .bottom_0()
                .left_0()
                .right_0()
                .cursor_col_resize()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.anchor_drag(event);
                    this.apply_timeline_drag(TimelineDrag::Scrub, f32::from(event.position.x), f32::from(event.position.y));
                    cx.notify();
                }))
                .on_drag(TimelineDrag::Scrub, |_, _, _, cx| cx.new(|_| EmptyDragView))
                .children(ticks)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(playhead))
                        .ml(px(-5.0))
                        .w(px(11.0))
                        .child(div().absolute().bottom_0().left_0().w(px(11.0)).h(px(10.0)).rounded_t(px(2.0)).bg(hsla(color::ACCENT)))
                        .child(div().absolute().top_0().bottom_0().left(px(5.0)).w(px(1.0)).bg(hsla(color::ACCENT))),
                ),
        )
}

/// The bar above the ruler showing the whole composition; its bright span is the visible range.
fn navigator(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let duration = workspace.state.composition().duration_frames as f64;
    let full = FrameRange::new(0.0, duration);
    let view = workspace.state.view_range;
    let (left, right) = (fraction(view.start, full), fraction(view.end, full));

    let handle = |id: &'static str, drag: TimelineDrag, cx: &mut Context<Workspace>| {
        div()
            .id(id)
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(8.0))
            .bg(hsla(color::ACCENT))
            .cursor_ew_resize()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.anchor_drag(event);
            }))
            .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragView))
    };

    div().relative().size_full().child(
        div().absolute().left_0().right_0().top(px(14.0)).h(px(12.0)).rounded(px(2.0)).bg(hsla(color::INSET)).child(
            div()
                .id("navigator-thumb")
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(left))
                .w(relative(right - left))
                .rounded(px(2.0))
                .bg(hsla(color::NAVIGATOR_THUMB))
                .cursor_grab()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, _| this.anchor_drag(event)))
                .on_drag(TimelineDrag::ViewBody, |_, _, _, cx| cx.new(|_| EmptyDragView))
                .child(handle("view-start", TimelineDrag::ViewStart, cx).left_0().rounded_l(px(2.0)))
                .child(handle("view-end", TimelineDrag::ViewEnd, cx).right_0().rounded_r(px(2.0))),
        ),
    )
}

/// Column titles above the outline.
fn outline_header() -> impl IntoElement {
    let icon = |name: IconName| Icon::new(name).size(px(12.0)).text_color(hsla(color::MUTED));
    let titled = |width: f32, title: &'static str| cell(width).pl_1p5().border_l_1().border_color(hsla(color::HAIRLINE)).child(title);
    div()
        .flex()
        .h_full()
        .min_w(px(columns::FULL))
        .items_center()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .child(cell(columns::AV).justify_around().px_1().child(icon(IconName::Eye)).child(icon(IconName::Volume2)).child(icon(IconName::CircleDot)).child(icon(IconName::Lock)))
        .child(cell(columns::LABEL).justify_center().child(icon(IconName::Tag)))
        .child(titled(columns::INDEX, "#"))
        .child(div().flex_1().min_w(px(columns::NAME_MIN)).h_full().flex().items_center().pl_1p5().border_l_1().border_color(hsla(color::HAIRLINE)).child("Source Name"))
        .child(
            cell(columns::SWITCHES)
                .justify_around()
                .child(icon(IconName::Ghost))
                .child(icon(IconName::Sun))
                .child(icon(IconName::Slash))
                .child(icon(IconName::SquareFunction))
                .child(icon(IconName::Film))
                .child(icon(IconName::Wind))
                .child(icon(IconName::Contrast)),
        )
        .child(titled(columns::MODE, "Mode"))
        .child(titled(columns::MATTE, "Track Matte"))
        .child(titled(columns::PARENT, "Parent & Link"))
}

/// Current time, frame count and the composition switches at the top-left.
fn controls(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let state = &workspace.state;
    let rate = state.composition().frame_rate;
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_2p5()
        .h_full()
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().font_family("Menlo").text_size(px(17.0)).text_color(hsla(color::VALUE)).child(format_timecode(state.current_frame, rate)))
                .child(div().text_size(px(10.0)).text_color(hsla(color::MUTED)).child(format!("{:05} ({})", state.current_frame, rate.label()))),
        )
        .child(div().flex_1())
        .child(icon_button("snapping", IconName::Magnet, "Snapping (S)", state.snapping, cx.listener(|this, _, _, cx| {
            this.state.snapping = !this.state.snapping;
            cx.notify();
        })))
        .child(icon_button("hide-shy", IconName::Ghost, "Hide Shy Layers", workspace.ui.hide_shy_layers, cx.listener(|this, _, _, cx| {
            this.ui.hide_shy_layers = !this.ui.hide_shy_layers;
            cx.notify();
        })))
}

pub fn render(workspace: &Workspace, _window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let ui = &workspace.ui.timeline;
    let outline_width = ui.outline_width;
    let rows = workspace.timeline_rows();
    let playhead = fraction(workspace.state.current_frame as f64, workspace.state.view_range);
    let drop_preview = ui.drop_preview.filter(|_| cx.has_active_drag());

    let band = |height: f32| div().h(px(height)).flex_none().flex().border_b_1().border_color(hsla(color::HAIRLINE));
    let gap = || div().w(px(SPLITTER)).flex_none().h_full();

    let outline_rows: Vec<AnyElement> = rows.iter().map(|row| outline_row(row, workspace, cx)).collect();
    let graph_rows: Vec<AnyElement> = rows.iter().map(|row| graph_row(row, workspace, cx)).collect();

    let indicator = drop_preview.map(|preview| {
        let top = px(preview.row as f32 * size::ROW_HEIGHT);
        if preview.is_effect {
            div().absolute().left_0().right_0().top(top).h(px(size::ROW_HEIGHT)).border_2().border_color(hsla(color::ACCENT)).rounded(px(2.0))
        } else {
            div().absolute().left_0().right_0().top(top).h(px(2.0)).mt(px(-1.0)).bg(hsla(color::ACCENT))
        }
    });
    let drop_frame_line = drop_preview.and_then(|preview| preview.frame).map(|frame| {
        div().absolute().top_0().bottom_0().left(relative(fraction(frame as f64, workspace.state.view_range))).w(px(1.0)).bg(hsla(color::ACCENT))
    });

    // The box selection rectangle, in the graph column's own coordinates.
    let marquee = ui.marquee.filter(|_| cx.has_active_drag()).map(|((x0, y0), (x1, y1))| {
        let (graph, rows) = (ui.graph.get(), ui.rows.get());
        div()
            .absolute()
            .left(px(x0.min(x1) - graph.left))
            .top(px(y0.min(y1) - rows.top))
            .w(px((x1 - x0).abs()))
            .h(px((y1 - y0).abs()))
            .border_1()
            .border_color(hsla(color::ACCENT))
            .bg(gpui_kit::rgba(0x2d8ceb22))
    });

    let rows_content = div()
        .id("timeline-rows-content")
        .relative()
        .min_h_full()
        .flex()
        .child(ui.rows.probe())
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ProjectItemDrag>, _, cx| {
            let bounds = to_rect(event.bounds);
            this.preview_drop(false, f32::from(event.event.position.x), f32::from(event.event.position.y), bounds);
            cx.notify();
        }))
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<EffectDrag>, _, cx| {
            let bounds = to_rect(event.bounds);
            this.preview_drop(true, f32::from(event.event.position.x), f32::from(event.event.position.y), bounds);
            cx.notify();
        }))
        .on_drop(cx.listener(|this, drag: &ProjectItemDrag, _, cx| {
            if let Some(preview) = this.ui.timeline.drop_preview.take() {
                let frame = preview.frame.unwrap_or(this.state.current_frame);
                this.state.add_layer_from_item(drag.item, preview.insert_index, frame);
            }
            cx.notify();
        }))
        .on_drop(cx.listener(|this, drag: &EffectDrag, _, cx| {
            if let Some(layer) = this.ui.timeline.drop_preview.take().and_then(|preview| preview.layer) {
                this.state.add_effect(layer, &drag.name);
            }
            cx.notify();
        }))
        .child(div().w(px(outline_width)).flex_none().overflow_hidden().children(outline_rows))
        .child(gap())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .relative()
                .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                    if !event.modifiers.alt {
                        return;
                    }
                    let graph = this.ui.timeline.graph.get();
                    let delta = f64::from(f32::from(event.delta.pixel_delta(px(16.0)).y));
                    let view = this.state.view_range;
                    let anchor = frame_at(f32::from(event.position.x), graph, view);
                    let factor = (delta * WHEEL_ZOOM_SENSITIVITY).exp();
                    this.state.set_view_range(FrameRange::new(
                        anchor - (anchor - view.start) * factor,
                        anchor + (view.end - anchor) * factor,
                    ));
                    cx.stop_propagation();
                    cx.notify();
                }))
                .children(graph_rows)
                .child(div().absolute().top_0().bottom_0().left(relative(playhead)).w(px(1.0)).bg(hsla(color::ACCENT)))
                // Where the current drag snapped to.
                .children(workspace.state.snap_line().map(|frame| {
                    div()
                        .id("snap-line")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(fraction(frame as f64, workspace.state.view_range)))
                        .w(px(1.0))
                        .bg(hsla(color::SNAP))
                        .test_support()
                }))
                .children(drop_frame_line)
                .children(marquee),
        )
        .children(indicator);

    let body = div()
        .id("timeline-body")
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<TimelineDrag>, _, cx| {
            let drag = *event.drag(cx);
            this.apply_timeline_drag(drag, f32::from(event.event.position.x), f32::from(event.event.position.y));
            cx.notify();
        }))
        .child(
            band(CONTROLS_BAND)
                .child(div().w(px(outline_width)).flex_none().overflow_hidden().child(controls(workspace, cx)))
                .child(gap())
                .child(div().flex_1().min_w_0().child(navigator(workspace, cx))),
        )
        .child(
            band(RULER_BAND)
                .child(div().w(px(outline_width)).flex_none().overflow_hidden().child(outline_header()))
                .child(gap())
                .child(div().flex_1().min_w_0().child(ruler(workspace, cx))),
        )
        .child(div().id("timeline-rows").flex_1().min_h_0().overflow_y_scroll().child(rows_content))
        // The splitter spans every band so the outline and graph resize together.
        .child(
            div()
                .id("timeline-splitter")
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(outline_width))
                .w(px(SPLITTER))
                .cursor_col_resize()
                .bg(hsla(color::BACKGROUND))
                .hover(|style| style.bg(hsla(color::ACCENT)))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, _| this.anchor_drag(event)))
                .on_drag(TimelineDrag::Splitter, |_, _, _, cx| cx.new(|_| EmptyDragView)),
        );

    let open_menu = ui.open_menu.as_ref().map(|open| deferred(anchored().position(open.position).child(open.menu.clone())).with_priority(1));
    let body = body.children(open_menu);

    let footer = div()
        .flex()
        .items_center()
        .w_full()
        .gap_1()
        .child(div().px_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Frame Render Time: 0ms"))
        .child(div().flex_1())
        .child(icon_button("zoom-out", IconName::Mountain, "Zoom Out (Alt + Scroll)", false, cx.listener(|this, _, _, cx| {
            zoom_around_playhead(this, 1.5);
            cx.notify();
        })))
        .child(icon_button("zoom-in", IconName::MountainSnow, "Zoom In (Alt + Scroll)", false, cx.listener(|this, _, _, cx| {
            zoom_around_playhead(this, 1.0 / 1.5);
            cx.notify();
        })));

    let comp_name = workspace.state.composition().name.clone();
    panel(PanelId::Timeline, &[comp_name.as_str()], Some(footer.into_any_element()), body, workspace, cx)
}

/// Zooms the visible range by `factor`, centered on the playhead.
fn zoom_around_playhead(workspace: &mut Workspace, factor: f64) {
    let view = workspace.state.view_range;
    let center = workspace.state.current_frame as f64;
    let half = view.len() * factor / 2.0;
    workspace.state.set_view_range(FrameRange::new(center - half, center + half));
}

fn to_rect(bounds: gpui_kit::Bounds<gpui_kit::Pixels>) -> Rect {
    Rect {
        left: f32::from(bounds.origin.x),
        top: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_and_frames_round_trip() {
        let view = FrameRange::new(48.0, 96.0);
        assert_eq!(fraction(72.0, view), 0.5);
        let graph = Rect { left: 100.0, top: 0.0, width: 480.0, height: 20.0 };
        assert_eq!(frame_at(340.0, graph, view), 72.0);
    }

    #[test]
    fn tick_labels_show_seconds_and_frames() {
        assert_eq!(tick_label(48, 24), "02s");
        assert_eq!(tick_label(54, 24), "6f");
    }
}
