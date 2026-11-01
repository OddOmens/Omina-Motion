//! The editor's side of libraries: editing brands, colors and text styles, using them on the
//! selected layers (linked), and keeping the open project in step when they change. Saving
//! them to disk is `library_store`'s job; this only marks them as needing a save.

use studio_core::library::{self, Libraries};
use studio_core::model::{Id, LayerContent, LinkId, Rgba};

use super::{ColorTarget, EditorState, PaintSlot};

/// What a library change means for saving.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LibrarySave {
    #[default]
    Saved,
    /// Changed: save soon.
    Changed,
    /// Something was deleted: back up before saving.
    Deleted,
}

impl EditorState {
    /// Replaces the libraries (loading them at startup) and brings the project in step.
    pub fn set_libraries(&mut self, libraries: Libraries) {
        self.libraries = libraries;
        self.sync_libraries();
    }

    /// Takes the pending save, if any: `Some(true)` if the save should back up first.
    pub fn take_library_save(&mut self) -> Option<bool> {
        match std::mem::take(&mut self.library_save) {
            LibrarySave::Saved => None,
            LibrarySave::Changed => Some(false),
            LibrarySave::Deleted => Some(true),
        }
    }

    fn library_changed(&mut self) {
        if self.library_save == LibrarySave::Saved {
            self.library_save = LibrarySave::Changed;
        }
        self.sync_libraries();
    }

    /// Updates every linked color and text style in the project. This follows a library edit
    /// rather than being one, so it is not an undo step of its own.
    fn sync_libraries(&mut self) {
        library::sync_project(&mut self.project, &self.libraries);
        let libraries = &self.libraries;
        self.history.absorb(|project| {
            library::sync_project(project, libraries);
        });
    }

    pub fn add_brand(&mut self, name: &str) -> LinkId {
        let id = self.libraries.add_brand(name);
        self.library_changed();
        id
    }

    /// Adds `color` to a brand's colors, unlinked from anything else.
    pub fn add_library_color(&mut self, brand: LinkId, color: Rgba) -> Option<LinkId> {
        let count = self.libraries.brand(brand)?.colors.len();
        let id = self.libraries.add_color(brand, &format!("Color {}", count + 1), color)?;
        self.library_changed();
        Some(id)
    }

    /// Saves a text layer's style in a brand, and links the layer to it.
    pub fn add_text_style_from(&mut self, brand: LinkId, layer: Id) -> Option<LinkId> {
        let style = self.text_of(layer)?.1.clone();
        let count = self.libraries.brand(brand)?.text_styles.len();
        let id = self.libraries.add_text_style(brand, &format!("Text Style {}", count + 1), &style)?;
        self.apply_text_style(id, &[layer]);
        self.library_changed();
        Some(id)
    }

    pub fn rename_library_item(&mut self, id: LinkId, name: &str) {
        if self.libraries.rename(id, name) {
            self.library_changed();
        }
    }

    /// Deletes a brand (with everything in it), color or text style. What used it keeps its look.
    pub fn delete_library_item(&mut self, id: LinkId) {
        if self.libraries.delete(id) {
            self.library_save = LibrarySave::Deleted;
        }
    }

    /// Changes a library color; everything linked to it follows.
    pub fn set_library_color(&mut self, id: LinkId, color: Rgba) {
        if self.libraries.set_color(id, color) {
            self.library_changed();
        }
    }

    /// Uses a library color on the selected layers, linked: a shape's fill, a text layer's
    /// fill, or a solid's color. Returns how many layers took it.
    pub fn apply_library_color(&mut self, id: LinkId) -> usize {
        let Some((_, item)) = self.libraries.color(id) else { return 0 };
        let color = item.color.linked(Some(id));
        let layers = self.selected_layers.clone();
        let mut applied = 0;
        for layer in layers {
            let Some(content) = self.composition().layer(layer).map(|layer| layer.content.clone()) else { continue };
            let target = match content {
                LayerContent::Shape { .. } => ColorTarget::Shape { layer, slot: PaintSlot::Fill, stop: 0 },
                LayerContent::Text { .. } => ColorTarget::TextFill(layer),
                LayerContent::Solid { .. } => {
                    if let Some(LayerContent::Solid { color: solid }) = self.composition_mut().layer_mut(layer).map(|layer| &mut layer.content) {
                        *solid = color;
                    }
                    applied += 1;
                    continue;
                }
                _ => continue,
            };
            self.set_color(target, color);
            applied += 1;
        }
        applied
    }

    /// Uses a library text style on `layers` (the text ones), linked.
    pub fn apply_text_style(&mut self, id: LinkId, layers: &[Id]) -> usize {
        let Some((_, template)) = self.libraries.text_style(id) else { return 0 };
        let template = template.clone();
        let mut applied = 0;
        for &layer in layers {
            if self.text_style_of(layer).is_some() {
                studio_core::edit::edit_text(self.composition_mut(), layer, |_, style| template.apply_to(style));
                applied += 1;
            }
        }
        applied
    }

    /// The library color a picker target is linked to, if any, with its brand's name.
    pub fn linked_color(&self, target: ColorTarget) -> Option<(String, String)> {
        let link = self.color_of(target)?.link?;
        let (brand, color) = self.libraries.color(link)?;
        Some((brand.name.clone(), color.name.clone()))
    }

    /// Every distinct color used in the open project, for the picker's "In this project" row.
    pub fn project_colors(&self) -> Vec<Rgba> {
        let mut project = self.project.clone();
        let mut colors: Vec<Rgba> = Vec::new();
        project.for_each_color_mut(|color| {
            if !colors.iter().any(|seen| seen.same_look(*color)) {
                colors.push(color.linked(None));
            }
        });
        colors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TextSetting;

    #[test]
    fn a_library_color_used_on_a_layer_follows_edits_without_an_undo_step() {
        let mut state = EditorState::new();
        let brand = state.add_brand("Brand A");
        let primary = state.add_library_color(brand, Rgba::hex(0xff0000)).unwrap();
        let title = state.composition().layers[0].id;
        state.select_layer(title, false);
        assert_eq!(state.apply_library_color(primary), 1);
        state.record_history(false);
        assert_eq!(state.text_of(title).unwrap().1.fill.unwrap().link, Some(primary));

        state.set_library_color(primary, Rgba::hex(0x0000ff));
        state.record_history(false);
        assert!(state.text_of(title).unwrap().1.fill.unwrap().same_look(Rgba::hex(0x0000ff)), "the title followed");
        // Undo takes back the user's edit (using the color), not the library's.
        state.undo();
        assert!(state.text_of(title).unwrap().1.fill.unwrap().link.is_none());
        assert_eq!(state.take_library_save(), Some(false));
        assert_eq!(state.take_library_save(), None);
    }

    #[test]
    fn text_styles_link_until_changed_by_hand() {
        let mut state = EditorState::new();
        let brand = state.add_brand("Brand A");
        let title = state.composition().layers[0].id;
        let style = state.add_text_style_from(brand, title).unwrap();
        assert_eq!(state.text_of(title).unwrap().1.template, Some(style));
        state.set_text_setting(title, TextSetting::FontSize, 33.0);
        assert_eq!(state.text_of(title).unwrap().1.template, None, "a hand edit unlinks it");
    }

    #[test]
    fn libraries_loaded_at_startup_update_linked_colors_without_an_undo_step() {
        let mut libraries = Libraries::default();
        let brand = libraries.add_brand("Brand A");
        let primary = libraries.add_color(brand, "Primary", Rgba::hex(0x00ff00)).unwrap();
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        // A project saved while Primary was still red.
        state.set_text_fill(title, Some(Rgba::hex(0xff0000).linked(Some(primary))));
        state.record_history(false);
        let steps_before = state.can_undo();
        state.set_libraries(libraries);
        state.record_history(false);
        assert!(state.text_of(title).unwrap().1.fill.unwrap().same_look(Rgba::hex(0x00ff00)));
        state.undo();
        assert!(steps_before && state.text_of(title).unwrap().1.fill.unwrap().link.is_none(), "undo skipped the library update");
    }

    #[test]
    fn deleting_asks_for_a_backup() {
        let mut state = EditorState::new();
        let brand = state.add_brand("Brand A");
        state.take_library_save();
        state.delete_library_item(brand);
        assert_eq!(state.take_library_save(), Some(true));
        assert!(state.libraries.brands.is_empty());
    }
}
