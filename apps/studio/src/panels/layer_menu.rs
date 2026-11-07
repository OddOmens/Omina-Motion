//! A layer's right-click menu, the same in the timeline (on a clip) and in the viewer (on the
//! picture): rename, duplicate, pre-compose or ungroup, morph one shape into another or turn
//! it into a path, flip, reset the transform, paste attributes, split, freeze a frame, delete, close gaps and export. It acts on the layer right-clicked, or on the whole selection when that
//! layer is part of it, as in After Effects. The menu reads the editor when it opens, so what
//! it offers matches the layers it will act on.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{Context, WeakEntity, Window};
use studio_core::model::Id;

use crate::workspace::{EditTarget, Workspace};

/// The layers a menu on `clicked` acts on: the selection if `clicked` is in it (or nothing
/// particular was clicked), otherwise `clicked` alone. Locked layers are left out.
fn targets(workspace: &Workspace, clicked: Option<Id>) -> Vec<Id> {
    let state = &workspace.state;
    let layers = match clicked {
        Some(layer) if !state.selected_layers.contains(&layer) => vec![layer],
        _ => state.selected_layers.clone(),
    };
    layers.into_iter().filter(|&id| state.composition().layer(id).is_some_and(|layer| !layer.switches.locked)).collect()
}

/// An item that first selects `layers`, then runs `act` on the editor.
fn item(label: &'static str, layers: &[Id], enabled: bool, weak: &WeakEntity<Workspace>, act: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)) -> PopupMenuItem {
    let (layers, weak) = (layers.to_vec(), weak.clone());
    PopupMenuItem::new(label).disabled(!enabled).on_click(move |_, window, cx| {
        let layers = layers.clone();
        weak.update(cx, |this, cx| {
            this.state.selected_layers = layers;
            act(this, window, cx);
            cx.notify();
        })
        .ok();
    })
}

/// The menu for `clicked` (a timeline clip), or with `None`, for the layer under the pointer
/// in the viewer when the menu opens (or the selection, over empty space).
pub fn build(clicked: Option<Id>, weak: WeakEntity<Workspace>) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, window, cx| {
        let Some(workspace) = weak.upgrade() else { return menu };
        let workspace = workspace.read(cx);
        let clicked = clicked.or_else(|| super::viewer::layer_under(workspace, window.mouse_position()));
        let layers = targets(workspace, clicked);
        if layers.is_empty() {
            return menu.label("No layer here");
        }
        let single = (layers.len() == 1).then(|| layers[0]);
        let nested = single.and_then(|layer| workspace.state.nested_composition(layer));
        let has_picture = single.and_then(|layer| workspace.state.composition().layer(layer)).is_some_and(|layer| layer.content.has_picture());
        let busy = workspace.ui.export_job.is_some();
        let depended_on = single.is_some_and(|layer| studio_core::bake::is_depended_on(&workspace.state.project, workspace.state.active_comp, layer));
        let some = !layers.is_empty();
        let can_freeze = single.is_some_and(|layer| workspace.state.can_freeze_at_playhead(layer));
        // Morph To works on two shapes, the first morphing into the second (see `panels::morph`).
        let can_morph = layers.len() == 2 && studio_core::morph::can_morph(workspace.state.composition(), layers[0], layers[1]).is_ok();
        let is_box_shape = single.and_then(|layer| workspace.state.composition().layer(layer)).is_some_and(|layer| {
            matches!(&layer.content, studio_core::model::LayerContent::Shape { style, .. } if style.kind != studio_core::model::ShapeKind::Path)
        });

        let can_paste_attributes = workspace.state.attributes_source().is_some();
        let mut menu = menu.item(item("Rename…", &layers, single.is_some(), &weak, |this, window, cx| {
            let Some(&layer) = this.state.selected_layers.first() else { return };
            let name = this.state.composition().layer(layer).map(|layer| layer.name.clone()).unwrap_or_default();
            this.begin_edit(EditTarget::LayerName(layer), name, window, cx);
        }));
        menu = menu
            .item(item("Duplicate", &layers, some, &weak, |this, _, _| {
                this.state.duplicate_selection();
            }))
            .item(item("Paste Attributes…", &layers, can_paste_attributes, &weak, super::paste_attributes::open))
            .separator()
            .item(item("Pre-compose…", &layers, some, &weak, super::precompose::open))
            .item(item("Ungroup Composition", &layers, nested.is_some(), &weak, |this, window, cx| this.ungroup_selected(window, cx)))
            .separator()
            .item(item("Morph To…", &layers, can_morph, &weak, super::morph::open))
            .item(item("Convert to Path", &layers, is_box_shape, &weak, |this, _, _| {
                if let Some(&layer) = this.state.selected_layers.first() {
                    this.state.convert_to_path(layer);
                }
            }));
        if let Some(inner) = nested {
            let weak = weak.clone();
            menu = menu.item(PopupMenuItem::new("Open Composition").on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.state.open_composition(inner);
                    cx.notify();
                })
                .ok();
            }));
        }
        menu.separator()
            .item(item("Flip Horizontal", &layers, some, &weak, |this, _, _| {
                this.state.flip_selection(true);
            }))
            .item(item("Flip Vertical", &layers, some, &weak, |this, _, _| {
                this.state.flip_selection(false);
            }))
            .item(item("Reset Transform", &layers, some, &weak, |this, _, _| this.state.reset_selection_transform()))
            .separator()
            .item(item("Split at Playhead", &layers, some, &weak, |this, window, cx| this.split_layer(window, cx)))
            .item(item("Insert Freeze Frame", &layers, can_freeze, &weak, |this, _, _| {
                if let Some(&layer) = this.state.selected_layers.first() {
                    this.state.insert_freeze_frame(layer);
                }
            }))
            .item(item("Delete", &layers, some, &weak, |this, _, _| {
                this.state.delete_selection();
            }))
            .item(item("Close Gaps on Track", &layers, some, &weak, |this, window, cx| {
                if this.state.close_gaps() == 0 {
                    window.push_notification("There are no gaps to close.", cx);
                }
            }))
            .separator()
            .item(item("Quick Export Layer", &layers, single.is_some() && has_picture && !busy, &weak, |this, window, cx| {
                if let Some(&layer) = this.state.selected_layers.first() {
                    super::export::quick_export_layer(this, layer, false, window, cx);
                }
            }))
            .item(item("Export and Replace Layer", &layers, single.is_some() && has_picture && !busy && !depended_on, &weak, |this, window, cx| {
                if let Some(&layer) = this.state.selected_layers.first() {
                    super::export::quick_export_layer(this, layer, true, window, cx);
                }
            }))
    }
}
