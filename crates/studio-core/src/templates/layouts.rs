//! The first twenty animated designs (see `Layout`; `fun`, `social` and `ads` have the rest). Each places its layers by fractions of the frame,
//! choosing a stacked arrangement for the tall formats and a side-by-side one for 16:9 where
//! that reads better, and times everything in seconds from the start.

use super::{Builder, EASE_IN_OUT, EASE_OUT, Layout};
use crate::animation::{Easing, Vec2};
use crate::model::{Paint, Rgba, ShapeKind, TextAlign};

/// How many words (entries of `copy`) each layout uses.
#[cfg(test)]
pub(super) fn words_needed(layout: Layout) -> usize {
    match layout {
        Layout::Headline | Layout::SplitPromo | Layout::SaleBurst | Layout::ProductCard | Layout::Event | Layout::Steps => 4,
        Layout::List => 5,
        Layout::Quote | Layout::Countdown | Layout::Review | Layout::BigStat | Layout::BeforeAfter | Layout::Announcement | Layout::Podcast | Layout::EndCard => 3,
        Layout::LowerThird | Layout::Hook | Layout::BrandIntro | Layout::Collage | Layout::Marquee => 2,
        Layout::Chat => 5,
        Layout::Notification | Layout::PostCard | Layout::Quiz | Layout::KineticStack | Layout::Coupon => 4,
        Layout::ThisOrThat | Layout::PhoneMockup | Layout::Progress => 3,
        Layout::Meme | Layout::Typewriter | Layout::Neon | Layout::Confetti | Layout::Polaroid | Layout::NowPlaying | Layout::MapPin | Layout::GlitchTitle => 2,
        Layout::ProfileIntro => 6,
        Layout::StoryFrame => 4,
        Layout::AskMeAnything | Layout::NameReveal | Layout::CommentReply | Layout::FloatingHearts | Layout::OutlineStack | Layout::VlogTitle => 3,
        Layout::UgcCaptions | Layout::ProblemSolution | Layout::FeatureCallouts => 5,
        Layout::VersusTable => 7,
        Layout::OfferTimer | Layout::SocialProof => 4,
        Layout::PosterStretch | Layout::PhotoTagline => 4,
        Layout::SlidingBlocks | Layout::CardStack => 3,
        Layout::TypeRepeater | Layout::BoldCountdown | Layout::BlurSpin | Layout::SplitReveal | Layout::GradientTitle | Layout::BouncyPeriod => 2,
    }
}

pub(super) fn build(b: &mut Builder) {
    match b.template.layout {
        Layout::Headline => headline(b),
        Layout::SplitPromo => split_promo(b),
        Layout::List => list(b),
        Layout::Quote => quote(b),
        Layout::Countdown => countdown(b),
        Layout::LowerThird => lower_third(b),
        Layout::Review => review(b),
        Layout::SaleBurst => sale_burst(b),
        Layout::Hook => hook(b),
        Layout::ProductCard => product_card(b),
        Layout::Event => event(b),
        Layout::BigStat => big_stat(b),
        Layout::BeforeAfter => before_after(b),
        Layout::Steps => steps(b),
        Layout::Announcement => announcement(b),
        Layout::Podcast => podcast(b),
        Layout::BrandIntro => brand_intro(b),
        Layout::Collage => collage(b),
        Layout::Marquee => marquee(b),
        Layout::EndCard => end_card(b),
        Layout::Chat => super::fun::chat(b),
        Layout::Notification => super::fun::notification(b),
        Layout::PostCard => super::fun::post_card(b),
        Layout::ThisOrThat => super::fun::this_or_that(b),
        Layout::Meme => super::fun::meme(b),
        Layout::Typewriter => super::fun::typewriter(b),
        Layout::Neon => super::fun::neon(b),
        Layout::Confetti => super::fun::confetti(b),
        Layout::PhoneMockup => super::fun::phone_mockup(b),
        Layout::Polaroid => super::fun::polaroid(b),
        Layout::Quiz => super::fun::quiz(b),
        Layout::NowPlaying => super::fun::now_playing(b),
        Layout::Progress => super::fun::progress(b),
        Layout::KineticStack => super::fun::kinetic_stack(b),
        Layout::MapPin => super::fun::map_pin(b),
        Layout::Coupon => super::fun::coupon(b),
        Layout::GlitchTitle => super::fun::glitch_title(b),
        Layout::ProfileIntro => super::social::profile_intro(b),
        Layout::AskMeAnything => super::social::ask_me_anything(b),
        Layout::NameReveal => super::social::name_reveal(b),
        Layout::StoryFrame => super::social::story_frame(b),
        Layout::CommentReply => super::social::comment_reply(b),
        Layout::FloatingHearts => super::social::floating_hearts(b),
        Layout::OutlineStack => super::social::outline_stack(b),
        Layout::VlogTitle => super::social::vlog_title(b),
        Layout::UgcCaptions => super::ads::ugc_captions(b),
        Layout::VersusTable => super::ads::versus_table(b),
        Layout::ProblemSolution => super::ads::problem_solution(b),
        Layout::FeatureCallouts => super::ads::feature_callouts(b),
        Layout::OfferTimer => super::ads::offer_timer(b),
        Layout::SocialProof => super::ads::social_proof(b),
        Layout::PosterStretch => super::editorial::poster_stretch(b),
        Layout::SlidingBlocks => super::editorial::sliding_blocks(b),
        Layout::TypeRepeater => super::editorial::type_repeater(b),
        Layout::CardStack => super::editorial::card_stack(b),
        Layout::BoldCountdown => super::editorial::bold_countdown(b),
        Layout::BlurSpin => super::editorial::blur_spin(b),
        Layout::SplitReveal => super::editorial::split_reveal(b),
        Layout::GradientTitle => super::editorial::gradient_title(b),
        Layout::PhotoTagline => super::editorial::photo_tagline(b),
        Layout::BouncyPeriod => super::editorial::bouncy_period(b),
    }
}

/// A pill-shaped button with words on it, popping in at `at`.
pub(super) fn button(b: &mut Builder, words: &str, center: Vec2, at: f64) {
    let p = b.palette();
    let width = (words.chars().count() as f32 * 26.0 + 120.0).max(280.0);
    let pill = b.block(Vec2::new(width, 104.0), center, p.accent, 52.0, at, "Button");
    b.pop_in(pill, at);
    let label = b.text(words, center, 44.0, p.on_accent, b.heavy(), at + 0.1);
    b.pop_in(label, at + 0.1);
}

fn headline(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (kicker, title, sub, cta) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let top = if b.vertical() { 0.34 } else { 0.3 };
    let kick = b.text(kicker, b.at(0.5, top), 42.0, p.accent, b.heavy(), 0.3);
    b.slide_in(kick, Vec2::new(0.0, 30.0), 0.3, 0.7);
    let head_at = b.at(0.5, top + 0.13);
    let head = b.text(title, head_at, 128.0, p.text, b.heavy(), 0.6);
    b.words_in(head, 0.9);
    // Under the headline however many lines it runs to.
    let bar_at = b.below(head_at, title, 128.0, 36.0, 10.0, b.at(0.5, top + 0.26));
    let bar = b.block(Vec2::new(220.0, 10.0), bar_at, p.accent, 5.0, 1.2, "Underline");
    b.scale(bar, 1.2, 0.0, 2.0, 100.0, EASE_OUT);
    let line_at = bar_at + Vec2::new(0.0, 66.0 * b.unit());
    let line = b.text(sub, line_at, 48.0, p.muted, b.light(), 1.6);
    b.slide_in(line, Vec2::new(0.0, 24.0), 1.6, 0.8);
    let button_at = Vec2::new(line_at.x, b.at(0.5, if b.vertical() { 0.76 } else { 0.82 }).y.max(line_at.y + 120.0 * b.unit()));
    button(b, cta, button_at, 2.6);
    b.fade_everything_out();
}

fn split_promo(b: &mut Builder) {
    let p = b.palette();
    let (title, sub, price, cta) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let tall = b.tall();
    // Two panels: side by side in 16:9, one above the other in the tall formats.
    let (first_size, first_at, second_size, second_at) = if tall {
        (Vec2::new(b.width, b.height * 0.5), b.at(0.5, 0.25), Vec2::new(b.width, b.height * 0.5), b.at(0.5, 0.75))
    } else {
        (Vec2::new(b.width * 0.55, b.height), b.at(0.275, 0.5), Vec2::new(b.width * 0.45, b.height), b.at(0.775, 0.5))
    };
    let unit = b.unit();
    let first = b.block(first_size * (1.0 / unit), first_at, p.background, 0.0, 0.0, "Panel");
    let second = b.block(second_size * (1.0 / unit), second_at, p.accent, 0.0, 0.0, "Panel");
    let away = if tall { Vec2::new(0.0, -900.0) } else { Vec2::new(-900.0, 0.0) };
    b.slide_in(first, away, 0.0, 0.8);
    b.slide_in(second, away * -1.0, 0.0, 0.8);
    let text_at = if tall { b.at(0.5, 0.24) } else { b.at(0.275, 0.42) };
    let head = b.text(title, text_at, 112.0, p.text, b.heavy(), 0.7);
    b.words_in(head, 0.8);
    let line_at = b.below(text_at, title, 112.0, 30.0, 46.0, text_at);
    let line = b.text(sub, line_at, 46.0, p.muted, b.light(), 1.2);
    b.slide_in(line, Vec2::new(0.0, 24.0), 1.2, 0.7);
    let badge_at = if tall { b.at(0.5, if b.vertical() { 0.64 } else { 0.7 }) } else { b.at(0.775, 0.45) };
    let badge = b.shape(ShapeKind::Ellipse, Vec2::new(330.0, 330.0), badge_at, Some(Paint::Solid(Rgba::hex(p.on_accent))), None, 0.0, 1.6, "Price badge");
    b.pop_in(badge, 1.6);
    let amount = b.text(price, badge_at, 96.0, p.accent, b.heavy(), 1.8);
    b.pop_in(amount, 1.8);
    b.pulse(badge, 3.0, 7.0, 2);
    b.effect(badge, "Drop Shadow", &[(0, 40.0), (2, 18.0), (3, 30.0)]);
    // Clear of the captions and buttons a phone app draws over the bottom of a 9:16 video.
    let button_at = if tall { b.at(0.5, if b.vertical() { 0.79 } else { 0.9 }) } else { b.at(0.775, 0.78) };
    let p2 = p;
    let width = (cta.chars().count() as f32 * 26.0 + 120.0).max(280.0);
    let pill = b.block(Vec2::new(width, 100.0), button_at, p2.background, 50.0, 3.2, "Button");
    b.pop_in(pill, 3.2);
    let label = b.text(cta, button_at, 42.0, p2.text, b.heavy(), 3.3);
    b.pop_in(label, 3.3);
    b.fade_everything_out();
}

fn list(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let title = b.copy(0);
    let top = if b.vertical() { 0.2 } else { 0.16 };
    let head = b.text(title, b.at(0.5, top), 96.0, p.text, b.heavy(), 0.3);
    b.words_in(head, 0.8);
    let gap = if b.vertical() { 0.14 } else if b.tall() { 0.15 } else { 0.18 };
    let left = if b.tall() { 0.14 } else { 0.28 };
    for index in 0..3 {
        let at = 1.4 + index as f64 * 1.2;
        let y = top + 0.2 + gap * index as f32;
        let badge = b.shape(ShapeKind::Ellipse, Vec2::new(96.0, 96.0), b.at(left, y), Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, at, "Number");
        b.pop_in(badge, at);
        let number = b.text(&format!("{}", index + 1), b.at(left, y), 52.0, p.on_accent, b.heavy(), at + 0.05);
        b.pop_in(number, at + 0.05);
        let words = b.copy(index + 1);
        let item = b.text_aligned(words, b.at(left + 0.08 + 0.32, y), 54.0, p.text, b.light().max(500), at + 0.15, TextAlign::Left);
        // Left-aligned text grows from its left edge, beside the number.
        b.anchor_left(item);
        let width = b.layer(item).size.x;
        let left_px = b.at(left + 0.07, y);
        b.transform(item).position = crate::animation::Animated::constant(left_px);
        let _ = width;
        b.slide_in(item, Vec2::new(60.0, 0.0), at + 0.15, 0.6);
    }
    button(b, b.copy(4), b.at(0.5, if b.vertical() { 0.8 } else { 0.86 }), 5.0);
    b.fade_everything_out();
}

fn quote(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (words, who, place) = (b.copy(0), b.copy(1), b.copy(2));
    let frame = Vec2::new(b.width / b.unit() - 120.0, b.height / b.unit() - 120.0);
    let border = b.shape(ShapeKind::Rectangle, frame, b.at(0.5, 0.5), None, Some((p.muted, 3.0)), 24.0, 0.0, "Frame");
    b.fade_in(border, 0.0, 1.2);
    let mark = b.text("“", b.at(0.5, if b.vertical() { 0.3 } else { 0.24 }), 260.0, p.accent, b.heavy(), 0.2);
    b.pop_in(mark, 0.2);
    let body = b.text(words, b.at(0.5, 0.5), 66.0, p.text, b.heavy(), 0.8);
    b.words_in(body, 1.4);
    let rule = b.block(Vec2::new(120.0, 6.0), b.at(0.5, if b.vertical() { 0.66 } else { 0.7 }), p.accent, 3.0, 2.0, "Rule");
    b.scale(rule, 2.0, 0.0, 2.6, 100.0, EASE_OUT);
    let name = b.text(who, b.at(0.5, if b.vertical() { 0.7 } else { 0.76 }), 46.0, p.text, b.heavy(), 2.3);
    b.fade_in(name, 2.3, 0.6);
    let role = b.text(place, b.at(0.5, if b.vertical() { 0.735 } else { 0.82 }), 36.0, p.muted, b.light(), 2.6);
    b.fade_in(role, 2.6, 0.6);
    b.fade_everything_out();
}

fn countdown(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (kicker, reveal, sub) = (b.copy(0), b.copy(1), b.copy(2));
    let kick = b.text(kicker, b.at(0.5, 0.2), 44.0, p.muted, b.heavy(), 0.2);
    b.fade_in(kick, 0.2, 0.6);
    b.until(kick, 3.6);
    // 3, 2, 1: each pops in, holds, and fades as the next arrives, with a ring spreading out.
    for (index, number) in ["3", "2", "1"].into_iter().enumerate() {
        let at = 0.4 + index as f64;
        let digit = b.text(number, b.at(0.5, 0.5), 360.0, p.text, b.heavy(), at);
        b.pop_in(digit, at);
        b.fade_out(digit, at + 0.95, 0.25);
        b.until(digit, at + 1.0);
        let ring = b.shape(ShapeKind::Ellipse, Vec2::new(420.0, 420.0), b.at(0.5, 0.5), None, Some((p.accent, 8.0)), 0.0, at, "Ring");
        b.scale(ring, at, 60.0, at + 1.0, 180.0, EASE_OUT);
        b.fade_out(ring, at + 1.0, 0.8);
        b.until(ring, at + 1.0);
    }
    let big = b.text(reveal, b.at(0.5, 0.46), 140.0, p.accent, b.heavy(), 3.4);
    b.letters_in(big, 0.6);
    b.effect(big, "Glow", &[(0, 40.0), (1, 30.0), (2, 1.2)]);
    let line_at = b.below(b.at(0.5, 0.46), reveal, 140.0, 40.0, 50.0, b.at(0.5, 0.6));
    let line = b.text(sub, line_at, 50.0, p.text, b.light(), 4.0);
    b.slide_in(line, Vec2::new(0.0, 40.0), 4.0, 0.7);
    b.fade_everything_out();
}

fn lower_third(b: &mut Builder) {
    let p = b.palette();
    let (name, title) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    // Bottom left in 16:9; higher in the tall formats, above where captions usually sit.
    let y = if b.vertical() { 0.66 } else if b.tall() { 0.74 } else { 0.8 };
    let x = if b.tall() { 0.08 } else { 0.06 };
    let bar_width = (name.chars().count().max(title.chars().count()) as f32 * 30.0 + 120.0).max(520.0);
    let bar_at = b.at(x, y) + Vec2::new(bar_width * 0.5 * unit, 0.0);
    let accent = b.block(Vec2::new(16.0, 170.0), b.at(x, y) + Vec2::new(-14.0 * unit, 0.0), p.accent, 4.0, 0.4, "Accent");
    b.scale(accent, 0.4, 0.0, 0.8, 100.0, EASE_OUT);
    let bar = b.block(Vec2::new(bar_width, 170.0), bar_at, p.background, 10.0, 0.5, "Bar");
    b.anchor_left(bar);
    b.grow_across(bar, 0.5, 0.6);
    b.effect(bar, "Drop Shadow", &[(0, 35.0), (2, 8.0), (3, 20.0)]);
    let left = b.at(x, y) + Vec2::new(32.0 * unit, 0.0);
    let who = b.text_aligned(name, left, 58.0, p.text, b.heavy(), 0.8, TextAlign::Left);
    b.anchor_left(who);
    b.transform(who).position = crate::animation::Animated::constant(left + Vec2::new(0.0, -32.0 * unit));
    b.slide_in(who, Vec2::new(-40.0, 0.0), 0.8, 0.6);
    let role = b.text_aligned(title, left, 38.0, p.accent, b.light().max(500), 1.0, TextAlign::Left);
    b.anchor_left(role);
    b.transform(role).position = crate::animation::Animated::constant(left + Vec2::new(0.0, 38.0 * unit));
    b.slide_in(role, Vec2::new(-40.0, 0.0), 1.0, 0.6);
    // Leaves before the end, as a lower third does.
    let end = b.end();
    for id in [accent, bar, who, role] {
        b.fade_out(id, end - 1.0, 0.5);
    }
}

fn review(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (words, who, product) = (b.copy(0), b.copy(1), b.copy(2));
    let card_size = if b.tall() { Vec2::new(900.0, if b.vertical() { 1100.0 } else { 980.0 }) } else { Vec2::new(1400.0, 780.0) };
    let card = b.block(card_size, b.at(0.5, 0.5), p.background_2, 48.0, 0.0, "Card");
    b.pop_in(card, 0.0);
    b.effect(card, "Drop Shadow", &[(0, 45.0), (2, 20.0), (3, 50.0)]);
    let unit = b.unit();
    let star_y = b.at(0.5, 0.5).y - card_size.y * 0.32 * unit;
    for index in 0..5 {
        let x = b.at(0.5, 0.5).x + (index as f32 - 2.0) * 110.0 * unit;
        let at = 0.6 + index as f64 * 0.18;
        let star = b.shape(ShapeKind::Star, Vec2::new(90.0, 90.0), Vec2::new(x, star_y), Some(Paint::Solid(Rgba::hex(p.accent))), None, 4.0, at, "Star");
        b.pop_in(star, at);
    }
    let body = b.text(words, b.at(0.5, 0.5), 60.0, p.text, b.heavy(), 1.8);
    b.words_in(body, 1.2);
    let name_y = b.at(0.5, 0.5).y + card_size.y * 0.3 * unit;
    let name = b.text(who, Vec2::new(b.at(0.5, 0.5).x, name_y), 42.0, p.text, b.heavy(), 2.6);
    b.fade_in(name, 2.6, 0.6);
    let item = b.text(product, Vec2::new(b.at(0.5, 0.5).x, name_y + 56.0 * unit), 34.0, p.muted, b.light(), 2.9);
    b.fade_in(item, 2.9, 0.6);
    b.fade_everything_out();
}

fn sale_burst(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (number, word, sub, code) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let center = b.at(0.5, if b.vertical() { 0.4 } else { 0.44 });
    let burst = b.shape(ShapeKind::Star, Vec2::new(760.0, 760.0), center, Some(Paint::radial(Rgba::hex(p.accent), Rgba::hex(p.background_2))), None, 0.0, 0.0, "Burst");
    if let Some(crate::model::LayerContent::Shape { style, .. }) = b.project.composition_mut(b.comp).and_then(|comp| comp.layer_mut(burst)).map(|layer| &mut layer.content) {
        style.points = 16;
        style.inner_radius = 0.72;
    }
    b.pop_in(burst, 0.0);
    b.spin(burst, 0.0, 90.0);
    let big = b.text(number, center + Vec2::new(0.0, -50.0 * b.unit()), 230.0, p.on_accent, b.heavy(), 0.5);
    b.letters_in(big, 0.5);
    b.pulse(big, 1.5, b.end() - 1.0, 4);
    let off = b.text(word, center + Vec2::new(0.0, 120.0 * b.unit()), 80.0, p.on_accent, b.heavy(), 0.8);
    b.pop_in(off, 0.8);
    let line = b.text(sub, b.at(0.5, if b.vertical() { 0.68 } else { 0.8 }), 52.0, p.text, b.light().max(500), 1.4);
    b.slide_in(line, Vec2::new(0.0, 40.0), 1.4, 0.7);
    let code_at = b.at(0.5, if b.vertical() { 0.78 } else { 0.9 });
    let width = (code.chars().count() as f32 * 28.0 + 100.0).max(320.0);
    let pill = b.shape(ShapeKind::Rectangle, Vec2::new(width, 90.0), code_at, None, Some((p.text, 4.0)), 45.0, 2.2, "Code");
    b.pop_in(pill, 2.2);
    let label = b.text(code, code_at, 40.0, p.text, b.heavy(), 2.3);
    b.pop_in(label, 2.3);
    b.fade_everything_out();
}

fn hook(b: &mut Builder) {
    let p = b.palette();
    let (hook_words, sub) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let video = b.placeholder(Vec2::new(b.width, b.height) * (1.0 / unit), b.at(0.5, 0.5), 0.0, 0.0, "video");
    let _ = video;
    let y = if b.vertical() { 0.2 } else { 0.18 };
    let lines = hook_words.lines().count().max(1) as f32;
    let width = (hook_words.lines().map(|line| line.chars().count()).max().unwrap_or(10) as f32 * 40.0 + 120.0).min(b.width / unit - 80.0);
    let card = b.block(Vec2::new(width, 90.0 * lines + 70.0), b.at(0.5, y), p.background, 28.0, 0.2, "Hook box");
    b.pop_in(card, 0.2);
    let words = b.text(hook_words, b.at(0.5, y), 70.0, p.text, b.heavy(), 0.35);
    b.words_in(words, 0.6);
    let line = b.text(sub, b.at(0.5, if b.vertical() { 0.72 } else { 0.84 }), 50.0, 0xffffff, b.heavy(), 1.2);
    b.slide_in(line, Vec2::new(0.0, 40.0), 1.2, 0.6);
    b.effect(line, "Drop Shadow", &[(0, 70.0), (2, 4.0), (3, 12.0)]);
    let arrow = b.shape(ShapeKind::Triangle, Vec2::new(60.0, 44.0), b.at(0.5, if b.vertical() { 0.77 } else { 0.92 }), Some(Paint::Solid(Rgba::hex(p.accent))), None, 6.0, 1.6, "Arrow");
    b.transform(arrow).rotation = crate::animation::Animated::constant(180.0);
    b.fade_in(arrow, 1.6, 0.4);
    let end = b.end();
    let down = b.at(0.5, if b.vertical() { 0.78 } else { 0.935 });
    let up = b.at(0.5, if b.vertical() { 0.77 } else { 0.92 });
    let mut keys = Vec::new();
    let mut t = 1.6;
    let mut low = true;
    while t < end {
        keys.push(b.key(t, if low { down } else { up }, EASE_IN_OUT));
        low = !low;
        t += 0.4;
    }
    super::Builder::keys(&mut b.transform(arrow).position, keys);
    b.fade_everything_out();
}

fn product_card(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (name, price, features, cta) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    let tall = b.tall();
    // 4:5 is short for a stack of photo and words: a smaller photo there.
    let portrait = tall && !b.vertical();
    let photo_at = if tall { b.at(0.5, if portrait { 0.27 } else { 0.3 }) } else { b.at(0.3, 0.5) };
    let photo_side = if portrait { 540.0 } else { 720.0 };
    let photo = b.placeholder(Vec2::new(photo_side, photo_side), photo_at, 40.0, 0.0, "product photo");
    b.pop_in(photo, 0.0);
    b.effect(photo, "Drop Shadow", &[(0, 40.0), (2, 20.0), (3, 40.0)]);
    let text_x = if tall { 0.5 } else { 0.7 };
    let name_y = if tall { if portrait { 0.56 } else { 0.55 } } else { 0.34 };
    let step = if portrait { 0.08 } else if tall { 0.065 } else { 0.09 };
    let title = b.text(name, b.at(text_x, name_y), 92.0, p.text, b.heavy(), 0.7);
    b.words_in(title, 0.6);
    let cost = b.text(price, b.at(text_x, name_y + step), 76.0, p.accent, b.heavy(), 1.1);
    b.pop_in(cost, 1.1);
    let detail = b.text(features, b.at(text_x, name_y + step * 1.85), 40.0, p.muted, b.light(), 1.6);
    b.fade_in(detail, 1.6, 0.8);
    button(b, cta, b.at(text_x, if b.vertical() { 0.76 } else if tall { 0.88 } else { 0.76 }), 2.6);
    b.fade_everything_out();
}

fn event(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (kicker, title, date, place) = (b.copy(0), b.copy(1), b.copy(2), b.copy(3));
    for (index, size) in [900.0_f32, 1300.0, 1700.0].into_iter().enumerate() {
        let ring = b.shape(ShapeKind::Ellipse, Vec2::new(size, size), b.at(0.5, 0.5), None, Some((p.muted, 2.0)), 0.0, 0.0, "Ring");
        b.scale(ring, index as f64 * 0.2, 70.0, 2.0 + index as f64 * 0.2, 100.0, EASE_OUT);
        b.fade_in(ring, index as f64 * 0.2, 1.0);
        let opacity = 30.0;
        b.transform(ring).opacity.value = opacity;
    }
    let kick = b.text(kicker, b.at(0.5, 0.26), 44.0, p.accent, b.heavy(), 0.4);
    b.slide_in(kick, Vec2::new(0.0, 30.0), 0.4, 0.6);
    let head = b.text(title, b.at(0.5, 0.4), 112.0, p.text, b.heavy(), 0.8);
    b.words_in(head, 0.8);
    let date_at = b.below(b.at(0.5, 0.4), title, 112.0, 40.0, 120.0, b.at(0.5, 0.58));
    let width = (date.chars().count() as f32 * 34.0 + 120.0).max(420.0);
    let block = b.block(Vec2::new(width, 120.0), date_at, p.accent, 20.0, 1.5, "Date");
    b.pop_in(block, 1.5);
    let day = b.text(date, date_at, 54.0, p.on_accent, b.heavy(), 1.6);
    b.pop_in(day, 1.6);
    let at = b.text(place, date_at + Vec2::new(0.0, 130.0 * b.unit()), 46.0, p.muted, b.light(), 2.2);
    b.fade_in(at, 2.2, 0.7);
    b.fade_everything_out();
}

fn big_stat(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (number, label, source) = (b.copy(0), b.copy(1), b.copy(2));
    let digits = b.text(number, b.at(0.5, 0.4), 300.0, p.accent, b.heavy(), 0.3);
    b.scale(digits, 0.3, 40.0, 1.3, 100.0, super::SPRING);
    b.fade_in(digits, 0.3, 0.3);
    let unit = b.unit();
    let track_width = (b.width / unit - 240.0).min(900.0);
    let track = b.block(Vec2::new(track_width, 20.0), b.at(0.5, 0.58), p.background_2, 10.0, 0.6, "Track");
    b.fade_in(track, 0.6, 0.5);
    let fill = b.block(Vec2::new(track_width, 20.0), b.at(0.5, 0.58), p.accent, 10.0, 0.8, "Fill");
    b.anchor_left(fill);
    b.grow_across(fill, 0.8, 2.0);
    let words = b.text(label, b.at(0.5, 0.67), 60.0, p.text, b.heavy(), 1.4);
    b.words_in(words, 0.7);
    let small = b.text(source, b.at(0.5, 0.76), 34.0, p.muted, b.light(), 2.2);
    b.fade_in(small, 2.2, 0.6);
    b.fade_everything_out();
}

fn before_after(b: &mut Builder) {
    let p = b.palette();
    let (before, after, title) = (b.copy(0), b.copy(1), b.copy(2));
    let unit = b.unit();
    let full = Vec2::new(b.width, b.height) * (1.0 / unit);
    let left = b.placeholder(full, b.at(0.5, 0.5), 0.0, 0.0, "before photo");
    b.layer(left).name = "Drop your before photo here".into();
    let right = b.block(full, b.at(0.5, 0.5), p.background_2, 0.0, 0.0, "Drop your after photo here");
    // The after picture is revealed by a wipe that follows the divider line across.
    b.effect(right, "Linear Wipe", &[(0, 100.0), (1, 90.0)]);
    if let Some(effect) = b.layer(right).effects.last_mut() {
        let frame = |seconds: f64| (seconds * 30.0) as i64;
        effect.properties[0].value = crate::animation::Animated::keyframed(vec![
            crate::animation::Keyframe { frame: frame(1.0), value: 100.0, easing: EASE_IN_OUT },
            crate::animation::Keyframe { frame: frame(3.0), value: 50.0, easing: Easing::Linear },
        ]);
    }
    let line = b.block(Vec2::new(8.0, full.y), b.at(1.0, 0.5), 0xffffff, 0.0, 0.0, "Divider");
    let middle = b.at(0.5, 0.5);
    b.move_to(line, 1.0, 3.0, middle, EASE_IN_OUT);
    let knob = b.shape(ShapeKind::Ellipse, Vec2::new(90.0, 90.0), b.at(1.0, 0.5), Some(Paint::Solid(Rgba::hex(0xffffff))), None, 0.0, 0.0, "Handle");
    b.move_to(knob, 1.0, 3.0, middle, EASE_IN_OUT);
    // The labels sit on small tags, so they read over any picture.
    let tag_y = if b.vertical() { 0.12 } else { 0.1 };
    for (words, x, color, ink, at) in [(before, 0.22, 0x111111, 0xffffff, 0.3), (after, 0.78, p.accent, p.on_accent, 3.0)] {
        let tag_at = b.at(x, tag_y);
        let tag = b.block(Vec2::new(words.chars().count() as f32 * 30.0 + 80.0, 76.0), tag_at, color, 38.0, at, "Tag");
        b.pop_in(tag, at);
        let label = b.text(words, tag_at, 40.0, ink, b.heavy(), at + 0.05);
        b.pop_in(label, at + 0.05);
    }
    // The headline goes on a card above the divider, higher in 9:16 so phone apps don't cover it.
    let head_at = b.at(0.5, if b.vertical() { 0.78 } else { 0.86 });
    let lines = title.lines().count().max(1) as f32;
    let card_width = (title.lines().map(|line| line.chars().count()).max().unwrap_or(10) as f32 * 36.0 + 120.0).min(b.width / unit - 80.0);
    let card = b.block(Vec2::new(card_width, 80.0 * lines + 60.0), head_at, 0x111111, 30.0, 3.5, "Headline card");
    b.slide_in(card, Vec2::new(0.0, 40.0), 3.5, 0.6);
    b.transform(card).opacity.value = 85.0;
    let head = b.text(title, head_at, 66.0, 0xffffff, b.heavy(), 3.6);
    b.words_in(head, 0.7);
    b.fade_everything_out();
}

fn steps(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let title = b.copy(0);
    let head = b.text(title, b.at(0.5, if b.vertical() { 0.16 } else { 0.14 }), 88.0, p.text, b.heavy(), 0.2);
    b.words_in(head, 0.7);
    let tall = b.tall();
    for index in 0..3 {
        let at = 1.0 + index as f64 * 1.1;
        // Down the frame in the tall formats, across it in 16:9.
        let (center, size) = if tall {
            (b.at(0.5, (if b.vertical() { 0.33 } else { 0.36 }) + index as f32 * (if b.vertical() { 0.2 } else { 0.22 })), Vec2::new(880.0, if b.vertical() { 300.0 } else { 250.0 }))
        } else {
            (b.at(0.2 + index as f32 * 0.3, 0.56), Vec2::new(520.0, 560.0))
        };
        let card = b.block(size, center, p.background_2, 32.0, at, "Step card");
        b.slide_in(card, Vec2::new(if tall { 300.0 } else { 0.0 }, if tall { 0.0 } else { 200.0 }), at, 0.7);
        let unit = b.unit();
        let number_at = center + if tall { Vec2::new(-size.x * 0.36 * unit, 0.0) } else { Vec2::new(0.0, -size.y * 0.28 * unit) };
        let number = b.text(&format!("0{}", index + 1), number_at, 80.0, p.accent, b.heavy(), at + 0.2);
        b.pop_in(number, at + 0.2);
        let words_at = center + if tall { Vec2::new(size.x * 0.1 * unit, 0.0) } else { Vec2::new(0.0, size.y * 0.08 * unit) };
        let words = b.text(b.copy(index + 1), words_at, if tall { 54.0 } else { 46.0 }, p.text, b.light().max(500), at + 0.3);
        b.slide_in(words, Vec2::new(if tall { 40.0 } else { 0.0 }, if tall { 0.0 } else { 30.0 }), at + 0.3, 0.6);
    }
    b.fade_everything_out();
}

fn announcement(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (badge_word, title, sub) = (b.copy(0), b.copy(1), b.copy(2));
    let badge_at = b.at(0.5, if b.vertical() { 0.3 } else { 0.26 });
    let badge = b.shape(ShapeKind::Star, Vec2::new(300.0, 300.0), badge_at, Some(Paint::Solid(Rgba::hex(p.accent))), None, 10.0, 0.2, "Badge");
    if let Some(crate::model::LayerContent::Shape { style, .. }) = b.project.composition_mut(b.comp).and_then(|comp| comp.layer_mut(badge)).map(|layer| &mut layer.content) {
        style.points = 12;
        style.inner_radius = 0.82;
    }
    b.pop_in(badge, 0.2);
    b.spin(badge, 0.0, 120.0);
    let word = b.text(badge_word, badge_at, 64.0, p.on_accent, b.heavy(), 0.4);
    b.pop_in(word, 0.4);
    // Clear of the badge above it and the line below, however many lines it runs to.
    let head_height = b.text_height(title, 120.0) / b.unit();
    let head_at = Vec2::new(badge_at.x, b.at(0.5, 0.52).y.max(badge_at.y + (150.0 + 36.0 + head_height * 0.5) * b.unit()));
    let head = b.text(title, head_at, 120.0, p.text, b.heavy(), 1.0);
    b.words_in(head, 0.8);
    b.effect(head, "Glow", &[(0, 70.0), (1, 24.0), (2, 0.8)]);
    let line_at = b.below(head_at, title, 120.0, 36.0, 50.0, b.at(0.5, 0.66));
    let line = b.text(sub, line_at, 50.0, p.muted, b.light(), 1.7);
    b.slide_in(line, Vec2::new(0.0, 24.0), 1.7, 0.7);
    b.fade_everything_out();
}

fn podcast(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (show, episode, guest) = (b.copy(0), b.copy(1), b.copy(2));
    let tall = b.tall();
    let photo_at = if tall { b.at(0.5, 0.3) } else { b.at(0.27, 0.45) };
    let photo = b.shape(ShapeKind::Ellipse, Vec2::new(440.0, 440.0), photo_at, Some(Paint::linear(Rgba::hex(0x3f3f46), Rgba::hex(0x27272a), 60.0)), Some((p.accent, 10.0)), 0.0, 0.2, "Drop your guest photo here");
    b.pop_in(photo, 0.2);
    let text_x = if tall { 0.5 } else { 0.66 };
    let top = if tall { 0.55 } else { 0.3 };
    let name = b.text(show, b.at(text_x, top), 44.0, p.accent, b.heavy(), 0.5);
    b.fade_in(name, 0.5, 0.6);
    let title = b.text(episode, b.at(text_x, top + 0.1), 76.0, p.text, b.heavy(), 0.8);
    b.words_in(title, 0.8);
    let who = b.text(guest, b.at(text_x, top + 0.2), 42.0, p.muted, b.light(), 1.3);
    b.fade_in(who, 1.3, 0.6);
    // A sound wave: bars rising and falling out of step.
    let unit = b.unit();
    let wave_y = b.at(text_x, if b.vertical() { 0.78 } else if tall { 0.84 } else { top + 0.36 }).y;
    let end = b.end();
    for index in 0..16 {
        let x = b.at(text_x, 0.0).x + (index as f32 - 7.5) * 36.0 * unit;
        let bar = b.block(Vec2::new(18.0, 140.0), Vec2::new(x, wave_y), p.accent, 9.0, 1.0, "Wave bar");
        let mut keys = Vec::new();
        let mut t = 1.0;
        let mut step = index;
        while t < end {
            let height = 20.0 + ((step * 37 + index * 11) % 80) as f32;
            keys.push(b.key(t, Vec2::new(100.0, height), EASE_IN_OUT));
            t += 0.25;
            step += 1;
        }
        super::Builder::keys(&mut b.transform(bar).scale, keys);
    }
    b.fade_everything_out();
}

fn brand_intro(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (brand, tagline) = (b.copy(0), b.copy(1));
    let logo = b.shape(ShapeKind::Ellipse, Vec2::new(260.0, 260.0), b.at(0.5, 0.4), Some(Paint::Solid(Rgba::hex(p.accent))), None, 0.0, 0.3, "Drop your logo here");
    b.pop_in(logo, 0.3);
    for side in [-1.0_f32, 1.0] {
        let line = b.block(Vec2::new(360.0, 4.0), b.at(0.5, 0.4) + Vec2::new(side * 360.0 * b.unit(), 0.0), p.muted, 2.0, 0.8, "Line");
        b.scale(line, 0.8, 0.0, 1.6, 100.0, EASE_OUT);
    }
    let name = b.text(brand, b.at(0.5, 0.58), 130.0, p.text, b.heavy(), 1.2);
    b.letters_in(name, 0.8);
    b.scale(name, 1.2, 110.0, b.end(), 100.0, EASE_OUT);
    let line = b.text(tagline, b.at(0.5, 0.68), 48.0, p.muted, b.light(), 2.0);
    b.fade_in(line, 2.0, 0.8);
    b.fade_everything_out();
}

fn collage(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (title, sub) = (b.copy(0), b.copy(1));
    let unit = b.unit();
    let (cell, gap) = if b.vertical() { (460.0, 30.0) } else if b.tall() { (330.0, 26.0) } else { (380.0, 30.0) };
    let center = b.at(if b.tall() { 0.5 } else { 0.32 }, if b.vertical() { 0.42 } else if b.tall() { 0.38 } else { 0.5 });
    for index in 0..4 {
        let (column, row) = ((index % 2) as f32 - 0.5, (index / 2) as f32 - 0.5);
        let at = center + Vec2::new(column * (cell + gap) * unit, row * (cell + gap) * unit);
        let spot = b.placeholder(Vec2::new(cell, cell), at, 28.0, 0.2 + index as f64 * 0.25, "photo");
        b.pop_in(spot, 0.2 + index as f64 * 0.25);
    }
    let text_at = if b.tall() { b.at(0.5, if b.vertical() { 0.76 } else { 0.8 }) } else { b.at(0.75, 0.45) };
    let head = b.text(title, text_at, 96.0, p.text, b.heavy(), 1.4);
    b.words_in(head, 0.7);
    // Below the title, however many lines it has.
    let lines = title.lines().count().max(1) as f32;
    let line = b.text(sub, text_at + Vec2::new(0.0, (55.0 * lines + 50.0) * unit), 44.0, p.muted, b.light(), 1.9);
    b.fade_in(line, 1.9, 0.6);
    b.fade_everything_out();
}

fn marquee(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (phrase, sub) = (b.copy(0), b.copy(1));
    let end = b.end();
    let repeated = format!("{phrase} • {phrase} • {phrase} • {phrase}");
    let rows = if b.vertical() { 7 } else { 5 };
    for row in 0..rows {
        let y = (row as f32 + 0.5) / rows as f32;
        let color = if row % 2 == 0 { p.accent } else { p.muted };
        let line = b.text_unfitted(&repeated, b.at(0.5, y), 110.0, color, b.heavy(), 0.0, TextAlign::Center);
        b.transform(line).opacity.value = if row % 2 == 0 { 90.0 } else { 35.0 };
        // Alternate rows scroll opposite ways.
        let shift = if row % 2 == 0 { -600.0 } else { 600.0 } * b.unit();
        let start = b.at(0.5, y);
        b.move_to(line, 0.0, end, start + Vec2::new(shift, 0.0), Easing::Linear);
    }
    let unit = b.unit();
    let card_width = (sub.chars().count() as f32 * 38.0 + 200.0).clamp(600.0, b.width / unit - 80.0);
    let card = b.block(Vec2::new(card_width, 180.0), b.at(0.5, 0.5), p.background, 90.0, 0.8, "Card");
    b.pop_in(card, 0.8);
    b.effect(card, "Drop Shadow", &[(0, 55.0), (2, 14.0), (3, 40.0)]);
    let words = b.text(sub, b.at(0.5, 0.5), 66.0, p.text, b.heavy(), 1.0);
    b.words_in(words, 0.6);
    b.fade_everything_out();
}

fn end_card(b: &mut Builder) {
    let p = b.palette();
    b.background();
    let (title, handle, cta) = (b.copy(0), b.copy(1), b.copy(2));
    // Smaller and higher in the shorter formats, so it clears the headline.
    let (avatar_size, avatar_y) = if b.vertical() { (300.0, 0.32) } else if b.tall() { (240.0, 0.24) } else { (200.0, 0.2) };
    let avatar = b.shape(ShapeKind::Ellipse, Vec2::new(avatar_size, avatar_size), b.at(0.5, avatar_y), Some(Paint::linear(Rgba::hex(0x3f3f46), Rgba::hex(0x27272a), 60.0)), Some((p.accent, 10.0)), 0.0, 0.2, "Drop your profile photo here");
    b.pop_in(avatar, 0.2);
    let head_y = if b.vertical() { 0.5 } else { 0.46 };
    let head = b.text(title, b.at(0.5, head_y), 96.0, p.text, b.heavy(), 0.7);
    b.words_in(head, 0.7);
    let handle_at = b.below(b.at(0.5, head_y), title, 96.0, 30.0, 52.0, b.at(0.5, head_y));
    let name = b.text(handle, handle_at, 52.0, p.accent, b.heavy(), 1.2);
    b.fade_in(name, 1.2, 0.6);
    let at = b.at(0.5, if b.vertical() { 0.74 } else { 0.76 });
    button(b, cta, at, 1.8);
    let end = b.end();
    let ids: Vec<_> = b.project.composition(b.comp).map(|comp| comp.layers.iter().filter(|layer| layer.name == "Button").map(|layer| layer.id).collect()).unwrap_or_default();
    for id in ids {
        b.pulse(id, 3.0, end - 1.0, 3);
    }
    b.fade_everything_out();
}
