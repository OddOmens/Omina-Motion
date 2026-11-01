//! Ready-made, animated compositions for social media and marketing: 100 templates, each in
//! three shapes (16:9, 4:5 and 9:16), about 10 to 15 seconds long.
//!
//! A template is a layout (one of `Layout`'s twenty animated designs) filled with its own words,
//! colors and font (see `catalog`). Making one builds an ordinary composition from text, shape
//! and solid layers with keyframes and effects, so everything in it can be edited afterwards.
//! Gray boxes named "Drop your … here" mark where footage or images go.

mod catalog;
mod layouts;

use crate::animation::{Animated, CubicBezier, EaseMode, Easing, Keyframe, Vec2};
use crate::edit::{self, CompositionSettings};
use crate::model::{Id, Paint, Project, Rgba, ShapeKind, ShapeStyle, Stroke, TextAlign, TextStyle};
use crate::time::FrameRate;

pub use catalog::TEMPLATES;

/// The shapes every template comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// 1920 × 1080: YouTube, websites, TV.
    Wide,
    /// 1080 × 1350: Instagram and Facebook feeds.
    Portrait,
    /// 1080 × 1920: Reels, TikTok, Shorts and Stories.
    Vertical,
}

impl Format {
    pub const ALL: [Self; 3] = [Self::Wide, Self::Portrait, Self::Vertical];

    pub fn label(self) -> &'static str {
        match self {
            Self::Wide => "16:9",
            Self::Portrait => "4:5",
            Self::Vertical => "9:16",
        }
    }

    pub fn size(self) -> (u32, u32) {
        match self {
            Self::Wide => (1920, 1080),
            Self::Portrait => (1080, 1350),
            Self::Vertical => (1080, 1920),
        }
    }
}

/// What a template is for, to browse them by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Marketing,
    Sales,
    Product,
    Ugc,
    Social,
    Testimonials,
    Events,
    Education,
    Podcast,
    Brand,
}

impl Category {
    pub const ALL: [Self; 10] = [Self::Marketing, Self::Sales, Self::Product, Self::Ugc, Self::Social, Self::Testimonials, Self::Events, Self::Education, Self::Podcast, Self::Brand];

    pub fn name(self) -> &'static str {
        match self {
            Self::Marketing => "Marketing",
            Self::Sales => "Sales & Offers",
            Self::Product => "Product",
            Self::Ugc => "UGC",
            Self::Social => "Social Hooks",
            Self::Testimonials => "Reviews",
            Self::Events => "Events",
            Self::Education => "Tips & How-To",
            Self::Podcast => "Podcast & Quotes",
            Self::Brand => "Brand",
        }
    }
}

/// A template's colors, as hex RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The background, and a second color it blends into.
    pub background: u32,
    pub background_2: u32,
    /// The color that stands out: badges, bars, buttons.
    pub accent: u32,
    /// Text on the background, and quieter text.
    pub text: u32,
    pub muted: u32,
    /// Text on the accent color.
    pub on_accent: u32,
}

/// The twenty animated designs templates are built from. Each takes its words from the
/// template's `copy`, in the order its description gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Kicker, headline, line under it, button.
    Headline,
    /// Headline, line under it, price, button: two color panels and a price badge.
    SplitPromo,
    /// Title, three items, button: items arrive one by one.
    List,
    /// Quote, who said it, where.
    Quote,
    /// Kicker, the reveal, line under it: 3, 2, 1, then the reveal.
    Countdown,
    /// Name, title: a lower-third bar over footage (transparent background).
    LowerThird,
    /// Review, name, product: five stars, then the words.
    Review,
    /// Big number, word beside it, line under it, code: a spinning burst behind.
    SaleBurst,
    /// Hook, line under it: a boxed hook over footage (transparent background).
    Hook,
    /// Product name, price, features, button: a product photo spot.
    ProductCard,
    /// Kicker, title, date, place.
    Event,
    /// Number, what it counts, source: a bar fills as it lands.
    BigStat,
    /// Label for before, label for after, headline: a wipe across two photo spots.
    BeforeAfter,
    /// Title, three steps: cards arrive one after another.
    Steps,
    /// Badge word, headline, line under it: a turning badge.
    Announcement,
    /// Show, episode, guest: a moving sound wave and a guest photo spot.
    Podcast,
    /// Brand, tagline: a logo spot and lines opening out.
    BrandIntro,
    /// Title, line under it: four photo spots.
    Collage,
    /// Phrase, line under it: rows of words scrolling across.
    Marquee,
    /// Headline, handle, button: an end card asking to follow.
    EndCard,
}

/// One template: its layout filled with words, colors and a font.
#[derive(Clone, Copy, Debug)]
pub struct Template {
    pub name: &'static str,
    pub category: Category,
    pub layout: Layout,
    pub copy: &'static [&'static str],
    pub palette: Palette,
    /// Headline font and weight; other text uses the same family, lighter.
    pub font: (&'static str, u16),
    pub seconds: f64,
}

/// Templates' frame rate.
const FPS: u32 = 30;

/// Makes `template` in `format` as a new composition in `project`. Returns the composition.
pub fn build(project: &mut Project, template: &Template, format: Format) -> Option<Id> {
    let (width, height) = format.size();
    let duration = (template.seconds * f64::from(FPS)).round() as i64;
    let settings = CompositionSettings {
        name: format!("{} {}", template.name, format.label()),
        width,
        height,
        frame_rate: FrameRate::new(FPS, 1),
        duration_frames: duration,
        background: Rgba::TRANSPARENT,
    };
    let comp = edit::new_composition(project, &settings).ok()?;
    let mut builder = Builder { project, comp, template, width: width as f32, height: height as f32, duration };
    layouts::build(&mut builder);
    Some(comp)
}

/// Gentle ease-out (most things arriving), and a springy one (things popping in).
const EASE_OUT: Easing = Easing::Curve(CubicBezier::new(0.16, 1.0, 0.3, 1.0));
const EASE_IN_OUT: Easing = Easing::Curve(CubicBezier::new(0.65, 0.0, 0.35, 1.0));
const SPRING: Easing = Easing::Elastic(EaseMode::Out);

/// Builds a template's layers into its composition. Positions are fractions of the frame, and
/// sizes are in "units": a pixel of a 1080-pixel short side, so every format looks alike.
pub(crate) struct Builder<'a> {
    project: &'a mut Project,
    comp: Id,
    template: &'a Template,
    width: f32,
    height: f32,
    duration: i64,
}

impl Builder<'_> {
    /// Whether the frame is taller than it is wide.
    fn tall(&self) -> bool {
        self.height > self.width
    }

    /// Whether it is the tallest format (9:16).
    fn vertical(&self) -> bool {
        self.height > self.width * 1.5
    }

    fn unit(&self) -> f32 {
        self.width.min(self.height) / 1080.0
    }

    fn at(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(self.width * x, self.height * y)
    }

    fn frame(&self, seconds: f64) -> i64 {
        ((seconds * f64::from(FPS)).round() as i64).clamp(0, self.duration)
    }

    fn end(&self) -> f64 {
        self.duration as f64 / f64::from(FPS)
    }

    fn copy(&self, index: usize) -> &'static str {
        self.template.copy.get(index).copied().unwrap_or("")
    }

    fn color(hex: u32) -> Rgba {
        Rgba::hex(hex)
    }

    fn palette(&self) -> Palette {
        self.template.palette
    }

    /// A text layer in the template's font, centered at `at`, shown from `from` seconds on.
    fn text(&mut self, text: &str, at: Vec2, size: f32, color: u32, weight: u16, from: f64) -> Id {
        self.text_aligned(text, at, size, color, weight, from, TextAlign::Center)
    }

    #[allow(clippy::too_many_arguments)]
    fn text_aligned(&mut self, text: &str, at: Vec2, size: f32, color: u32, weight: u16, from: f64, align: TextAlign) -> Id {
        let size = size * self.unit();
        let style = TextStyle {
            font_family: self.template.font.0.into(),
            font_weight: weight,
            font_size: size,
            line_height: size * 1.15,
            tracking: 0.0,
            fill: Some(Self::color(color)),
            stroke: None,
            align,
            template: None,
        };
        let frame = self.frame(from);
        let id = edit::add_text_layer(self.project, self.comp, text, at, frame).expect("the composition was just made");
        let composition = self.project.composition_mut(self.comp).expect("the composition was just made");
        crate::edit::edit_text(composition, id, |_, current| *current = style.clone());
        let box_size = style.estimate_size(text);
        crate::edit::set_text_box(composition, id, box_size);
        id
    }

    /// The headline weight and a lighter one for the rest.
    fn heavy(&self) -> u16 {
        self.template.font.1
    }

    fn light(&self) -> u16 {
        (self.template.font.1.saturating_sub(300)).max(400)
    }

    /// A shape layer of `size` units centered at `at`.
    #[allow(clippy::too_many_arguments)]
    fn shape(&mut self, kind: ShapeKind, size: Vec2, at: Vec2, fill: Option<Paint>, stroke: Option<(u32, f32)>, corner: f32, from: f64, name: &str) -> Id {
        let unit = self.unit();
        let style = ShapeStyle {
            fill,
            stroke: stroke.map(|(color, width)| Stroke { paint: Paint::Solid(Self::color(color)), width: width * unit }),
            corner_radius: corner * unit,
            ..ShapeStyle::new(kind)
        };
        let frame = self.frame(from);
        let id = edit::add_shape_layer(self.project, self.comp, style, size * unit, at, frame).expect("the composition was just made");
        self.layer(id).name = name.to_owned();
        // Corners are kept within the box.
        edit::edit_shape(self.project.composition_mut(self.comp).expect("made"), id, |_| {});
        id
    }

    /// A rectangle in one color.
    fn block(&mut self, size: Vec2, at: Vec2, color: u32, corner: f32, from: f64, name: &str) -> Id {
        self.shape(ShapeKind::Rectangle, size, at, Some(Paint::Solid(Self::color(color))), None, corner, from, name)
    }

    /// A gray box marking where footage or a photo goes.
    fn placeholder(&mut self, size: Vec2, at: Vec2, corner: f32, from: f64, what: &str) -> Id {
        let fill = Paint::Linear { start: Rgba::hex(0x3f3f46), end: Rgba::hex(0x27272a), angle: 60.0 };
        self.shape(ShapeKind::Rectangle, size, at, Some(fill), Some((0x71717a, 2.0)), corner, from, &format!("Drop your {what} here"))
    }

    /// The whole-frame background: a gentle gradient between the palette's two background
    /// colors, slowly drifting larger for life.
    fn background(&mut self) -> Id {
        let palette = self.palette();
        let paint = Paint::Linear { start: Self::color(palette.background), end: Self::color(palette.background_2), angle: 120.0 };
        let size = Vec2::new(self.width, self.height) * (1.0 / self.unit());
        let center = self.at(0.5, 0.5);
        let id = self.shape(ShapeKind::Rectangle, size, center, Some(paint), None, 0.0, 0.0, "Background");
        let end = self.end();
        self.scale(id, 0.0, 100.0, end, 108.0, Easing::Linear);
        id
    }

    fn layer(&mut self, id: Id) -> &mut crate::model::Layer {
        self.project.composition_mut(self.comp).and_then(|composition| composition.layer_mut(id)).expect("the layer was just made")
    }

    fn transform(&mut self, id: Id) -> &mut crate::model::Transform {
        self.layer(id).transform.as_mut().expect("picture layers have a transform")
    }

    /// Ends a layer at `seconds`.
    fn until(&mut self, id: Id, seconds: f64) {
        let frame = self.frame(seconds).max(1);
        let layer = self.layer(id);
        layer.out_frame = frame.max(layer.in_frame + 1);
    }

    /// Adds keyframes to an animated value, keeping those it has.
    fn keys<T: Copy + crate::animation::Lerp>(animated: &mut Animated<T>, keys: Vec<Keyframe<T>>) {
        let mut all = animated.keyframes().to_vec();
        all.extend(keys);
        *animated = Animated::keyframed(all);
    }

    fn key<T>(&self, seconds: f64, value: T, easing: Easing) -> Keyframe<T> {
        Keyframe { frame: self.frame(seconds), value, easing }
    }

    /// Opacity from 0 to 100 over `length` seconds from `at`.
    fn fade_in(&mut self, id: Id, at: f64, length: f64) {
        let keys = vec![self.key(at, 0.0, EASE_OUT), self.key(at + length, 100.0, Easing::Linear)];
        Self::keys(&mut self.transform(id).opacity, keys);
    }

    /// Opacity down to 0 over `length` seconds, ending at `at`.
    fn fade_out(&mut self, id: Id, at: f64, length: f64) {
        let keys = vec![self.key(at - length, 100.0, EASE_IN_OUT), self.key(at, 0.0, Easing::Linear)];
        Self::keys(&mut self.transform(id).opacity, keys);
    }

    /// Slides in from `offset` units away while fading in.
    fn slide_in(&mut self, id: Id, offset: Vec2, at: f64, length: f64) {
        let home = self.transform(id).position.value;
        let start = home + offset * self.unit();
        let keys = vec![self.key(at, start, EASE_OUT), self.key(at + length, home, Easing::Linear)];
        Self::keys(&mut self.transform(id).position, keys);
        self.fade_in(id, at, length * 0.6);
    }

    /// Scales from `from` % to `to` %.
    fn scale(&mut self, id: Id, at: f64, from: f32, until: f64, to: f32, easing: Easing) {
        let keys = vec![self.key(at, Vec2::new(from, from), easing), self.key(until, Vec2::new(to, to), Easing::Linear)];
        Self::keys(&mut self.transform(id).scale, keys);
    }

    /// Pops in from nothing with a spring.
    fn pop_in(&mut self, id: Id, at: f64) {
        self.scale(id, at, 0.0, at + 0.7, 100.0, SPRING);
        self.fade_in(id, at, 0.15);
    }

    /// Grows across from its left edge (a bar filling).
    fn grow_across(&mut self, id: Id, at: f64, length: f64) {
        let keys = vec![self.key(at, Vec2::new(0.0, 100.0), EASE_OUT), self.key(at + length, Vec2::new(100.0, 100.0), Easing::Linear)];
        Self::keys(&mut self.transform(id).scale, keys);
    }

    /// Puts the anchor point on the left edge, so the layer grows from there.
    fn anchor_left(&mut self, id: Id) {
        let size = self.layer(id).size;
        let comp = self.comp;
        edit::move_anchor(self.project.composition_mut(comp).expect("made"), id, Vec2::new(0.0, size.y * 0.5), 0);
    }

    /// Turns steadily from `from` to `to` degrees over the template.
    fn spin(&mut self, id: Id, from: f32, to: f32) {
        let end = self.end();
        let keys = vec![self.key(0.0, from, Easing::Linear), self.key(end, to, Easing::Linear)];
        Self::keys(&mut self.transform(id).rotation, keys);
    }

    /// A gentle pulse: larger and back, `times` times from `at` to `until`.
    fn pulse(&mut self, id: Id, at: f64, until: f64, times: usize) {
        let step = (until - at) / (times * 2) as f64;
        let keys = (0..=times * 2).map(|index| self.key(at + step * index as f64, if index % 2 == 1 { Vec2::new(108.0, 108.0) } else { Vec2::new(100.0, 100.0) }, EASE_IN_OUT)).collect();
        Self::keys(&mut self.transform(id).scale, keys);
    }

    /// Moves to `to` (fractions of the frame) between `at` and `until`.
    fn move_to(&mut self, id: Id, at: f64, until: f64, to: Vec2, easing: Easing) {
        let frame = self.frame(at) as f64;
        let from = self.transform(id).position.value_at(frame);
        let keys = vec![self.key(at, from, easing), self.key(until, to, Easing::Linear)];
        Self::keys(&mut self.transform(id).position, keys);
    }

    /// Adds a catalog effect with some of its settings changed, by position.
    fn effect(&mut self, id: Id, name: &str, settings: &[(usize, f32)]) {
        let comp = self.comp;
        let Some(effect) = edit::add_effect(self.project, comp, id, name) else { return };
        let layer = self.layer(id);
        if let Some(effect) = layer.effects.iter_mut().find(|candidate| candidate.id == effect) {
            for &(index, value) in settings {
                if let Some(property) = effect.properties.get_mut(index) {
                    property.value = Animated::constant(value);
                }
            }
        }
    }

    /// Fades every layer out over the last half second, so the template ends cleanly.
    fn fade_everything_out(&mut self) {
        let end = self.end();
        let ids: Vec<(Id, bool)> = self
            .project
            .composition(self.comp)
            .map(|composition| composition.layers.iter().map(|layer| (layer.id, layer.out_frame >= self.duration && layer.name != "Background")).collect())
            .unwrap_or_default();
        for (id, lasts) in ids {
            if lasts {
                self.fade_out(id, end, 0.5);
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_a_hundred_templates_with_words_for_their_layouts() {
        assert_eq!(TEMPLATES.len(), 100);
        let mut names = std::collections::HashSet::new();
        for template in &TEMPLATES {
            assert!(names.insert(template.name), "{} is named twice", template.name);
            assert!((10.0..=20.0).contains(&template.seconds), "{} runs {} s", template.name, template.seconds);
            assert!(template.copy.len() >= layouts::words_needed(template.layout), "{} is missing words", template.name);
        }
        for category in Category::ALL {
            assert!(TEMPLATES.iter().filter(|template| template.category == category).count() >= 5, "{} has few templates", category.name());
        }
    }

    #[test]
    fn every_template_builds_in_every_format_and_passes_the_file_checks() {
        for template in &TEMPLATES {
            for format in Format::ALL {
                let mut project = Project::new(Vec::new(), Vec::new());
                let comp = build(&mut project, template, format).unwrap_or_else(|| panic!("{} {format:?}", template.name));
                let composition = project.composition(comp).unwrap();
                assert_eq!((composition.width, composition.height), format.size());
                assert!(composition.layers.len() >= 3, "{} has only {} layers", template.name, composition.layers.len());
                let view = crate::project_file::EditorView { active_composition: comp, current_frame: 0, work_area: crate::time::FrameRange::new(0.0, composition.duration_frames as f64) };
                let text = crate::project_file::to_text(&project, view).unwrap();
                crate::project_file::from_text(&text).unwrap_or_else(|error| panic!("{} {format:?}: {error}", template.name));
            }
        }
    }
}
