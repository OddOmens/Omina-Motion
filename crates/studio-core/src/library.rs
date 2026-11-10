//! Libraries, as in Figma: brands (or clients, or any folder you like) that hold shared colors
//! and text styles, plus the user's own effect presets (any mix of effects, color corrections
//! and sound effects with their settings and keyframes, saved from a layer and applied to
//! others). They are global, shared by every project on the computer, and stored by the app
//! outside any project file.
//!
//! A color or text style used from a library stays linked: the project remembers which library
//! item it came from (`Rgba::link`, `TextStyle::template`), and `sync_project` brings every
//! linked use up to date when the library changes, or when a project is opened.

use crate::model::{Effect, Id, Layer, LinkId, Project, Rgba, TextStyle};

/// The longest name a library item can have, in characters.
pub const MAX_NAME: usize = 80;

/// All brands, and the next id to hand out. Ids are never reused, so a link from an old
/// project can't point at a different item made later.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Libraries {
    pub brands: Vec<Brand>,
    /// The user's effect presets, shown under My Presets in the Effects panel.
    #[serde(default)]
    pub presets: Vec<EffectPreset>,
    next_id: u64,
}

/// A user-made effect preset: effects with their settings (and keyframes, timed from the start
/// of the layer they were saved from), applied together.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EffectPreset {
    pub id: LinkId,
    pub name: String,
    pub effects: Vec<Effect>,
}

impl EffectPreset {
    /// "3 effects", for the Effects panel.
    pub fn summary(&self) -> String {
        match self.effects.len() {
            1 => "1 effect".to_owned(),
            count => format!("{count} effects"),
        }
    }
}

/// The most presets the library keeps, and the most effects one preset holds.
pub const MAX_PRESETS: usize = 500;
pub const MAX_PRESET_EFFECTS: usize = 64;

/// One brand (or client, or folder): its colors and text styles.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Brand {
    pub id: LinkId,
    pub name: String,
    pub colors: Vec<LibraryColor>,
    pub text_styles: Vec<TextTemplate>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LibraryColor {
    pub id: LinkId,
    pub name: String,
    /// The color itself (never linked).
    pub color: Rgba,
}

/// A saved text style: "Header: Inter Bold 48".
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextTemplate {
    pub id: LinkId,
    pub name: String,
    pub font_family: String,
    pub font_weight: u16,
    pub font_size: f32,
    pub line_height: f32,
    pub tracking: f32,
    pub fill: Option<Rgba>,
}

impl TextTemplate {
    /// A short description, such as "Inter 700 · 48 px".
    pub fn summary(&self) -> String {
        format!("{} {} · {} px", self.font_family, self.font_weight, self.font_size.round())
    }

    /// Applies the template to `style`, linking it.
    pub fn apply_to(&self, style: &mut TextStyle) {
        style.font_family = self.font_family.clone();
        style.font_weight = self.font_weight;
        style.font_size = self.font_size;
        style.line_height = self.line_height;
        style.tracking = self.tracking;
        if let Some(fill) = self.fill {
            style.fill = Some(fill);
        }
        style.template = Some(self.id);
    }
}

/// Keeps a name usable: trimmed, not empty, and at most `MAX_NAME` characters.
fn clean_name(name: &str, fallback: &str) -> String {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    if name.is_empty() { fallback.to_owned() } else { name }
}

impl Libraries {
    fn allocate(&mut self) -> LinkId {
        self.next_id += 1;
        LinkId(self.next_id)
    }

    pub fn brand(&self, id: LinkId) -> Option<&Brand> {
        self.brands.iter().find(|brand| brand.id == id)
    }

    fn brand_mut(&mut self, id: LinkId) -> Option<&mut Brand> {
        self.brands.iter_mut().find(|brand| brand.id == id)
    }

    /// The library color `id`, with the brand it is in.
    pub fn color(&self, id: LinkId) -> Option<(&Brand, &LibraryColor)> {
        self.brands.iter().find_map(|brand| brand.colors.iter().find(|color| color.id == id).map(|color| (brand, color)))
    }

    /// The text style `id`, with the brand it is in.
    pub fn text_style(&self, id: LinkId) -> Option<(&Brand, &TextTemplate)> {
        self.brands.iter().find_map(|brand| brand.text_styles.iter().find(|style| style.id == id).map(|style| (brand, style)))
    }

    /// Adds a brand and returns its id.
    pub fn add_brand(&mut self, name: &str) -> LinkId {
        let id = self.allocate();
        let name = clean_name(name, "New Brand");
        self.brands.push(Brand { id, name, colors: Vec::new(), text_styles: Vec::new() });
        id
    }

    /// Adds a color to a brand. Returns its id, or `None` if there is no such brand.
    pub fn add_color(&mut self, brand: LinkId, name: &str, color: Rgba) -> Option<LinkId> {
        self.brand(brand)?;
        let id = self.allocate();
        let name = clean_name(name, &color.to_hex());
        self.brand_mut(brand)?.colors.push(LibraryColor { id, name, color: color.linked(None) });
        Some(id)
    }

    /// Saves `style` as a text style in a brand. Returns its id, or `None` if there is no such brand.
    pub fn add_text_style(&mut self, brand: LinkId, name: &str, style: &TextStyle) -> Option<LinkId> {
        self.brand(brand)?;
        let id = self.allocate();
        let template = TextTemplate {
            id,
            name: clean_name(name, "Text Style"),
            font_family: style.font_family.clone(),
            font_weight: style.font_weight,
            font_size: style.font_size,
            line_height: style.line_height,
            tracking: style.tracking,
            fill: style.fill.map(|fill| fill.linked(None)),
        };
        self.brand_mut(brand)?.text_styles.push(template);
        Some(id)
    }

    /// Changes a library color's value. Returns whether it exists.
    pub fn set_color(&mut self, id: LinkId, color: Rgba) -> bool {
        let found = self.brands.iter_mut().flat_map(|brand| &mut brand.colors).find(|item| item.id == id);
        found.map(|item| item.color = color.linked(None)).is_some()
    }

    /// Replaces a text style's settings with those of `style`, keeping its name. Returns whether it exists.
    pub fn update_text_style(&mut self, id: LinkId, style: &TextStyle) -> bool {
        let found = self.brands.iter_mut().flat_map(|brand| &mut brand.text_styles).find(|item| item.id == id);
        found
            .map(|item| {
                item.font_family = style.font_family.clone();
                item.font_weight = style.font_weight;
                item.font_size = style.font_size;
                item.line_height = style.line_height;
                item.tracking = style.tracking;
                item.fill = style.fill.map(|fill| fill.linked(None));
            })
            .is_some()
    }

    /// Saves `layer`'s effects (all of them, in order, with their settings, keyframes and on/off
    /// state) as a preset called `name`. Returns its id, or `None` when the layer has no effects
    /// or the library is full.
    pub fn add_preset(&mut self, name: &str, layer: &Layer) -> Option<LinkId> {
        if layer.effects.is_empty() || self.presets.len() >= MAX_PRESETS {
            return None;
        }
        let id = self.allocate();
        let effects = layer
            .effects
            .iter()
            .take(MAX_PRESET_EFFECTS)
            .map(|effect| {
                let mut saved = effect.clone();
                saved.id = Id(0);
                // Keyframes count from the layer's start, so they land the same on another layer.
                for property in &mut saved.properties {
                    property.value.shift(-layer.in_frame);
                }
                saved
            })
            .collect();
        self.presets.push(EffectPreset { id, name: clean_name(name, "My Preset"), effects });
        Some(id)
    }

    /// The preset with id `id`.
    pub fn preset(&self, id: LinkId) -> Option<&EffectPreset> {
        self.presets.iter().find(|preset| preset.id == id)
    }

    /// Renames a brand, color, text style or preset. Returns whether it exists.
    pub fn rename(&mut self, id: LinkId, name: &str) -> bool {
        if let Some(preset) = self.presets.iter_mut().find(|preset| preset.id == id) {
            preset.name = clean_name(name, &preset.name);
            return true;
        }
        for brand in &mut self.brands {
            if brand.id == id {
                brand.name = clean_name(name, &brand.name);
                return true;
            }
            if let Some(color) = brand.colors.iter_mut().find(|color| color.id == id) {
                color.name = clean_name(name, &color.name);
                return true;
            }
            if let Some(style) = brand.text_styles.iter_mut().find(|style| style.id == id) {
                style.name = clean_name(name, &style.name);
                return true;
            }
        }
        false
    }

    /// Deletes a brand with everything in it, a single color or text style, or a preset.
    /// Projects that used it keep what they had; they just stop following it.
    pub fn delete(&mut self, id: LinkId) -> bool {
        let before = self.item_count() + self.brands.len();
        self.brands.retain(|brand| brand.id != id);
        self.presets.retain(|preset| preset.id != id);
        for brand in &mut self.brands {
            brand.colors.retain(|color| color.id != id);
            brand.text_styles.retain(|style| style.id != id);
        }
        self.item_count() + self.brands.len() != before
    }

    /// How many colors, text styles and presets there are in all.
    pub fn item_count(&self) -> usize {
        self.brands.iter().map(|brand| brand.colors.len() + brand.text_styles.len()).sum::<usize>() + self.presets.len()
    }

    /// Checks a library read from disk: unique ids below the next one to hand out, and finite
    /// numbers. Fixes what can be fixed (names, the next id) and refuses the rest.
    pub fn validated(mut self) -> Result<Self, String> {
        let mut seen = std::collections::HashSet::new();
        let mut highest = 0;
        for brand in &mut self.brands {
            brand.name = clean_name(&brand.name, "Brand");
            let ids = std::iter::once(brand.id).chain(brand.colors.iter().map(|c| c.id)).chain(brand.text_styles.iter().map(|s| s.id));
            for id in ids.collect::<Vec<_>>() {
                if !seen.insert(id) {
                    return Err(format!("two library items share the id {}", id.0));
                }
                highest = highest.max(id.0);
            }
            for color in &mut brand.colors {
                color.name = clean_name(&color.name, "Color");
                color.color = color.color.linked(None);
            }
            for style in &mut brand.text_styles {
                style.name = clean_name(&style.name, "Text Style");
                let numbers = [style.font_size, style.line_height, style.tracking];
                if !numbers.iter().all(|n| n.is_finite()) || !(1.0..=2000.0).contains(&style.font_size) {
                    return Err(format!("text style \"{}\" has an impossible size", style.name));
                }
            }
        }
        // Presets: effects the catalog no longer has, and numbers that aren't, are dropped;
        // a preset left with no effects goes. Settings are brought into the catalog's ranges.
        self.presets.truncate(MAX_PRESETS);
        for preset in &mut self.presets {
            if !seen.insert(preset.id) {
                return Err(format!("two library items share the id {}", preset.id.0));
            }
            highest = highest.max(preset.id.0);
            preset.name = clean_name(&preset.name, "My Preset");
            preset.effects.truncate(MAX_PRESET_EFFECTS);
            preset.effects.retain(|effect| crate::effects::find(&effect.name).is_some());
            preset.effects.iter_mut().for_each(crate::effects::restore_info);
        }
        self.presets.retain(|preset| !preset.effects.is_empty());
        self.next_id = self.next_id.max(highest);
        Ok(self)
    }
}

/// Adds the preset's effects to the end of `layer` in composition `comp` (in a node graph,
/// just before its Output), with fresh ids and keyframes timed from the layer's start. Effects
/// that don't suit the layer (color on a sound clip, sound on a title) are left out. Returns
/// how many were added; a locked layer takes none.
pub fn apply_preset(project: &mut Project, comp: Id, layer: Id, preset: &EffectPreset) -> usize {
    let Some(target) = project.composition(comp).and_then(|composition| composition.layer(layer)).filter(|target| !target.switches.locked) else { return 0 };
    let (content, start) = (target.content.clone(), target.in_frame);
    let suited: Vec<&Effect> = preset.effects.iter().filter(|effect| crate::effects::suits_layer(&effect.name, &content)).collect();
    let ids: Vec<Id> = suited.iter().map(|_| project.allocate_id()).collect();
    let added_count = suited.len();
    let Some(target) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer)) else { return 0 };
    for (effect, id) in suited.into_iter().zip(ids) {
        let mut added = effect.clone();
        added.id = id;
        for property in &mut added.properties {
            property.value.shift(start);
        }
        crate::effects::restore_info(&mut added);
        target.effects.push(added);
        crate::effect_graph::effect_added(target, id);
    }
    added_count
}

/// Brings every linked color and text style in `project` up to date with `libraries`. Uses of
/// items that are gone are left as they are. Returns whether anything changed.
pub fn sync_project(project: &mut Project, libraries: &Libraries) -> bool {
    let mut changed = false;
    project.for_each_color_mut(|color| {
        if let Some((_, item)) = color.link.and_then(|link| libraries.color(link)) {
            let updated = item.color.linked(color.link);
            if *color != updated {
                *color = updated;
                changed = true;
            }
        }
    });
    for composition in &mut project.compositions {
        let ids: Vec<_> = composition.layers.iter().map(|layer| layer.id).collect();
        for id in ids {
            let Some(layer) = composition.layer(id) else { continue };
            let crate::model::LayerContent::Text { style, .. } = &layer.content else { continue };
            let Some((_, template)) = style.template.and_then(|link| libraries.text_style(link)) else { continue };
            let mut updated = style.clone();
            template.apply_to(&mut updated);
            if updated != *style {
                crate::edit::edit_text(composition, id, |_, style| *style = updated);
                changed = true;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LayerContent, Paint, ShapeKind, ShapeStyle};
    use crate::sample::{ids, sample_project};

    /// The sample's a-cam footage (id 104, a Gaussian Blur keyframed at frames 108 and 132,
    /// starting at its in point) given a Basic Correction and a Gain too.
    fn graded_footage(project: &mut Project) -> Id {
        let a_cam = Id(104);
        crate::edit::add_effect(project, ids::MAIN_COMP, a_cam, "Basic Correction").unwrap();
        crate::edit::add_effect(project, ids::MAIN_COMP, a_cam, "Gain").unwrap();
        a_cam
    }

    #[test]
    fn a_preset_saves_a_layers_effects_and_applies_them_to_another_in_step() {
        let mut project = sample_project();
        let a_cam = graded_footage(&mut project);
        let source = project.composition(ids::MAIN_COMP).unwrap().layer(a_cam).unwrap().clone();
        let mut libraries = Libraries::default();
        let id = libraries.add_preset("  Warm Blur  ", &source).unwrap();
        let preset = libraries.preset(id).unwrap().clone();
        assert_eq!(preset.name, "Warm Blur");
        assert_eq!(preset.summary(), "3 effects");
        assert_eq!(preset.effects.iter().map(|effect| effect.name.as_str()).collect::<Vec<_>>(), ["Gaussian Blur", "Basic Correction", "Gain"]);

        // Onto another footage layer starting 30 frames later: the blur's keyframes move with it.
        let info = crate::edit::FootageInfo { size: crate::animation::Vec2::new(64.0, 36.0), duration_seconds: 9.0, frame_rate: crate::time::FrameRate::new(24, 1), has_audio: true };
        let item = crate::edit::import_footage(&mut project, "/media/b-cam.mp4".into(), &info);
        let b_cam = crate::edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, source.in_frame + 30).unwrap();
        assert_eq!(apply_preset(&mut project, ids::MAIN_COMP, b_cam, &preset), 3);
        let target = project.composition(ids::MAIN_COMP).unwrap().layer(b_cam).unwrap();
        let blur = &target.effects[0];
        assert_eq!(blur.properties[0].value.value_at((108 + 30) as f64), source.effects[0].properties[0].value.value_at(108.0));
        assert_ne!(blur.properties[0].info.name, "", "range information comes from the catalog");
        // Fresh ids, unique in the project.
        assert!(target.effects.iter().all(|effect| effect.id != Id(0) && !source.effects.iter().any(|other| other.id == effect.id)));
    }

    #[test]
    fn effects_that_dont_suit_a_layer_are_left_out_and_locked_layers_take_none() {
        let mut project = sample_project();
        let a_cam = graded_footage(&mut project);
        let source = project.composition(ids::MAIN_COMP).unwrap().layer(a_cam).unwrap().clone();
        let mut libraries = Libraries::default();
        let id = libraries.add_preset("Look", &source).unwrap();
        let preset = libraries.preset(id).unwrap().clone();
        // A title takes the blur, but not color correction or sound.
        let title = Id(100);
        let before = project.composition(ids::MAIN_COMP).unwrap().layer(title).unwrap().effects.len();
        assert_eq!(apply_preset(&mut project, ids::MAIN_COMP, title, &preset), 1);
        assert_eq!(project.composition(ids::MAIN_COMP).unwrap().layer(title).unwrap().effects.len(), before + 1);
        project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(title).unwrap().switches.locked = true;
        assert_eq!(apply_preset(&mut project, ids::MAIN_COMP, title, &preset), 0);
        // A layer with no effects makes no preset.
        let mut bare = source.clone();
        bare.effects.clear();
        assert_eq!(libraries.add_preset("Empty", &bare), None);
    }

    #[test]
    fn presets_survive_a_save_and_unknown_effects_are_dropped_on_loading() {
        let mut project = sample_project();
        let a_cam = graded_footage(&mut project);
        let source = project.composition(ids::MAIN_COMP).unwrap().layer(a_cam).unwrap().clone();
        let mut libraries = Libraries::default();
        let id = libraries.add_preset("Look", &source).unwrap();
        libraries.presets[0].effects[1].name = "Retired Effect".into();
        let text = serde_json::to_string(&libraries).unwrap();
        let mut loaded: Libraries = serde_json::from_str::<Libraries>(&text).unwrap().validated().unwrap();
        assert_eq!(loaded.preset(id).unwrap().effects.len(), 2, "the unknown effect is dropped");
        assert!(loaded.rename(id, "Renamed"));
        assert_eq!(loaded.preset(id).unwrap().name, "Renamed");
        assert!(loaded.delete(id));
        assert!(loaded.preset(id).is_none());
        // Libraries saved before presets existed still load.
        let old: Libraries = serde_json::from_str(r#"{"brands": [], "next_id": 3}"#).unwrap();
        assert!(old.presets.is_empty());
    }

    fn brand_a() -> (Libraries, LinkId) {
        let mut libraries = Libraries::default();
        let brand = libraries.add_brand("Brand A");
        (libraries, brand)
    }

    #[test]
    fn changing_a_library_color_updates_every_linked_use() {
        let (mut libraries, brand) = brand_a();
        let primary = libraries.add_color(brand, "Primary", Rgba::hex(0xff0000)).unwrap();
        let mut project = sample_project();
        let linked = Rgba::hex(0xff0000).linked(Some(primary));
        let style = ShapeStyle { fill: Some(Paint::linear(linked, Rgba::hex(0x000000), 0.0)), ..ShapeStyle::new(ShapeKind::Rectangle) };
        let shape = crate::edit::add_shape_layer(&mut project, ids::MAIN_COMP, style, crate::animation::Vec2::new(100.0, 100.0), crate::animation::Vec2::new(0.0, 0.0), 0).unwrap();
        project.composition_mut(ids::MAIN_COMP).unwrap().background = linked;

        libraries.set_color(primary, Rgba::hex(0x00ff00));
        assert!(sync_project(&mut project, &libraries));
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        assert!(comp.background.same_look(Rgba::hex(0x00ff00)) && comp.background.link == Some(primary));
        let LayerContent::Shape { style, .. } = &comp.layer(shape).unwrap().content else { panic!() };
        let colors = style.fill.unwrap().colors();
        assert!(colors[0].same_look(Rgba::hex(0x00ff00)), "the linked gradient stop followed");
        assert!(colors[1].same_look(Rgba::hex(0x000000)), "the unlinked one did not");
        assert!(!sync_project(&mut project, &libraries), "nothing left to change");
    }

    #[test]
    fn text_styles_apply_and_follow_their_template() {
        let (mut libraries, brand) = brand_a();
        let header = TextStyle { font_family: "Inter".into(), font_weight: 700, font_size: 48.0, line_height: 58.0, ..TextStyle::default() };
        let id = libraries.add_text_style(brand, "Header", &header).unwrap();
        let mut project = sample_project();
        let title = project.compositions[0].layers[0].id;
        let (_, template) = libraries.text_style(id).unwrap();
        let template = template.clone();
        crate::edit::edit_text(project.composition_mut(ids::MAIN_COMP).unwrap(), title, |_, style| template.apply_to(style));

        libraries.update_text_style(id, &TextStyle { font_size: 64.0, ..header });
        assert!(sync_project(&mut project, &libraries));
        let LayerContent::Text { style, .. } = &project.composition(ids::MAIN_COMP).unwrap().layer(title).unwrap().content else { panic!() };
        assert_eq!((style.font_size, style.font_weight, style.template), (64.0, 700, Some(id)));
    }

    #[test]
    fn deleting_a_brand_removes_everything_in_it_and_leaves_projects_alone() {
        let (mut libraries, brand) = brand_a();
        let color = libraries.add_color(brand, "Primary", Rgba::hex(0x123456)).unwrap();
        libraries.add_text_style(brand, "Body", &TextStyle::default()).unwrap();
        let other = libraries.add_brand("Brand B");
        let kept = libraries.add_color(other, "Accent", Rgba::hex(0xabcdef)).unwrap();
        assert!(libraries.delete(color), "one color on its own");
        assert_eq!(libraries.brand(brand).unwrap().colors.len(), 0);
        assert!(libraries.delete(brand), "and a whole brand");
        assert!(libraries.brand(brand).is_none() && libraries.color(kept).is_some());
        assert!(!libraries.delete(brand), "nothing left to delete");
        // A fresh id never repeats an old one.
        assert!(libraries.add_brand("C").0 > kept.0);
    }

    #[test]
    fn libraries_read_from_disk_are_checked() {
        let (mut libraries, brand) = brand_a();
        libraries.add_color(brand, "  ", Rgba::hex(0xff0000)).unwrap();
        let mut text = serde_json::to_value(&libraries).unwrap();
        text["next_id"] = serde_json::json!(0);
        let reread: Libraries = serde_json::from_value(text).unwrap();
        let checked = reread.validated().unwrap();
        assert_eq!(checked.brands[0].colors[0].name, "#FF0000", "a blank name falls back to the hex");
        let mut fresh = checked.clone();
        assert!(fresh.add_brand("New").0 > 2, "the next id moved past the ones in use");

        let mut twins = checked;
        let duplicate = twins.brands[0].colors[0].clone();
        twins.brands[0].colors.push(duplicate);
        assert!(twins.validated().is_err());
    }
}
