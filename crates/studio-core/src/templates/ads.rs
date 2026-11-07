//! Designs made for paid ads and creator (UGC) videos: captions over a talking-head video, an
//! "us versus them" comparison, a problem-then-solution story, a product with its features
//! pointed out, a countdown offer and a star-rating proof card. Each ends on a clear call to
//! action, and in 9:16 keeps everything that matters clear of the captions and buttons phone
//! apps draw over the bottom and right of a video.

use super::layouts::button;
use super::social::{full_frame_video, outline, tinted};
use super::{Builder, EASE_IN_OUT, EASE_OUT};
use crate::animation::{Animated, Easing, Vec2};
use crate::model::{Id, Paint, Rgba, ShapeKind};

/// A pill button in any colors, popping in at `at` (for buttons on a colored panel, where the
/// usual accent button would vanish).
fn button_in(b: &mut Builder, words: &str, center: Vec2, fill: u32, ink: u32, at: f64) {
    let width = (words.chars().count() as f32 * 26.0 + 120.0).max(280.0);
    let pill = b.block(Vec2::new(width, 104.0), center, fill, 52.0, at, "Button");
    b.pop_in(pill, at);
    let label = b.text(words, center, 44.0, ink, b.heavy(), at + 0.1);
    b.pop_in(label, at + 0.1);
}

/// A tick drawn from two bars, `size` units across, centered at `at`.
fn tick(b: &mut Builder, at: Vec2, size: f32, color: u32, from: f64) -> [Id; 2] {
    let unit = b.unit() * size / 100.0;
    // The short stroke runs down to the right, the long one up to the right from where it ends.
    let short = b.block(Vec2::new(34.0, 13.0) * (size / 100.0), at + Vec2::new(-18.0, 8.0) * unit, color, 6.5 * size / 100.0, from, "Tick");
    b.transform(short).rotation = Animated::constant(45.0);
    let long = b.block(Vec2::new(62.0, 13.0) * (size / 100.0), at + Vec2::new(8.0, -3.0) * unit, color, 6.5 * size / 100.0, from, "Tick");
    b.transform(long).rotation = Animated::constant(-50.0);
    [short, long]
}

/// A cross drawn from two bars, `size` units across, centered at `at`.
fn cross(b: &mut Builder, at: Vec2, size: f32, color: u32, from: f64) -> [Id; 2] {
    let scale = size / 100.0;
    let first = b.block(Vec2::new(60.0, 13.0) * scale, at, color, 6.5 * scale, from, "Cross");
    b.transform(first).rotation = Animated::constant(45.0);
    let second = b.block(Vec2::new(60.0, 13.0) * scale, at, color, 6.5 * scale, from, "Cross");
    b.transform(second).rotation = Animated::constant(-45.0);
    [first, second]
}

/// White words with a dark outline, the way creators caption their videos.
fn caption(b: &mut Builder, words: &str, at: Vec2, size: f32, from: f64) -> Id {
    let id = b.text(words, at, size, 0xffffff, b.heavy(), from);
    outline(b, id, 0x000000, size * 0.1);
    id
}

/// A creator talking to camera: a hook in a box at the top, three captions one after another,
/// then a call to action. Words: hook, three captions, button.
pub(super) fn ugc_captions(b: &mut Builder) {
    let p = b.palette();
    full_frame_video(b, "talking-head video");
    let (hook, cta) = (b.copy(0), b.copy(4));
    let unit = b.unit();
    let end = b.end();
    // The hook: dark words on a light box, as phone apps style it, for the first few seconds.
    let hook_at = b.at(0.5, if b.vertical() { 0.17 } else { 0.15 });
    let lines = hook.lines().count().max(1) as f32;
    let widest = hook.lines().map(|line| line.chars().count()).max().unwrap_or(10) as f32;
    let box_width = (widest * 36.0 + 110.0).min(b.width / unit - 100.0);
    let hook_box = b.block(Vec2::new(box_width, lines * 80.0 + 60.0), hook_at, p.background, 22.0, 0.15, "Hook box");
    b.pop_in(hook_box, 0.15);
    let hook_words = b.text(hook, hook_at, 64.0, p.text, b.heavy(), 0.25);
    b.words_in(hook_words, 0.7);
    for id in [hook_box, hook_words] {
        b.fade_out(id, 3.4, 0.3);
        b.until(id, 3.4);
    }
    // Three captions, each word popping in, lower down where captions sit.
    let caption_at = b.at(0.5, if b.vertical() { 0.62 } else if b.tall() { 0.7 } else { 0.78 });
    let slots: [(f64, f64); 3] = [(0.6, 3.6), (3.6, 6.6), (6.6, 9.6)];
    for (index, (from, until)) in slots.into_iter().enumerate() {
        let words = b.copy(index + 1);
        let line = caption(b, words, caption_at, 80.0, from);
        let count = words.split_whitespace().count().max(1) as f32;
        let animator = crate::text_animation::TextAnimator {
            motion: crate::text_animation::TextMotion::Pop,
            unit: crate::text_animation::TextUnit::Word,
            order: crate::text_animation::TextOrder::Forward,
            ease: crate::text_animation::TextEase::Overshoot,
            seconds: (0.18 * count).clamp(0.3, 1.2),
            overlap: 0.3,
        };
        b.layer(line).animation.text_in = Some(animator);
        b.until(line, until.min(end));
    }
    // The call to action: a card in the background color sliding up with the button, gently pulsing until the end.
    let card_at = b.at(0.5, if b.vertical() { 0.72 } else { 0.8 });
    let card = tinted(b, Vec2::new((b.width / unit - 140.0).min(760.0), 170.0), card_at, p.background, 0.92, 40.0, 9.8, "Call to action card");
    b.slide_in(card, Vec2::new(0.0, 120.0), 9.8, 0.7);
    button(b, cta, card_at, 10.1);
    let ids: Vec<_> = b.project.composition(b.comp).map(|comp| comp.layers.iter().filter(|layer| layer.name == "Button").map(|layer| layer.id).collect()).unwrap_or_default();
    for id in ids {
        b.pulse(id, 11.0, end - 0.8, 2);
    }
    b.fade_everything_out();
}

/// Us versus them: a title, two column headings, three rows of ticks for us and crosses for
/// them, and a button. Words: title, our name, their name, three rows, button.
pub(super) fn versus_table(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (title, ours, theirs, cta) = (b.copy(0), b.copy(1), b.copy(2), b.copy(6));
    let unit = b.unit();
    let vertical = b.vertical();
    let tall = b.tall();
    let head_at = b.at(0.5, if vertical { 0.17 } else if tall { 0.12 } else { 0.13 });
    let head = b.text(title, head_at, 96.0, p.text, b.heavy(), 0.2);
    b.words_in(head, 0.7);
    // The table: a label column, then ours (highlighted), then theirs.
    let table_width = (b.width / unit - 100.0).min(1000.0);
    let center_x = b.at(0.5, 0.0).x;
    let label_x = center_x - table_width * 0.2 * unit;
    let ours_x = center_x + table_width * 0.18 * unit;
    let theirs_x = center_x + table_width * 0.39 * unit;
    let row_height = if vertical { 170.0 } else if tall { 150.0 } else { 125.0 };
    let header_y = b.below(head_at, title, 96.0, 70.0, 90.0, b.at(0.5, if vertical { 0.32 } else { 0.3 })).y;
    let column_height = row_height * 3.0 + 150.0;
    let highlight_at = Vec2::new(ours_x, header_y + (column_height * 0.5 - 60.0) * unit);
    let highlight = tinted(b, Vec2::new(table_width * 0.2, column_height), highlight_at, p.accent, 0.18, 36.0, 0.8, "Highlight");
    b.scale(highlight, 0.8, 0.0, 1.4, 100.0, EASE_OUT);
    b.fade_in(highlight, 0.8, 0.3);
    for (words, x, fill, ink, at) in [(ours, ours_x, p.accent, p.on_accent, 0.9), (theirs, theirs_x, p.background_2, p.muted, 1.1)] {
        let center = Vec2::new(x, header_y);
        let width = (words.chars().count() as f32 * 26.0 + 70.0).clamp(150.0, table_width * 0.2);
        let pill = b.block(Vec2::new(width, 80.0), center, fill, 40.0, at, "Column heading");
        b.pop_in(pill, at);
        let label = b.text(words, center, 40.0, ink, b.heavy(), at + 0.05);
        b.pop_in(label, at + 0.05);
    }
    for index in 0..3 {
        let at = 1.6 + index as f64 * 0.7;
        let y = header_y + row_height * (index as f32 + 1.0) * unit;
        let rule = b.block(Vec2::new(table_width, 3.0), Vec2::new(center_x, y - row_height * 0.5 * unit), p.muted, 1.5, at, "Rule");
        b.transform(rule).opacity.value = 30.0;
        b.scale(rule, at, 0.0, at + 0.5, 100.0, EASE_OUT);
        let words = b.text(b.copy(index + 3), Vec2::new(label_x, y), 52.0, p.text, b.light().max(500), at);
        b.slide_in(words, Vec2::new(-40.0, 0.0), at, 0.5);
        let yes = b.shape(ShapeKind::Ellipse, Vec2::new(84.0, 84.0), Vec2::new(ours_x, y), Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, at + 0.2, "Yes");
        b.pop_in(yes, at + 0.2);
        for id in tick(b, Vec2::new(ours_x, y), 70.0, p.on_accent, at + 0.3) {
            b.pop_in(id, at + 0.3);
        }
        for id in cross(b, Vec2::new(theirs_x, y), 60.0, p.muted, at + 0.4) {
            b.pop_in(id, at + 0.4);
        }
    }
    let table_bottom = header_y + (row_height * 3.5) * unit;
    let button_at = Vec2::new(center_x, b.at(0.5, if vertical { 0.79 } else { 0.88 }).y.max(table_bottom + 90.0 * unit));
    button(b, cta, button_at, 4.0);
    b.fade_everything_out();
}

/// A problem, crossed out, then the answer on a panel sweeping up over it. Words: problem
/// kicker, the problem, solution kicker, the solution, button.
pub(super) fn problem_solution(b: &mut Builder) {
    let p = b.palette();
    let (problem_kicker, problem, fix_kicker, fix, cta) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3), b.copy(4));
    let unit = b.unit();
    let full = Vec2::new(b.width, b.height) * (1.0 / unit);
    let (vertical, tall) = (b.vertical(), b.tall());
    let switch = 4.4;
    // Act one, on a dark ground: the problem, struck through.
    let ground = b.block(full, b.at(0.5, 0.5), 0x111114, 0.0, 0.0, "Background");
    b.scale(ground, 0.0, 100.0, switch + 1.0, 106.0, Easing::Linear);
    // The kicker sits just above the problem, however many lines that runs to.
    let statement_at = b.at(0.5, 0.48);
    let kick_at = statement_at - Vec2::new(0.0, b.text_height(problem, 104.0) * 0.5 + 50.0 * unit);
    let kick = b.text(problem_kicker, kick_at, 42.0, 0xef4444, b.heavy(), 0.2);
    b.slide_in(kick, Vec2::new(0.0, 24.0), 0.2, 0.6);
    let statement = b.text(problem, statement_at, 104.0, 0xffffff, b.heavy(), 0.5);
    b.words_in(statement, 1.0);
    b.effect(statement, "Camera Shake", &[(0, 0.0), (1, 14.0), (2, 1.0), (3, 2.0)]);
    b.animate_effect(statement, 0, &[(2.5, 10.0, Easing::Linear), (3.0, 0.0, Easing::Linear)]);
    let mut struck = vec![kick, statement];
    // A red line through each line of the problem, one after the other.
    let lines: Vec<&str> = problem.lines().collect();
    for (index, words) in lines.iter().enumerate() {
        let offset = (index as f32 - (lines.len() as f32 - 1.0) * 0.5) * 104.0 * 1.15 * unit;
        let width = (words.chars().count() as f32 * 56.0).min(full.x - 120.0);
        let at = 2.4 + index as f64 * 0.2;
        let strike = b.block(Vec2::new(width, 14.0), statement_at + Vec2::new(0.0, offset), 0xef4444, 7.0, at, "Strike");
        b.transform(strike).rotation = Animated::constant(-3.0);
        b.scale(strike, at, 0.0, at + 0.4, 100.0, EASE_OUT);
        struck.push(strike);
    }
    for id in struck {
        b.until(id, switch + 0.8);
    }
    // Act two: the accent panel sweeps up and the answer arrives on it.
    let panel = b.block(full, b.at(0.5, 0.5), p.accent, 0.0, switch, "Panel");
    b.transform(panel).position = Animated::constant(b.at(0.5, 1.5));
    b.move_to(panel, switch, switch + 0.7, b.at(0.5, 0.5), super::EASE_OUT);
    let (kick_at, head_at, photo_at, photo_side, button_at) = if vertical {
        (b.at(0.5, 0.15), b.at(0.5, 0.26), b.at(0.5, 0.56), 560.0, b.at(0.5, 0.79))
    } else if tall {
        (b.at(0.5, 0.1), b.at(0.5, 0.2), b.at(0.5, 0.53), 420.0, b.at(0.5, 0.85))
    } else {
        (b.at(0.3, 0.3), b.at(0.3, 0.45), b.at(0.72, 0.5), 600.0, b.at(0.3, 0.72))
    };
    let fix_kick = b.text(fix_kicker, kick_at, 42.0, p.on_accent, b.heavy(), switch + 0.5);
    b.slide_in(fix_kick, Vec2::new(0.0, 24.0), switch + 0.5, 0.5);
    let answer = b.text(fix, head_at, 100.0, p.on_accent, b.heavy(), switch + 0.7);
    b.words_in(answer, 0.8);
    let photo = b.placeholder(Vec2::new(photo_side, photo_side), photo_at, 44.0, switch + 1.0, "product photo");
    b.pop_in(photo, switch + 1.0);
    b.effect(photo, "Drop Shadow", &[(0, 45.0), (2, 20.0), (3, 45.0)]);
    b.move_to(photo, switch + 1.6, b.end(), photo_at + Vec2::new(0.0, -20.0 * unit), EASE_IN_OUT);
    button_in(b, cta, button_at, p.background, p.text, switch + 1.8);
    b.fade_everything_out();
}

/// A product with three of its features pointed out, one after another. Words: title, three
/// features, button.
pub(super) fn feature_callouts(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (title, cta) = (b.copy(0), b.copy(4));
    let unit = b.unit();
    let (vertical, tall) = (b.vertical(), b.tall());
    let head_at = b.at(0.5, if vertical { 0.15 } else if tall { 0.1 } else { 0.12 });
    let head = b.text(title, head_at, 92.0, p.text, b.heavy(), 0.2);
    b.words_in(head, 0.7);
    let (photo_at, side) = if vertical { (b.at(0.5, 0.46), 600.0) } else if tall { (b.at(0.5, 0.5), 520.0) } else { (b.at(0.5, 0.55), 560.0) };
    let photo = b.placeholder(Vec2::new(side, side), photo_at, 48.0, 0.4, "product photo");
    b.pop_in(photo, 0.4);
    b.effect(photo, "Drop Shadow", &[(0, 45.0), (2, 22.0), (3, 50.0)]);
    let margin = 40.0 * unit;
    for index in 0..3 {
        let at = 1.4 + index as f64 * 0.9;
        let words = b.copy(index + 1);
        let direction = if index == 1 { 1.0 } else { -1.0 };
        let dy = [-0.3, 0.02, 0.34][index] * side * unit;
        let dot_at = photo_at + Vec2::new(direction * 0.16 * side * unit, dy);
        // The label beside the photo, kept inside the frame.
        let width = words.chars().count() as f32 * 27.0 + 90.0;
        let wanted = photo_at.x + direction * (side * 0.5 + 40.0 + width * 0.5) * unit;
        let half = width * 0.5 * unit;
        let x = wanted.clamp(margin + half, b.width - margin - half);
        let label_at = Vec2::new(x, dot_at.y);
        // The line from the dot to the label's near edge, growing outwards.
        let edge = x - direction * half;
        let length = ((edge - dot_at.x) * direction).max(0.0) / unit;
        if length > 4.0 {
            let line_at = Vec2::new(dot_at.x + direction * length * 0.5 * unit, dot_at.y);
            let line = b.block(Vec2::new(length, 5.0), line_at, p.accent, 2.5, at + 0.2, "Pointer");
            b.scale(line, at + 0.2, 0.0, at + 0.6, 100.0, EASE_OUT);
        }
        let ring = b.shape(ShapeKind::Ellipse, Vec2::new(70.0, 70.0), dot_at, None, Some((p.accent, 6.0)), 0.0, at, "Ring");
        b.pop_in(ring, at);
        b.pulse(ring, at + 0.8, b.end() - 0.8, 3);
        let dot = b.shape(ShapeKind::Ellipse, Vec2::new(30.0, 30.0), dot_at, Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, at, "Dot");
        b.pop_in(dot, at);
        let pill = b.block(Vec2::new(width, 92.0), label_at, p.background_2, 46.0, at + 0.45, "Feature");
        b.pop_in(pill, at + 0.45);
        b.effect(pill, "Drop Shadow", &[(0, 35.0), (2, 8.0), (3, 18.0)]);
        let label = b.text(words, label_at, 46.0, p.text, b.heavy(), at + 0.5);
        b.pop_in(label, at + 0.5);
    }
    let button_at = b.at(0.5, if vertical { 0.79 } else { 0.9 });
    button(b, cta, button_at, 4.4);
    b.fade_everything_out();
}

/// A limited-time offer with a clock counting down. Words: kicker, the offer, line under it,
/// button.
pub(super) fn offer_timer(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (kicker, offer, sub, cta) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let unit = b.unit();
    let (vertical, tall) = (b.vertical(), b.tall());
    let end = b.end();
    let ys: [f32; 6] = if vertical { [0.17, 0.3, 0.42, 0.52, 0.6, 0.79] } else if tall { [0.1, 0.24, 0.36, 0.46, 0.57, 0.84] } else { [0.12, 0.28, 0.44, 0.54, 0.68, 0.88] };
    let kick_at = b.at(0.5, ys[0]);
    let width = kicker.chars().count() as f32 * 26.0 + 90.0;
    let tag = b.block(Vec2::new(width, 78.0), kick_at, p.accent, 39.0, 0.2, "Tag");
    b.pop_in(tag, 0.2);
    let tag_words = b.text(kicker, kick_at, 38.0, p.on_accent, b.heavy(), 0.25);
    b.pop_in(tag_words, 0.25);
    let big = b.text(offer, b.at(0.5, ys[1]), if tall { 200.0 } else { 170.0 }, p.text, b.heavy(), 0.5);
    b.letters_in(big, 0.6);
    b.pulse(big, 1.6, end - 1.0, 4);
    let line = b.text(sub, b.at(0.5, ys[2]), 50.0, p.muted, b.light().max(500), 1.0);
    b.slide_in(line, Vec2::new(0.0, 24.0), 1.0, 0.6);
    let label = b.text("ENDS IN", b.at(0.5, ys[3]), 36.0, p.accent, b.heavy(), 1.4);
    b.fade_in(label, 1.4, 0.4);
    // Hours, minutes and seconds in boxes. The seconds tick down each second; when they pass
    // zero, the minutes go down one.
    let box_side = if tall { 190.0 } else { 160.0 };
    let gap = box_side + 60.0;
    let row_y = b.at(0.5, ys[4]).y;
    let start = 1.5;
    let first_second = 7u32;
    for (column, caption) in ["HRS", "MIN", "SEC"].into_iter().enumerate() {
        let x = b.at(0.5, 0.0).x + (column as f32 - 1.0) * gap * unit;
        let center = Vec2::new(x, row_y);
        let cell = b.block(Vec2::new(box_side, box_side), center, p.background_2, 28.0, start + column as f64 * 0.1, "Clock box");
        b.pop_in(cell, start + column as f64 * 0.1);
        let small = b.text(caption, center + Vec2::new(0.0, (box_side * 0.5 + 38.0) * unit), 34.0, p.muted, b.heavy(), start + 0.3);
        b.fade_in(small, start + 0.3, 0.4);
        if column < 2 {
            let colon = b.text(":", Vec2::new(x + gap * 0.5 * unit, row_y - 8.0 * unit), 90.0, p.muted, b.heavy(), start + 0.2);
            b.fade_in(colon, start + 0.2, 0.3);
        }
    }
    let digits_at = |b: &Builder, column: usize| Vec2::new(b.at(0.5, 0.0).x + (column as f32 - 1.0) * gap * unit, row_y);
    let hours = b.text("05", digits_at(b, 0), box_side * 0.5, p.text, b.heavy(), start + 0.2);
    b.pop_in(hours, start + 0.2);
    // One layer per value the minutes and seconds show, each on screen for its turn.
    let mut tick_at = start + 0.3;
    let mut second = first_second;
    let mut minute = 42u32;
    let mut minute_from = start + 0.3;
    while tick_at < end {
        let next = (tick_at.floor() + 1.0).min(end);
        let digits = b.text(&format!("{second:02}"), digits_at(b, 2), box_side * 0.5, p.text, b.heavy(), tick_at);
        if tick_at <= start + 0.3 {
            b.pop_in(digits, tick_at);
        } else {
            drop_in(b, digits, tick_at);
        }
        b.until(digits, next);
        if second == 0 {
            let minutes = b.text(&format!("{minute:02}"), digits_at(b, 1), box_side * 0.5, p.text, b.heavy(), minute_from);
            b.pop_in(minutes, minute_from);
            b.until(minutes, next);
            minute -= 1;
            minute_from = next;
            second = 59;
        } else {
            second -= 1;
        }
        tick_at = next;
    }
    let minutes = b.text(&format!("{minute:02}"), digits_at(b, 1), box_side * 0.5, p.text, b.heavy(), minute_from);
    if minute_from <= start + 0.3 {
        b.pop_in(minutes, minute_from);
    } else {
        drop_in(b, minutes, minute_from);
    }
    button(b, cta, b.at(0.5, ys[5]), 2.4);
    b.fade_everything_out();
}

/// Drops a clock digit into place from a little above, fully shown from its first frame, so
/// the box never flashes empty as one value replaces the last.
fn drop_in(b: &mut Builder, id: Id, at: f64) {
    let home = b.transform(id).position.value;
    let keys = vec![b.key(at, home - Vec2::new(0.0, 24.0 * b.unit()), EASE_OUT), b.key(at + 0.25, home, Easing::Linear)];
    Builder::keys(&mut b.transform(id).position, keys);
}

/// A star rating as proof: a headline, a big score, five stars filling in, how many reviews, a
/// row of customer photos and a button. Words: headline, score, reviews line, button.
pub(super) fn social_proof(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (title, score, reviews, cta) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let unit = b.unit();
    let (vertical, tall) = (b.vertical(), b.tall());
    let ys: [f32; 6] = if vertical { [0.18, 0.37, 0.48, 0.545, 0.64, 0.79] } else if tall { [0.12, 0.32, 0.45, 0.52, 0.64, 0.84] } else { [0.14, 0.36, 0.53, 0.61, 0.73, 0.89] };
    let head_at = b.at(0.5, ys[0]);
    let head = b.text(title, head_at, 92.0, p.text, b.heavy(), 0.2);
    b.words_in(head, 0.8);
    let number = b.text(score, b.at(0.5, ys[1]), if tall { 260.0 } else { 200.0 }, p.accent, b.heavy(), 0.8);
    b.letters_in(number, 0.6);
    let star_y = b.at(0.5, ys[2]).y;
    for index in 0..5 {
        let x = b.at(0.5, 0.0).x + (index as f32 - 2.0) * 100.0 * unit;
        let at = 1.4 + index as f64 * 0.12;
        let empty = b.shape(ShapeKind::Star, Vec2::new(84.0, 84.0), Vec2::new(x, star_y), None, Some((p.muted, 4.0)), 4.0, 1.2, "Star outline");
        b.fade_in(empty, 1.2, 0.3);
        let star = b.shape(ShapeKind::Star, Vec2::new(84.0, 84.0), Vec2::new(x, star_y), Some(Paint::Solid(Rgba::hex(0xfacc15))), None, 4.0, at, "Star");
        b.pop_in(star, at);
    }
    let line = b.text(reviews, b.at(0.5, ys[3]), 46.0, p.muted, b.light().max(500), 2.0);
    b.slide_in(line, Vec2::new(0.0, 24.0), 2.0, 0.5);
    // Customer photos overlapping in a row, the last one a "+" count.
    let face_y = b.at(0.5, ys[4]).y;
    for index in 0..5 {
        let x = b.at(0.5, 0.0).x + (index as f32 - 2.0) * 96.0 * unit;
        let at = 2.4 + index as f64 * 0.1;
        let face = b.shape(ShapeKind::Ellipse, Vec2::new(124.0, 124.0), Vec2::new(x, face_y), Some(Paint::linear(Rgba::hex(0x3f3f46), Rgba::hex(0x27272a), 60.0)), Some((p.background, 8.0)), 0.0, at, "Drop a customer photo here");
        b.pop_in(face, at);
    }
    button(b, cta, b.at(0.5, ys[5]), 3.0);
    b.fade_everything_out();
}
