//! Ready-made, animated compositions for social media, ads and marketing: 73 templates, each
//! in three shapes (16:9, 4:5 and 9:16), about 10 to 15 seconds long.
//!
//! A template is a layout (one of `Layout`'s 73 animated designs, one template each) filled with
//! its own words, colors and font (see `catalog`). Any template can take another palette from
//! `LOOKS` (`Template::recolored`) or a brand's colors and font (`Template::branded`). Making one builds an ordinary composition from text, shape
//! and solid layers with keyframes and effects, so everything in it can be edited afterwards.
//! Each has a decoration behind its words (`Decor`). Tinted boxes named "Drop your … here" mark where footage or images go.

mod ads;
mod catalog;
mod editorial;
mod fun;
mod layouts;
mod magazine;
mod photos;
mod social;

use crate::animation::{Animated, CubicBezier, EaseMode, Easing, Keyframe, Vec2};
use crate::edit::{self, CompositionSettings};
use crate::model::{Id, Paint, Project, Rgba, ShapeKind, ShapeStyle, Stroke, TextAlign, TextStyle};
use crate::text_animation::{TextAnimator, TextEase, TextMotion, TextOrder, TextUnit};
use crate::time::FrameRate;

pub use catalog::{LOOKS, TEMPLATES};
pub use photos::{PhotoKind, SamplePhoto, fill_photo_spots};

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
    Ads,
    Sales,
    Product,
    Ugc,
    Social,
    Testimonials,
    Events,
    Education,
    Podcast,
    Brand,
    Fun,
    Celebrations,
    Intros,
}

impl Category {
    pub const ALL: [Self; 14] = [Self::Marketing, Self::Ads, Self::Sales, Self::Product, Self::Ugc, Self::Social, Self::Intros, Self::Fun, Self::Celebrations, Self::Testimonials, Self::Events, Self::Education, Self::Podcast, Self::Brand];

    pub fn name(self) -> &'static str {
        match self {
            Self::Marketing => "Marketing",
            Self::Ads => "Ads",
            Self::Sales => "Sales & Offers",
            Self::Product => "Product",
            Self::Ugc => "UGC",
            Self::Social => "Social Hooks",
            Self::Testimonials => "Reviews",
            Self::Events => "Events",
            Self::Education => "Tips & How-To",
            Self::Podcast => "Podcast & Quotes",
            Self::Brand => "Brand",
            Self::Fun => "Fun & Memes",
            Self::Celebrations => "Celebrations",
            Self::Intros => "Intros",
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

/// What is drawn behind a template's words, over its background color: a few large, simple
/// shapes in the palette's own colors, slowly moving, so a frame never reads as a plain panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decor {
    /// Nothing: the layout brings its own picture (a chat, a phone, a full-frame video).
    None,
    /// A soft glow wandering across, for dark, moody designs.
    Glow,
    /// Big soft-edged circles in two corners and a small bright one.
    Blobs,
    /// A scalloped badge shape turning behind the middle.
    Burst,
    /// Sun rays fanning out from the middle to the edges.
    Rays,
    /// Four-pointed sparkles twinkling near the corners.
    Sparkles,
    /// A thin line just inside the frame's edges, like a printed poster.
    Frame,
    /// Wide slanted bands across two corners.
    Stripes,
    /// A tall arch standing behind the middle.
    Arch,
    /// A circle, a triangle, a ring and a tilted square scattered around the edges.
    Shapes,
    /// Half circles peeking in from the sides.
    HalfCircles,
}

/// The animated designs templates are built from. Each takes its words from the
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
    /// Contact, then four messages (theirs, yours, theirs, yours): a text-message chat.
    Chat,
    /// App, title, message, headline: a phone notification sliding down.
    Notification,
    /// Name, handle, post, likes: a social post card.
    PostCard,
    /// Question, one option, the other: a This-or-That poll with results.
    ThisOrThat,
    /// Top line, bottom line: a meme over a photo.
    Meme,
    /// Line, line under it: text typed out with a blinking cursor.
    Typewriter,
    /// Word, line under it: a flickering neon sign.
    Neon,
    /// Headline, line under it: confetti falling.
    Confetti,
    /// Headline, line under it, button: an app on a phone.
    PhoneMockup,
    /// Caption, line under it: instant photos dropping onto a pile.
    Polaroid,
    /// Question, three answers (the second is right): a quiz with a timer.
    Quiz,
    /// Song, artist: a music player card.
    NowPlaying,
    /// Title, a percentage ("75%"), line under it: a goal bar filling.
    Progress,
    /// Four words: big words stacking up one by one.
    KineticStack,
    /// Place, address: a pin dropping on a map.
    MapPin,
    /// Title, amount, code, expiry: a coupon.
    Coupon,
    /// Title, line under it: a glitching title.
    GlitchTitle,
    /// Greeting, name, handle, three facts: a "meet me" card with a round photo.
    ProfileIntro,
    /// Header, question, answer: a question sticker over footage, then the answer.
    AskMeAnything,
    /// Kicker, name, line under it: the name dropping in letter by letter.
    NameReveal,
    /// Name, time, caption, reply field: a story with its progress bars.
    StoryFrame,
    /// Handle, comment, caption: a "reply to a comment" sticker over footage.
    CommentReply,
    /// Headline, name, line under it: a greeting with hearts rising.
    FloatingHearts,
    /// Three words: huge outlines over a photo, filling in one by one.
    OutlineStack,
    /// Word, line under it, date: a bouncy outlined title with a date sticker.
    VlogTitle,
    /// Hook, three captions, button: captions over a creator talking to camera.
    UgcCaptions,
    /// Title, our name, their name, three rows, button: an "us versus them" table.
    VersusTable,
    /// Problem kicker, problem, solution kicker, solution, button: the problem struck through,
    /// then the answer on a panel sweeping up.
    ProblemSolution,
    /// Title, three features, button: a product photo with its features pointed out.
    FeatureCallouts,
    /// Kicker, offer, line under it, button: a limited-time offer with a clock counting down.
    OfferTimer,
    /// Headline, score, reviews line, button: a star rating with customer photos.
    SocialProof,
    /// Headline (lines), date, place, tag: a poster whose lines stretch up into place.
    PosterStretch,
    /// Kicker, title, date: color bands slide across, then away to show the title.
    SlidingBlocks,
    /// Word, caption: the word repeated down the frame in outline, rows drifting.
    TypeRepeater,
    /// Quote, name, role: photo cards drop onto a stack, then a quote card on top.
    CardStack,
    /// Word, caption: 3, 2, 1 filling the screen on changing colors, then the word.
    BoldCountdown,
    /// Headline, line: a title spinning in out of a blur, with rings rippling out.
    BlurSpin,
    /// Title, kicker: shutters slide off a photo, then a title band.
    SplitReveal,
    /// Title, line: gradient bars drifting behind a huge title.
    GradientTitle,
    /// Word, word, brand, button: big words over a photo, echoing across it in outline.
    PhotoTagline,
    /// Word, line: the word's full stop bounces in, then swells to fill the frame.
    BouncyPeriod,
    /// Name, role: a portrait with the name echoed in outline across the frame behind it.
    NameEcho,
    /// Kicker, date, headline, name, role, badge: a talk promo with the speaker over a color block.
    SpeakerPromo,
    /// Headline, offer, button: a framed photo with beams of colored light fanning up behind it.
    SaleSpotlight,
    /// Title, line: a thin title, two large circles and a product, on paper.
    MinimalProduct,
    /// Tape word, title, what, when: a script title over a photo, with tape and a paper note.
    TapeSale,
    /// Title, three events each followed by its date, details: a lineup of sticker pills.
    EventSchedule,
    /// Top line, bottom line: two photo prints dropping onto a starry ground.
    PrintStack,
    /// Name, role: a portrait in an oval with sparkles around it.
    ArchPortrait,
    /// Offer, handle: enormous type running off the frame behind a black-and-white photo.
    HugeTypePhoto,
    /// Kicker, title, small print: a script title over a full-frame portrait.
    ScriptBeauty,
    /// Brand, three tips: a list between torn paper strips, with worn-film dust and scratches.
    GrungeTips,
    /// Caption, date: a photo in a gold frame, aged like an old print.
    FramedMemory,
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
    /// The shapes behind its words.
    pub decor: Decor,
}

impl Template {
    /// The template in another palette, keeping its words and font.
    pub fn recolored(&self, palette: Palette) -> Template {
        Template { palette, ..*self }
    }

    /// The template in a brand's look: its first color becomes the accent; a second, the
    /// background; a third, the text. Its first text style sets the font. Colors and text styles
    /// it doesn't have keep the template's own.
    pub fn branded(&self, brand: &crate::library::Brand) -> Template {
        let hex = |color: Rgba| color.to_rgba_u32() >> 8;
        let colors: Vec<u32> = brand.colors.iter().map(|item| hex(item.color)).collect();
        let mut palette = self.palette;
        if let Some(&accent) = colors.first() {
            palette.accent = accent;
            palette.on_accent = readable_on(accent);
        }
        if let Some(&background) = colors.get(1) {
            palette.background = background;
            palette.background_2 = shade(background);
            let text = colors.get(2).copied().unwrap_or_else(|| readable_on(background));
            palette.text = text;
            palette.muted = blend(text, background);
        }
        let font = brand.text_styles.first().map_or(self.font, |style| (leak(style.font_family.clone()), style.font_weight));
        Template { palette, font, ..*self }
    }
}

/// A font name kept for the rest of the run: templates hold `&'static str`, and a branded
/// template is built at once and dropped. Each distinct name is stored once.
fn leak(name: String) -> &'static str {
    use std::sync::Mutex;
    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(&known) = names.iter().find(|known| **known == name) {
        return known;
    }
    let stored: &'static str = Box::leak(name.into_boxed_str());
    names.push(stored);
    stored
}

fn channels(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 0xff) as f32, ((hex >> 8) & 0xff) as f32, (hex & 0xff) as f32]
}

fn from_channels([r, g, b]: [f32; 3]) -> u32 {
    let byte = |value: f32| value.round().clamp(0.0, 255.0) as u32;
    (byte(r) << 16) | (byte(g) << 8) | byte(b)
}

/// Black or white, whichever reads better on `hex`.
fn readable_on(hex: u32) -> u32 {
    let [r, g, b] = channels(hex);
    if 0.2126 * r + 0.7152 * g + 0.0722 * b > 150.0 { 0x111111 } else { 0xffffff }
}

/// A second background color: a little darker (or lighter, for dark colors).
fn shade(hex: u32) -> u32 {
    let dark = readable_on(hex) == 0xffffff;
    from_channels(channels(hex).map(|value| if dark { value + (255.0 - value) * 0.18 } else { value * 0.86 }))
}

/// Halfway between two colors, for quieter text.
fn blend(a: u32, b: u32) -> u32 {
    mix(b, a, 0.6)
}

/// A color `amount` of the way (0 to 1) from `from` to `to`.
fn mix(from: u32, to: u32, amount: f32) -> u32 {
    let (from, to) = (channels(from), channels(to));
    from_channels([0, 1, 2].map(|index| from[index] + (to[index] - from[index]) * amount))
}

/// How far apart two colors are, 0 (the same) to 1 (black and white).
fn difference(a: u32, b: u32) -> f32 {
    let (a, b) = (channels(a), channels(b));
    [0, 1, 2].map(|index| (a[index] - b[index]).abs()).iter().sum::<f32>() / (3.0 * 255.0)
}

/// Layers of a template's decoration start with this, so the ending leaves them in place.
const DECOR_PREFIX: &str = "Backdrop";

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

/// Gentle ease-out (most things arriving), ease-in (things leaving), and a springy one (things
/// popping in): it goes a little past its size and settles back once, rather than wobbling.
const EASE_OUT: Easing = Easing::Curve(CubicBezier::new(0.16, 1.0, 0.3, 1.0));
const EASE_IN: Easing = Easing::Curve(CubicBezier::new(0.7, 0.0, 0.84, 0.0));
const EASE_IN_OUT: Easing = Easing::Curve(CubicBezier::new(0.65, 0.0, 0.35, 1.0));
const SPRING: Easing = Easing::Curve(CubicBezier::new(0.34, 1.56, 0.64, 1.0));
/// A real bounce, for the few playful things that should wobble into place.
const WOBBLE: Easing = Easing::Elastic(EaseMode::Out);

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
        let fit = self.fitted_scale(text, size);
        self.text_unfitted(text, at, size * fit, color, weight, from, align)
    }

    /// How much `text` asked for at `size` units is scaled when drawn: words wider than the frame
    /// (long lines in the narrow shapes) are made smaller to fit, and headlines in the tall
    /// shapes larger (see `headline_boost`), never past the frame's width.
    fn fitted_scale(&self, text: &str, size: f32) -> f32 {
        let room = self.width - 90.0 * self.unit();
        let wide = TextStyle { font_size: size * self.unit(), ..TextStyle::default() }.estimate_size(text).x;
        if wide > room { room / wide } else { self.headline_boost(text, size).min(room / wide) }
    }

    /// How much larger a headline is drawn in the tall shapes, where there is height to spare:
    /// big type that nearly fills the width is what makes a phone-sized design read at a glance.
    /// Only lines of several words at headline size grow; single big numbers and words sit in
    /// badges and boxes sized for them.
    fn headline_boost(&self, text: &str, size: f32) -> f32 {
        let headline = (96.0..=170.0).contains(&size) && text.split_whitespace().count() > 1;
        match (headline, self.vertical(), self.tall()) {
            (true, true, _) => 1.4,
            (true, false, true) => 1.2,
            _ => 1.0,
        }
    }

    /// A text layer at exactly `size`, even if it runs past the frame (scrolling text).
    #[allow(clippy::too_many_arguments)]
    fn text_unfitted(&mut self, text: &str, at: Vec2, size: f32, color: u32, weight: u16, from: f64, align: TextAlign) -> Id {
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

    /// What fills a spot waiting for footage or a photo: a two-tone blend that looks like a
    /// picture rather than an empty box. It is cool where the accent is warm (and the other way
    /// round) so accent-colored words over it stay readable, unless that would hide it against
    /// the background.
    fn photo_fill(&self) -> Paint {
        const WARM: (u32, u32) = (0xffb38a, 0xc2416b);
        const COOL: (u32, u32) = (0x8ec5ff, 0x4a3aa8);
        let palette = self.palette();
        let [red, _, blue] = channels(palette.accent);
        let (mut first, mut other) = if red >= blue { (COOL, WARM) } else { (WARM, COOL) };
        let shows = |(light, dark): (u32, u32)| difference(mix(light, dark, 0.5), palette.background);
        if shows(first) < 0.2 && shows(other) > shows(first) {
            std::mem::swap(&mut first, &mut other);
        }
        Paint::linear(Self::color(first.0), Self::color(first.1), 60.0)
    }

    /// A box marking where footage or a photo goes.
    fn placeholder(&mut self, size: Vec2, at: Vec2, corner: f32, from: f64, what: &str) -> Id {
        let fill = self.photo_fill();
        self.shape(ShapeKind::Rectangle, size, at, Some(fill), None, corner, from, &crate::slots::placeholder_name(what))
    }

    /// A round spot for a photo (a profile picture), ringed in `ring` when given a color and width.
    fn round_placeholder(&mut self, diameter: f32, at: Vec2, ring: Option<(u32, f32)>, from: f64, what: &str) -> Id {
        let fill = self.photo_fill();
        self.shape(ShapeKind::Ellipse, Vec2::new(diameter, diameter), at, Some(fill), ring, 0.0, from, &crate::slots::placeholder_name(what))
    }

    /// A star of `points` points whose inner corners sit `inner` of the way out (0 to 1).
    #[allow(clippy::too_many_arguments)]
    fn star(&mut self, points: u32, inner: f32, size: f32, at: Vec2, color: u32, from: f64, name: &str) -> Id {
        let id = self.shape(ShapeKind::Star, Vec2::new(size, size), at, Some(Paint::Solid(Self::color(color))), None, 0.0, from, name);
        if let crate::model::LayerContent::Shape { style, .. } = &mut self.layer(id).content {
            style.points = points;
            style.inner_radius = inner;
        }
        id
    }

    /// The whole-frame background: a gentle gradient between the palette's two background
    /// colors, slowly drifting larger, with the template's decoration over it.
    fn background(&mut self) -> Id {
        let palette = self.palette();
        let paint = Paint::linear(Self::color(palette.background), Self::color(palette.background_2), 120.0);
        let size = Vec2::new(self.width, self.height) * (1.0 / self.unit());
        let center = self.at(0.5, 0.5);
        let id = self.shape(ShapeKind::Rectangle, size, center, Some(paint), None, 0.0, 0.0, "Background");
        let end = self.end();
        self.scale(id, 0.0, 100.0, end, 108.0, Easing::Linear);
        self.decorate();
        id
    }

    /// Draws the template's decoration (see `Decor`) just above the background. Its shapes are
    /// tones of the background nudged toward the accent, so words over them stay easy to read,
    /// with one small piece in the accent itself.
    fn decorate(&mut self) {
        let p = self.palette();
        let tone = mix(p.background, p.accent, 0.2);
        let deep = mix(p.background_2, p.accent, 0.38);
        let short = self.width.min(self.height) / self.unit();
        let long = self.width.max(self.height) / self.unit();
        let end = self.end();
        let name = |what: &str| format!("{DECOR_PREFIX} {what}");
        let solid = |color: u32| Some(Paint::Solid(Self::color(color)));
        match self.template.decor {
            Decor::None => {}
            Decor::Glow => self.ambient_glow(),
            Decor::Blobs => {
                let spots = [(0.02, 0.06, short * 0.95, tone), (1.0, 0.96, short * 1.1, deep)];
                for (index, (x, y, side, color)) in spots.into_iter().enumerate() {
                    let at = self.at(x, y);
                    let blob = self.shape(ShapeKind::Ellipse, Vec2::new(side, side), at, solid(color), None, 0.0, 0.0, &name("circle"));
                    // Each drifts a little toward the middle and back out over the template.
                    let toward = self.at(x + if index == 0 { 0.06 } else { -0.06 }, y);
                    self.move_to(blob, 0.0, end, toward, EASE_IN_OUT);
                }
                let dot_at = self.at(0.86, 0.1);
                let dot = self.shape(ShapeKind::Ellipse, Vec2::new(110.0, 110.0), dot_at, solid(p.accent), None, 0.0, 0.0, &name("dot"));
                self.pulse(dot, 0.0, end, 4);
            }
            Decor::Burst => {
                let at = self.at(0.5, 0.42);
                let burst = self.star(22, 0.86, short * 0.98, at, tone, 0.0, &name("badge"));
                self.spin(burst, 0.0, 60.0);
            }
            Decor::Rays => {
                let at = self.at(0.5, 0.45);
                let rays = self.star(14, 0.08, long * 2.4, at, tone, 0.0, &name("rays"));
                self.spin(rays, 0.0, 40.0);
            }
            Decor::Sparkles => {
                let spots = [(0.12, 0.1, 150.0, p.accent), (0.9, 0.2, 90.0, deep), (0.1, 0.9, 100.0, deep), (0.88, 0.88, 170.0, p.accent)];
                for (index, (x, y, side, color)) in spots.into_iter().enumerate() {
                    let at = self.at(x, y);
                    let sparkle = self.star(4, 0.28, side, at, color, 0.0, &name("sparkle"));
                    // Neighbors twinkle out of step.
                    self.pulse(sparkle, index as f64 * 0.35, end, 5);
                    self.spin(sparkle, 0.0, if index % 2 == 0 { 45.0 } else { -45.0 });
                }
            }
            Decor::Frame => {
                let size = Vec2::new(self.width, self.height) * (1.0 / self.unit()) - Vec2::new(88.0, 88.0);
                let at = self.at(0.5, 0.5);
                self.shape(ShapeKind::Rectangle, size, at, None, Some((p.text, 5.0)), 0.0, 0.0, &name("frame"));
                let corner = self.at(0.5, 0.5) + Vec2::new(size.x, -size.y) * (0.5 * self.unit());
                let dot = self.shape(ShapeKind::Ellipse, Vec2::new(64.0, 64.0), corner, solid(p.accent), None, 0.0, 0.0, &name("dot"));
                self.pulse(dot, 0.0, end, 4);
            }
            Decor::Stripes => {
                for (x, y, color) in [(0.0, 0.1, tone), (0.12, 0.0, deep), (1.0, 0.9, tone), (0.88, 1.0, deep)] {
                    let at = self.at(x, y);
                    let band = self.block(Vec2::new(long * 0.9, 120.0), at, color, 0.0, 0.0, &name("band"));
                    self.transform(band).rotation.value = -38.0;
                }
            }
            Decor::Arch => {
                let size = Vec2::new(short * 0.8, long * 0.74);
                let at = self.at(0.5, if self.tall() { 0.62 } else { 0.7 });
                self.shape(ShapeKind::Rectangle, size, at, solid(tone), None, short * 0.4, 0.0, &name("arch"));
                let sun_at = self.at(0.84, 0.12);
                let sun = self.shape(ShapeKind::Ellipse, Vec2::new(130.0, 130.0), sun_at, solid(p.accent), None, 0.0, 0.0, &name("dot"));
                self.pulse(sun, 0.0, end, 4);
            }
            Decor::Shapes => {
                let circle_at = self.at(0.02, 0.1);
                self.shape(ShapeKind::Ellipse, Vec2::new(360.0, 360.0), circle_at, solid(deep), None, 0.0, 0.0, &name("circle"));
                let triangle_at = self.at(0.92, 0.9);
                let triangle = self.shape(ShapeKind::Triangle, Vec2::new(260.0, 240.0), triangle_at, solid(p.accent), None, 0.0, 0.0, &name("triangle"));
                self.spin(triangle, -12.0, 18.0);
                let ring_at = self.at(0.9, 0.13);
                self.shape(ShapeKind::Ellipse, Vec2::new(150.0, 150.0), ring_at, None, Some((p.text, 10.0)), 0.0, 0.0, &name("ring"));
                let square_at = self.at(0.08, 0.88);
                let square = self.block(Vec2::new(130.0, 130.0), square_at, tone, 14.0, 0.0, &name("square"));
                self.spin(square, 20.0, 65.0);
            }
            Decor::HalfCircles => {
                for (x, y, side, color) in [(0.0, 0.3, short * 0.62, deep), (1.0, 0.72, short * 0.74, tone)] {
                    let at = self.at(x, y);
                    let half = self.shape(ShapeKind::Ellipse, Vec2::new(side, side), at, solid(color), None, 0.0, 0.0, &name("half circle"));
                    self.pulse(half, 0.0, end, 3);
                }
                let dot_at = self.at(0.14, 0.9);
                self.shape(ShapeKind::Ellipse, Vec2::new(90.0, 90.0), dot_at, solid(p.accent), None, 0.0, 0.0, &name("dot"));
            }
        }
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

    /// Opacity down to 0 over `length` seconds, ending at `at`, from whatever it is then.
    fn fade_out(&mut self, id: Id, at: f64, length: f64) {
        let from = self.frame(at - length) as f64;
        let before = self.transform(id).opacity.value_at(from);
        let keys = vec![self.key(at - length, before, EASE_IN_OUT), self.key(at, 0.0, Easing::Linear)];
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
        self.scale(id, at, 0.0, at + 0.55, 100.0, SPRING);
        self.fade_in(id, at, 0.15);
    }

    /// Brings a text layer's words in one after another, each rising into place: the way
    /// headlines arrive in most social and ad videos. It is a text animator, so it can be changed
    /// in Properties afterwards. The layer must start at `at`.
    fn words_in(&mut self, id: Id, seconds: f32) {
        let words = self.text_of(id).split_whitespace().count().max(1);
        // A word or two takes a moment; a long line takes longer, but never drags.
        let seconds = (seconds * (0.6 + 0.1 * words as f32)).clamp(0.4, seconds * 1.4);
        let animator = TextAnimator { motion: TextMotion::Rise, unit: TextUnit::Word, order: TextOrder::Forward, ease: TextEase::Smooth, seconds, overlap: 0.55 };
        self.layer(id).animation.text_in = Some(animator);
    }

    /// Brings a short word in letter by letter, each popping up from nothing: for big single
    /// words such as names, numbers and badges.
    fn letters_in(&mut self, id: Id, seconds: f32) {
        let animator = TextAnimator { motion: TextMotion::Pop, unit: TextUnit::Character, order: TextOrder::Forward, ease: TextEase::Overshoot, seconds, overlap: 0.7 };
        self.layer(id).animation.text_in = Some(animator);
    }

    /// The words of a text layer.
    fn text_of(&mut self, id: Id) -> String {
        match &self.layer(id).content {
            crate::model::LayerContent::Text { text, .. } => text.clone(),
            _ => String::new(),
        }
    }

    /// The height, in pixels, `text` takes when asked for at `size` units (all its lines), at
    /// the size it is really drawn.
    fn text_height(&self, text: &str, size: f32) -> f32 {
        text.lines().count().max(1) as f32 * size * self.fitted_scale(text, size) * 1.15 * self.unit()
    }

    /// Where to center something `height` units tall so it sits `gap` units below the text
    /// `text` (at `size` units, centered at `above`), and no higher than `at_least`. Keeps a
    /// line under a headline clear of it when the headline runs to two or three lines.
    fn below(&self, above: Vec2, text: &str, size: f32, gap: f32, height: f32, at_least: Vec2) -> Vec2 {
        let unit = self.unit();
        let y = above.y + self.text_height(text, size) * 0.5 + (gap + height * 0.5) * unit;
        Vec2::new(at_least.x, at_least.y.max(y))
    }

    /// A soft glow in the accent color, drifting slowly behind everything, so a held frame is
    /// never quite still. Sits just above the background.
    fn ambient_glow(&mut self) {
        let p = self.palette();
        let side = self.width.max(self.height) / self.unit() * 0.9;
        let paint = Paint::radial(Self::color(p.accent).with_alpha(0.35), Self::color(p.accent).with_alpha(0.0));
        let start = self.at(0.25, 0.3);
        let glow = self.shape(ShapeKind::Ellipse, Vec2::new(side, side), start, Some(paint), None, 0.0, 0.0, "Glow");
        self.transform(glow).opacity.value = 55.0;
        let end = self.end();
        let finish = self.at(0.75, 0.7);
        self.move_to(glow, 0.0, end, finish, EASE_IN_OUT);
        self.scale(glow, 0.0, 90.0, end, 120.0, EASE_IN_OUT);
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

    /// Keyframes setting `index` of the last effect added to a layer: (seconds, value) pairs.
    fn animate_effect(&mut self, id: Id, index: usize, keys: &[(f64, f32, Easing)]) {
        let keys: Vec<Keyframe<f32>> = keys.iter().map(|&(at, value, easing)| self.key(at, value, easing)).collect();
        if let Some(property) = self.layer(id).effects.last_mut().and_then(|effect| effect.properties.get_mut(index)) {
            property.value = Animated::keyframed(keys);
        }
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

    /// An effect over the whole frame, on an adjustment layer named `label` at the top, so it
    /// can be changed or turned off in one place. `settings` are (position, value) pairs.
    fn overlay(&mut self, effect: &str, label: &str, settings: &[(usize, f32)]) {
        let Some(id) = edit::add_adjustment_layer(self.project, self.comp, 0) else { return };
        self.layer(id).name = label.to_owned();
        self.effect(id, effect, settings);
    }

    /// Film grain over the whole frame (see `overlay`), at `amount` percent.
    fn grain(&mut self, amount: f32) {
        self.overlay("Film Grain", "Grain", &[(0, amount)]);
    }

    /// Sets a text layer's font, for a second typeface beside the template's own: small
    /// sans-serif details under a script title, say.
    fn set_font(&mut self, id: Id, family: &str, weight: u16) {
        let comp = self.comp;
        let composition = self.project.composition_mut(comp).expect("the composition was just made");
        crate::edit::edit_text(composition, id, |_, style| {
            style.font_family = family.into();
            style.font_weight = weight;
        });
    }

    /// Sends every layer still showing at the end off in a quick ripple: each fades while
    /// lifting a little, a beat after the one before, so the template ends with a clean exit
    /// rather than a flat fade. What fills the frame (the background, color panels, the spot for
    /// footage) stays, so the last frame isn't empty.
    fn fade_everything_out(&mut self) {
        const LENGTH: f64 = 0.45;
        const STAGGER: f64 = 0.03;
        let end = self.end();
        let (width, height) = (self.width, self.height);
        let fills_frame = |layer: &crate::model::Layer| matches!(layer.name.as_str(), "Background" | "Glow" | "Panel") || layer.name.starts_with(DECOR_PREFIX) || (layer.size.x >= width * 0.99 && layer.size.y >= height * 0.99);
        let lasting: Vec<Id> = self
            .project
            .composition(self.comp)
            .map(|composition| composition.layers.iter().filter(|layer| layer.out_frame >= self.duration && !fills_frame(layer)).map(|layer| layer.id).collect())
            .unwrap_or_default();
        let count = lasting.len();
        let unit = self.unit();
        // `layers` lists the top layer first; the bottom ones (made first) leave first, as they came.
        for (index, id) in lasting.into_iter().enumerate() {
            let order = (count - 1 - index) as f64;
            let finish = end - 0.02 - (0.2 - (order * STAGGER).min(0.2));
            self.fade_out(id, finish, LENGTH);
            // Lifts only layers that aren't already moving then (scrolling rows keep scrolling).
            let start = self.frame(finish - LENGTH);
            let position = &self.transform(id).position;
            if position.keyframes().iter().all(|key| key.frame < start) {
                let from = position.value_at(start as f64);
                let keys = vec![self.key(finish - LENGTH, from, EASE_IN), self.key(finish, from - Vec2::new(0.0, 40.0 * unit), Easing::Linear)];
                Self::keys(&mut self.transform(id).position, keys);
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_has_words_for_its_layout() {
        assert_eq!(TEMPLATES.len(), 73);
        let mut names = std::collections::HashSet::new();
        for template in &TEMPLATES {
            assert!(names.insert(template.name), "{} is named twice", template.name);
            assert!((10.0..=20.0).contains(&template.seconds), "{} runs {} s", template.name, template.seconds);
            assert!(template.copy.len() >= layouts::words_needed(template.layout), "{} is missing words", template.name);
        }
        for category in Category::ALL {
            assert!(TEMPLATES.iter().filter(|template| template.category == category).count() >= 3, "{} has few templates", category.name());
        }
    }

    #[test]
    fn no_two_templates_are_the_same_design_in_other_colors() {
        let mut layouts = std::collections::HashSet::new();
        for template in &TEMPLATES {
            assert!(layouts.insert(format!("{:?}", template.layout)), "{} repeats the {:?} layout", template.name, template.layout);
        }
        let mut names = std::collections::HashSet::new();
        assert!(LOOKS.iter().all(|(name, _)| names.insert(*name)), "palette names are unique");
    }

    #[test]
    fn recoloring_keeps_the_words_and_font() {
        let template = TEMPLATES[0];
        let (_, palette) = LOOKS[LOOKS.len() - 1];
        let recolored = template.recolored(palette);
        assert_eq!(recolored.palette, palette);
        assert_eq!((recolored.copy, recolored.font, recolored.name), (template.copy, template.font, template.name));
    }

    #[test]
    fn a_brand_sets_a_templates_colors_and_font() {
        let mut libraries = crate::library::Libraries::default();
        let brand = libraries.add_brand("Acme");
        libraries.add_color(brand, "Orange", Rgba::hex(0xff6600));
        libraries.add_color(brand, "Navy", Rgba::hex(0x0a1a3a));
        let branded = TEMPLATES[0].branded(libraries.brand(brand).unwrap());
        assert_eq!((branded.palette.accent, branded.palette.background), (0xff6600, 0x0a1a3a));
        assert_eq!(branded.palette.text, 0xffffff, "light text on the dark brand color");
        assert_eq!(branded.font, TEMPLATES[0].font, "no text style: the template's own font");
        let mut project = Project::new(Vec::new(), Vec::new());
        assert!(build(&mut project, &branded, Format::Vertical).is_some());
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
