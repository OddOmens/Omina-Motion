//! The playful layouts, after the social formats people make most in Canva and Adobe Express:
//! chats, notifications, post cards, polls, memes, quizzes, music cards, typewriter, neon and
//! glitch titles, confetti, phone mockups, polaroids, goals, maps and coupons. Like the others,
//! each places its layers by fractions of the frame and adapts to the three shapes.

use super::layouts::button;
use super::{Builder, EASE_IN_OUT, EASE_OUT, SPRING};
use crate::animation::{Animated, Easing, Vec2};
use crate::model::{LayerContent, Paint, Rgba, ShapeKind, TextAlign};

/// A rounded box with words in it (a chat bubble, a pill), popping in at `at`. Returns the
/// box's height in units.
#[allow(clippy::too_many_arguments)]
fn bubble(b: &mut Builder, words: &str, center: Vec2, width: f32, fill: u32, ink: u32, size: f32, at: f64) -> f32 {
    let lines = words.lines().count().max(1) as f32;
    let height = size * 1.25 * lines + 56.0;
    let card = b.block(Vec2::new(width, height), center, fill, 34.0, at, "Bubble");
    b.pop_in(card, at);
    let text = b.text(words, center, size, ink, b.light().max(500), at + 0.05);
    b.pop_in(text, at + 0.05);
    height
}

/// Rough width of `words` in units at `size`, for fitting boxes around them.
fn width_of(words: &str, size: f32) -> f32 {
    words.lines().map(|line| line.chars().count()).max().unwrap_or(1) as f32 * size * 0.56
}

pub(super) fn chat(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let unit = b.unit();
    let full = b.width / unit;
    // The contact's name in a bar across the top, as in a messages app.
    let bar_y = 0.1;
    let bar = b.block(Vec2::new(full, 150.0), b.at(0.5, bar_y), p.background_2, 0.0, 0.0, "Header");
    b.fade_in(bar, 0.0, 0.4);
    let avatar = b.shape(ShapeKind::Ellipse, Vec2::new(70.0, 70.0), b.at(0.5, bar_y) + Vec2::new(0.0, -18.0 * unit), Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, 0.1, "Contact photo");
    b.pop_in(avatar, 0.1);
    let name = b.text(b.copy(0), b.at(0.5, bar_y) + Vec2::new(0.0, 42.0 * unit), 30.0, p.text, b.heavy(), 0.2);
    b.fade_in(name, 0.2, 0.4);
    // Messages down the frame: theirs on the left, yours on the right.
    let column = (full - 160.0).min(820.0);
    let mut y = b.at(0.5, bar_y).y / unit + 150.0;
    let gap = if b.vertical() { 90.0 } else { 34.0 };
    for index in 0..4 {
        let words = b.copy(index + 1);
        let mine = index % 2 == 1;
        let size = if b.vertical() { 54.0 } else { 42.0 };
        let width = (width_of(words, size) + 70.0).min(column * 0.75);
        let lines = words.lines().count().max(1) as f32;
        let height = size * 1.25 * lines + 56.0;
        y += height * 0.5;
        let x = if mine { (full + column) * 0.5 - width * 0.5 } else { (full - column) * 0.5 + width * 0.5 };
        let at = 0.8 + index as f64 * 1.3;
        let (fill, ink) = if mine { (p.accent, p.on_accent) } else { (p.background_2, p.text) };
        bubble(b, words, Vec2::new(x * unit, y * unit), width, fill, ink, size, at);
        y += height * 0.5 + gap;
    }
    b.fade_everything_out();
}

pub(super) fn notification(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (app, title, message, headline) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let unit = b.unit();
    let width = (b.width / unit - 120.0).min(900.0);
    let y = if b.vertical() { 0.2 } else { 0.24 };
    let center = b.at(0.5, y);
    let card = b.block(Vec2::new(width, 230.0), center, 0xf4f4f5, 44.0, 0.4, "Notification");
    b.slide_in(card, Vec2::new(0.0, -400.0), 0.4, 0.8);
    b.effect(card, "Drop Shadow", &[(0, 40.0), (2, 14.0), (3, 30.0)]);
    let left = center.x - (width * 0.5 - 50.0) * unit;
    let icon_at = Vec2::new(left + 32.0 * unit, center.y - 60.0 * unit);
    let icon = b.block(Vec2::new(64.0, 64.0), icon_at, p.accent, 16.0, 0.5, "App icon");
    b.slide_in(icon, Vec2::new(0.0, -400.0), 0.4, 0.8);
    let lines: [(&str, f32, u32, u16, f32); 3] = [(app, 30.0, 0x52525b, b.light().max(500), -60.0), (title, 38.0, 0x18181b, b.heavy(), 4.0), (message, 34.0, 0x3f3f46, b.light().max(500), 56.0)];
    for (words, size, ink, weight, dy) in lines {
        let x = if dy < -10.0 { left + 90.0 * unit } else { left };
        let line = b.text_aligned(words, Vec2::new(x, center.y + dy * unit), size, ink, weight, 0.5, TextAlign::Left);
        b.anchor_left(line);
        b.transform(line).position = Animated::constant(Vec2::new(x, center.y + dy * unit));
        b.slide_in(line, Vec2::new(0.0, -400.0), 0.4, 0.8);
    }
    let now = b.text("now", Vec2::new(center.x + (width * 0.5 - 70.0) * unit, center.y - 60.0 * unit), 28.0, 0x71717a, 500, 0.5);
    b.slide_in(now, Vec2::new(0.0, -400.0), 0.4, 0.8);
    b.pulse(card, 2.0, 3.0, 1);
    let head = b.text(headline, b.at(0.5, if b.vertical() { 0.6 } else { 0.68 }), 96.0, p.text, b.heavy(), 1.6);
    b.slide_in(head, Vec2::new(0.0, 50.0), 1.6, 0.8);
    b.fade_everything_out();
}

pub(super) fn post_card(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (name, handle, post, likes) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let unit = b.unit();
    let width = (b.width / unit - 140.0).min(940.0);
    let lines = post.lines().count().max(1) as f32;
    let height = 330.0 + lines * 58.0;
    let center = b.at(0.5, 0.5);
    let card = b.block(Vec2::new(width, height), center, 0xffffff, 36.0, 0.2, "Post");
    b.pop_in(card, 0.2);
    b.effect(card, "Drop Shadow", &[(0, 35.0), (2, 16.0), (3, 40.0)]);
    let top = center.y - (height * 0.5 - 80.0) * unit;
    let left = center.x - (width * 0.5 - 60.0) * unit;
    let avatar = b.shape(ShapeKind::Ellipse, Vec2::new(84.0, 84.0), Vec2::new(left + 42.0 * unit, top), Some(Paint::linear(Rgba::hex(p.accent), Rgba::hex(p.background_2), 45.0)), None, 0.0, 0.5, "Drop your profile photo here");
    b.pop_in(avatar, 0.5);
    for (words, size, ink, weight, dy) in [(name, 38.0, 0x111111, b.heavy(), -20.0), (handle, 30.0, 0x71717a, 500, 22.0)] {
        let x = left + 110.0 * unit;
        let line = b.text_aligned(words, Vec2::new(x, top + dy * unit), size, ink, weight, 0.6, TextAlign::Left);
        b.anchor_left(line);
        b.transform(line).position = Animated::constant(Vec2::new(x, top + dy * unit));
        b.fade_in(line, 0.6, 0.5);
    }
    let body_y = top + (90.0 + lines * 29.0) * unit;
    let body = b.text_aligned(post, Vec2::new(left, body_y), 44.0, 0x111111, b.light().max(500), 1.0, TextAlign::Left);
    b.anchor_left(body);
    b.transform(body).position = Animated::constant(Vec2::new(left, body_y));
    b.fade_in(body, 1.0, 0.8);
    let bottom = center.y + (height * 0.5 - 60.0) * unit;
    let heart = b.text("♥", Vec2::new(left + 24.0 * unit, bottom), 52.0, 0xe11d48, 700, 2.4);
    b.scale(heart, 2.4, 0.0, 3.0, 100.0, SPRING);
    b.pulse(heart, 3.2, 4.6, 2);
    let count = b.text_aligned(likes, Vec2::new(left + 70.0 * unit, bottom), 34.0, 0x3f3f46, b.heavy(), 2.6, TextAlign::Left);
    b.anchor_left(count);
    b.transform(count).position = Animated::constant(Vec2::new(left + 70.0 * unit, bottom));
    b.fade_in(count, 2.6, 0.4);
    b.fade_everything_out();
}

pub(super) fn this_or_that(b: &mut Builder) {
    let p = b.palette();
    let (question, first, second) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let tall = b.tall();
    // Two photo spots: side by side in 16:9, one above the other in tall shapes.
    let (size, at_a, at_b) = if tall {
        (Vec2::new(b.width / unit, b.height / unit * 0.5), b.at(0.5, 0.25), b.at(0.5, 0.75))
    } else {
        (Vec2::new(b.width / unit * 0.5, b.height / unit), b.at(0.25, 0.5), b.at(0.75, 0.5))
    };
    let a = b.placeholder(size, at_a, 0.0, 0.0, "first photo");
    let away = if tall { Vec2::new(-1200.0, 0.0) } else { Vec2::new(0.0, -1200.0) };
    b.slide_in(a, away, 0.0, 0.7);
    let bb = b.placeholder(size, at_b, 0.0, 0.0, "second photo");
    b.slide_in(bb, away * -1.0, 0.0, 0.7);
    let q_at = b.at(0.5, if tall { 0.08 } else { 0.1 });
    let q_width = (width_of(question, 54.0) + 80.0).min(b.width / unit - 60.0);
    let q_box = b.block(Vec2::new(q_width, 100.0), q_at, p.background, 50.0, 0.5, "Question");
    b.pop_in(q_box, 0.5);
    let q = b.text(question, q_at, 54.0, p.text, b.heavy(), 0.6);
    b.pop_in(q, 0.6);
    let label_y = |center: Vec2| center + Vec2::new(0.0, if tall { 0.0 } else { 120.0 * unit });
    for (words, center, at) in [(first, at_a, 1.0), (second, at_b, 1.2)] {
        let width = (width_of(words, 60.0) + 80.0).max(260.0);
        let pill = b.block(Vec2::new(width, 110.0), label_y(center), p.accent, 55.0, at, "Option");
        b.pop_in(pill, at);
        let label = b.text(words, label_y(center), 60.0, p.on_accent, b.heavy(), at + 0.05);
        b.pop_in(label, at + 0.05);
    }
    let or = b.shape(ShapeKind::Ellipse, Vec2::new(170.0, 170.0), b.at(0.5, 0.5), Some(Paint::Solid(Rgba::hex(0xffffff))), None, 0.0, 1.6, "OR");
    b.pop_in(or, 1.6);
    let or_text = b.text("OR", b.at(0.5, 0.5), 64.0, 0x111111, b.heavy(), 1.7);
    b.pop_in(or_text, 1.7);
    // Results: each option's share, popping up.
    for (share, center, at) in [("62%", at_a, 4.0), ("38%", at_b, 4.2)] {
        // Above the label in 16:9; below it in tall shapes, clear of the question at the top.
        let offset = if tall { 150.0 } else { -150.0 };
        let result = b.text(share, label_y(center) + Vec2::new(0.0, offset * unit), 110.0, 0xffffff, b.heavy(), at);
        b.pop_in(result, at);
        b.effect(result, "Drop Shadow", &[(0, 60.0), (2, 4.0), (3, 12.0)]);
    }
    b.fade_everything_out();
}

pub(super) fn meme(b: &mut Builder) {
    let (top, bottom) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    b.placeholder(Vec2::new(b.width, b.height) * (1.0 / unit), b.at(0.5, 0.5), 0.0, 0.0, "photo or video");
    let style = |b: &mut Builder, id| {
        let comp = b.comp;
        crate::edit::edit_text(b.project.composition_mut(comp).expect("made"), id, |_, style| style.stroke = Some((Rgba::hex(0x000000), 8.0 * unit)));
    };
    let first = b.text(top, b.at(0.5, if b.vertical() { 0.14 } else { 0.12 }), 96.0, 0xffffff, b.heavy(), 0.2);
    style(b, first);
    b.pop_in(first, 0.2);
    let second = b.text(bottom, b.at(0.5, if b.vertical() { 0.86 } else { 0.88 }), 96.0, 0xffffff, b.heavy(), 1.6);
    style(b, second);
    b.pop_in(second, 1.6);
    b.effect(second, "Camera Shake", &[(0, 6.0), (1, 10.0), (2, 0.5), (3, 1.0)]);
    b.fade_everything_out();
}

pub(super) fn typewriter(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (line, sub) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let center = b.at(0.5, 0.45);
    let words = b.text(line, center, 84.0, p.text, b.heavy(), 0.2);
    // Typed out left to right: a wipe uncovering it at an even pace.
    let length = line.chars().count().max(1) as f64;
    let typed = (length * 0.07).clamp(1.2, 4.0);
    b.effect(words, "Linear Wipe", &[(1, 270.0)]);
    b.animate_effect(words, 0, &[(0.3, 100.0, Easing::Linear), (0.3 + typed, 0.0, Easing::Linear)]);
    // A blinking cursor after the words.
    let width = b.layer(words).size.x;
    let cursor_at = center + Vec2::new(width * 0.5 + 18.0 * unit, 0.0);
    let cursor = b.block(Vec2::new(10.0, 96.0), cursor_at, p.accent, 2.0, 0.2, "Cursor");
    let end = b.end();
    let mut keys = Vec::new();
    let mut t = 0.2;
    let mut on = true;
    while t < end {
        keys.push(b.key(t, if on { 100.0 } else { 0.0 }, Easing::Hold));
        on = !on;
        t += 0.5;
    }
    super::Builder::keys(&mut b.transform(cursor).opacity, keys);
    let below = b.text(sub, b.at(0.5, 0.6), 44.0, p.muted, b.light(), 0.6 + typed);
    b.fade_in(below, 0.6 + typed, 0.6);
    b.fade_everything_out();
}

pub(super) fn neon(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (word, sub) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    // A thin tube outline around the sign.
    let width = (width_of(word, 140.0) + 200.0).min(b.width / unit - 80.0);
    let frame = b.shape(ShapeKind::Rectangle, Vec2::new(width, 360.0), b.at(0.5, 0.46), None, Some((p.muted, 6.0)), 60.0, 0.0, "Tube");
    b.effect(frame, "Glow", &[(0, 10.0), (1, 30.0), (2, 1.5)]);
    let sign = b.text(word, b.at(0.5, 0.46), 140.0, p.accent, b.heavy(), 0.0);
    b.effect(sign, "Glow", &[(0, 20.0), (1, 40.0), (2, 2.2)]);
    // Flickering on, as a neon sign does, then steady with the odd stutter.
    let flicker = [(0.0, 0.0), (0.3, 100.0), (0.38, 15.0), (0.5, 100.0), (0.56, 30.0), (0.7, 100.0), (4.0, 100.0), (4.05, 40.0), (4.12, 100.0)];
    let keys = flicker.iter().map(|&(at, value)| b.key(at, value, Easing::Hold)).collect();
    super::Builder::keys(&mut b.transform(sign).opacity, keys);
    let line = b.text(sub, b.at(0.5, 0.68), 46.0, p.text, b.light(), 1.2);
    b.fade_in(line, 1.2, 0.8);
    b.fade_everything_out();
}

pub(super) fn confetti(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (headline, sub) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let colors = [p.accent, 0xfacc15, 0x38bdf8, 0xf472b6, 0x4ade80, 0xffffff];
    let kinds = [ShapeKind::Rectangle, ShapeKind::Ellipse, ShapeKind::Triangle];
    let end = b.end();
    for index in 0..44u32 {
        // Spread across the top, each falling at its own pace and turning.
        let x = ((index * 37) % 100) as f32 / 100.0;
        let start = (index % 11) as f64 * 0.12;
        let fall = 2.6 + (index % 5) as f64 * 0.5;
        let size = Vec2::new(18.0 + (index % 4) as f32 * 6.0, 28.0 + (index % 3) as f32 * 8.0);
        let piece = b.shape(kinds[(index % 3) as usize], size, Vec2::new(b.width * x, -60.0 * unit), Some(Paint::Solid(Rgba::hex(colors[(index % 6) as usize]))), None, 3.0, start, "Confetti");
        let drift = ((index % 7) as f32 - 3.0) * 30.0 * unit;
        let landing = Vec2::new(b.width * x + drift, b.height + 80.0 * unit);
        b.move_to(piece, start, (start + fall).min(end), landing, Easing::Linear);
        b.spin(piece, (index * 40) as f32, (index * 40) as f32 + 540.0);
    }
    let head = b.text(headline, b.at(0.5, 0.45), 130.0, p.text, b.heavy(), 0.4);
    b.pop_in(head, 0.4);
    b.pulse(head, 1.5, end - 1.0, 4);
    let line = b.text(sub, b.at(0.5, 0.6), 50.0, p.muted, b.light(), 1.0);
    b.fade_in(line, 1.0, 0.7);
    b.fade_everything_out();
}

pub(super) fn phone_mockup(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (headline, sub, cta) = (b.copy(0), b.copy(1), b.copy(2));
    let tall = b.tall();
    let phone_at = if tall { b.at(0.5, if b.vertical() { 0.36 } else { 0.4 }) } else { b.at(0.3, 0.52) };
    let phone_size = if b.vertical() { Vec2::new(460.0, 940.0) } else if tall { Vec2::new(340.0, 690.0) } else { Vec2::new(380.0, 780.0) };
    let body = b.block(phone_size, phone_at, 0x18181b, 70.0, 0.0, "Phone");
    b.slide_in(body, Vec2::new(0.0, 900.0), 0.0, 1.0);
    b.effect(body, "Drop Shadow", &[(0, 50.0), (2, 24.0), (3, 60.0)]);
    let screen = b.placeholder(phone_size - Vec2::new(36.0, 36.0), phone_at, 54.0, 0.0, "app screen recording");
    b.slide_in(screen, Vec2::new(0.0, 900.0), 0.0, 1.0);
    let unit = b.unit();
    let notch = b.block(Vec2::new(120.0, 30.0), phone_at - Vec2::new(0.0, (phone_size.y * 0.5 - 40.0) * unit), 0x000000, 15.0, 0.0, "Camera");
    b.slide_in(notch, Vec2::new(0.0, 900.0), 0.0, 1.0);
    let text_at = if tall { b.at(0.5, if b.vertical() { 0.76 } else { 0.83 }) } else { b.at(0.7, 0.4) };
    let head = b.text(headline, text_at, 84.0, p.text, b.heavy(), 0.9);
    b.slide_in(head, Vec2::new(0.0, 40.0), 0.9, 0.7);
    let line = b.text(sub, text_at + Vec2::new(0.0, 110.0 * unit), 42.0, p.muted, b.light(), 1.3);
    b.fade_in(line, 1.3, 0.6);
    let button_at = if tall { text_at + Vec2::new(0.0, 210.0 * unit) } else { b.at(0.7, 0.7) };
    if !(tall && !b.vertical()) {
        button(b, cta, button_at, 2.0);
    }
    b.fade_everything_out();
}

pub(super) fn polaroid(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (caption, sub) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let center = b.at(0.5, if b.vertical() { 0.42 } else { 0.46 });
    let frame_size = Vec2::new(520.0, 620.0);
    for (index, (dx, angle)) in [(-120.0_f32, -9.0_f32), (110.0, 7.0), (0.0, -2.0)].into_iter().enumerate() {
        let at = 0.2 + index as f64 * 0.6;
        let spot = center + Vec2::new(dx * unit, 0.0);
        let frame = b.block(frame_size, spot, 0xfafafa, 6.0, at, "Instant photo");
        let photo = b.placeholder(Vec2::new(460.0, 460.0), spot - Vec2::new(0.0, 50.0 * unit), 2.0, at, "photo");
        for id in [frame, photo] {
            b.transform(id).rotation = Animated::constant(angle);
            b.scale(id, at, 160.0, at + 0.45, 100.0, EASE_OUT);
            b.fade_in(id, at, 0.2);
        }
        b.effect(frame, "Drop Shadow", &[(0, 40.0), (2, 12.0), (3, 24.0)]);
        // The last photo's handwritten caption, on its white strip.
        if index == 2 {
            let words = b.text(caption, spot + Vec2::new(0.0, 250.0 * unit), 48.0, 0x27272a, 400, at + 0.4);
            let comp = b.comp;
            crate::edit::edit_text(b.project.composition_mut(comp).expect("made"), words, |_, style| style.font_family = "Marker Felt".into());
            b.transform(words).rotation = Animated::constant(angle);
            b.fade_in(words, at + 0.4, 0.5);
        }
    }
    let line = b.text(sub, b.at(0.5, if b.vertical() { 0.78 } else { 0.92 }), 46.0, p.text, b.light().max(500), 2.2);
    b.fade_in(line, 2.2, 0.7);
    b.fade_everything_out();
}

pub(super) fn quiz(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let question = b.copy(0);
    let unit = b.unit();
    let head = b.text(question, b.at(0.5, if b.vertical() { 0.22 } else { 0.18 }), 76.0, p.text, b.heavy(), 0.2);
    b.slide_in(head, Vec2::new(0.0, 40.0), 0.2, 0.7);
    let width = (b.width / unit - 160.0).min(820.0);
    let first_y = if b.vertical() { 0.4 } else { 0.38 };
    let step = if b.vertical() { 0.11 } else { 0.17 };
    let reveal = 6.0;
    for index in 0..3 {
        let y = first_y + step * index as f32;
        let at = 0.8 + index as f64 * 0.3;
        let center = b.at(0.5, y);
        let pill = b.block(Vec2::new(width, 120.0), center, p.background_2, 60.0, at, "Answer");
        b.slide_in(pill, Vec2::new(400.0, 0.0), at, 0.6);
        let letter = ["A", "B", "C"][index];
        let label = b.text(&format!("{letter}.  {}", b.copy(index + 1)), center, 48.0, p.text, b.light().max(600), at + 0.1);
        b.slide_in(label, Vec2::new(400.0, 0.0), at + 0.1, 0.6);
        if index == 1 {
            // The right answer lights up when time runs out.
            let right = b.block(Vec2::new(width, 120.0), center, p.accent, 60.0, reveal, "Right answer");
            b.pop_in(right, reveal);
            let words = b.text(&format!("{letter}.  {}  ✓", b.copy(index + 1)), center, 48.0, p.on_accent, b.heavy(), reveal + 0.05);
            b.pop_in(words, reveal + 0.05);
        } else {
            let keys = vec![b.key(reveal, 100.0, EASE_IN_OUT), b.key(reveal + 0.4, 35.0, Easing::Linear)];
            for id in [pill, label] {
                super::Builder::keys(&mut b.transform(id).opacity, keys.clone());
            }
        }
    }
    // The timer: a bar running down until the answer shows.
    let timer_y = if b.vertical() { 0.8 } else { 0.9 };
    let track = b.block(Vec2::new(width, 16.0), b.at(0.5, timer_y), p.background_2, 8.0, 1.0, "Timer track");
    b.fade_in(track, 1.0, 0.3);
    let bar = b.block(Vec2::new(width, 16.0), b.at(0.5, timer_y), p.accent, 8.0, 1.0, "Timer");
    b.anchor_left(bar);
    let keys = vec![b.key(1.2, Vec2::new(100.0, 100.0), Easing::Linear), b.key(reveal, Vec2::new(0.0, 100.0), Easing::Linear)];
    super::Builder::keys(&mut b.transform(bar).scale, keys);
    b.until(bar, reveal);
    b.until(track, reveal);
    b.fade_everything_out();
}

pub(super) fn now_playing(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (song, artist) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let tall = b.tall();
    let card_size = if tall { Vec2::new(860.0, if b.vertical() { 1180.0 } else { 1100.0 }) } else { Vec2::new(1400.0, 620.0) };
    let center = b.at(0.5, 0.5);
    let card = b.block(card_size, center, p.background_2, 48.0, 0.0, "Player");
    b.pop_in(card, 0.0);
    b.effect(card, "Drop Shadow", &[(0, 45.0), (2, 20.0), (3, 50.0)]);
    let art_side = if tall { 640.0 } else { 480.0 };
    let art_at = if tall { center - Vec2::new(0.0, (card_size.y * 0.5 - art_side * 0.5 - 70.0) * unit) } else { center - Vec2::new((card_size.x * 0.5 - art_side * 0.5 - 70.0) * unit, 0.0) };
    let art = b.placeholder(Vec2::new(art_side, art_side), art_at, 24.0, 0.2, "album art");
    b.pop_in(art, 0.2);
    let text_x = if tall { center.x } else { center.x + 240.0 * unit };
    let text_y = if tall { art_at.y + (art_side * 0.5 + 80.0) * unit } else { center.y - 120.0 * unit };
    let title = b.text(song, Vec2::new(text_x, text_y), 64.0, p.text, b.heavy(), 0.5);
    b.slide_in(title, Vec2::new(0.0, 30.0), 0.5, 0.6);
    let who = b.text(artist, Vec2::new(text_x, text_y + 70.0 * unit), 40.0, p.muted, b.light(), 0.7);
    b.fade_in(who, 0.7, 0.5);
    let bar_width = if tall { 680.0 } else { 600.0 };
    let bar_y = text_y + 150.0 * unit;
    let track = b.block(Vec2::new(bar_width, 10.0), Vec2::new(text_x, bar_y), p.muted, 5.0, 0.8, "Track");
    b.transform(track).opacity.value = 40.0;
    let fill = b.block(Vec2::new(bar_width, 10.0), Vec2::new(text_x, bar_y), p.accent, 5.0, 0.8, "Progress");
    b.anchor_left(fill);
    let end = b.end();
    let keys = vec![b.key(0.8, Vec2::new(8.0, 100.0), Easing::Linear), b.key(end, Vec2::new(72.0, 100.0), Easing::Linear)];
    super::Builder::keys(&mut b.transform(fill).scale, keys);
    // Controls: back, play, forward.
    let controls_y = bar_y + 110.0 * unit;
    let play = b.shape(ShapeKind::Ellipse, Vec2::new(120.0, 120.0), Vec2::new(text_x, controls_y), Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, 1.0, "Play");
    b.pop_in(play, 1.0);
    let pause = b.text("❚❚", Vec2::new(text_x, controls_y), 44.0, p.on_accent, 700, 1.05);
    b.pop_in(pause, 1.05);
    for side in [-1.0_f32, 1.0] {
        let skip = b.shape(ShapeKind::Triangle, Vec2::new(46.0, 40.0), Vec2::new(text_x + side * 170.0 * unit, controls_y), Some(Paint::Solid(Rgba::hex(p.text))), None, 4.0, 1.1, "Skip");
        b.transform(skip).rotation = Animated::constant(side * 90.0);
        b.pop_in(skip, 1.1);
    }
    b.fade_everything_out();
}

pub(super) fn progress(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (title, percent, sub) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let fraction = percent.trim_end_matches('%').trim().parse::<f32>().unwrap_or(75.0).clamp(0.0, 100.0);
    let head = b.text(title, b.at(0.5, 0.3), 88.0, p.text, b.heavy(), 0.2);
    b.slide_in(head, Vec2::new(0.0, 40.0), 0.2, 0.7);
    let width = (b.width / unit - 200.0).min(1100.0);
    let track = b.block(Vec2::new(width, 70.0), b.at(0.5, 0.5), p.background_2, 35.0, 0.6, "Track");
    b.fade_in(track, 0.6, 0.4);
    let fill = b.block(Vec2::new(width, 70.0), b.at(0.5, 0.5), p.accent, 35.0, 0.8, "Fill");
    b.anchor_left(fill);
    let keys = vec![b.key(0.9, Vec2::new(0.0, 100.0), EASE_IN_OUT), b.key(3.4, Vec2::new(fraction, 100.0), Easing::Linear)];
    super::Builder::keys(&mut b.transform(fill).scale, keys);
    let label = b.text(percent, b.at(0.5, 0.62), 120.0, p.accent, b.heavy(), 3.2);
    b.pop_in(label, 3.2);
    let line = b.text(sub, b.at(0.5, 0.74), 46.0, p.muted, b.light(), 3.6);
    b.fade_in(line, 3.6, 0.6);
    b.fade_everything_out();
}

pub(super) fn kinetic_stack(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let rows = 4;
    let top = if b.vertical() { 0.3 } else { 0.2 };
    let step = if b.vertical() { 0.13 } else { 0.2 };
    for index in 0..rows {
        let words = b.copy(index);
        let at = 0.3 + index as f64 * 0.55;
        let color = if index % 2 == 0 { p.text } else { p.accent };
        let size = if index == rows - 1 { 190.0 } else { 150.0 };
        let word = b.text(words, b.at(0.5, top + step * index as f32), size, color, b.heavy(), at);
        b.scale(word, at, 260.0, at + 0.35, 100.0, EASE_OUT);
        b.fade_in(word, at, 0.15);
    }
    // A shake as the last word lands.
    let ids: Vec<_> = b.project.composition(b.comp).map(|comp| comp.layers.iter().filter(|layer| matches!(layer.content, LayerContent::Text { .. })).map(|layer| layer.id).collect()).unwrap_or_default();
    if let Some(&last) = ids.first() {
        b.effect(last, "Camera Shake", &[(0, 10.0), (1, 14.0), (2, 1.0), (3, 2.0)]);
        b.animate_effect(last, 0, &[(1.9, 14.0, Easing::Linear), (2.6, 0.0, Easing::Linear)]);
    }
    b.fade_everything_out();
}

pub(super) fn map_pin(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (place, address) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    // Streets: a faint grid with a couple of main roads.
    for index in 0..9 {
        let x = (index as f32 + 0.5) / 9.0;
        let road = b.block(Vec2::new(if index % 3 == 1 { 22.0 } else { 8.0 }, b.height / unit), b.at(x, 0.5), p.muted, 0.0, 0.0, "Street");
        b.transform(road).opacity.value = 18.0;
        b.transform(road).rotation = Animated::constant(8.0);
    }
    for index in 0..7 {
        let y = (index as f32 + 0.5) / 7.0;
        let road = b.block(Vec2::new(b.width / unit * 1.2, if index % 3 == 2 { 22.0 } else { 8.0 }), b.at(0.5, y), p.muted, 0.0, 0.0, "Street");
        b.transform(road).opacity.value = 18.0;
        b.transform(road).rotation = Animated::constant(8.0);
    }
    let spot = b.at(0.5, if b.vertical() { 0.42 } else { 0.45 });
    let ring = b.shape(ShapeKind::Ellipse, Vec2::new(200.0, 200.0), spot + Vec2::new(0.0, 90.0 * unit), None, Some((p.accent, 6.0)), 0.0, 1.0, "Ripple");
    b.scale(ring, 1.0, 20.0, 2.2, 160.0, EASE_OUT);
    b.fade_out(ring, 2.2, 1.0);
    b.until(ring, 2.2);
    // The pin: a round head over a point, dropping in with a bounce.
    let pin_head = b.shape(ShapeKind::Ellipse, Vec2::new(150.0, 150.0), spot, Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, 0.3, "Pin");
    let pin_point = b.shape(ShapeKind::Triangle, Vec2::new(110.0, 100.0), spot + Vec2::new(0.0, 80.0 * unit), Some(Paint::Solid(Rgba::hex(p.accent))), None, 10.0, 0.3, "Pin point");
    b.transform(pin_point).rotation = Animated::constant(180.0);
    let dot = b.shape(ShapeKind::Ellipse, Vec2::new(56.0, 56.0), spot, Some(Paint::Solid(Rgba::hex(p.on_accent))), None, 0.0, 0.3, "Pin dot");
    for id in [pin_head, pin_point, dot] {
        let home = b.transform(id).position.value;
        let keys = vec![b.key(0.3, home - Vec2::new(0.0, 700.0 * unit), Easing::Bounce(crate::animation::EaseMode::Out)), b.key(1.1, home, Easing::Linear)];
        super::Builder::keys(&mut b.transform(id).position, keys);
        b.fade_in(id, 0.3, 0.2);
    }
    let label_y = if b.vertical() { 0.66 } else { 0.76 };
    let width = (width_of(place, 64.0).max(width_of(address, 36.0)) + 120.0).min(b.width / unit - 80.0);
    let card = b.block(Vec2::new(width, 210.0), b.at(0.5, label_y), p.background_2, 30.0, 1.4, "Place card");
    b.pop_in(card, 1.4);
    let name = b.text(place, b.at(0.5, label_y) - Vec2::new(0.0, 32.0 * unit), 64.0, p.text, b.heavy(), 1.5);
    b.pop_in(name, 1.5);
    let line = b.text(address, b.at(0.5, label_y) + Vec2::new(0.0, 50.0 * unit), 36.0, p.muted, b.light(), 1.7);
    b.fade_in(line, 1.7, 0.5);
    b.fade_everything_out();
}

pub(super) fn coupon(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (title, amount, code, expiry) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let unit = b.unit();
    let tall = b.tall();
    let size = if tall { Vec2::new(860.0, 1000.0) } else { Vec2::new(1400.0, 600.0) };
    let center = b.at(0.5, 0.5);
    let ticket = b.block(size, center, 0xffffff, 40.0, 0.0, "Coupon");
    b.pop_in(ticket, 0.0);
    b.effect(ticket, "Drop Shadow", &[(0, 40.0), (2, 18.0), (3, 40.0)]);
    // The stub with the amount: the top part in tall shapes, the left part in 16:9.
    let (stub_size, stub_at) = if tall { (Vec2::new(size.x, size.y * 0.45), center - Vec2::new(0.0, size.y * 0.275 * unit)) } else { (Vec2::new(size.x * 0.4, size.y), center - Vec2::new(size.x * 0.3 * unit, 0.0)) };
    let stub = b.block(stub_size, stub_at, p.accent, 40.0, 0.1, "Stub");
    b.pop_in(stub, 0.1);
    // Notches where the ticket tears, cut out with the background color.
    // Where the stub meets the rest: across the ticket in tall shapes, down it in 16:9.
    let notch_offsets = if tall { [Vec2::new(-size.x * 0.5, -size.y * 0.05), Vec2::new(size.x * 0.5, -size.y * 0.05)] } else { [Vec2::new(-size.x * 0.1, -size.y * 0.5), Vec2::new(-size.x * 0.1, size.y * 0.5)] };
    for offset in notch_offsets {
        let notch = b.shape(ShapeKind::Ellipse, Vec2::new(90.0, 90.0), center + offset * unit, Some(Paint::Solid(Rgba::hex(p.background))), None, 0.0, 0.1, "Notch");
        b.pop_in(notch, 0.1);
    }
    let big = b.text(amount, stub_at, 150.0, p.on_accent, b.heavy(), 0.5);
    b.pop_in(big, 0.5);
    let body_at = if tall { center + Vec2::new(0.0, size.y * 0.2 * unit) } else { center + Vec2::new(size.x * 0.2 * unit, 0.0) };
    let head = b.text(title, body_at - Vec2::new(0.0, 110.0 * unit), 56.0, 0x18181b, b.heavy(), 0.8);
    b.fade_in(head, 0.8, 0.5);
    let code_width = (width_of(code, 48.0) + 100.0).max(360.0);
    let box_at = body_at + Vec2::new(0.0, 10.0 * unit);
    let code_box = b.shape(ShapeKind::Rectangle, Vec2::new(code_width, 100.0), box_at, None, Some((0x18181b, 4.0)), 16.0, 1.1, "Code");
    b.pop_in(code_box, 1.1);
    let code_text = b.text(code, box_at, 48.0, 0x18181b, b.heavy(), 1.2);
    b.pop_in(code_text, 1.2);
    b.pulse(code_box, 2.5, 5.5, 2);
    let small = b.text(expiry, body_at + Vec2::new(0.0, 110.0 * unit), 32.0, 0x71717a, 500, 1.4);
    b.fade_in(small, 1.4, 0.5);
    b.fade_everything_out();
}

pub(super) fn glitch_title(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (title, sub) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let head = b.text(title, b.at(0.5, 0.45), 150.0, p.text, b.heavy(), 0.0);
    // Glitching hard as it arrives, settling, and bursting again now and then.
    b.effect(head, "RGB Split", &[(0, 0.0)]);
    b.animate_effect(head, 0, &[(0.0, 40.0, EASE_OUT), (0.8, 6.0, Easing::Linear), (3.0, 6.0, Easing::Hold), (3.1, 30.0, EASE_OUT), (3.5, 6.0, Easing::Linear)]);
    b.effect(head, "Glitch", &[(0, 0.0), (1, 30.0), (2, 14.0)]);
    b.animate_effect(head, 0, &[(0.0, 90.0, EASE_OUT), (0.9, 0.0, Easing::Linear), (3.0, 0.0, Easing::Hold), (3.1, 70.0, EASE_OUT), (3.6, 0.0, Easing::Linear), (6.0, 0.0, Easing::Hold), (6.1, 60.0, EASE_OUT), (6.5, 0.0, Easing::Linear)]);
    b.fade_in(head, 0.0, 0.2);
    let line = b.text(sub, b.at(0.5, 0.62), 50.0, p.accent, b.heavy(), 1.0);
    b.slide_in(line, Vec2::new(0.0, 30.0), 1.0, 0.6);
    // A screen look over everything.
    let adjust = b.block(Vec2::new(b.width, b.height) * (1.0 / unit), b.at(0.5, 0.5), 0x000000, 0.0, 0.0, "Screen look");
    b.layer(adjust).content = LayerContent::Adjustment;
    b.effect(adjust, "Scanlines", &[(0, 4.0), (1, 25.0), (2, 2.0)]);
    b.fade_everything_out();
}
