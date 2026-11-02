//! Layouts after the social and intro videos people make most on TikTok, Reels and Shorts:
//! a "meet me" profile card, an Ask Me Anything sticker, a name spelled out letter by letter, a
//! story frame, a reply-to-a-comment sticker, a greeting with rising hearts, huge outlined words
//! over a photo, and a bouncy vlog title. Like the others, each places its layers by fractions
//! of the frame and adapts to the three shapes.

use super::{Builder, EASE_IN_OUT, EASE_OUT, SPRING};
use crate::animation::{Animated, Easing, Vec2};
use crate::model::{Id, Paint, Rgba, ShapeKind, TextAlign};

/// A round spot for a photo (a profile picture), filled like the other drop spots.
fn round_photo(b: &mut Builder, diameter: f32, at: Vec2, from: f64, what: &str) -> Id {
    let fill = Paint::Linear { start: Rgba::hex(0x3f3f46), end: Rgba::hex(0x27272a), angle: 60.0 };
    b.shape(ShapeKind::Ellipse, Vec2::new(diameter, diameter), at, Some(fill), Some((0x71717a, 2.0)), 0.0, from, &format!("Drop your {what} here"))
}

/// A drop spot for footage filling the whole frame.
fn full_frame_video(b: &mut Builder, what: &str) -> Id {
    let size = Vec2::new(b.width, b.height) * (1.0 / b.unit());
    let center = b.at(0.5, 0.5);
    b.placeholder(size, center, 0.0, 0.0, what)
}

/// A see-through rectangle: `color` at `alpha` (0–1). The transparency is in the color, so
/// fades and slides (which animate opacity) keep it.
#[allow(clippy::too_many_arguments)]
fn tinted(b: &mut Builder, size: Vec2, at: Vec2, color: u32, alpha: f32, corner: f32, from: f64, name: &str) -> Id {
    b.shape(ShapeKind::Rectangle, size, at, Some(Paint::Solid(Rgba::hex(color).with_alpha(alpha))), None, corner, from, name)
}

/// Gives a text layer an outline `width` units thick.
fn outline(b: &mut Builder, id: Id, color: u32, width: f32) {
    let (comp, unit) = (b.comp, b.unit());
    crate::edit::edit_text(b.project.composition_mut(comp).expect("made"), id, |_, style| style.stroke = Some((Rgba::hex(color), width * unit)));
}

/// Turns a text layer's fill off (or back on), keeping its outline.
fn set_fill(b: &mut Builder, id: Id, fill: Option<u32>) {
    let comp = b.comp;
    crate::edit::edit_text(b.project.composition_mut(comp).expect("made"), id, |_, style| style.fill = fill.map(Rgba::hex));
}

/// Left-aligned text whose left edge stays at `at.x`.
#[allow(clippy::too_many_arguments)]
fn text_left(b: &mut Builder, words: &str, at: Vec2, size: f32, color: u32, weight: u16, from: f64) -> Id {
    let line = b.text_aligned(words, at, size, color, weight, from, TextAlign::Left);
    b.anchor_left(line);
    b.transform(line).position = Animated::constant(at);
    line
}

/// Rough width of `words` in units at `size`, for fitting boxes around them.
fn width_of(words: &str, size: f32) -> f32 {
    words.lines().map(|line| line.chars().count()).max().unwrap_or(1) as f32 * size * 0.58
}

/// "Hi, I'm", the name, the handle and three facts in pills: a photo, then everything arrives.
pub(super) fn profile_intro(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (greeting, name, handle) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let tall = b.tall();
    // Photo above the words when tall; on the left in 16:9.
    let (photo_at, text_x, top, diameter, step) = if b.vertical() {
        (b.at(0.5, 0.27), 0.5, 0.47, 460.0, 0.06)
    } else if tall {
        (b.at(0.5, 0.2), 0.5, 0.4, 330.0, 0.072)
    } else {
        (b.at(0.27, 0.5), 0.64, 0.26, 400.0, 0.1)
    };
    let ring = b.shape(ShapeKind::Ellipse, Vec2::new(diameter + 36.0, diameter + 36.0), photo_at, None, Some((p.accent, 10.0)), 0.0, 0.2, "Ring");
    b.pop_in(ring, 0.2);
    let photo = round_photo(b, diameter, photo_at, 0.3, "photo");
    b.pop_in(photo, 0.3);
    let hello = b.text(greeting, b.at(text_x, top), 48.0, p.muted, b.light().max(500), 0.8);
    b.slide_in(hello, Vec2::new(0.0, 30.0), 0.8, 0.6);
    let title = b.text(name, b.at(text_x, top + step * 1.4), 130.0, p.text, b.heavy(), 1.1);
    b.scale(title, 1.1, 140.0, 1.5, 100.0, EASE_OUT);
    b.fade_in(title, 1.1, 0.2);
    let at_handle = b.text(handle, b.at(text_x, top + step * 2.7), 44.0, p.accent, b.heavy(), 1.6);
    b.fade_in(at_handle, 1.6, 0.5);
    // Three facts, one after another, in pills.
    let pill_y = top + step * (if tall { 4.1 } else { 4.3 });
    let size = 40.0;
    let widths: Vec<f32> = (3..6).map(|index| width_of(b.copy(index), size) + 70.0).collect();
    let room = if tall { b.width / unit - 120.0 } else { b.width * 0.62 / unit };
    let side_by_side = widths.iter().sum::<f32>() + 40.0 < room;
    for (index, width) in widths.iter().enumerate() {
        let words = b.copy(index + 3);
        let at = 2.2 + index as f64 * 0.45;
        let center = if side_by_side {
            let start = -(widths.iter().sum::<f32>() + 40.0) * 0.5;
            let before: f32 = widths[..index].iter().sum::<f32>() + 20.0 * index as f32;
            b.at(text_x, pill_y) + Vec2::new((start + before + width * 0.5) * unit, 0.0)
        } else {
            b.at(text_x, pill_y) + Vec2::new(0.0, 96.0 * unit * index as f32)
        };
        let pill = b.block(Vec2::new(*width, 76.0), center, p.accent, 38.0, at, "Fact");
        b.pop_in(pill, at);
        let fact = b.text(words, center, size, p.on_accent, b.heavy(), at + 0.05);
        b.pop_in(fact, at + 0.05);
    }
    b.fade_everything_out();
}

/// A question sticker over footage, then the answer in a caption: header, question, answer.
pub(super) fn ask_me_anything(b: &mut Builder) {
    let p = b.palette();
    full_frame_video(b, "video");
    let (header, question, answer) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let width = (b.width / unit - 160.0).min(820.0);
    let lines = question.lines().count().max(1) as f32;
    let body = lines * 62.0 + 70.0;
    let center = b.at(0.5, if b.vertical() { 0.3 } else { 0.34 });
    // The sticker: a colored band with the header over a white card with the question.
    let card = b.block(Vec2::new(width, body + 110.0), center, 0xffffff, 40.0, 0.3, "Sticker");
    let band_at = center - Vec2::new(0.0, (body + 110.0) * 0.5 * unit - 55.0 * unit);
    let band = b.block(Vec2::new(width, 110.0), band_at, p.accent, 40.0, 0.3, "Sticker band");
    let title = b.text(header, band_at, 40.0, p.on_accent, b.heavy(), 0.35);
    let words = b.text(question, center + Vec2::new(0.0, 55.0 * unit), 50.0, 0x18181b, b.heavy(), 0.45);
    for id in [card, band, title, words] {
        b.transform(id).rotation = Animated::constant(-3.0);
        b.pop_in(id, 0.3);
    }
    b.effect(card, "Drop Shadow", &[(0, 40.0), (2, 14.0), (3, 30.0)]);
    // The answer, typed into a caption box lower down.
    let answer_at = b.at(0.5, if b.vertical() { 0.72 } else { 0.78 });
    let box_width = (width_of(answer, 56.0) + 90.0).min(b.width / unit - 100.0);
    let answer_lines = answer.lines().count().max(1) as f32;
    let caption_box = tinted(b, Vec2::new(box_width, answer_lines * 70.0 + 60.0), answer_at, 0x000000, 0.7, 24.0, 1.8, "Caption box");
    b.slide_in(caption_box, Vec2::new(0.0, 60.0), 1.8, 0.6);
    let reply = b.text(answer, answer_at, 56.0, 0xffffff, b.heavy(), 2.0);
    b.slide_in(reply, Vec2::new(0.0, 60.0), 2.0, 0.6);
    b.fade_everything_out();
}

/// A name spelled out letter by letter, each dropping in: kicker, name, line under it.
pub(super) fn name_reveal(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (kicker, name, line) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let y = b.at(0.5, 0.48).y;
    let kick = b.text(kicker, b.at(0.5, 0.33), 46.0, p.muted, b.light().max(500), 0.2);
    b.fade_in(kick, 0.2, 0.6);
    // Letters sized so the whole name fits across the frame.
    let letters: Vec<char> = name.chars().collect();
    let widths: Vec<f32> = letters.iter().map(|&letter| letter_width(letter)).collect();
    let total: f32 = widths.iter().sum::<f32>().max(0.5);
    let room = b.width / unit - 140.0;
    let size = (room / total).min(220.0);
    let mut left = -total * size * 0.5;
    for (index, letter) in letters.iter().enumerate() {
        let advance = widths[index] * size;
        left += advance;
        if letter.is_whitespace() {
            continue;
        }
        let x = b.width * 0.5 + (left - advance * 0.5) * unit;
        let at = 0.7 + index as f64 * 0.12;
        let glyph = b.text(&letter.to_string(), Vec2::new(x, y), size, if index % 2 == 0 { p.text } else { p.accent }, b.heavy(), at);
        b.slide_in(glyph, Vec2::new(0.0, -260.0), at, 0.7);
        b.scale(glyph, at, 60.0, at + 0.7, 100.0, SPRING);
    }
    let settled = 0.7 + letters.len() as f64 * 0.12 + 0.5;
    let bar_width = (total * size).min(room);
    let bar = b.block(Vec2::new(bar_width, 12.0), b.at(0.5, 0.48) + Vec2::new(0.0, size * 0.62 * unit), p.accent, 6.0, settled, "Underline");
    b.scale(bar, settled, 0.0, settled + 0.6, 100.0, EASE_OUT);
    let under = b.text(line, b.at(0.5, 0.48) + Vec2::new(0.0, (size * 0.62 + 90.0) * unit), 48.0, p.text, b.light().max(500), settled + 0.3);
    b.fade_in(under, settled + 0.3, 0.6);
    b.fade_everything_out();
}

/// Roughly how wide a capital letter is, as a fraction of the type size, so a name spelled
/// out letter by letter is spaced like the word would be.
fn letter_width(letter: char) -> f32 {
    match letter {
        'I' | 'i' | 'l' | 'j' | '1' | '!' | '.' | ',' | '\'' => 0.34,
        ' ' => 0.3,
        'f' | 't' | 'r' | 'J' => 0.5,
        'M' | 'W' | 'm' | 'w' => 0.86,
        _ => 0.66,
    }
}

/// A story, as the apps show one: progress bars, who posted it and when, a caption and the
/// reply field. Fills the frame in 9:16 and stands in the middle of the others. Name, time,
/// caption, reply field.
pub(super) fn story_frame(b: &mut Builder) {
    let p = b.palette();
    let unit = b.unit();
    // The story's own box: the whole frame in 9:16, a 9:16 card in the middle otherwise.
    let (story_size, center) = if b.vertical() {
        (Vec2::new(b.width, b.height) * (1.0 / unit), b.at(0.5, 0.5))
    } else {
        b.background();
        let height = b.height / unit - 80.0;
        (Vec2::new(height * 9.0 / 16.0, height), b.at(0.5, 0.5))
    };
    let corner = if b.vertical() { 0.0 } else { 36.0 };
    let photo = b.placeholder(story_size, center, corner, 0.0, "photo or video");
    b.fade_in(photo, 0.0, 0.4);
    let top = center.y - (story_size.y * 0.5 - 50.0) * unit;
    let left = center.x - (story_size.x * 0.5 - 30.0) * unit;
    // Three segments filling one after another across the story's length.
    let gap = 10.0;
    let segment = (story_size.x - 60.0 - gap * 2.0) / 3.0;
    let end = b.end() - 0.5;
    for index in 0..3 {
        let x = left + (segment * 0.5 + (segment + gap) * index as f32) * unit;
        tinted(b, Vec2::new(segment, 8.0), Vec2::new(x, top), 0xffffff, 0.4, 4.0, 0.0, "Progress track");
        let fill = b.block(Vec2::new(segment, 8.0), Vec2::new(x, top), 0xffffff, 4.0, 0.0, "Progress");
        b.anchor_left(fill);
        b.transform(fill).position = Animated::constant(Vec2::new(x - segment * 0.5 * unit, top));
        let from = end * index as f64 / 3.0;
        b.grow_across(fill, from, end / 3.0);
        if index > 0 {
            // Empty until its turn.
            let keys = vec![b.key(0.0, Vec2::new(0.0, 100.0), Easing::Hold), b.key(from, Vec2::new(0.0, 100.0), Easing::Linear), b.key(from + end / 3.0, Vec2::new(100.0, 100.0), Easing::Linear)];
            b.transform(fill).scale = Animated::keyframed(keys);
        }
    }
    // Who posted it and when.
    let header_y = top + 70.0 * unit;
    let avatar = b.shape(ShapeKind::Ellipse, Vec2::new(70.0, 70.0), Vec2::new(left + 35.0 * unit, header_y), Some(Paint::Linear { start: Rgba::hex(p.accent), end: Rgba::hex(p.background_2), angle: 45.0 }), Some((0xffffff, 3.0)), 0.0, 0.3, "Drop your profile photo here");
    b.pop_in(avatar, 0.3);
    let (name, time, caption, reply) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let poster = text_left(b, name, Vec2::new(left + 90.0 * unit, header_y), 32.0, 0xffffff, b.heavy(), 0.4);
    b.fade_in(poster, 0.4, 0.4);
    let when_x = left + (100.0 + width_of(name, 32.0)) * unit;
    let when = text_left(b, time, Vec2::new(when_x, header_y), 28.0, 0xe4e4e7, 500, 0.4);
    b.fade_in(when, 0.4, 0.4);
    // The caption in a pill, as stickers sit on stories.
    let caption_at = center + Vec2::new(0.0, story_size.y * 0.12 * unit);
    let pill_width = (width_of(caption, 46.0) + 80.0).min(story_size.x - 80.0);
    let lines = caption.lines().count().max(1) as f32;
    let pill = b.block(Vec2::new(pill_width, lines * 58.0 + 50.0), caption_at, p.accent, 28.0, 1.0, "Caption");
    b.pop_in(pill, 1.0);
    let words = b.text(caption, caption_at, 46.0, p.on_accent, b.heavy(), 1.05);
    b.pop_in(words, 1.05);
    // The reply field and a heart along the bottom.
    let bottom = center.y + (story_size.y * 0.5 - 70.0) * unit;
    let field_width = story_size.x - 160.0;
    let field_at = Vec2::new(left - 30.0 * unit + (30.0 + field_width * 0.5) * unit, bottom);
    let field = b.shape(ShapeKind::Rectangle, Vec2::new(field_width, 80.0), field_at, None, Some((0xffffff, 3.0)), 40.0, 0.5, "Reply field");
    b.fade_in(field, 0.5, 0.4);
    let hint = text_left(b, reply, Vec2::new(field_at.x - (field_width * 0.5 - 34.0) * unit, bottom), 30.0, 0xffffff, 500, 0.5);
    b.fade_in(hint, 0.5, 0.4);
    let heart = b.text("♡", Vec2::new(center.x + (story_size.x * 0.5 - 60.0) * unit, bottom), 56.0, 0xffffff, 400, 0.5);
    b.fade_in(heart, 0.5, 0.4);
    b.pulse(heart, 2.5, 3.5, 1);
    b.fade_everything_out();
}

/// A "replying to a comment" sticker over footage, and a caption: handle, comment, caption.
pub(super) fn comment_reply(b: &mut Builder) {
    let p = b.palette();
    full_frame_video(b, "video");
    let (handle, comment, caption) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let lines = comment.lines().count().max(1) as f32;
    let width = (width_of(comment, 44.0).max(width_of(handle, 28.0) + 260.0) + 150.0).min(b.width / unit - 120.0);
    let height = lines * 56.0 + 110.0;
    let center = b.at(0.5, if b.vertical() { 0.24 } else { 0.27 });
    let card = b.block(Vec2::new(width, height), center, 0xffffff, 26.0, 0.3, "Comment");
    b.effect(card, "Drop Shadow", &[(0, 35.0), (2, 10.0), (3, 24.0)]);
    let left = center.x - (width * 0.5 - 30.0) * unit;
    let top = center.y - (height * 0.5 - 40.0) * unit;
    let avatar = b.shape(ShapeKind::Ellipse, Vec2::new(54.0, 54.0), Vec2::new(left + 27.0 * unit, center.y), Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, 0.35, "Drop their profile photo here");
    let label = text_left(b, &format!("Reply to {handle}'s comment"), Vec2::new(left + 76.0 * unit, top), 28.0, 0x71717a, 500, 0.35);
    let words = text_left(b, comment, Vec2::new(left + 76.0 * unit, top + (36.0 + lines * 28.0) * unit), 44.0, 0x18181b, b.heavy(), 0.4);
    for id in [card, avatar, label, words] {
        b.pop_in(id, 0.3);
    }
    // The caption, in white with a dark outline so it reads over any footage.
    let line = b.text(caption, b.at(0.5, if b.vertical() { 0.7 } else { 0.78 }), 64.0, 0xffffff, b.heavy(), 1.4);
    outline(b, line, 0x000000, 6.0);
    b.slide_in(line, Vec2::new(0.0, 50.0), 1.4, 0.6);
    b.fade_everything_out();
}

/// A greeting with hearts rising behind it: headline, name, line under it.
pub(super) fn floating_hearts(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (headline, name, line) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let end = b.end();
    // A thin card border just inside the frame.
    let inset = Vec2::new(b.width, b.height) * (1.0 / unit) - Vec2::new(70.0, 70.0);
    let border = b.shape(ShapeKind::Rectangle, inset, b.at(0.5, 0.5), None, Some((p.accent, 4.0)), 30.0, 0.0, "Border");
    b.fade_in(border, 0.0, 0.8);
    // Hearts drifting up from below, each at its own pace, fading as they rise.
    let colors = [p.accent, 0xf472b6, 0xfb7185, p.muted];
    for index in 0..18u32 {
        let x = ((index * 41 + 7) % 100) as f32 / 100.0;
        let start = (index % 9) as f64 * 0.45;
        let rise = 3.5 + (index % 4) as f64 * 0.6;
        let size = 50.0 + (index % 5) as f32 * 18.0;
        let heart = b.text("♥", Vec2::new(b.width * x, b.height + 60.0 * unit), size, colors[(index % 4) as usize], 400, start);
        let sway = ((index % 5) as f32 - 2.0) * 40.0 * unit;
        b.move_to(heart, start, (start + rise).min(end), Vec2::new(b.width * x + sway, -80.0 * unit), Easing::Linear);
        let keys = vec![b.key(start, 0.0, Easing::Linear), b.key(start + 0.4, 85.0, Easing::Linear), b.key((start + rise).min(end), 0.0, Easing::Linear)];
        super::Builder::keys(&mut b.transform(heart).opacity, keys);
    }
    let head = b.text(headline, b.at(0.5, 0.38), 110.0, p.text, b.heavy(), 0.4);
    b.slide_in(head, Vec2::new(0.0, 40.0), 0.4, 0.9);
    let who = b.text(name, b.at(0.5, 0.53), 150.0, p.accent, b.heavy(), 1.1);
    b.pop_in(who, 1.1);
    b.pulse(who, 2.2, end - 1.0, 3);
    let under = b.text(line, b.at(0.5, 0.67), 46.0, p.muted, b.light().max(500), 1.8);
    b.fade_in(under, 1.8, 0.8);
    b.fade_everything_out();
}

/// Huge outlined words over a photo, each filling in after it lands: three words.
pub(super) fn outline_stack(b: &mut Builder) {
    let p = b.palette();
    full_frame_video(b, "photo or video");
    // Darkened so the words stand out.
    let size = Vec2::new(b.width, b.height) * (1.0 / b.unit());
    let center = b.at(0.5, 0.5);
    tinted(b, size, center, 0x000000, 0.35, 0.0, 0.0, "Shade");
    let step = if b.vertical() { 0.14 } else { 0.24 };
    let top = 0.5 - step;
    for index in 0..3 {
        let words = b.copy(index);
        let at = 0.3 + index as f64 * 0.6;
        let y = top + step * index as f32;
        let text_size = if b.vertical() { 170.0 } else { 190.0 };
        // An outline that slams in, then the same words filling it in.
        let hollow = b.text(words, b.at(0.5, y), text_size, p.accent, b.heavy(), at);
        set_fill(b, hollow, None);
        outline(b, hollow, p.accent, 5.0);
        b.scale(hollow, at, 220.0, at + 0.3, 100.0, EASE_OUT);
        b.fade_in(hollow, at, 0.1);
        let solid = b.text(words, b.at(0.5, y), text_size, p.accent, b.heavy(), at + 0.45);
        b.fade_in(solid, at + 0.45, 0.25);
        // The outline goes once the fill is in, so the letters keep their shape.
        b.until(hollow, at + 0.7);
    }
    // A shake as the last word fills in.
    if let Some(last) = b.project.composition(b.comp).and_then(|comp| comp.layers.first()).map(|layer| layer.id) {
        b.effect(last, "Camera Shake", &[(0, 10.0), (1, 12.0), (2, 1.0), (3, 2.0)]);
        b.animate_effect(last, 0, &[(1.7, 12.0, Easing::Linear), (2.3, 0.0, Easing::Linear)]);
    }
    b.fade_everything_out();
}

/// A bouncy, outlined title on a blob, with a date sticker and twinkles: word, line, date.
pub(super) fn vlog_title(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (word, line, date) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let end = b.end();
    let center = b.at(0.5, 0.46);
    let blob_width = (width_of(word, 220.0) + 220.0).min(b.width / unit - 40.0);
    let blob = b.shape(ShapeKind::Ellipse, Vec2::new(blob_width, 420.0), center, Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, 0.1, "Blob");
    b.pop_in(blob, 0.1);
    b.spin(blob, -4.0, 4.0);
    let title = b.text(word, center, 220.0, 0xffffff, b.heavy(), 0.4);
    outline(b, title, p.text, 12.0);
    b.effect(title, "Drop Shadow", &[(0, 60.0), (2, 4.0), (3, 0.0)]);
    b.pop_in(title, 0.4);
    // A gentle wobble while it sits there.
    let wobble = (0..8).map(|index| b.key(1.2 + index as f64 * 0.9, if index % 2 == 0 { -3.0 } else { 3.0 }, EASE_IN_OUT)).collect();
    super::Builder::keys(&mut b.transform(title).rotation, wobble);
    let pill_at = center + Vec2::new(0.0, 290.0 * unit);
    let pill = b.block(Vec2::new(width_of(line, 48.0) + 90.0, 90.0), pill_at, p.text, 45.0, 1.0, "Line");
    b.slide_in(pill, Vec2::new(0.0, 60.0), 1.0, 0.6);
    let words = b.text(line, pill_at, 48.0, p.background, b.light().max(600), 1.1);
    b.slide_in(words, Vec2::new(0.0, 60.0), 1.1, 0.6);
    // The date, as a tilted sticker up and to the right.
    let sticker_at = center + Vec2::new((blob_width * 0.5 - 80.0).min(380.0) * unit, -230.0 * unit);
    let sticker = b.block(Vec2::new(width_of(date, 40.0) + 60.0, 76.0), sticker_at, 0xffffff, 12.0, 1.5, "Date");
    let day = b.text(date, sticker_at, 40.0, 0x18181b, b.heavy(), 1.5);
    for id in [sticker, day] {
        b.transform(id).rotation = Animated::constant(10.0);
        b.pop_in(id, 1.5);
    }
    // Twinkles around the title.
    for (index, (dx, dy)) in [(-0.36_f32, -0.2_f32), (0.4, 0.17), (-0.3, 0.2), (0.28, -0.24)].into_iter().enumerate() {
        let at = 1.8 + index as f64 * 0.3;
        let star = b.text("✦", b.at(0.5 + dx * if b.tall() { 1.0 } else { 0.6 }, 0.46 + dy), 64.0, 0xffffff, 400, at);
        b.pop_in(star, at);
        b.pulse(star, at + 0.8, end - 0.6, 3);
    }
    b.fade_everything_out();
}
