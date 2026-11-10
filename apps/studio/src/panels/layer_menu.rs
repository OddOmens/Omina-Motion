//! A layer's right-click menu, the same in the timeline (on a clip) and in the viewer (on the
//! picture): rename, duplicate, pre-compose or ungroup, morph one shape into another or turn
//! it into a path, flip, reset the transform, stabilize a video, paste attributes, split (at the
//! playhead, or a video at its scene cuts), freeze a frame, delete, close gaps and export (a
//! caption layer also as subtitles). It acts on the layer right-clicked, or on the whole selection
//! when that layer is part of it, as in After Effects. The menu reads the editor when it opens, so what
//! it offers matches the layers it will act on.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Context, WeakEntity, Window};
use studio_core::model::Id;
use studio_core::subtitles::SubtitleFormat;

use super::analysis::SceneCutAction;
use crate::features::Feature;
use crate::workspace::{EditTarget, Workspace};

/// Finds the scene cuts in the selected clip (the one the menu was opened on).
fn scene_cuts(workspace: &mut Workspace, action: SceneCutAction, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some(&layer) = workspace.state.selected_layers.first() {
        super::analysis::scene_cuts(workspace, layer, action, window, cx);
    }
}

/// Saves the selected caption layer's captions as a subtitle file.
fn export_subtitles(workspace: &mut Workspace, format: SubtitleFormat, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some(&layer) = workspace.state.selected_layers.first() {
        super::subtitles::export(workspace, layer, format, window, cx);
    }
}

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
        // A video clip at normal speed can be split at its scene cuts, when no other analysis runs.
        let video_clip = single.and_then(|layer| workspace.state.composition().layer(layer)).filter(|layer| matches!(layer.content, studio_core::model::LayerContent::Footage { .. }) && layer.time.hold.is_none());
        let can_find_cuts = video_clip.is_some_and(|layer| layer.time.is_normal()) && workspace.ui.analysis_job.is_none();
        let is_captions = single.and_then(|layer| workspace.state.composition().layer(layer)).is_some_and(|layer| matches!(layer.content, studio_core::model::LayerContent::Captions { .. }));
        let depended_on = single.is_some_and(|layer| studio_core::bake::is_depended_on(&workspace.state.project, workspace.state.active_comp, layer));
        let some = !layers.is_empty();
        let can_freeze = single.is_some_and(|layer| workspace.state.can_freeze_at_playhead(layer));
        // Stabilize is for one video layer; once on, the menu offers to take it off instead.
        let footage = single.and_then(|layer| workspace.state.composition().layer(layer)).filter(|layer| matches!(layer.content, studio_core::model::LayerContent::Footage { .. }));
        let stabilized = footage.is_some_and(|layer| layer.stabilization.is_some());
        let analysing = workspace.ui.analysis_job.is_some();
        // Morph To works on two shapes, the first morphing into the second (see `panels::morph`).
        let can_morph = layers.len() == 2 && studio_core::morph::can_morph(workspace.state.composition(), layers[0], layers[1]).is_ok();
        let is_box_shape = single.and_then(|layer| workspace.state.composition().layer(layer)).is_some_and(|layer| {
            matches!(&layer.content, studio_core::model::LayerContent::Shape { style, .. } if style.kind != studio_core::model::ShapeKind::Path)
        });

        let can_paste_attributes = workspace.state.attributes_source().is_some();
        let can_export_layer = single.is_some() && has_picture;
        let (is_video, is_footage) = (video_clip.is_some(), footage.is_some());

        // Features turned off in Settings ▸ Features are left out of the menu.
        let on = |feature: Feature| workspace.has(feature);
        let (paste_attributes, precompose, morph) = (on(Feature::PasteAttributes), on(Feature::Precompose), can_morph && on(Feature::Morph));
        let (freeze, find_cuts, close_gaps) = (is_video && on(Feature::FreezeFrame), is_video && on(Feature::SceneCuts), on(Feature::CloseGaps));
        let stabilize = is_footage && on(Feature::Stabilize);
        let export_layer = can_export_layer && on(Feature::Export);
        let caption_files = is_captions && on(Feature::CaptionFiles);
        let structure = precompose || nested.is_some() || morph || is_box_shape;

        // The main list: naming, copying and structure, then the everyday cuts. Items that can't
        // apply to this kind of layer are left out; ones waiting on something (a copy to paste,
        // a job to finish) are shown greyed so they can still be found.
        let mut menu = menu;
        if single.is_some() {
            menu = menu.item(item("Rename…", &layers, true, &weak, |this, window, cx| {
                let Some(&layer) = this.state.selected_layers.first() else { return };
                let name = this.state.composition().layer(layer).map(|layer| layer.name.clone()).unwrap_or_default();
                this.begin_edit(EditTarget::LayerName(layer), name, window, cx);
            }));
        }
        menu = menu
            .item(item("Duplicate", &layers, some, &weak, |this, _, _| {
                this.state.duplicate_selection();
            }))
            .when(paste_attributes, |menu| menu.item(item("Paste Attributes…", &layers, can_paste_attributes, &weak, super::paste_attributes::open)))
            .when(structure, |menu| menu.separator())
            .when(precompose, |menu| menu.item(item("Pre-compose…", &layers, some, &weak, super::precompose::open)));
        if let Some(inner) = nested {
            let weak = weak.clone();
            // An SVG's composition breaks into its shapes; any composition ungroups the same way.
            let is_svg = workspace.state.project.item(inner).is_some_and(|item| item.name.to_lowercase().ends_with(".svg"));
            let label = if is_svg { "Break into Shapes" } else { "Ungroup Composition" };
            menu = menu.item(item(label, &layers, true, &weak, |this, window, cx| this.ungroup_selected(window, cx))).item(PopupMenuItem::new("Open Composition").on_click(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.state.open_composition(inner);
                    cx.notify();
                })
                .ok();
            }));
        }
        if morph {
            menu = menu.item(item("Morph To…", &layers, true, &weak, super::morph::open));
        }
        if is_box_shape {
            menu = menu.item(item("Convert to Path", &layers, true, &weak, |this, _, _| {
                if let Some(&layer) = this.state.selected_layers.first() {
                    this.state.convert_to_path(layer);
                }
            }));
        }
        menu = menu
            .separator()
            .item(item("Split at Playhead", &layers, some, &weak, |this, window, cx| this.split_layer(window, cx)))
            .item(item("Delete", &layers, some, &weak, |this, _, _| {
                this.state.delete_selection();
            }))
            .separator();

        // Time: freeze frames, scene cuts and gaps.
        if freeze || find_cuts || close_gaps {
            let (time_layers, time_weak) = (layers.clone(), weak.clone());
            menu = menu.submenu("Time", window, cx, move |menu, _, _| {
                let (layers, weak) = (&time_layers, &time_weak);
                menu.when(freeze, |menu| {
                    menu.item(item("Insert Freeze Frame", layers, can_freeze, weak, |this, _, _| {
                        if let Some(&layer) = this.state.selected_layers.first() {
                            this.state.insert_freeze_frame(layer);
                        }
                    }))
                })
                .when(find_cuts, |menu| {
                    menu.item(item("Split at Scene Cuts", layers, can_find_cuts, weak, |this, window, cx| scene_cuts(this, SceneCutAction::Split, window, cx)))
                        .item(item("Mark Scene Cuts", layers, can_find_cuts, weak, |this, window, cx| scene_cuts(this, SceneCutAction::Mark, window, cx)))
                })
                .when(close_gaps, |menu| {
                    menu.item(item("Close Gaps on Track", layers, true, weak, |this, window, cx| {
                        if this.state.close_gaps() == 0 {
                            window.push_notification("There are no gaps to close.", cx);
                        }
                    }))
                })
            });
        }

        // Transform: flips, reset and (for video) stabilizing.
        let (move_layers, move_weak) = (layers.clone(), weak.clone());
        menu = menu.submenu("Transform", window, cx, move |menu, _, _| {
            let (layers, weak) = (&move_layers, &move_weak);
            menu.item(item("Flip Horizontal", layers, true, weak, |this, _, _| {
                this.state.flip_selection(true);
            }))
            .item(item("Flip Vertical", layers, true, weak, |this, _, _| {
                this.state.flip_selection(false);
            }))
            .item(item("Reset Transform", layers, true, weak, |this, _, _| this.state.reset_selection_transform()))
            .when(stabilize, |menu| {
                menu.separator().item(if stabilized {
                    item("Remove Stabilization", layers, true, weak, |this, _, _| {
                        if let Some(&layer) = this.state.selected_layers.first() {
                            this.state.remove_stabilization(layer);
                        }
                    })
                } else {
                    item("Stabilize", layers, !analysing, weak, |this, window, cx| {
                        if let Some(&layer) = this.state.selected_layers.first() {
                            super::analysis::stabilize(this, layer, window, cx);
                        }
                    })
                })
            })
        });

        // Export: the layer alone, and a caption layer as subtitles.
        if export_layer || caption_files {
            let (export_layers, export_weak) = (layers.clone(), weak.clone());
            menu = menu.submenu("Export", window, cx, move |menu, _, _| {
                let (layers, weak) = (&export_layers, &export_weak);
                menu.when(export_layer, |menu| {
                    menu.item(item("Quick Export Layer", layers, !busy, weak, |this, window, cx| {
                        if let Some(&layer) = this.state.selected_layers.first() {
                            super::export::quick_export_layer(this, layer, false, window, cx);
                        }
                    }))
                    .item(item("Export and Replace Layer", layers, !busy && !depended_on, weak, |this, window, cx| {
                        if let Some(&layer) = this.state.selected_layers.first() {
                            super::export::quick_export_layer(this, layer, true, window, cx);
                        }
                    }))
                })
                .when(caption_files, |menu| {
                    menu.item(item("Export Captions as SRT…", layers, true, weak, |this, window, cx| export_subtitles(this, SubtitleFormat::Srt, window, cx)))
                        .item(item("Export Captions as WebVTT…", layers, true, weak, |this, window, cx| export_subtitles(this, SubtitleFormat::WebVtt, window, cx)))
                })
            });
        }
        menu
    }
}
