//! Photo-led layouts in the style of social media design kits: a portrait or product photo at
//! the heart of the design, large type around it, and print-like finishes (film grain, paper,
//! worn film). A name echoed in outline behind a portrait, a speaker promo, a sale under a
//! spotlight, a minimal product poster, a market flyer with tape and torn paper, an event
//! lineup of stickers, stacked photo prints, a portrait in an oval, huge type behind a photo, a
//! script title over a portrait, a grungy list of tips, and a photo in a gold frame.
//!
//! Like the others, each places its layers by fractions of the frame, sizes them in units (see
//! `Builder`), and adapts to the three shapes, usually stacking in the tall ones and setting
//! the photo beside the words in 16:9.

use super::editorial::{anchored_text, flat_background};
use super::social::{full_frame_video, outline, set_fill, tinted};
use super::{Builder, EASE_IN_OUT, EASE_OUT, SPRING, mix};
use crate::animation::{Easing, Vec2};
use crate::model::{Id, Paint, Rgba, ShapeKind, TextAlign};

/// The dark ink the sticker-style layouts outline their shapes with.
const INK: u32 = 0x1a1a1a;

/// The sans-serif that script templates use for their small print.
const SANS: &str = "Avenir Next";

/// The frame's size in units.
fn frame_units(b: &Builder) -> Vec2 {
    Vec2::new(b.width, b.height) * (1.0 / b.unit())
}

/// Turns a layer to `degrees`, clockwise.
fn tilt(b: &mut Builder, id: Id, degrees: f32) {
    b.transform(id).rotation.value = degrees;
}

/// A spot for a photo, `size` units, in any shape: a rectangle with `corner` rounding, or an
/// ellipse for an oval portrait.
fn photo_spot(b: &mut Builder, kind: ShapeKind, size: Vec2, at: Vec2, corner: f32, from: f64, what: &str) -> Id {
    let fill = b.photo_fill();
    b.shape(kind, size, at, Some(fill), None, corner, from, &crate::slots::placeholder_name(what))
}

/// Where something `offset` units from `at` (pixels) lands, in pixels.
fn nudge(b: &Builder, at: Vec2, offset: Vec2) -> Vec2 {
    at + offset * b.unit()
}

/// A text layer in the small sans-serif (`SANS`), for the details under a script title.
#[allow(clippy::too_many_arguments)]
fn sans_text(b: &mut Builder, words: &str, at: Vec2, size: f32, color: u32, weight: u16, from: f64) -> Id {
    let id = b.text(words, at, size, color, weight, from);
    b.set_font(id, SANS, weight);
    id
}

/// A portrait with the person's name echoed in outline across the whole frame behind it,
/// drifting, and once more in solid type over the photo's top edge: name, role.
pub(super) fn name_echo(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (name, role) = (b.copy(0), b.copy(1));
    let frame = frame_units(b);
    let end = b.end();
    let size = 130.0;
    let rows = (frame.y / (size * 1.05)).ceil() as usize;
    let repeated = format!("{name}   {name}   {name}");
    for row in 0..rows {
        let y = (row as f32 + 0.5) / rows as f32;
        let echo = b.text_unfitted(&repeated, b.at(0.5, y), size, p.muted, b.heavy(), 0.0, TextAlign::Center);
        outline(b, echo, p.muted, 2.0);
        set_fill(b, echo, None);
        b.fade_in(echo, row as f64 * 0.04, 0.5);
        // Neighboring rows drift opposite ways.
        let shift = if row % 2 == 0 { -120.0 } else { 120.0 };
        let start = b.at(0.5, y);
        let finish = nudge(b, start, Vec2::new(shift, 0.0));
        b.move_to(echo, 0.0, end, finish, Easing::Linear);
    }
    // The portrait, as large as fits at three by four.
    let width = (frame.x * 0.66).min(frame.y * 0.62 * 0.75);
    let photo_size = Vec2::new(width, width / 0.75);
    let photo_at = b.at(0.5, 0.52);
    let photo = photo_spot(b, ShapeKind::Rectangle, photo_size, photo_at, 0.0, 0.3, "model photo");
    b.slide_in(photo, Vec2::new(0.0, 80.0), 0.3, 0.8);
    let half_height = photo_size.y * 0.5 * b.unit() / b.height;
    let title = b.text(name, b.at(0.5, 0.52 - half_height), 150.0, p.text, b.heavy(), 0.8);
    b.effect(title, "Drop Shadow", &[(0, 45.0), (2, 8.0), (3, 18.0)]);
    b.letters_in(title, 0.8);
    let role_line = b.text(role, b.at(0.5, (0.52 + half_height + 0.05).min(0.93)), 64.0, p.accent, b.heavy(), 1.4);
    b.words_in(role_line, 0.6);
    b.fade_everything_out();
    b.grain(18.0);
}

/// A talk or webinar promo: a big left-aligned headline, the speaker's photo in black and white
/// over a color block, their name and role beside it, a round "join" badge, and a kicker and
/// date across the top: kicker, date, headline, name, role, badge.
pub(super) fn speaker_promo(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (kicker, date, headline, name, role, badge) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3), b.copy(4), b.copy(5));
    let frame = frame_units(b);
    let tall = b.tall();
    let side = 70.0 * b.unit() / b.width;
    let top_line = anchored_text(b, kicker, b.at(side, 0.05), 30.0, p.muted, b.light().max(500), 0.2, TextAlign::Left, (0.0, 0.5));
    b.fade_in(top_line, 0.2, 0.4);
    let when = anchored_text(b, date, b.at(1.0 - side, 0.05), 44.0, p.text, b.heavy(), 0.3, TextAlign::Right, (1.0, 0.5));
    b.fade_in(when, 0.3, 0.4);
    let head = anchored_text(b, headline, b.at(side, 0.1), if tall { 150.0 } else { 130.0 }, p.text, b.heavy(), 0.5, TextAlign::Left, (0.0, 0.0));
    b.words_in(head, 1.0);
    // The color block and photo: low and to the left in the tall shapes, on the right in 16:9.
    let (block_size, block_at, photo_size, photo_at) = if tall {
        (Vec2::new(frame.x * 0.64, frame.y * 0.34), b.at(0.68, 0.83), Vec2::new(frame.x * 0.58, frame.y * 0.46), b.at(0.42, 0.77))
    } else {
        (Vec2::new(frame.x * 0.36, frame.y * 0.62), b.at(0.8, 0.69), Vec2::new(frame.x * 0.3, frame.y * 0.8), b.at(0.7, 0.6))
    };
    let block = b.block(block_size, block_at, p.accent, 0.0, 0.9, "Color block");
    b.slide_in(block, Vec2::new(500.0, 0.0), 0.9, 0.7);
    let photo = photo_spot(b, ShapeKind::Rectangle, photo_size, photo_at, 0.0, 1.1, "speaker photo");
    b.effect(photo, "Black & White", &[(1, 25.0)]);
    b.slide_in(photo, Vec2::new(0.0, 160.0), 1.1, 0.8);
    // Name and role: right of the photo when tall, under the headline in 16:9.
    let (name_at, align, anchor) = if tall { (b.at(1.0 - side, 0.6), TextAlign::Right, (1.0, 0.5)) } else { (b.at(side, 0.66), TextAlign::Left, (0.0, 0.5)) };
    let speaker = anchored_text(b, name, name_at, 60.0, p.text, b.heavy(), 1.6, align, anchor);
    b.slide_in(speaker, Vec2::new(0.0, 30.0), 1.6, 0.6);
    let role_at = Vec2::new(name_at.x, name_at.y + b.text_height(name, 60.0) * 0.5 + 50.0 * b.unit());
    let job = anchored_text(b, role, role_at, 30.0, p.muted, b.light().max(500), 1.8, align, anchor);
    b.fade_in(job, 1.8, 0.5);
    let badge_at = if tall { b.at(0.15, 0.62) } else { b.at(0.44, 0.84) };
    let disc = b.shape(ShapeKind::Ellipse, Vec2::new(190.0, 190.0), badge_at, Some(Paint::Solid(Rgba::hex(0xffffff))), None, 0.0, 2.2, "Badge");
    b.pop_in(disc, 2.2);
    let words = b.text(badge, badge_at, 40.0, INK, b.heavy(), 2.3);
    b.pop_in(words, 2.3);
    let end = b.end();
    b.pulse(disc, 3.2, end - 1.0, 3);
    b.fade_everything_out();
    b.grain(30.0);
}

/// A sale under a spotlight: a huge headline, beams of colored light fanning up behind a framed
/// photo, the offer over the photo's corner and a button on its lower edge: headline, offer,
/// button.
pub(super) fn sale_spotlight(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (headline, offer, cta) = (b.copy(0), b.copy(1), b.copy(2));
    let frame = frame_units(b);
    let tall = b.tall();
    let size = if b.vertical() { 260.0 } else { 220.0 };
    // The headline across the top when tall, down the left in 16:9; the photo fills the rest.
    let (head_at, photo_size, photo_at) = if tall {
        let head_at = Vec2::new(b.width * 0.5, b.height * 0.03 + b.text_height(headline, size) * 0.5);
        let top = (head_at.y + b.text_height(headline, size) * 0.5) / b.height + 0.03;
        let height = (0.86 - top) * frame.y;
        (head_at, Vec2::new((frame.x * 0.66).min(height * 0.8), height), b.at(0.5, (top + 0.86) * 0.5))
    } else {
        (b.at(0.27, 0.5), Vec2::new(frame.y * 0.62, frame.y * 0.76), b.at(0.72, 0.47))
    };
    let head = b.text(headline, head_at, size, p.text, b.heavy(), 0.2);
    b.scale(head, 0.2, 125.0, 0.8, 100.0, EASE_OUT);
    b.fade_in(head, 0.2, 0.2);
    // Beams of light, each turning about its foot at the photo's middle.
    let beam_length = photo_size.y * 1.3;
    let colors = [p.accent, 0xff3b6b, 0xffd23f, 0x3bd16f, 0x3b7bff];
    for (index, color) in colors.into_iter().enumerate() {
        let foot = photo_at;
        let paint = Paint::linear(Rgba::hex(color).with_alpha(0.0), Rgba::hex(color).with_alpha(0.8), 90.0);
        let center = nudge(b, foot, Vec2::new(0.0, -beam_length * 0.5));
        let beam = b.shape(ShapeKind::Rectangle, Vec2::new(90.0, beam_length), center, Some(paint), None, 0.0, 0.5, "Beam");
        // Turning about its foot, which sits at the photo's middle.
        let (comp, unit) = (b.comp, b.unit());
        crate::edit::move_anchor(b.project.composition_mut(comp).expect("made"), beam, Vec2::new(45.0 * unit, beam_length * unit), 0);
        b.transform(beam).position = crate::animation::Animated::constant(foot);
        b.effect(beam, "Gaussian Blur", &[(0, 30.0)]);
        let angle = (index as f32 - 2.0) * 13.0;
        let keys = vec![b.key(0.5, 0.0, EASE_OUT), b.key(1.3, angle, EASE_IN_OUT), b.key(b.end(), angle * 1.15, Easing::Linear)];
        Builder::keys(&mut b.transform(beam).rotation, keys);
        b.fade_in(beam, 0.5, 0.6);
    }
    let rim = b.shape(ShapeKind::Rectangle, photo_size + Vec2::new(44.0, 44.0), photo_at, None, Some((p.text, 6.0)), 52.0, 0.9, "Frame");
    b.pop_in(rim, 0.9);
    let photo = photo_spot(b, ShapeKind::Rectangle, photo_size, photo_at, 32.0, 1.0, "model photo");
    b.pop_in(photo, 1.0);
    let corner = nudge(b, photo_at, Vec2::new(-photo_size.x * 0.5 + 40.0, photo_size.y * 0.5 - 90.0));
    let deal = anchored_text(b, offer, corner, 150.0, p.text, b.heavy(), 1.7, TextAlign::Left, (0.0, 1.0));
    b.effect(deal, "Drop Shadow", &[(0, 50.0), (2, 10.0), (3, 20.0)]);
    b.slide_in(deal, Vec2::new(-60.0, 0.0), 1.7, 0.6);
    let button_at = nudge(b, photo_at, Vec2::new(0.0, photo_size.y * 0.5));
    let width = cta.chars().count() as f32 * 30.0 + 120.0;
    let tag = b.block(Vec2::new(width, 92.0), button_at, 0xffffff, 0.0, 2.3, "Button");
    b.pop_in(tag, 2.3);
    let label = b.text(cta, button_at, 46.0, INK, b.heavy(), 2.4);
    b.pop_in(label, 2.4);
    b.fade_everything_out();
    b.grain(35.0);
}

/// A quiet product poster: a tall, thin title, two large circles at the sides, the product in
/// the middle and one line under it, on paper: title, line.
pub(super) fn minimal_product(b: &mut Builder) {
    let p = b.palette();
    let paper = flat_background(b, p.background);
    b.effect(paper, "Paper Texture", &[(0, 30.0), (1, 220.0)]);
    let (title, line) = (b.copy(0), b.copy(1));
    let frame = frame_units(b);
    let tone = mix(p.background, p.accent, 0.12);
    let diameter = frame.x.min(frame.y) * 0.95;
    for (x, from) in [(0.0, 0.1), (1.0, 0.25)] {
        let circle = b.shape(ShapeKind::Ellipse, Vec2::new(diameter, diameter), b.at(x, 0.62), Some(Paint::Solid(Rgba::hex(tone))), None, 0.0, from, "Backdrop circle");
        b.scale(circle, from, 0.0, from + 1.0, 100.0, EASE_OUT);
    }
    let size = if b.tall() { 230.0 } else { 200.0 };
    let head = b.text(title, b.at(0.5, 0.15), size, p.accent, b.heavy(), 0.4);
    b.letters_in(head, 1.0);
    let photo_size = if b.tall() { Vec2::new(frame.x * 0.5, frame.x * 0.62) } else { Vec2::new(frame.y * 0.46, frame.y * 0.58) };
    let photo = photo_spot(b, ShapeKind::Rectangle, photo_size, b.at(0.5, 0.56), 20.0, 0.7, "product photo");
    b.slide_in(photo, Vec2::new(0.0, 60.0), 0.7, 0.9);
    // The product drifts gently up and back while it holds.
    let end = b.end();
    let home = b.at(0.5, 0.56);
    let raised = nudge(b, home, Vec2::new(0.0, -18.0));
    b.move_to(photo, 1.6, end * 0.6, raised, EASE_IN_OUT);
    b.move_to(photo, end * 0.6, end - 0.6, home, EASE_IN_OUT);
    let under = (0.56 + photo_size.y * 0.5 * b.unit() / b.height + 0.05).min(0.93);
    let words = b.text(line, b.at(0.5, under), 42.0, p.muted, b.light(), 1.5);
    b.fade_in(words, 1.5, 0.7);
    b.fade_everything_out();
}

/// A market flyer over a full-frame photo: a script title with a strip of tape above it, and a
/// torn-paper note in the corner with what and when: tape word, title, what, when.
pub(super) fn tape_sale(b: &mut Builder) {
    let p = b.palette();
    full_frame_video(b, "photo or video");
    let (tape, title, what, when) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let frame = frame_units(b);
    let shade = Paint::linear(Rgba::hex(0x000000).with_alpha(0.5), Rgba::hex(0x000000).with_alpha(0.05), 90.0);
    b.shape(ShapeKind::Rectangle, frame, b.at(0.5, 0.5), Some(shade), None, 0.0, 0.0, "Shade");
    let head_y = if b.tall() { 0.25 } else { 0.32 };
    let head = b.text(title, b.at(0.5, head_y), 160.0, 0xffffff, b.heavy(), 0.7);
    b.words_in(head, 1.0);
    let tape_y = (head_y - b.text_height(title, 160.0) * 0.5 / b.height - 0.025).max(0.05);
    let tape_at = b.at(0.5, tape_y);
    let strip = b.block(Vec2::new(tape.chars().count() as f32 * 42.0 + 170.0, 96.0), tape_at, p.accent, 0.0, 0.3, "Tape");
    tilt(b, strip, -5.0);
    b.transform(strip).opacity.value = 94.0;
    b.pop_in(strip, 0.3);
    let tape_word = sans_text(b, tape, tape_at, 48.0, p.on_accent, 700, 0.35);
    tilt(b, tape_word, -5.0);
    b.pop_in(tape_word, 0.35);
    // The note: a rough-edged scrap of paper tucked in the lower right.
    let (note_size, note_at) = if b.tall() { (Vec2::new(frame.x * 0.5, frame.y * 0.24), b.at(0.74, 0.83)) } else { (Vec2::new(frame.x * 0.3, frame.y * 0.42), b.at(0.82, 0.74)) };
    let note = b.block(note_size, note_at, p.muted, 4.0, 1.3, "Paper note");
    b.effect(note, "Paper Texture", &[(0, 70.0), (1, 60.0)]);
    tilt(b, note, -4.0);
    b.slide_in(note, Vec2::new(300.0, 0.0), 1.3, 0.7);
    for (words, offset, at) in [(what, -note_size.y * 0.2, 1.6), (when, note_size.y * 0.22, 1.8)] {
        let place = nudge(b, note_at, Vec2::new(0.0, offset));
        let id = sans_text(b, words, place, 46.0, p.on_accent, 600, at);
        tilt(b, id, -4.0);
        b.fade_in(id, at, 0.5);
    }
    b.fade_everything_out();
    b.grain(20.0);
}

/// An event lineup of stickers: the title on an outlined pill, then three events, each on a
/// tilted pill with its date in a circle, arriving one by one, and the details in a band along
/// the bottom: title, then each event and its date, then the details.
pub(super) fn event_schedule(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let title = b.copy(0);
    let events = [(b.copy(1), b.copy(2)), (b.copy(3), b.copy(4)), (b.copy(5), b.copy(6))];
    let details = b.copy(7);
    let frame = frame_units(b);
    let tall = b.tall();
    let title_size = 110.0;
    let pill_height = b.text_height(title, title_size) / b.unit() + 70.0;
    let (title_at, title_width, row_x, row_width, rows_y) = if tall {
        let y = 0.03 + pill_height / frame.y * 0.5;
        (b.at(0.5, y), frame.x * 0.84, 0.5, frame.x * 0.68, [y + 0.25, y + 0.42, y + 0.59])
    } else {
        (b.at(0.27, 0.44), frame.x * 0.42, 0.7, frame.x * 0.4, [0.2, 0.43, 0.66])
    };
    let title_pill = b.shape(ShapeKind::Rectangle, Vec2::new(title_width, pill_height), title_at, Some(Paint::Solid(Rgba::hex(p.accent))), Some((INK, 8.0)), pill_height * 0.5, 0.2, "Title pill");
    tilt(b, title_pill, -3.0);
    b.pop_in(title_pill, 0.2);
    let head = b.text(title, title_at, title_size, INK, b.heavy(), 0.3);
    tilt(b, head, -3.0);
    b.pop_in(head, 0.3);
    let pills = [0xfff3d6, 0x6ec6ff, 0xfff3d6];
    let dates = [0x8fd694, 0xffd84a, 0x8fd694];
    for (index, ((name, date), y)) in events.into_iter().zip(rows_y).enumerate() {
        let at = 0.9 + index as f64 * 0.4;
        // Rows lean alternately, with the date at alternate ends.
        let toward = if index % 2 == 0 { -1.0 } else { 1.0 };
        let center = b.at(row_x + 0.03 * -toward * f32::from(tall), y);
        let pill = b.shape(ShapeKind::Rectangle, Vec2::new(row_width, 170.0), center, Some(Paint::Solid(Rgba::hex(pills[index]))), Some((INK, 6.0)), 85.0, at, "Event pill");
        tilt(b, pill, 3.0 * toward);
        b.slide_in(pill, Vec2::new(-500.0 * toward, 0.0), at, 0.6);
        let words_at = nudge(b, center, Vec2::new(-45.0 * toward, 0.0));
        let words = b.text(name, words_at, 58.0, INK, b.heavy(), at + 0.15);
        tilt(b, words, 3.0 * toward);
        b.slide_in(words, Vec2::new(-500.0 * toward, 0.0), at + 0.15, 0.6);
        let circle_at = nudge(b, center, Vec2::new(toward * (row_width * 0.5 - 40.0), 0.0));
        let circle = b.shape(ShapeKind::Ellipse, Vec2::new(150.0, 150.0), circle_at, Some(Paint::Solid(Rgba::hex(dates[index]))), Some((INK, 6.0)), 0.0, at + 0.3, "Date");
        b.pop_in(circle, at + 0.3);
        let day = b.text(date, circle_at, 40.0, INK, b.heavy(), at + 0.35);
        b.pop_in(day, at + 0.35);
        // A sparkle at the other end.
        let sparkle_at = nudge(b, center, Vec2::new(-toward * (row_width * 0.5 + 10.0), -60.0));
        let sparkle = b.star(4, 0.3, 110.0, sparkle_at, p.accent, at + 0.4, "Sparkle");
        b.pop_in(sparkle, at + 0.4);
        b.spin(sparkle, 0.0, 90.0 * toward);
    }
    let band = b.block(Vec2::new(frame.x, frame.y * 0.1), b.at(0.5, 0.95), p.accent, 0.0, 2.4, "Band");
    b.slide_in(band, Vec2::new(0.0, 120.0), 2.4, 0.5);
    let small = b.text(details, b.at(0.5, 0.95), 30.0, INK, b.light(), 2.6);
    b.fade_in(small, 2.6, 0.5);
    b.fade_everything_out();
}

/// A collection launch: two photo prints dropping onto each other over a starry dark ground,
/// with the words above and below: top line, bottom line.
pub(super) fn print_stack(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (top, bottom) = (b.copy(0), b.copy(1));
    let frame = frame_units(b);
    let end = b.end();
    // Faint stars, scattered the same way every time.
    for index in 0..40 {
        let pick = |seed: f32| ((index as f32 * seed).sin() * 43_758.547).fract().abs();
        let side = 3.0 + pick(3.1) * 6.0;
        let star = b.shape(ShapeKind::Ellipse, Vec2::new(side, side), b.at(pick(12.99), pick(78.23)), Some(Paint::Solid(Rgba::hex(0xffffff))), None, 0.0, 0.0, "Backdrop star");
        b.transform(star).opacity.value = 35.0 + pick(5.7) * 55.0;
        if index % 3 == 0 {
            b.pulse(star, pick(9.1) as f64, end, 4);
        }
    }
    let print_height = (frame.y * 0.58).min(frame.x * 0.74 / 0.8);
    let print_size = Vec2::new(print_height * 0.8, print_height);
    for (x, angle, at) in [(0.53, 8.0, 0.2), (0.48, -5.0, 0.55)] {
        let print = b.block(print_size, b.at(x, 0.5), 0xf4f1ea, 4.0, at, "Print");
        b.effect(print, "Drop Shadow", &[(0, 55.0), (2, 14.0), (3, 30.0)]);
        tilt(b, print, angle);
        b.slide_in(print, Vec2::new(0.0, -700.0), at, 0.7);
    }
    let photo = photo_spot(b, ShapeKind::Rectangle, print_size - Vec2::new(56.0, 56.0), b.at(0.48, 0.5), 0.0, 0.55, "model photo");
    tilt(b, photo, -5.0);
    b.slide_in(photo, Vec2::new(0.0, -700.0), 0.55, 0.7);
    let first = b.text(top, b.at(0.5, 0.08), 84.0, p.text, b.heavy(), 1.2);
    b.letters_in(first, 0.8);
    let last = b.text(bottom, b.at(0.5, 0.92), 84.0, p.text, b.heavy(), 1.6);
    b.letters_in(last, 0.6);
    b.fade_everything_out();
    b.grain(30.0);
}

/// An introduction: a name and what they do over a soft gradient, and their portrait in an oval
/// with sparkles around it: name, role.
pub(super) fn arch_portrait(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (name, role) = (b.copy(0), b.copy(1));
    let frame = frame_units(b);
    let (name_at, oval_size, oval_at) = if b.tall() {
        (b.at(0.5, 0.1), Vec2::new(frame.x * 0.74, frame.y * 0.62), b.at(0.5, 0.57))
    } else {
        (b.at(0.3, 0.42), Vec2::new(frame.y * 0.7, frame.y * 0.86), b.at(0.7, 0.5))
    };
    let title = b.text(name, name_at, 120.0, p.text, b.heavy(), 0.3);
    b.words_in(title, 0.8);
    let role_at = b.below(name_at, name, 120.0, 24.0, 52.0, name_at);
    let job = b.text(role, role_at, 52.0, p.muted, b.light(), 0.8);
    b.fade_in(job, 0.8, 0.6);
    let oval = b.shape(ShapeKind::Ellipse, oval_size, oval_at, Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, 0.2, "Oval");
    b.scale(oval, 0.2, 0.0, 0.9, 100.0, SPRING);
    let photo = photo_spot(b, ShapeKind::Ellipse, oval_size * 0.9, oval_at, 0.0, 0.5, "model photo");
    b.scale(photo, 0.5, 80.0, 1.2, 100.0, EASE_OUT);
    b.fade_in(photo, 0.5, 0.4);
    let end = b.end();
    for (index, (dx, dy, side)) in [(0.42, -0.36, 130.0), (-0.46, -0.1, 90.0), (0.47, 0.22, 100.0), (-0.36, 0.4, 70.0)].into_iter().enumerate() {
        let at = nudge(b, oval_at, Vec2::new(dx * oval_size.x, dy * oval_size.y));
        let from = 1.0 + index as f64 * 0.15;
        let sparkle = b.star(4, 0.28, side, at, INK, from, "Sparkle");
        b.pop_in(sparkle, from);
        b.pulse(sparkle, from + 0.6, end, 3);
    }
    b.fade_everything_out();
}

/// An offer in enormous type running off the frame, with a black-and-white photo over its
/// lower half and a handle along the photo's foot: offer, handle.
pub(super) fn huge_type_photo(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (offer, handle) = (b.copy(0), b.copy(1));
    let frame = frame_units(b);
    let lines = offer.lines().count().max(1) as f32;
    let (size, type_at, photo_size, photo_at) = if b.tall() {
        let size = frame.x * 0.62;
        (size, Vec2::new(b.width * 0.5, b.height * 0.03 + lines * size * 1.15 * b.unit() * 0.5), Vec2::new(frame.x * 0.74, frame.y * 0.58), b.at(0.5, 0.66))
    } else {
        (frame.y * 0.5, b.at(0.28, 0.5), Vec2::new(frame.x * 0.38, frame.y * 0.84), b.at(0.74, 0.5))
    };
    let big = b.text_unfitted(offer, type_at, size, p.accent, b.heavy(), 0.0, TextAlign::Center);
    b.scale(big, 0.0, 118.0, 0.7, 100.0, EASE_OUT);
    b.fade_in(big, 0.0, 0.25);
    let photo = photo_spot(b, ShapeKind::Rectangle, photo_size, photo_at, 0.0, 0.6, "model photo");
    b.effect(photo, "Black & White", &[(1, 30.0)]);
    b.slide_in(photo, Vec2::new(0.0, 200.0), 0.6, 0.8);
    let foot = nudge(b, photo_at, Vec2::new(0.0, photo_size.y * 0.5 - 50.0));
    let rule = b.block(Vec2::new(photo_size.x - 80.0, 3.0), nudge(b, foot, Vec2::new(0.0, -36.0)), 0xffffff, 0.0, 1.4, "Rule");
    b.scale(rule, 1.4, 0.0, 2.0, 100.0, EASE_OUT);
    let name = b.text(handle, foot, 34.0, 0xffffff, b.light(), 1.6);
    b.fade_in(name, 1.6, 0.5);
    b.fade_everything_out();
    b.grain(35.0);
}

/// A beauty or styling title over a full-frame portrait: a small spaced word, a large script
/// title and a line of small print, all centered: kicker, title, small print.
pub(super) fn script_beauty(b: &mut Builder) {
    full_frame_video(b, "model photo");
    let (kicker, title, small) = (b.copy(0), b.copy(1), b.copy(2));
    let frame = frame_units(b);
    tinted(b, frame, b.at(0.5, 0.5), 0x000000, 0.28, 0.0, 0.0, "Shade");
    let middle = b.at(0.5, 0.46);
    let head = b.text(title, middle, 190.0, 0xffffff, b.heavy(), 0.8);
    b.words_in(head, 1.1);
    let gap = b.text_height(title, 190.0) * 0.5;
    let above = Vec2::new(middle.x, middle.y - gap - 30.0 * b.unit());
    let top = sans_text(b, kicker, above, 54.0, 0xffffff, 500, 0.4);
    b.slide_in(top, Vec2::new(0.0, 20.0), 0.4, 0.6);
    let below = Vec2::new(middle.x, middle.y + gap + 40.0 * b.unit());
    let fine = sans_text(b, small, below, 26.0, 0xffffff, 500, 1.6);
    b.fade_in(fine, 1.6, 0.6);
    b.fade_everything_out();
    b.grain(22.0);
}

/// Tips on a black ground between two torn strips of gray paper, with a burst in the accent
/// color, and worn-film dust and scratches over it all: brand, then three tips.
pub(super) fn grunge_tips(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let brand = b.copy(0);
    let tips = [b.copy(1), b.copy(2), b.copy(3)];
    let frame = frame_units(b);
    let tall = b.tall();
    let (band_height, top_y, bottom_y) = if tall { (190.0, 0.11, 0.9) } else { (140.0, 0.07, 0.93) };
    for (y, angle, at) in [(top_y, -2.0, 0.0), (bottom_y, 2.0, 0.15)] {
        let strip = b.block(Vec2::new(frame.x * 1.2, band_height), b.at(0.5, y), 0x8c8c8c, 0.0, at, "Paper strip");
        b.effect(strip, "Paper Texture", &[(0, 100.0), (1, 30.0)]);
        tilt(b, strip, angle);
        b.slide_in(strip, Vec2::new(if angle < 0.0 { -400.0 } else { 400.0 }, 0.0), at, 0.6);
    }
    let tag_at = b.at(0.5, top_y);
    let tag = b.block(Vec2::new(brand.chars().count() as f32 * 26.0 + 90.0, 64.0), tag_at, 0x0b0b0b, 0.0, 0.5, "Tag");
    b.pop_in(tag, 0.5);
    let tag_words = b.text(brand, tag_at, 34.0, 0xffffff, b.heavy(), 0.55);
    b.pop_in(tag_words, 0.55);
    // The burst above the list when tall (smaller in 4:5, which is shorter), beside it in 16:9.
    let (burst_at, burst_size, left, first_y, size) = if b.vertical() {
        (b.at(0.5, 0.31), 300.0, 0.12, 0.46, 70.0)
    } else if tall {
        (b.at(0.5, 0.28), 240.0, 0.12, 0.41, 56.0)
    } else {
        (b.at(0.22, 0.5), 360.0, 0.42, 0.26, 62.0)
    };
    let burst = b.star(14, 0.55, burst_size, burst_at, p.accent, 0.7, "Burst");
    b.pop_in(burst, 0.7);
    b.spin(burst, 0.0, 40.0);
    let mut y = b.height * first_y;
    for (index, tip) in tips.into_iter().enumerate() {
        let at = 1.2 + index as f64 * 0.5;
        let x = b.width * left;
        let first_line = y + size * 1.15 * b.unit() * 0.5;
        let dot = b.shape(ShapeKind::Ellipse, Vec2::new(18.0, 18.0), Vec2::new(x, first_line), Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, at, "Bullet");
        b.pop_in(dot, at);
        let words = anchored_text(b, tip, Vec2::new(x + 40.0 * b.unit(), y), size, p.text, b.heavy(), at, TextAlign::Left, (0.0, 0.0));
        b.words_in(words, 0.6);
        y += b.text_height(tip, size) + 44.0 * b.unit();
    }
    b.fade_everything_out();
    b.overlay("Old Film", "Worn film", &[(0, 30.0), (1, 60.0), (2, 40.0), (3, 25.0)]);
    b.grain(30.0);
}

/// A keepsake: a photo in a gold frame with a peach mount, aged like an old print, over sparkles,
/// with a script caption and a date: caption, date.
pub(super) fn framed_memory(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (caption, date) = (b.copy(0), b.copy(1));
    let frame = frame_units(b);
    let (frame_height, frame_at, caption_size) = if b.tall() {
        let height = (frame.x * 0.72 * 1.22).min(frame.y * 0.56);
        (height, b.at(0.5, 0.42), if b.vertical() { 130.0 } else { 100.0 })
    } else {
        (frame.y * 0.8, b.at(0.33, 0.5), 130.0)
    };
    let outer = Vec2::new(frame_height / 1.22, frame_height);
    let gold = Paint::linear(Rgba::hex(0xf7dc8b), Rgba::hex(0x9c6b1f), 45.0);
    let rim = b.shape(ShapeKind::Rectangle, outer, frame_at, Some(gold), Some((0x6b4a12, 4.0)), 6.0, 0.2, "Gold frame");
    b.effect(rim, "Drop Shadow", &[(0, 60.0), (2, 16.0), (3, 36.0)]);
    let mount = b.block(outer - Vec2::new(70.0, 70.0), frame_at, 0xf3c9a8, 2.0, 0.2, "Mount");
    let photo = photo_spot(b, ShapeKind::Rectangle, outer - Vec2::new(140.0, 140.0), frame_at, 0.0, 0.2, "photo");
    b.effect(photo, "Old Film", &[(0, 40.0), (1, 25.0), (2, 20.0), (3, 15.0)]);
    for id in [rim, mount, photo] {
        b.scale(id, 0.2, 90.0, 1.1, 100.0, EASE_OUT);
        b.fade_in(id, 0.2, 0.5);
    }
    // Under the frame when tall, beside it in 16:9.
    let caption_at = if b.tall() {
        Vec2::new(frame_at.x, frame_at.y + (frame_height * 0.5 + 40.0) * b.unit() + b.text_height(caption, caption_size) * 0.5)
    } else {
        b.at(0.72, 0.45)
    };
    let words = b.text(caption, caption_at, caption_size, p.accent, b.heavy(), 1.0);
    b.words_in(words, 1.0);
    let date_at = Vec2::new(caption_at.x, caption_at.y + b.text_height(caption, caption_size) * 0.5 + 40.0 * b.unit());
    let when = sans_text(b, date, date_at, 32.0, p.muted, 500, 1.7);
    b.fade_in(when, 1.7, 0.6);
    b.fade_everything_out();
    b.grain(25.0);
}
