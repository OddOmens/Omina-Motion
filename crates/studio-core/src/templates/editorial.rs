//! Layouts in the editorial style of today's motion design galleries: one or two flat colors,
//! very large type that fills the frame, small details tucked into the corners, and quick,
//! snappy moves. Type that stretches up into place, color blocks that slide across and away,
//! one word repeated down the frame, photo cards that stack, numbers that fill the screen, a
//! title spinning out of a blur, a photo uncovered by shutters, gradient bars that drift, big
//! words echoing across a photo, and a word whose full stop bounces in.

use super::social::{full_frame_video, outline, set_fill, tinted};
use super::{Builder, EASE_IN, EASE_IN_OUT, EASE_OUT, SPRING};
use crate::animation::{Animated, CubicBezier, EaseMode, Easing, Vec2};
use crate::model::{Id, Paint, Rgba, ShapeKind, TextAlign};

/// A very quick start that lands crisply: for type stretching and blocks snapping into place.
const SNAP: Easing = Easing::Curve(CubicBezier::new(0.2, 0.9, 0.1, 1.0));

/// One flat color over the whole frame, named "Background" so the ending leaves it in place.
fn flat_background(b: &mut Builder, color: u32) -> Id {
    let size = Vec2::new(b.width, b.height) * (1.0 / b.unit());
    let center = b.at(0.5, 0.5);
    b.block(size, center, color, 0.0, 0.0, "Background")
}

/// The frame's margins, as fractions: left and right, top, and the lowest a line may sit. The
/// tall phone shape keeps clear of the apps' captions and buttons at the bottom.
fn margins(b: &Builder) -> (f32, f32, f32) {
    let side = 72.0 * b.unit() / b.width;
    if b.vertical() {
        (side, 0.1, 0.8)
    } else {
        (side, 72.0 * b.unit() / b.height, 1.0 - 72.0 * b.unit() / b.height)
    }
}

/// A text layer whose anchor is the point (`fx`, `fy`) of its box (0 to 1 each way, from the top
/// left), placed so that point sits at `at`. A bottom anchor lets type stretch up from its baseline.
#[allow(clippy::too_many_arguments)]
fn anchored_text(b: &mut Builder, words: &str, at: Vec2, size: f32, color: u32, weight: u16, from: f64, align: TextAlign, (fx, fy): (f32, f32)) -> Id {
    let id = b.text_aligned(words, at, size, color, weight, from, align);
    let box_size = b.layer(id).size;
    let comp = b.comp;
    crate::edit::move_anchor(b.project.composition_mut(comp).expect("made"), id, Vec2::new(box_size.x * fx, box_size.y * fy), 0);
    b.transform(id).position = Animated::constant(at);
    id
}

/// A small line of detail text, the kind tucked into a poster's corners.
fn corner_note(b: &mut Builder, words: &str, at: Vec2, color: u32, from: f64, align: TextAlign) -> Id {
    let anchor = match align {
        TextAlign::Left => (0.0, 0.5),
        TextAlign::Right => (1.0, 0.5),
        _ => (0.5, 0.5),
    };
    let weight = b.light().max(500);
    let id = anchored_text(b, words, at, 36.0, color, weight, from, align, anchor);
    b.fade_in(id, from, 0.35);
    id
}

/// Stretches a layer up from nothing (its height only), overshooting a little: type snapping
/// into place. Anchor it at the bottom to grow from the baseline.
fn stretch_in(b: &mut Builder, id: Id, at: f64) {
    let keys = vec![b.key(at, Vec2::new(100.0, 0.0), SNAP), b.key(at + 0.28, Vec2::new(100.0, 112.0), EASE_IN_OUT), b.key(at + 0.5, Vec2::new(100.0, 100.0), Easing::Linear)];
    Builder::keys(&mut b.transform(id).scale, keys);
    b.fade_in(id, at, 0.08);
}

/// A quick squash and spring back, as if the layer were tapped: a beat in the middle of a hold.
fn squash(b: &mut Builder, id: Id, at: f64) {
    let keys = vec![b.key(at, Vec2::new(100.0, 100.0), EASE_IN), b.key(at + 0.14, Vec2::new(104.0, 72.0), SPRING), b.key(at + 0.55, Vec2::new(100.0, 100.0), Easing::Linear)];
    Builder::keys(&mut b.transform(id).scale, keys);
}

/// A rounded pill in `fill` with `words` on it in `ink`, popping in at `at`.
fn pill(b: &mut Builder, words: &str, at: Vec2, fill: u32, ink: u32, from: f64) -> (Id, Id) {
    let weight = b.heavy().min(700);
    let width = (words.chars().count() as f32 * 46.0 * 0.6 + 110.0).max(240.0);
    let back = b.block(Vec2::new(width, 104.0), at, fill, 52.0, from, "Button");
    let label = b.text(words, at, 46.0, ink, weight, from);
    for id in [back, label] {
        b.pop_in(id, from);
    }
    (back, label)
}

/// A big stacked headline, one line at a time stretching up into place, on a flat color, with a
/// date, a place and a tag in the corners: headline (lines), date, place, tag.
pub(super) fn poster_stretch(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (side, top, bottom) = margins(b);
    let (headline, date, place, tag) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let size = if b.vertical() { 240.0 } else if b.tall() { 210.0 } else { 175.0 };
    let unit = b.unit();
    let step = size * 0.98 * unit;
    let lines: Vec<&str> = headline.lines().collect();
    // The block of lines sits a little above the middle, its left edge on the margin.
    let first_baseline = b.height * 0.48 - step * lines.len() as f32 * 0.5 + step;
    let mut stretched = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let at = 0.25 + index as f64 * 0.13;
        let baseline = Vec2::new(b.width * side, first_baseline + step * index as f32);
        let id = anchored_text(b, line, baseline, size, p.text, b.heavy(), at, TextAlign::Left, (0.0, 1.0));
        stretch_in(b, id, at);
        stretched.push(id);
    }
    // A rule under the block, filling across from the left.
    let rule_y = (first_baseline + step * (lines.len() as f32 - 1.0)) / b.height + 40.0 * unit / b.height;
    let rule = b.block(Vec2::new((b.width / unit) * (1.0 - side * 2.0), 8.0), b.at(0.5, rule_y), p.accent, 0.0, 0.9, "Rule");
    b.anchor_left(rule);
    b.transform(rule).position = Animated::constant(b.at(side, rule_y));
    b.grow_across(rule, 0.9, 0.7);
    corner_note(b, date, b.at(side, top), p.text, 1.1, TextAlign::Left);
    corner_note(b, tag, b.at(1.0 - side, top), p.accent, 1.2, TextAlign::Right);
    corner_note(b, place, b.at(side, bottom), p.muted, 1.3, TextAlign::Left);
    // A snap through the lines in the middle of the hold keeps it alive.
    for (index, id) in stretched.into_iter().enumerate() {
        squash(b, id, 3.6 + index as f64 * 0.08);
    }
    b.fade_everything_out();
}

/// Bands of color slide in from alternate sides to fill the frame, then slide on out to
/// uncover the title: kicker, title, date.
pub(super) fn sliding_blocks(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (kicker, title, date) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let colors = [p.accent, p.background_2, p.text, p.accent];
    let band = b.height / colors.len() as f32;
    let full_width = b.width / unit;
    for (index, &color) in colors.iter().enumerate() {
        let y = (index as f32 + 0.5) / colors.len() as f32;
        let home = b.at(0.5, y);
        let block = b.block(Vec2::new(full_width, band / unit + 2.0), home, color, 0.0, 0.0, "Block");
        // In from one side, out the other, each a beat after the band above it.
        let side = if index % 2 == 0 { -1.0 } else { 1.0 };
        let away_in = home + Vec2::new(b.width * side, 0.0);
        let away_out = home - Vec2::new(b.width * side, 0.0);
        let (arrive, leave) = (0.1 + index as f64 * 0.08, 1.35 + index as f64 * 0.07);
        let keys = vec![b.key(arrive, away_in, SNAP), b.key(arrive + 0.5, home, Easing::Linear), b.key(leave, home, EASE_IN), b.key(leave + 0.45, away_out, Easing::Linear)];
        Builder::keys(&mut b.transform(block).position, keys);
        b.until(block, leave + 0.45);
    }
    let size = if b.vertical() { 150.0 } else { 140.0 };
    let center = b.at(0.5, 0.48);
    let heading = b.text(title, center, size, p.text, b.heavy(), 1.6);
    b.words_in(heading, 0.6);
    let above = center - Vec2::new(0.0, b.text_height(title, size) * 0.5 + 60.0 * unit);
    let label = b.text(kicker, above, 38.0, p.accent, b.heavy().min(700), 1.55);
    b.slide_in(label, Vec2::new(0.0, 30.0), 1.55, 0.5);
    let under = b.below(center, title, size, 50.0, 40.0, b.at(0.5, 0.0));
    let when = b.text(date, under, 40.0, p.muted, b.light().max(500), 2.0);
    b.fade_in(when, 2.0, 0.5);
    b.fade_everything_out();
}

/// One word repeated down the frame in outline, the middle row filled in, rows drifting in
/// opposite directions: word, caption.
pub(super) fn type_repeater(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (word, caption) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let rows: i32 = if b.vertical() { 9 } else if b.tall() { 7 } else { 5 };
    let middle = rows / 2;
    let step = b.height / rows as f32;
    let size = step / unit / 1.15 * 0.98;
    let line = [word; 6].join("  ");
    let end = b.end();
    let drift = b.width * 0.18;
    for row in 0..rows {
        let y = (row as f32 + 0.5) / rows as f32;
        let from = 0.15 + f64::from((row - middle).unsigned_abs()) * 0.07;
        let filled = row == middle;
        let id = b.text_unfitted(&line, b.at(0.5, y), size, if filled { p.accent } else { p.text }, b.heavy(), from, TextAlign::Center);
        if !filled {
            set_fill(b, id, None);
            outline(b, id, p.text, 3.0);
            b.transform(id).opacity.value = 70.0;
        }
        // Rows drift steadily, alternating direction.
        let direction = if row % 2 == 0 { 1.0 } else { -1.0 };
        let center = b.at(0.5, y);
        let keys = vec![b.key(from, center - Vec2::new(drift * direction, 0.0), Easing::Linear), b.key(end, center + Vec2::new(drift * direction, 0.0), Easing::Linear)];
        Builder::keys(&mut b.transform(id).position, keys);
        stretch_in(b, id, from);
        if filled {
            squash(b, id, 2.2);
            squash(b, id, 5.0);
        }
    }
    // The caption on a block of the background color, so it reads over the rows.
    let (_, _, bottom) = margins(b);
    let caption_at = b.at(0.5, bottom - 0.02);
    let width = caption.chars().count() as f32 * 40.0 * 0.6 + 120.0;
    let back = b.block(Vec2::new(width, 96.0), caption_at, p.background, 48.0, 2.4, "Caption back");
    b.pop_in(back, 2.4);
    let words = b.text(caption, caption_at, 40.0, p.text, b.heavy().min(700), 2.5);
    b.fade_in(words, 2.5, 0.4);
    b.fade_everything_out();
}

/// Photo cards dropping onto a stack, each a little turned, then a quote card on top: quote,
/// name, role.
pub(super) fn card_stack(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (quote, name, role) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let wide = !b.tall();
    let card = if b.vertical() { Vec2::new(720.0, 940.0) } else if b.tall() { Vec2::new(640.0, 760.0) } else { Vec2::new(560.0, 700.0) };
    let stack_at = if wide { b.at(0.32, 0.5) } else { b.at(0.5, 0.4) };
    for (index, turn) in [-9.0_f32, 7.0, -2.0].into_iter().enumerate() {
        let at = 0.2 + index as f64 * 0.28;
        let photo = b.placeholder(card, stack_at, 28.0, at, "photo");
        b.effect(photo, "Drop Shadow", &[(0, 45.0), (2, 30.0), (3, 18.0)]);
        // Drops in from below, turning a little more than it ends.
        let start = stack_at + Vec2::new(0.0, b.height * 0.8);
        let keys = vec![b.key(at, start, EASE_OUT), b.key(at + 0.6, stack_at, Easing::Linear)];
        Builder::keys(&mut b.transform(photo).position, keys);
        let turns = vec![b.key(at, turn * 2.5, EASE_OUT), b.key(at + 0.7, turn, Easing::Linear)];
        Builder::keys(&mut b.transform(photo).rotation, turns);
    }
    // The quote: dark words on a nearly white card, so it reads on any color.
    let quote_size = if wide { 58.0 } else { 66.0 };
    let card_width = if wide { 760.0 } else { (b.width / unit - 140.0).min(880.0) };
    let card_height = b.text_height(quote, quote_size) / unit + 220.0;
    let card_at = if wide { b.at(0.7, 0.5) } else { b.at(0.5, if b.vertical() { 0.66 } else { 0.72 }) };
    let back = tinted(b, Vec2::new(card_width, card_height), card_at, 0xffffff, 0.94, 36.0, 1.3, "Quote card");
    b.effect(back, "Drop Shadow", &[(0, 35.0), (2, 24.0), (3, 12.0)]);
    b.slide_in(back, Vec2::new(0.0, 90.0), 1.3, 0.6);
    let words_at = card_at - Vec2::new(0.0, 40.0 * unit);
    let said = b.text(quote, words_at, quote_size, 0x111111, b.heavy().min(700), 1.6);
    b.words_in(said, 0.9);
    let by_at = card_at + Vec2::new(0.0, card_height * 0.5 * unit - 70.0 * unit);
    let by = b.text(&format!("{name} · {role}"), by_at, 40.0, p.accent, 600, 2.4);
    b.fade_in(by, 2.4, 0.5);
    b.fade_everything_out();
}

/// 3, 2, 1 filling the screen, each on its own color, then the word: word, caption.
pub(super) fn bold_countdown(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (word, caption) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let full = Vec2::new(b.width, b.height) * (1.0 / unit);
    let center = b.at(0.5, 0.5);
    let number_size = if b.tall() { 900.0 } else { 760.0 };
    let beats = [("3", p.accent, p.on_accent), ("2", p.text, p.background), ("1", p.background_2, p.text)];
    for (index, (number, back, ink)) in beats.into_iter().enumerate() {
        let at = 0.15 + index as f64 * 0.7;
        let panel = b.block(full, center, back, 0.0, at, "Panel");
        let low = center + Vec2::new(0.0, b.height);
        let keys = vec![b.key(at, low, SNAP), b.key(at + 0.3, center, Easing::Linear)];
        Builder::keys(&mut b.transform(panel).position, keys);
        b.until(panel, at + 0.7);
        let digit = anchored_text(b, number, center + Vec2::new(0.0, number_size * 0.42 * unit), number_size, ink, b.heavy(), at + 0.1, TextAlign::Center, (0.5, 1.0));
        stretch_in(b, digit, at + 0.1);
        b.until(digit, at + 0.7);
    }
    let size = if b.vertical() { 230.0 } else { 200.0 };
    let reveal = 0.15 + 3.0 * 0.7;
    let heading = anchored_text(b, word, center, size, p.text, b.heavy(), reveal, TextAlign::Center, (0.5, 0.5));
    b.letters_in(heading, 0.5);
    b.scale(heading, reveal, 125.0, reveal + 0.6, 100.0, EASE_OUT);
    let under = b.below(center, word, size, 50.0, 44.0, b.at(0.5, 0.0));
    let line = b.text(caption, under, 44.0, p.muted, b.light().max(500), reveal + 0.6);
    b.words_in(line, 0.5);
    b.fade_everything_out();
}

/// A title spinning in out of a blur while rings ripple out behind it: headline, line.
pub(super) fn blur_spin(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (headline, line) = (b.copy(0), b.copy(1));
    let center = b.at(0.5, 0.48);
    let end = b.end();
    // Rings, one every 0.6 s, each growing out and fading.
    let mut at = 0.2;
    while at < end - 1.0 {
        let ring = b.shape(ShapeKind::Ellipse, Vec2::new(260.0, 260.0), center, None, Some((p.accent, 3.0)), 0.0, at, "Ring");
        b.scale(ring, at, 40.0, at + 2.6, 720.0, EASE_OUT);
        let fade = vec![b.key(at, 0.0, Easing::Linear), b.key(at + 0.3, 70.0, Easing::Linear), b.key(at + 2.6, 0.0, Easing::Linear)];
        Builder::keys(&mut b.transform(ring).opacity, fade);
        b.until(ring, at + 2.6);
        at += 0.6;
    }
    let size = if b.vertical() { 140.0 } else { 130.0 };
    let title = b.text(headline, center, size, p.text, b.heavy(), 0.4);
    let turn = vec![b.key(0.4, -40.0, EASE_OUT), b.key(1.5, 0.0, Easing::Linear)];
    Builder::keys(&mut b.transform(title).rotation, turn);
    b.scale(title, 0.4, 170.0, 1.5, 100.0, EASE_OUT);
    b.fade_in(title, 0.4, 0.4);
    b.effect(title, "Gaussian Blur", &[(0, 60.0)]);
    b.animate_effect(title, 0, &[(0.4, 60.0, EASE_OUT), (1.4, 0.0, Easing::Linear)]);
    let under = b.below(center, headline, size, 50.0, 44.0, b.at(0.5, 0.0));
    let sub = b.text(line, under, 44.0, p.muted, b.light().max(500), 1.5);
    b.words_in(sub, 0.5);
    b.fade_everything_out();
}

/// A photo uncovered by shutters sliding up and down, then a title on a band: title, kicker.
pub(super) fn split_reveal(b: &mut Builder) {
    let p = b.palette();
    full_frame_video(b, "photo or video");
    let (title, kicker) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let strips = 4;
    let strip_width = b.width / strips as f32 / unit + 2.0;
    let full_height = b.height / unit;
    for index in 0..strips {
        let x = (index as f32 + 0.5) / strips as f32;
        let home = b.at(x, 0.5);
        let color = if index % 2 == 0 { p.accent } else { p.background };
        let shutter = b.block(Vec2::new(strip_width, full_height), home, color, 0.0, 0.0, "Shutter");
        let away = home + Vec2::new(0.0, if index % 2 == 0 { -b.height } else { b.height });
        let at = 0.35 + index as f64 * 0.09;
        let keys = vec![b.key(at, home, EASE_IN_OUT), b.key(at + 0.7, away, Easing::Linear)];
        Builder::keys(&mut b.transform(shutter).position, keys);
        b.until(shutter, at + 0.7);
    }
    // The title band, low in the frame but clear of the apps' buttons.
    let (side, _, bottom) = margins(b);
    let size = if b.vertical() { 120.0 } else { 110.0 };
    let band_height = b.text_height(title, size) / unit + 120.0;
    let band_y = bottom - band_height * unit * 0.5 / b.height;
    let band = b.block(Vec2::new(b.width / unit, band_height), b.at(0.5, band_y), p.background, 0.0, 1.2, "Band");
    b.slide_in(band, Vec2::new(0.0, 160.0), 1.2, 0.5);
    let heading = anchored_text(b, title, b.at(side, band_y), size, p.text, b.heavy(), 1.45, TextAlign::Left, (0.0, 0.5));
    b.words_in(heading, 0.6);
    let label_y = band_y - (band_height * 0.5 + 50.0) * unit / b.height;
    let (back, label) = pill(b, kicker, b.at(0.5, label_y), p.accent, p.on_accent, 1.8);
    for id in [back, label] {
        let at = b.at(side, label_y) + Vec2::new(b.layer(back).size.x * 0.5, 0.0);
        b.transform(id).position = Animated::constant(at);
    }
    b.fade_everything_out();
}

/// Gradient bars drifting up and down behind a huge title: title, line.
pub(super) fn gradient_title(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (title, line) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let end = b.end();
    let bars = if b.tall() { 5 } else { 7 };
    let bar_width = b.width / bars as f32 / unit + 2.0;
    let bar_height = b.height / unit * 1.7;
    for index in 0..bars {
        let x = (index as f32 + 0.5) / bars as f32;
        let paint = Paint::linear(Rgba::hex(p.background), Rgba::hex(p.background_2), 90.0);
        // Named as a color panel, so the ending leaves the bars in place.
        let bar = b.shape(ShapeKind::Rectangle, Vec2::new(bar_width, bar_height), b.at(x, 0.5), Some(paint), None, 0.0, 0.0, "Panel");
        // Each bar drifts up and down, out of step with its neighbours.
        let lift = b.height * 0.22;
        let phase = if index % 2 == 0 { 1.0 } else { -1.0 };
        let keys: Vec<_> = (0..=4).map(|step| {
            let at = end * step as f64 / 4.0;
            let up = if step % 2 == 0 { phase } else { -phase };
            b.key(at, b.at(x, 0.5) + Vec2::new(0.0, lift * up * (0.6 + 0.4 * (index % 3) as f32 / 2.0)), EASE_IN_OUT)
        }).collect();
        Builder::keys(&mut b.transform(bar).position, keys);
        b.fade_in(bar, index as f64 * 0.05, 0.4);
    }
    let size = if b.vertical() { 260.0 } else if b.tall() { 240.0 } else { 220.0 };
    let center = b.at(0.5, 0.46);
    let heading = b.text(title, center, size, 0xffffff, b.heavy(), 0.5);
    b.letters_in(heading, 0.7);
    b.scale(heading, 0.5, 115.0, 1.3, 100.0, EASE_OUT);
    let (_, _, bottom) = margins(b);
    let note = b.text(line, b.at(0.5, bottom - 0.02), 40.0, 0xffffff, b.heavy().min(700), 1.5);
    b.fade_in(note, 1.5, 0.5);
    b.fade_everything_out();
}

/// Two huge words over a photo stretching into place, with outlined copies gliding across
/// above and below them, a brand name at the top and a button: word, word, brand, button.
pub(super) fn photo_tagline(b: &mut Builder) {
    let p = b.palette();
    full_frame_video(b, "photo or video");
    let size = Vec2::new(b.width, b.height) * (1.0 / b.unit());
    let center = b.at(0.5, 0.5);
    tinted(b, size, center, 0x000000, 0.3, 0.0, 0.0, "Shade");
    let (first, second, brand, button) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let end = b.end();
    let unit = b.unit();
    let word_size = if b.vertical() { 230.0 } else if b.tall() { 200.0 } else { 170.0 };
    let row = word_size * 1.0 * unit / b.height;
    let middle = 0.46;
    // The echoes: each word in outline, one row above and one below, gliding opposite ways.
    for (index, word) in [first, second].into_iter().enumerate() {
        let y = if index == 0 { middle - row * 1.5 } else { middle + row * 1.5 };
        let at = b.at(0.5, y);
        let echo = b.text_unfitted(&[word; 3].join("  "), at, word_size, 0xffffff, b.heavy(), 0.1, TextAlign::Center);
        set_fill(b, echo, None);
        outline(b, echo, 0xffffff, 3.0);
        b.transform(echo).opacity.value = 60.0;
        let drift = b.width * 0.3 * if index == 0 { 1.0 } else { -1.0 };
        let keys = vec![b.key(0.1, at + Vec2::new(drift, 0.0), Easing::Linear), b.key(end, at - Vec2::new(drift, 0.0), Easing::Linear)];
        Builder::keys(&mut b.transform(echo).position, keys);
        b.fade_in(echo, 0.1, 0.5);
    }
    // The words themselves, solid, stretching up into place one after the other.
    for (index, word) in [first, second].into_iter().enumerate() {
        let y = middle + if index == 0 { -row * 0.5 } else { row * 0.5 };
        let at = 0.5 + index as f64 * 0.18;
        let baseline = b.at(0.5, y) + Vec2::new(0.0, word_size * 0.5 * unit);
        let solid = anchored_text(b, word, baseline, word_size, 0xffffff, b.heavy(), at, TextAlign::Center, (0.5, 1.0));
        stretch_in(b, solid, at);
        squash(b, solid, 3.4 + index as f64 * 0.1);
    }
    let (_, top, bottom) = margins(b);
    let name = b.text(brand, b.at(0.5, top + 0.02), 40.0, 0xffffff, b.heavy().min(700), 1.0);
    b.fade_in(name, 1.0, 0.5);
    let (back, _) = pill(b, button, b.at(0.5, bottom - 0.04), p.accent, p.on_accent, 1.6);
    b.pulse(back, 2.6, end - 1.0, 3);
    b.fade_everything_out();
}

/// A big word, then its full stop dropping in and bouncing to rest, then the dot swelling to
/// fill the frame for the closing line: word, line.
pub(super) fn bouncy_period(b: &mut Builder) {
    let p = b.palette();
    flat_background(b, p.background);
    let (word, line) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let end = b.end();
    let size = if b.vertical() { 250.0 } else { 260.0 };
    let dot_size = size * 0.26;
    // The word sits left of center, so word and dot together are centered.
    let baseline_y = b.height * 0.52;
    let heading = anchored_text(b, word, Vec2::new(b.width * 0.5, baseline_y), size, p.text, b.heavy(), 0.3, TextAlign::Center, (0.5, 1.0));
    let word_width = b.layer(heading).size.x;
    let shift = (dot_size * 1.1 * unit) * 0.5;
    b.transform(heading).position = Animated::constant(Vec2::new(b.width * 0.5 - shift, baseline_y));
    stretch_in(b, heading, 0.3);
    // The dot rests on the baseline just after the last letter.
    // The box's bottom sits below the letters' baseline by the room left for descenders.
    let rest = Vec2::new(b.width * 0.5 - shift + word_width * 0.5 + dot_size * 0.75 * unit, baseline_y - size * 0.31 * unit);
    let dot = b.shape(ShapeKind::Ellipse, Vec2::new(dot_size, dot_size), rest, Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, 0.9, "Dot");
    let high = Vec2::new(rest.x, -dot_size * unit);
    let fall = vec![b.key(0.9, high, Easing::Bounce(EaseMode::Out)), b.key(2.0, rest, Easing::Linear)];
    Builder::keys(&mut b.transform(dot).position, fall);
    // Near the end the dot swells to fill the frame, and the line appears on it.
    let swell = end - 2.2;
    let cover = (b.width.hypot(b.height) / (dot_size * unit)) * 100.0 * 2.2;
    b.scale(dot, swell, 100.0, swell + 0.8, cover, EASE_IN);
    let closing = b.text(line, b.at(0.5, 0.5), 64.0, p.on_accent, b.heavy().min(700), swell + 0.7);
    b.words_in(closing, 0.6);
    b.fade_out(closing, end - 0.05, 0.4);
}
