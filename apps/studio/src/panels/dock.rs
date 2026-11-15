//! Movable panels. Every panel lives in a dock (gpui-kit's `DockArea`): dragging a panel's tab
//! onto another panel puts it beside that panel (left, right, above or below, by where it is
//! dropped) or into its tabs, and the dividers between panels resize. The arrangement is saved
//! in the app settings after every change and restored on the next launch. Window ▸ Reset
//! Workspace brings back the standard arrangement.
//!
//! The dock itself only keeps the layout and handles the dragging; this module supplies how it
//! looks (`StudioSkin`), the standard layout, and the conversion to and from the saved form.

use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::dock::{
    BasePanelView, DockAreaRenderer, DockAreaState, DockLayout, DragPanel, DropIndicator, PanelInfo, PanelState,
    TabGroupContext, TabGroupRenderer,
};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, DismissEvent, ElementId, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled as _, TestSupportExt as _, WeakEntity, Window, anchored,
    deferred, div, prelude::FluentBuilder as _, px,
};

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName};

use crate::panels::views::PanelView;
use crate::theme::{color, hsla, size};
use crate::workspace::{LeftPanel, PanelId, Workspace};

/// Written with every saved layout. A saved layout from a different version is set aside for
/// the standard one, so a change to what the saved form means can't scramble someone's screen.
pub const LAYOUT_VERSION: usize = 1;

/// Every panel that can be moved, in the order the standard layout lists them.
pub const DOCKABLE: [PanelId; 7] = [
    PanelId::Side,
    PanelId::Composition,
    PanelId::Timeline,
    PanelId::Nodes,
    PanelId::Properties,
    PanelId::Align,
    PanelId::Mixer,
];

/// The panels the side panel shows, which layouts saved before it became one panel list as
/// panels of their own. Read back, each stands for the side panel.
const FORMER_SIDE_PANELS: [PanelId; 7] = [
    PanelId::Project,
    PanelId::Libraries,
    PanelId::Effects,
    PanelId::Elements,
    PanelId::Templates,
    PanelId::Mockups,
    PanelId::Captions,
];

/// Height of the Align panel under Properties in the standard layout.
const ALIGN_HEIGHT: f32 = 230.0;

impl PanelId {
    /// The panel's name in a saved layout. These never change, or saved layouts would lose
    /// the panel.
    pub fn name(self) -> &'static str {
        match self {
            PanelId::Side => "Side",
            PanelId::Project => "Project",
            PanelId::Composition => "Composition",
            PanelId::Timeline => "Timeline",
            PanelId::Properties => "Properties",
            PanelId::Effects => "Effects",
            PanelId::Libraries => "Libraries",
            PanelId::Align => "Align",
            PanelId::Templates => "Templates",
            PanelId::Captions => "Captions",
            PanelId::Elements => "Elements",
            PanelId::Mockups => "Mockups",
            PanelId::Stock => "Stock",
            PanelId::Nodes => "Nodes",
            PanelId::Mixer => "Mixer",
        }
    }

    /// The movable panel a saved name stands for, if this version of Studio has it. The names
    /// of the panels the side panel shows stand for the side panel.
    pub fn from_name(name: &str) -> Option<PanelId> {
        if FORMER_SIDE_PANELS.iter().any(|id| id.name() == name) {
            return Some(PanelId::Side);
        }
        DOCKABLE.into_iter().find(|id| id.name() == name)
    }

    /// What the panel's tab says. The viewer's names the composition it shows; the side
    /// panel's names what the rail picked for it.
    pub fn title(self, workspace: &Workspace) -> SharedString {
        match self {
            PanelId::Composition => format!("Composition: {}", workspace.state.composition().name).into(),
            PanelId::Side => match workspace.ui.left_panel {
                LeftPanel::Media => "Project",
                LeftPanel::Effects => "Effects",
                LeftPanel::Libraries => "Libraries",
                LeftPanel::Templates => "Templates",
                LeftPanel::Captions => "Captions",
                LeftPanel::Elements => "Elements",
                LeftPanel::Mockups => "Mockups",
                LeftPanel::Stock => "Stock",
                LeftPanel::History => "History",
                // Her tab carries the name she's been given, as the rail button does.
                LeftPanel::Assistant => return super::assistant::name(&workspace.ui.settings).into(),
            }
            .into(),
            other => other.name().into(),
        }
    }
}

/// The standard arrangement, the one Studio starts with: the side panel beside the viewer, the timeline (with the Nodes panel, while it is open) under them, and Properties
/// (with the Mixer as a tab behind it) above Align down the right.
pub fn default_layout(view: impl Fn(PanelId) -> Entity<PanelView>) -> DockLayout {
    let tabs = |ids: &[PanelId]| ids.iter().fold(DockLayout::tabs(), |tabs, id| tabs.panel(view(*id)));
    let top = DockLayout::h_split().child(tabs(&[PanelId::Side]), Some(px(300.0))).child(tabs(&[PanelId::Composition]), None);
    let bottom = DockLayout::h_split().child(tabs(&[PanelId::Timeline]), None).child(tabs(&[PanelId::Nodes]), Some(px(560.0)));
    let main = DockLayout::v_split().child(top, Some(px(520.0))).child(bottom, None);
    let right = DockLayout::v_split().child(tabs(&[PanelId::Properties, PanelId::Mixer]), None).child(tabs(&[PanelId::Align]), Some(px(ALIGN_HEIGHT)));
    DockLayout::h_split().child(main, None).child(right, Some(px(430.0)))
}

/// The Color Correction arrangement: a large viewer with a wide Properties panel beside it for
/// the color controls, and a short timeline (with the Nodes panel, while it is open) under the
/// viewer. Align shares Properties' tabs.
fn color_layout(view: impl Fn(PanelId) -> Entity<PanelView>) -> DockLayout {
    let tabs = |ids: &[PanelId]| ids.iter().fold(DockLayout::tabs(), |tabs, id| tabs.panel(view(*id)));
    let bottom = DockLayout::h_split().child(tabs(&[PanelId::Timeline]), None).child(tabs(&[PanelId::Nodes]), Some(px(520.0)));
    let middle = DockLayout::v_split().child(tabs(&[PanelId::Composition]), None).child(bottom, Some(px(260.0)));
    DockLayout::h_split()
        .child(tabs(&[PanelId::Side]), Some(px(240.0)))
        .child(middle, None)
        .child(tabs(&[PanelId::Properties, PanelId::Align, PanelId::Mixer]), Some(px(480.0)))
}

/// The Audio Editing arrangement: the timeline takes most of the window, for tall tracks and
/// their waveforms, with a small viewer between the side panel and Properties above it.
fn audio_layout(view: impl Fn(PanelId) -> Entity<PanelView>) -> DockLayout {
    let tabs = |ids: &[PanelId]| ids.iter().fold(DockLayout::tabs(), |tabs, id| tabs.panel(view(*id)));
    let top = DockLayout::h_split()
        .child(tabs(&[PanelId::Side]), Some(px(300.0)))
        .child(tabs(&[PanelId::Composition]), None)
        .child(tabs(&[PanelId::Properties, PanelId::Mixer, PanelId::Align]), Some(px(420.0)));
    let bottom = DockLayout::h_split().child(tabs(&[PanelId::Timeline]), None).child(tabs(&[PanelId::Nodes]), Some(px(520.0)));
    DockLayout::v_split().child(top, Some(px(300.0))).child(bottom, None)
}

/// The arrangement of a built-in workspace (`None` for Standard).
pub fn built_in_layout(built_in: Option<crate::panels::workspaces::BuiltIn>, view: impl Fn(PanelId) -> Entity<PanelView>) -> DockLayout {
    use crate::panels::workspaces::BuiltIn;
    match built_in {
        None => default_layout(view),
        Some(BuiltIn::ColorCorrection) => color_layout(view),
        Some(BuiltIn::AudioEditing) => audio_layout(view),
    }
}

/// Rebuilds a saved layout with Studio's own panel views. Panels the saved layout doesn't
/// mention (ones added in a later version, say) join the side panel's group, so none is ever
/// lost; the Mixer, which belongs on the right, joins Properties' group instead when there is one; names this version doesn't know are skipped. In a layout saved when the side panel's
/// choices were panels of their own, the first of them becomes the side panel. `None` when the saved layout
/// is from another version or holds no panels at all, so the standard one is used instead.
pub fn restore_layout(saved: &DockAreaState, view: impl Fn(PanelId) -> Entity<PanelView>) -> Option<DockLayout> {
    if saved.version != Some(LAYOUT_VERSION) {
        return None;
    }
    let mut center = saved.center.clone();
    let mut placed = HashSet::new();
    collect_panels(&center, &mut placed);
    if placed.is_empty() {
        return None;
    }
    let mut missing: Vec<PanelId> = DOCKABLE.into_iter().filter(|id| !placed.contains(id)).collect();
    if missing.contains(&PanelId::Mixer) && add_to_group_of(&mut center, PanelId::Properties, &[PanelId::Mixer]) {
        missing.retain(|id| *id != PanelId::Mixer);
    }
    if !missing.is_empty() && !add_to_group_of(&mut center, PanelId::Side, &missing) {
        add_to_first_group(&mut center, &missing);
    }
    let mut used = HashSet::new();
    build(&center, &view, &mut used)
}

/// The panels a saved layout places, by name.
fn collect_panels(state: &PanelState, placed: &mut HashSet<PanelId>) {
    if let Some(id) = PanelId::from_name(&state.panel_name) {
        placed.insert(id);
    }
    for child in &state.children {
        collect_panels(child, placed);
    }
}

/// Appends `missing` to the first tab group holding panel `beside`. False if there is none.
fn add_to_group_of(state: &mut PanelState, beside: PanelId, missing: &[PanelId]) -> bool {
    let holds_it = state.children.iter().any(|child| PanelId::from_name(&child.panel_name) == Some(beside));
    if matches!(state.info, PanelInfo::Tabs { .. }) && holds_it {
        state.children.extend(missing.iter().map(|id| PanelState::new(id.name())));
        return true;
    }
    state.children.iter_mut().any(|child| add_to_group_of(child, beside, missing))
}

/// Appends `missing` to the first tab group of all. False if there is none.
fn add_to_first_group(state: &mut PanelState, missing: &[PanelId]) -> bool {
    if matches!(state.info, PanelInfo::Tabs { .. }) {
        state.children.extend(missing.iter().map(|id| PanelState::new(id.name())));
        return true;
    }
    state.children.iter_mut().any(|child| add_to_first_group(child, missing))
}

/// One saved container as a layout. A panel named twice keeps only its first place; empty
/// containers are dropped, and a split left with one child becomes that child.
fn build(state: &PanelState, view: &impl Fn(PanelId) -> Entity<PanelView>, used: &mut HashSet<PanelId>) -> Option<DockLayout> {
    match &state.info {
        PanelInfo::Stack { sizes, .. } => {
            let axis = state.info.axis()?;
            // A split saved before the dock had measured it records every slot at the dock's
            // placeholder size. Taken literally, the first panels would be that narrow and the
            // last would get all the rest, so such a split is left unsized: its panels share
            // the space equally, as they did on screen when it was saved.
            let unmeasured = sizes.len() > 1 && sizes.iter().all(|size| *size == gpui_kit::base::PANEL_MIN_SIZE);
            let children: Vec<(DockLayout, Option<Pixels>)> = state
                .children
                .iter()
                .enumerate()
                .filter_map(|(index, child)| {
                    let size = sizes.get(index).copied().filter(|size| *size > px(0.0) && !unmeasured);
                    build(child, view, used).map(|layout| (layout, size))
                })
                .collect();
            if children.len() <= 1 {
                return children.into_iter().next().map(|(layout, _)| layout);
            }
            let split = if axis == gpui_kit::Axis::Horizontal { DockLayout::h_split() } else { DockLayout::v_split() };
            Some(children.into_iter().fold(split, |split, (child, size)| split.child(child, size)))
        }
        PanelInfo::Tabs { active_index } => {
            let mut tabs = DockLayout::tabs();
            let mut count = 0;
            let mut active = 0;
            for (index, child) in state.children.iter().enumerate() {
                let Some(id) = PanelId::from_name(&child.panel_name).filter(|id| used.insert(*id)) else { continue };
                if index == *active_index {
                    active = count;
                }
                tabs = tabs.panel(view(id));
                count += 1;
            }
            (count > 0).then(|| tabs.active_index(active))
        }
        // A lone panel outside any tab group: give it a group of its own.
        PanelInfo::Panel(_) => {
            let id = PanelId::from_name(&state.panel_name).filter(|id| used.insert(*id))?;
            Some(DockLayout::tabs().panel(view(id)))
        }
    }
}

/// The dockable panel a dock entry draws, if it is one of Studio's.
fn studio_panel(panel: &Arc<dyn BasePanelView>, cx: &App) -> Option<PanelId> {
    panel.as_any().downcast_ref::<Entity<PanelView>>().and_then(|view| view.read(cx).kind.dock_id())
}

/// Half the gap between two neighboring panels: each panel group is inset by this much, so
/// a thin line of the window's darker background shows between them.
const PANEL_INSET: f32 = 1.0;

/// Height of the chip holding a tab's grip and name.
const TAB_CHIP_HEIGHT: f32 = 22.0;

/// Studio's look for the dock: every panel group is a block of its own with a gap around it,
/// each tab has a grip that opens the panel's menu, and an accent wash covers the spot a
/// dragged panel would land in.
pub struct StudioSkin {
    workspace: WeakEntity<Workspace>,
}

impl StudioSkin {
    pub fn new(workspace: WeakEntity<Workspace>) -> Rc<Self> {
        Rc::new(Self { workspace })
    }
}

impl DockAreaRenderer for StudioSkin {
    /// No line between panels: the gap between them is the divider. (The dock still handles
    /// the dragging there.)
    fn render_split_handle(&self, _: &gpui_kit::base::ResizeHandleContext, _: &mut Window, _: &mut App) -> Option<AnyElement> {
        Some(div().into_any_element())
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(StudioTabs { workspace: self.workspace.clone() })
    }
}

/// Draws one panel group: its tabs, and the panel showing.
struct StudioTabs {
    workspace: WeakEntity<Workspace>,
}

impl TabGroupRenderer for StudioTabs {
    fn frame(&self, group: &TabGroupContext, _: &mut Window, _: &mut App) -> gpui_kit::Stateful<gpui_kit::Div> {
        // The group's own box is only the gap; the tab bar and the panel under it draw the
        // panel's surface.
        div().id(("dock-group", group.node().as_u64())).p(px(PANEL_INSET))
    }

    fn render_tab_bar(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        let workspace = self.workspace.upgrade();
        let shown: Vec<(usize, PanelId)> = group
            .panels()
            .iter()
            .enumerate()
            .filter(|(_, panel)| panel.visible(cx))
            .filter_map(|(index, panel)| studio_panel(panel, cx).map(|id| (index, id)))
            .collect();
        // The tab shown, which is the front one unless that panel is turned off.
        let active = group.active_panel().map(|panel| panel.panel_id(cx));
        let tabs = shown.into_iter().map(|(index, id)| {
            let title = workspace.as_ref().map_or_else(|| SharedString::from(id.name()), |workspace| id.title(workspace.read(cx)));
            let selected = group.panels().get(index).map(|panel| panel.panel_id(cx)) == active;
            let (select, drop_before) = (group.clone(), group.clone());
            div()
                .id(ElementId::Name(format!("dock-tab:{}", id.name()).into()))
                .h_full()
                .min_w(px(36.0))
                .flex_shrink(1.0)
                .flex()
                .items_center()
                .cursor_grab()
                .text_color(hsla(if selected { color::TEXT } else { color::MUTED }))
                .hover(|style| style.text_color(hsla(color::TEXT)))
                .on_click(move |_, window, cx| select.select_tab(index, window, cx))
                // Dragging the tab carries the panel; dropping it on another panel docks it there.
                .when_some(group.is_draggable().then(|| group.drag_panel(index, cx)).flatten(), |tab, drag| {
                    tab.on_drag(drag, move |drag: &DragPanel, offset, _, cx| {
                        // Where the tab was grabbed and how big the label is, so the drop
                        // highlight can grow out of the label.
                        drag.set_drag_offset(offset);
                        drag.set_preview_size(gpui_kit::size(px(120.0), px(24.0)));
                        cx.stop_propagation();
                        cx.new(|_| DragPreview { title: id.name().into() })
                    })
                })
                // Dropped on a tab, a panel joins the group just before it.
                .on_drop(move |drag: &DragPanel, window, cx| drop_before.drop_panel(drag.clone(), Some(index), true, window, cx))
                .drag_over::<DragPanel>(|style, _, _, _| style.bg(hsla(color::HOVER)))
                // The grip and the label share a chip, filled for the tab that is showing.
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_1()
                        .h(px(TAB_CHIP_HEIGHT))
                        .pl_0p5()
                        .pr_1p5()
                        .rounded(px(4.0))
                        .when(selected, |chip| chip.bg(hsla(color::HOVER)))
                        .child(tab_grip(id, group.clone(), self.workspace.clone(), cx))
                        .child(div().min_w_0().truncate().child(title)),
                )
                .test_support()
        }).collect::<Vec<_>>();
        let append = group.clone();
        // The Timeline's transport and track buttons share its tab's row instead of taking a
        // row of their own under it.
        let showing_timeline = group.active_panel().and_then(|panel| studio_panel(panel, cx)) == Some(PanelId::Timeline);
        let timeline_bar = workspace
            .clone()
            .filter(|_| showing_timeline)
            .map(|workspace| workspace.update(cx, |workspace, cx| crate::panels::timeline::toolbar(workspace, cx).into_any_element()));
        let height = if timeline_bar.is_some() { crate::panels::timeline::CONTROLS_BAND } else { size::PANEL_HEADER_HEIGHT };
        let has_timeline_bar = timeline_bar.is_some();
        div()
            .id(("dock-tab-bar", group.node().as_u64()))
            .h(px(height))
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            // The first tab's chip sits as far from the panel's left edge as from its top.
            .pl(px((height - TAB_CHIP_HEIGHT) / 2.0))
            .pr_1()
            .overflow_hidden()
            .bg(hsla(color::PANEL))
            .rounded_t(px(size::PANEL_RADIUS))
            .children(tabs)
            // Dropped on the empty end of the bar, a panel joins the group as its last tab.
            .child(
                div()
                    .id(("dock-tab-bar-end", group.node().as_u64()))
                    .when(has_timeline_bar, |end| end.w(px(12.0)).flex_none())
                    .when(!has_timeline_bar, |end| end.flex_1())
                    .h_full()
                    .on_drop(move |drag: &DragPanel, window, cx| append.drop_panel(drag.clone(), None, true, window, cx))
                    .drag_over::<DragPanel>(|style, _, _, _| style.bg(hsla(color::HOVER)))
                    .test_support(),
            )
            .children(timeline_bar.map(|bar| div().flex_1().min_w_0().h_full().child(bar)))
            .into_any_element()
    }

    fn render_active_panel(&self, panel: AnyView, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        let name = group.active_panel().and_then(|panel| studio_panel(panel, cx)).map_or("Other", PanelId::name);
        // Cached, so a panel only redraws when its own content changes.
        div()
            .id(ElementId::Name(format!("dock-content:{name}").into()))
            .flex_1()
            .min_h_0()
            .relative()
            .bg(hsla(color::PANEL))
            .rounded_b(px(size::PANEL_RADIUS))
            .child(panel.cached(StyleRefinement::default().absolute().size_full()))
            .test_support()
            .into_any_element()
    }

    fn render_drop_indicator(&self, indicator: DropIndicator, _: &mut Window, _: &mut App) -> Option<AnyElement> {
        let to = indicator.to();
        Some(
            div()
                .id("dock-drop-target")
                .absolute()
                .left(to.origin().x)
                .top(to.origin().y)
                .w(to.size().width)
                .h(to.size().height)
                .bg(hsla(color::ACCENT).opacity(0.25))
                .border_2()
                .border_color(hsla(color::ACCENT))
                .test_support()
                .into_any_element(),
        )
    }
}

/// The menu opened from the grip on a panel's tab, while it is open.
pub struct PanelMenu {
    /// The panel whose grip opened it, which stays lit meanwhile.
    pub panel: PanelId,
    pub menu: Entity<PopupMenu>,
    /// Where it was opened: the pointer's place at the click.
    pub at: Point<Pixels>,
}

/// The grip at the start of a panel's tab: three short lines. Dragging it carries the panel,
/// like the rest of the tab; a plain click opens the panel's menu. It is a plain element and
/// not a button or a menu trigger, because those keep the press to themselves and the tab
/// under them could then not be dragged by its grip.
fn tab_grip(id: PanelId, group: TabGroupContext, workspace: WeakEntity<Workspace>, cx: &App) -> impl IntoElement {
    let open = workspace.upgrade().is_some_and(|workspace| workspace.read(cx).ui.panel_menu.as_ref().is_some_and(|menu| menu.panel == id));
    div()
        .id(ElementId::Name(format!("dock-tab-menu:{}", id.name()).into()))
        .flex_none()
        .size(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .text_color(hsla(if open { color::TEXT } else { color::MUTED }))
        .when(open, |grip| grip.bg(hsla(color::HAIRLINE)))
        .hover(|style| style.bg(hsla(color::HAIRLINE)).text_color(hsla(color::TEXT)))
        .tooltip(|window, cx| Tooltip::new("Drag to move this panel, or click for its menu").build(window, cx))
        .on_click(move |_, window, cx| open_panel_menu(id, group.clone(), workspace.clone(), window, cx))
        .child(Icon::new(IconName::Menu).size(px(12.0)))
        .test_support()
}

/// Opens the menu for panel `id` at the pointer: filling the window with the panel (or
/// putting it back), and saving or resetting the arrangement. It closes itself when an item
/// is picked, on Escape, or on a click elsewhere.
fn open_panel_menu(id: PanelId, group: TabGroupContext, workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
    let zoomed = group.is_zoomed();
    let (save, reset) = (workspace.clone(), workspace.clone());
    let menu = PopupMenu::build(window, cx, move |menu, _, _| {
        let (group, save, reset) = (group.clone(), save.clone(), reset.clone());
        menu.item(PopupMenuItem::new(if zoomed { "Restore Panel Size" } else { "Maximize Panel" }).on_click(move |_, window, cx| group.toggle_zoom(window, cx)))
            .separator()
            .item(PopupMenuItem::new("Save as New Workspace…").on_click(move |_, window, cx| {
                save.update(cx, |workspace, cx| crate::panels::workspaces::save_as_new(workspace, window, cx)).ok();
            }))
            .item(PopupMenuItem::new("Reset Workspace").on_click(move |_, window, cx| {
                reset.update(cx, |workspace, cx| workspace.reset_workspace(window, cx)).ok();
            }))
    });
    // The menu takes the keyboard, so Escape and the arrow keys reach it.
    window.focus(&menu.focus_handle(cx), cx);
    let closing = workspace.clone();
    cx.subscribe(&menu, move |_, _: &DismissEvent, cx| {
        closing
            .update(cx, |workspace, cx| {
                workspace.ui.panel_menu = None;
                cx.notify();
            })
            .ok();
    })
    .detach();
    let at = window.mouse_position();
    workspace
        .update(cx, |workspace, cx| {
            workspace.ui.panel_menu = Some(PanelMenu { panel: id, menu, at });
            cx.notify();
        })
        .ok();
}

/// The open panel menu, drawn above everything else at the place it was opened.
pub fn panel_menu(workspace: &Workspace) -> Option<AnyElement> {
    let open = workspace.ui.panel_menu.as_ref()?;
    Some(
        deferred(anchored().position(open.at).snap_to_window_with_margin(px(8.0)).child(open.menu.clone()))
            .with_priority(gpui_kit::base::POPUP_PRIORITY)
            .into_any_element(),
    )
}

/// The label that follows the pointer while a panel is dragged.
struct DragPreview {
    title: SharedString,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(120.0))
            .h(px(24.0))
            .px_2()
            .flex()
            .items_center()
            .overflow_hidden()
            .rounded(px(3.0))
            .bg(hsla(color::PANEL))
            .border_1()
            .border_color(hsla(color::ACCENT))
            .text_size(px(size::TEXT))
            .text_color(hsla(color::TEXT))
            .child(self.title.clone())
    }
}
