//! The viewer's guides: lines drawn over the picture to help place things, never rendered or
//! exported. A grid (rule of thirds and a center cross) suits any frame. Wide 16:9 frames can
//! also show broadcast title and action safe boxes, and tall 9:16 frames the safe zones of the
//! social apps, the part of the picture their buttons, captions and names don't cover.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{Context, Div, InteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt as _, div, px, relative};

use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// A social app whose on-screen buttons and text cover the edges of a tall video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocialApp {
    TikTok,
    InstagramReels,
    YouTubeShorts,
}

impl SocialApp {
    pub const ALL: [Self; 3] = [Self::TikTok, Self::InstagramReels, Self::YouTubeShorts];

    pub const fn name(self) -> &'static str {
        match self {
            Self::TikTok => "TikTok",
            Self::InstagramReels => "Instagram Reels",
            Self::YouTubeShorts => "YouTube Shorts",
        }
    }

    /// How far in from each edge the app's buttons and text reach, as shares of the frame:
    /// `[top, right, bottom, left]`. The apps don't all publish exact figures and their layouts
    /// change, so these follow the commonly used 1080 × 1920 templates and are approximate.
    pub const fn insets(self) -> [f32; 4] {
        match self {
            // About 900 × 1490 clear: the like, comment and share column takes the right side,
            // and the caption and sound name the bottom.
            Self::TikTok => [130.0 / 1920.0, 120.0 / 1080.0, 300.0 / 1920.0, 60.0 / 1080.0],
            // Meta's Reels guidance: keep 14% at the top, 35% at the bottom and 6% at the sides clear.
            Self::InstagramReels => [0.14, 0.06, 0.35, 0.06],
            // About 936 × 1500 clear, with the button column on the right.
            Self::YouTubeShorts => [120.0 / 1920.0, 96.0 / 1080.0, 300.0 / 1920.0, 48.0 / 1080.0],
        }
    }

    /// Each app's outline color, so several zones can be told apart at once.
    const fn line_color(self) -> u32 {
        match self {
            Self::TikTok => 0x25f4eecc,
            Self::InstagramReels => 0xff5fa2cc,
            Self::YouTubeShorts => 0xffb02ecc,
        }
    }

    /// The id the zone's box is drawn with, for finding it in tests.
    const fn element_id(self) -> &'static str {
        match self {
            Self::TikTok => "guide-zone-tiktok",
            Self::InstagramReels => "guide-zone-instagram-reels",
            Self::YouTubeShorts => "guide-zone-youtube-shorts",
        }
    }
}

/// Which guides the viewer draws while guides are shown. Each kind only appears on frames it
/// suits: title safe on 16:9, the social zones on 9:16.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideChoice {
    pub grid: bool,
    pub title_safe: bool,
    pub social: Vec<SocialApp>,
}

impl Default for GuideChoice {
    fn default() -> Self {
        Self { grid: true, title_safe: true, social: Vec::new() }
    }
}

/// One guide that can be picked from the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guide {
    Grid,
    TitleSafe,
    Social(SocialApp),
}

impl GuideChoice {
    pub fn is_on(&self, guide: Guide) -> bool {
        match guide {
            Guide::Grid => self.grid,
            Guide::TitleSafe => self.title_safe,
            Guide::Social(app) => self.social.contains(&app),
        }
    }

    pub fn set(&mut self, guide: Guide, on: bool) {
        match guide {
            Guide::Grid => self.grid = on,
            Guide::TitleSafe => self.title_safe = on,
            Guide::Social(app) => {
                self.social.retain(|&chosen| chosen != app);
                if on {
                    self.social.push(app);
                }
            }
        }
    }
}

/// Picks `guide` from the menu. While the guides are shown it turns on or off; while they are
/// hidden it is turned on and the guides shown, since its checkmark showed it as not drawn.
pub fn pick_guide(workspace: &mut Workspace, guide: Guide) {
    let choice = &mut workspace.ui.guide_choice;
    let on = !workspace.ui.guides || !choice.is_on(guide);
    choice.set(guide, on);
    workspace.ui.guides |= on;
}

/// The frame shapes that have guides of their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameShape {
    /// 16:9, such as 1920 × 1080 or 3840 × 2160.
    Wide,
    /// 9:16, such as 1080 × 1920.
    Tall,
    Other,
}

impl FrameShape {
    /// The shape of a `width` × `height` frame. A frame within 1% of 16:9 or 9:16 counts, so
    /// sizes rounded to even pixels (such as 1366 × 768) still get their guides.
    pub fn of(width: u32, height: u32) -> Self {
        if width == 0 || height == 0 {
            return Self::Other;
        }
        let ratio = width as f64 / height as f64;
        let near = |target: f64| (ratio / target - 1.0).abs() <= 0.01;
        if near(16.0 / 9.0) {
            Self::Wide
        } else if near(9.0 / 16.0) {
            Self::Tall
        } else {
            Self::Other
        }
    }
}

/// The shape of the composition in the viewer.
fn shape(workspace: &Workspace) -> FrameShape {
    let composition = workspace.state.composition();
    FrameShape::of(composition.width, composition.height)
}

/// The viewer footer's guides button. A click opens a menu to show or hide the guides and pick
/// which ones are drawn; only the choices that suit the composition's shape are listed.
pub fn guides_menu(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    use gpui_kit::component::{Selectable as _, Sizable as _};
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};

    let shown = workspace.ui.guides;
    let choice = workspace.ui.guide_choice.clone();
    let shape = shape(workspace);
    let target = cx.weak_entity();
    // A menu item that changes the guides, then redraws the viewer.
    let item = move |label: &'static str, checked: bool, change: Box<dyn Fn(&mut Workspace)>| {
        let target = target.clone();
        PopupMenuItem::new(label).checked(checked).on_click(move |_, _, cx| {
            target
                .update(cx, |this, cx| {
                    change(this);
                    cx.notify();
                })
                .ok();
        })
    };
    // A guide's item is checked only while it is drawn.
    let guide_item = {
        let item = item.clone();
        move |label: &'static str, guide: Guide| item(label, shown && choice.is_on(guide), Box::new(move |this| pick_guide(this, guide)))
    };
    Button::new("viewer-guides")
        .ghost()
        .small()
        .icon(Icon::new(IconName::Grid3x3))
        .tooltip("Guides and Safe Zones (')")
        .selected(shown)
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu
                .item(item("Show Guides (')", shown, Box::new(|this| this.ui.guides = !this.ui.guides)))
                .separator()
                .item(guide_item("Grid and Center", Guide::Grid));
            match shape {
                FrameShape::Wide => menu = menu.item(guide_item("Title and Action Safe", Guide::TitleSafe)),
                FrameShape::Tall => {
                    menu = menu.separator().label("Social Safe Zones");
                    for app in SocialApp::ALL {
                        menu = menu.item(guide_item(app.name(), Guide::Social(app)));
                    }
                }
                FrameShape::Other => {}
            }
            menu
        })
}

/// The chosen guides over the picture, sized to the frame they sit in.
pub fn guides_overlay(workspace: &Workspace) -> impl IntoElement + use<> {
    let choice = &workspace.ui.guide_choice;
    let shape = shape(workspace);
    let mut overlay = div().id("viewer-guides-overlay").absolute().size_full().overflow_hidden();
    if choice.grid {
        overlay = overlay.child(grid());
    }
    if shape == FrameShape::Wide && choice.title_safe {
        // The action safe (93%) and title safe (90%) boxes broadcast uses.
        overlay = overlay.child(inset_box([0.035; 4], gpui_kit::rgba(0xffffff66)).id("guide-action-safe").test_support());
        overlay = overlay.child(inset_box([0.05; 4], gpui_kit::rgba(0xffffff66)).id("guide-title-safe").test_support());
    }
    if shape == FrameShape::Tall {
        for &app in &choice.social {
            overlay = overlay.child(social_zone(app));
        }
    }
    overlay.test_support()
}

/// A box `insets` (`[top, right, bottom, left]`, shares of the frame) in from the frame's edges.
fn inset_box([top, right, bottom, left]: [f32; 4], line: gpui_kit::Rgba) -> Div {
    div().absolute().top(relative(top)).right(relative(right)).bottom(relative(bottom)).left(relative(left)).border_1().border_color(line)
}

/// The rule of thirds, faintly, and a small cross at the center.
fn grid() -> Div {
    let line = gpui_kit::rgba(0xffffff66);
    let faint = gpui_kit::rgba(0xffffff33);
    let across = |at: f32| div().absolute().left_0().right_0().top(relative(at)).h(px(1.0)).bg(faint);
    let down = |at: f32| div().absolute().top_0().bottom_0().left(relative(at)).w(px(1.0)).bg(faint);
    let cross = div()
        .absolute()
        .left(relative(0.5))
        .top(relative(0.5))
        .child(div().absolute().left(px(-8.0)).top(px(-0.5)).w(px(16.0)).h(px(1.0)).bg(line))
        .child(div().absolute().top(px(-8.0)).left(px(-0.5)).h(px(16.0)).w(px(1.0)).bg(line));
    div()
        .absolute()
        .size_full()
        .child(across(1.0 / 3.0))
        .child(across(2.0 / 3.0))
        .child(down(1.0 / 3.0))
        .child(down(2.0 / 3.0))
        .child(cross)
}

/// `app`'s safe zone: the covered edges lightly shaded, the clear part outlined in the app's
/// color, and its name in the outline's top corner. Several zones shade over each other, so the
/// area every chosen app leaves clear stays the brightest.
fn social_zone(app: SocialApp) -> impl IntoElement {
    let [top, right, bottom, left] = app.insets();
    let shade = gpui_kit::rgba(0x00000033);
    let line = gpui_kit::rgba(app.line_color());
    // The four shaded bands: full-width at the top and bottom, the sides between them.
    let band = || div().absolute().bg(shade);
    let shading = div()
        .absolute()
        .size_full()
        .child(band().left_0().right_0().top_0().h(relative(top)))
        .child(band().left_0().right_0().bottom_0().h(relative(bottom)))
        .child(band().top(relative(top)).bottom(relative(bottom)).left_0().w(relative(left)))
        .child(band().top(relative(top)).bottom(relative(bottom)).right_0().w(relative(right)));
    let name = div()
        .absolute()
        .top(px(2.0))
        .left(px(2.0))
        .px_1()
        .rounded(px(2.0))
        .bg(gpui_kit::rgba(0x000000aa))
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::TEXT))
        .child(app.name());
    div()
        .absolute()
        .size_full()
        .child(shading)
        .child(inset_box(app.insets(), line).border_2().child(name).id(app.element_id()).test_support())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_near_sixteen_by_nine_count_as_wide_or_tall() {
        assert_eq!(FrameShape::of(1920, 1080), FrameShape::Wide);
        assert_eq!(FrameShape::of(1366, 768), FrameShape::Wide, "rounded to even pixels");
        assert_eq!(FrameShape::of(1080, 1920), FrameShape::Tall);
        assert_eq!(FrameShape::of(720, 1280), FrameShape::Tall);
        assert_eq!(FrameShape::of(1080, 1080), FrameShape::Other);
        assert_eq!(FrameShape::of(1080, 1350), FrameShape::Other, "4:5");
        assert_eq!(FrameShape::of(0, 1080), FrameShape::Other);
    }

    #[test]
    fn every_social_zone_leaves_the_middle_of_the_frame_clear() {
        for app in SocialApp::ALL {
            let [top, right, bottom, left] = app.insets();
            assert!(top + bottom < 0.6 && left + right < 0.25, "{} covers too much", app.name());
            assert!([top, right, bottom, left].iter().all(|&inset| inset > 0.0));
        }
    }

    #[test]
    fn social_zones_turn_on_and_off() {
        let mut choice = GuideChoice::default();
        choice.set(Guide::Social(SocialApp::TikTok), true);
        choice.set(Guide::Social(SocialApp::YouTubeShorts), true);
        choice.set(Guide::Social(SocialApp::YouTubeShorts), true);
        assert_eq!(choice.social, [SocialApp::TikTok, SocialApp::YouTubeShorts], "each app once");
        choice.set(Guide::Social(SocialApp::TikTok), false);
        assert_eq!(choice.social, [SocialApp::YouTubeShorts]);
        assert!(choice.is_on(Guide::Grid) && !choice.is_on(Guide::Social(SocialApp::TikTok)));
    }
}
