//! Libraries, as in Figma: brands (or clients, or any folder you like) that hold shared colors
//! and text styles. They are global, shared by every project on the computer, and stored by
//! the app outside any project file.
//!
//! A color or text style used from a library stays linked: the project remembers which library
//! item it came from (`Rgba::link`, `TextStyle::template`), and `sync_project` brings every
//! linked use up to date when the library changes, or when a project is opened.

use crate::model::{LinkId, Project, Rgba, TextStyle};

/// The longest name a library item can have, in characters.
pub const MAX_NAME: usize = 80;

/// All brands, and the next id to hand out. Ids are never reused, so a link from an old
/// project can't point at a different item made later.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Libraries {
    pub brands: Vec<Brand>,
    next_id: u64,
}

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

    /// Renames a brand, color or text style. Returns whether it exists.
    pub fn rename(&mut self, id: LinkId, name: &str) -> bool {
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

    /// Deletes a brand with everything in it, or a single color or text style. Projects that
    /// used it keep the color or style they had; they just stop following it.
    pub fn delete(&mut self, id: LinkId) -> bool {
        let before = self.item_count() + self.brands.len();
        self.brands.retain(|brand| brand.id != id);
        for brand in &mut self.brands {
            brand.colors.retain(|color| color.id != id);
            brand.text_styles.retain(|style| style.id != id);
        }
        self.item_count() + self.brands.len() != before
    }

    /// How many colors and text styles there are in all brands.
    pub fn item_count(&self) -> usize {
        self.brands.iter().map(|brand| brand.colors.len() + brand.text_styles.len()).sum()
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
        self.next_id = self.next_id.max(highest);
        Ok(self)
    }
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
        let style = ShapeStyle { fill: Some(Paint::Linear { start: linked, end: Rgba::hex(0x000000), angle: 0.0 }), ..ShapeStyle::new(ShapeKind::Rectangle) };
        let shape = crate::edit::add_shape_layer(&mut project, ids::MAIN_COMP, style, crate::animation::Vec2::new(100.0, 100.0), crate::animation::Vec2::new(0.0, 0.0), 0).unwrap();
        project.composition_mut(ids::MAIN_COMP).unwrap().background = linked;

        libraries.set_color(primary, Rgba::hex(0x00ff00));
        assert!(sync_project(&mut project, &libraries));
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        assert!(comp.background.same_look(Rgba::hex(0x00ff00)) && comp.background.link == Some(primary));
        let LayerContent::Shape { style } = &comp.layer(shape).unwrap().content else { panic!() };
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
