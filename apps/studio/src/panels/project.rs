//! The Project panel: a preview of the selected item and the folder tree of
//! footage, images, solids and compositions. Items drag onto the timeline as layers.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AppContext as _, Context, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};
use studio_core::edit::can_add_item;
use studio_core::model::{Id, ProjectItem, ProjectItemKind};

use super::chrome::{icon_button, label_swatch, panel};
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, ProjectItemDrag, Workspace};

fn kind_icon(kind: ProjectItemKind) -> IconName {
    match kind {
        ProjectItemKind::Folder => IconName::Folder,
        ProjectItemKind::Composition => IconName::Clapperboard,
        ProjectItemKind::Footage => IconName::FileVideoCamera,
        ProjectItemKind::Image => IconName::Image,
        ProjectItemKind::Solid => IconName::Square,
        ProjectItemKind::Audio => IconName::Music,
    }
}

/// Flattens the folder tree into visible rows with their nesting depth. Folders sort first.
fn visible_rows<'a>(items: &'a [ProjectItem], collapsed: &std::collections::HashSet<Id>) -> Vec<(&'a ProjectItem, usize)> {
    fn visit<'a>(
        items: &'a [ProjectItem],
        parent: Option<Id>,
        depth: usize,
        collapsed: &std::collections::HashSet<Id>,
        rows: &mut Vec<(&'a ProjectItem, usize)>,
    ) {
        let mut children: Vec<&ProjectItem> = items.iter().filter(|item| item.parent == parent).collect();
        children.sort_by(|a, b| {
            (b.kind == ProjectItemKind::Folder).cmp(&(a.kind == ProjectItemKind::Folder)).then(a.name.cmp(&b.name))
        });
        for item in children {
            rows.push((item, depth));
            if item.kind == ProjectItemKind::Folder && !collapsed.contains(&item.id) {
                visit(items, Some(item.id), depth + 1, collapsed, rows);
            }
        }
    }
    let mut rows = Vec::new();
    visit(items, None, 0, collapsed, &mut rows);
    rows
}

/// What follows the pointer while a project item is dragged.
pub struct DragPreview {
    label: String,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(px(3.0))
            .bg(hsla(color::ROW_SELECTED))
            .border_1()
            .border_color(hsla(color::ACCENT))
            .text_size(px(size::TEXT))
            .text_color(hsla(color::TEXT))
            .child(self.label.clone())
    }
}

/// The Type column: the item kind, plus proxy status for footage ("Proxy 42%", "Proxy").
fn type_label(item: &ProjectItem, workspace: &Workspace) -> String {
    let status = item.path.as_ref().filter(|_| item.kind == ProjectItemKind::Footage).and_then(|path| workspace.render.proxies.status(path));
    match status {
        Some(studio_media::ProxyStatus::Queued) => "Proxy queued".to_owned(),
        Some(studio_media::ProxyStatus::Building(progress)) => format!("Proxy {:.0}%", progress * 100.0),
        Some(studio_media::ProxyStatus::Ready(_)) => "Footage · Proxy".to_owned(),
        Some(studio_media::ProxyStatus::Failed(_)) => "Proxy failed".to_owned(),
        None => item.kind.label().to_owned(),
    }
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let project = &workspace.state.project;
    let composition = workspace.state.composition();
    let selected = workspace.state.selected_item.and_then(|id| project.item(id));

    let preview = div().h(px(100.0)).flex_none().flex().gap_3().p_2p5().children(selected.map(|item| {
        div()
            .flex()
            .gap_3()
            .child(
                div()
                    .w(px(140.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(2.0))
                    .bg(hsla(color::INSET))
                    .child(Icon::new(kind_icon(item.kind)).size(px(22.0)).text_color(hsla(color::MUTED))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(item.name.clone())
                    .child(div().text_color(hsla(color::MUTED)).text_size(px(size::SMALL_TEXT)).child(item.kind.label()))
                    .child(div().text_color(hsla(color::MUTED)).text_size(px(size::SMALL_TEXT)).child(item.details())),
            )
    }));

    let column_header = div()
        .h(px(size::ROW_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .border_y_1()
        .border_color(hsla(color::HAIRLINE))
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .child(div().w(px(26.0)).flex().justify_center().child(Icon::new(IconName::Tag).size(px(12.0))))
        .child(div().flex_1().pl_2().child("Name"))
        .child(div().w(px(96.0)).pl_2().child("Type"));

    let rows = visible_rows(&project.items, &workspace.ui.collapsed_folders).into_iter().map(|(item, depth)| {
        let id = item.id;
        let kind = item.kind;
        let is_folder = kind == ProjectItemKind::Folder;
        let is_open = !workspace.ui.collapsed_folders.contains(&id);
        let is_selected = workspace.state.selected_item == Some(id);
        let twirl = if is_folder {
            Icon::new(if is_open { IconName::ChevronDown } else { IconName::ChevronRight })
                .size(px(12.0))
                .text_color(hsla(color::MUTED))
                .into_any_element()
        } else {
            div().w(px(12.0)).into_any_element()
        };

        let mut row = div()
            .id(("project-item", id.0 as usize))
            .h(px(size::ROW_HEIGHT))
            .flex()
            .items_center()
            .when(is_selected, |row| row.bg(hsla(color::ROW_SELECTED)))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                this.state.selected_item = Some(id);
                // Double-click (or Enter) opens folders and compositions.
                let opens = event.click_count() >= 2 || event.is_keyboard();
                if opens && is_folder && !this.ui.collapsed_folders.remove(&id) {
                    this.ui.collapsed_folders.insert(id);
                }
                if opens && kind == ProjectItemKind::Composition {
                    this.state.open_composition(id);
                }
                cx.notify();
            }))
            .child(div().w(px(26.0)).flex().justify_center().child(label_swatch(item.label.color().to_rgba_u32())))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .pl(px(8.0 + depth as f32 * 14.0))
                    .child(twirl)
                    .child(Icon::new(kind_icon(item.kind)).size(px(14.0)).text_color(hsla(color::MUTED)))
                    .child(div().truncate().child(item.name.clone())),
            )
            .child(div().w(px(96.0)).pl_2().text_color(hsla(color::MUTED)).child(type_label(item, workspace)));

        if can_add_item(item, composition) {
            let drag = ProjectItemDrag { item: id, name: item.name.clone() };
            row = row.on_drag(drag, |drag, _offset, _window, cx| cx.new(|_| DragPreview { label: drag.name.clone() }));
        }
        row.test_support()
    });

    let footer = div()
        .flex()
        .items_center()
        .gap_1()
        .w_full()
        .child(icon_button("import", IconName::FileInput, "Import Images and Video (⌘I)", false, cx.listener(|this, _, window, cx| {
            this.import(window, cx);
        })))
        .child(icon_button("new-folder", IconName::FolderPlus, "New Folder", false, cx.listener(|this, _, _, cx| {
            this.state.new_folder();
            cx.notify();
        })))
        .child(icon_button("new-comp", IconName::Clapperboard, "New Composition (⌘N)", false, cx.listener(|this, _, window, cx| {
            super::composition_settings::open(this, true, window, cx);
        })))
        .child(div().flex_1())
        .child(icon_button("delete-item", IconName::Trash, "Delete (⌫)", false, |_, window, cx| {
            window.dispatch_action(Box::new(crate::workspace::Delete), cx);
        }));

    panel(
        PanelId::Project,
        &["Project"],
        Some(footer.into_any_element()),
        div()
            .id("project-drop")
            .size_full()
            .flex()
            .flex_col()
            // Files dragged in from the Finder are imported (linked where they are on disk).
            .drag_over::<gpui_kit::ExternalPaths>(|style, _, _, _| style.bg(hsla(color::HOVER)))
            .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, window, cx| {
                this.import_paths(paths.paths().to_vec(), window, cx);
            }))
            .child(preview)
            .child(column_header)
            .child(div().id("project-items").flex_1().min_h_0().overflow_y_scroll().children(rows).test_support()),
        workspace,
        cx,
    )
}
