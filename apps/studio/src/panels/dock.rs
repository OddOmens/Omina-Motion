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
    AnyElement, AnyView, App, AppContext as _, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, StyleRefinement, Styled as _,
    TestSupportExt as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use crate::panels::views::PanelView;
use crate::theme::{color, hsla, size};
use crate::workspace::{LeftPanel, PanelId, Workspace};

/// Written with every saved layout. A saved layout from a different version is set aside for
/// the standard one, so a change to what the saved form means can't scramble someone's screen.
pub const LAYOUT_VERSION: usize = 1;

/// Every panel that can be moved, in the order the standard layout lists them.
pub const DOCKABLE: [PanelId; 6] = [
    PanelId::Side,
    PanelId::Composition,
    PanelId::Timeline,
    PanelId::Nodes,
    PanelId::Properties,
    PanelId::Align,
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
            PanelId::Nodes => "Nodes",
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
            }
            .into(),
            other => other.name().into(),
        }
    }
}

/// The standard arrangement, the one Studio starts with: the side panel beside the viewer, the timeline (with the Nodes panel, while it is open) under them, and Properties
/// above Align down the right.
pub fn default_layout(view: impl Fn(PanelId) -> Entity<PanelView>) -> DockLayout {
    let tabs = |ids: &[PanelId]| ids.iter().fold(DockLayout::tabs(), |tabs, id| tabs.panel(view(*id)));
    let top = DockLayout::h_split().child(tabs(&[PanelId::Side]), Some(px(300.0))).child(tabs(&[PanelId::Composition]), None);
    let bottom = DockLayout::h_split().child(tabs(&[PanelId::Timeline]), None).child(tabs(&[PanelId::Nodes]), Some(px(560.0)));
    let main = DockLayout::v_split().child(top, Some(px(520.0))).child(bottom, None);
    let right = DockLayout::v_split().child(tabs(&[PanelId::Properties]), None).child(tabs(&[PanelId::Align]), Some(px(ALIGN_HEIGHT)));
    DockLayout::h_split().child(main, None).child(right, Some(px(430.0)))
}

/// Rebuilds a saved layout with Studio's own panel views. Panels the saved layout doesn't
/// mention (ones added in a later version, say) join the side panel's group, so none is ever
/// lost; names this version doesn't know are skipped. In a layout saved when the side panel's
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
    let missing: Vec<PanelId> = DOCKABLE.into_iter().filter(|id| !placed.contains(id)).collect();
    if !missing.is_empty() && !add_to_side_group(&mut center, &missing) {
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

/// Appends `missing` to the first tab group holding the side panel. False if there is none.
fn add_to_side_group(state: &mut PanelState, missing: &[PanelId]) -> bool {
    let holds_side_panel = state.children.iter().any(|child| PanelId::from_name(&child.panel_name) == Some(PanelId::Side));
    if matches!(state.info, PanelInfo::Tabs { .. }) && holds_side_panel {
        state.children.extend(missing.iter().map(|id| PanelState::new(id.name())));
        return true;
    }
    state.children.iter_mut().any(|child| add_to_side_group(child, missing))
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
            let children: Vec<(DockLayout, Option<Pixels>)> = state
                .children
                .iter()
                .enumerate()
                .filter_map(|(index, child)| {
                    let size = sizes.get(index).copied().filter(|size| *size > px(0.0));
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

/// Studio's look for the dock: a tab bar on every panel group, Studio's panel colors, and an
/// accent wash over the spot a dragged panel would land in.
pub struct StudioSkin {
    workspace: WeakEntity<Workspace>,
}

impl StudioSkin {
    pub fn new(workspace: WeakEntity<Workspace>) -> Rc<Self> {
        Rc::new(Self { workspace })
    }
}

impl DockAreaRenderer for StudioSkin {
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
        div()
            .id(("dock-group", group.node().as_u64()))
            .bg(hsla(color::PANEL))
            .rounded(px(size::PANEL_RADIUS))
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
                .px_1p5()
                .flex()
                .items_center()
                .cursor_grab()
                .text_color(hsla(if selected { color::TEXT } else { color::MUTED }))
                .when(selected, |tab| tab.border_b_2().border_color(hsla(color::TEXT)))
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
                .child(div().min_w_0().truncate().child(title))
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
            .px_1()
            .overflow_hidden()
            .border_b_1()
            .border_color(hsla(color::HAIRLINE))
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
