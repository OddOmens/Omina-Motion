//! The welcome setup, shown over the start screen the first time Studio opens (and again from
//! Settings ▸ General). Four short pages:
//!
//! 1. Look: the theme and the accent color.
//! 2. Features: a starting point ("What will you make?") and every tool and feature to keep or
//!    hide (the same switches as Settings ▸ Tools & Features).
//! 3. AI: whether to use the features that run AI models on this Mac, and which. AI isn't for
//!    everyone, so turning it all off is a first-class choice, not a buried setting.
//! 4. Done: what was picked, and where to change it later.
//!
//! Every choice applies and is saved as it is made, so the editor behind already looks the way
//! it was set up. Skipping keeps whatever was chosen so far (Studio's defaults if nothing was).

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, div, prelude::FluentBuilder as _, px,
    relative,
};

use crate::app_settings::{self, AppSettings};
use crate::features::{AI_GROUP, Focus, GROUPS, non_ai_features};
use crate::theme::{Accent, ThemeName, color, hsla, size};
use crate::workspace::Workspace;

/// The setup's pages, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Look,
    Features,
    Ai,
    Done,
}

impl Step {
    const ALL: [Step; 4] = [Self::Look, Self::Features, Self::Ai, Self::Done];

    fn index(self) -> usize {
        Self::ALL.iter().position(|step| *step == self).unwrap_or(0)
    }

    fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }

    fn previous(self) -> Option<Self> {
        self.index().checked_sub(1).map(|index| Self::ALL[index])
    }

    /// Its name in the side panel's list of steps.
    fn name(self) -> &'static str {
        match self {
            Self::Look => "Look",
            Self::Features => "Tools & features",
            Self::Ai => "Local AI",
            Self::Done => "Ready",
        }
    }
}

/// The width of the setup's column.
const COLUMN_WIDTH: f32 = 720.0;
/// The width of the side panel listing the steps.
const SIDE_WIDTH: f32 = 260.0;

/// Shows the setup from its first page.
pub fn show(workspace: &mut Workspace, cx: &mut Context<Workspace>) {
    workspace.ui.onboarding = Some(Step::Look);
    // The editor's own floating panels would draw over it, so they close.
    workspace.ui.template_gallery = false;
    workspace.ui.color_picker = None;
    cx.notify();
}

/// Closes the setup and remembers it has been through, so it doesn't show on the next launch.
fn finish(workspace: &mut Workspace, cx: &mut Context<Workspace>) {
    change(workspace, cx, |settings| settings.onboarded = true);
    workspace.ui.onboarding = None;
    cx.notify();
}

/// Saves a choice, and applies it to the editor behind: its theme, its keyboard shortcuts and
/// menus (which follow the features that are on), and its panels.
fn change(workspace: &mut Workspace, cx: &mut Context<Workspace>, edit: impl Fn(&mut AppSettings)) {
    app_settings::update(&edit);
    let before = workspace.ui.settings.clone();
    let mut settings = before.clone();
    edit(&mut settings);
    if settings.features_off != before.features_off || settings.ai != before.ai {
        crate::keymap::apply(&settings, cx);
    }
    if (settings.theme, settings.accent) != (before.theme, before.accent) {
        crate::theme::apply(settings.theme, settings.accent, cx);
        cx.refresh_windows();
    }
    workspace.use_settings(settings);
    cx.notify();
}

/// The setup over the whole window, while it is showing.
pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let step = workspace.ui.onboarding?;
    let settings = &workspace.ui.settings;
    let (title, about, body) = match step {
        Step::Look => ("Welcome to Studio", "Let's set it up the way you like. First, pick a theme and an accent color.", look_page(settings, cx).into_any_element()),
        Step::Features => (
            "What will you make?",
            "Start from a set that fits, then fine-tune. What you turn off is hidden everywhere: tools, panels, menus and shortcuts.",
            features_page(settings, cx).into_any_element(),
        ),
        Step::Ai => (
            "Local AI, if you want it",
            "A few features use AI models. They run entirely on this Mac, and nothing you make is uploaded. AI isn't for everyone, so Studio works fully without it.",
            ai_page(settings, cx).into_any_element(),
        ),
        Step::Done => ("You're all set", "Everything here can be changed later in Settings.", done_page(settings).into_any_element()),
    };
    let column = div()
        .w(px(COLUMN_WIDTH))
        .max_w_full()
        .h_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .pt_10()
                .pb_5()
                .flex()
                .flex_col()
                .gap_2()
                .child(step_dots(step))
                .child(div().id("onboarding-title").text_size(px(26.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(title).test_support())
                .child(div().text_color(hsla(color::MUTED)).child(about)),
        )
        .child(div().id("onboarding-body").flex_1().min_h_0().overflow_y_scroll().pb_4().child(body))
        .child(footer(step, cx));
    let content = div().relative().flex_1().min_w_0().h_full().flex().justify_center().px_8().child(super::start_screen::accent_glow(360.0)).child(column);
    Some(
        div()
            .id("onboarding")
            .absolute()
            .inset_0()
            .flex()
            .bg(hsla(color::BACKGROUND))
            .text_color(hsla(color::TEXT))
            .text_size(px(size::TEXT))
            .occlude()
            .child(side_panel(step, cx))
            .child(content)
            .test_support()
            .into_any_element(),
    )
}

/// The panel down the left: Studio's mark, the steps (done ones ticked, the current one
/// highlighted, earlier ones clickable to go back), and a reassuring line at the bottom.
fn side_panel(step: Step, cx: &mut Context<Workspace>) -> impl IntoElement {
    let steps = Step::ALL.into_iter().map(|each| {
        let (done, current) = (each.index() < step.index(), each == step);
        let badge = div()
            .size(px(24.0))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(size::SMALL_TEXT))
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .map(|badge| match (done, current) {
                (true, _) => badge.bg(hsla(color::ACCENT)).child(Icon::new(IconName::Check).size(px(13.0)).text_color(gpui_kit::white())),
                (_, true) => badge.border_2().border_color(hsla(color::ACCENT)).text_color(hsla(color::TEXT)).child((each.index() + 1).to_string()),
                _ => badge.border_1().border_color(hsla(color::MUTED).opacity(0.5)).text_color(hsla(color::MUTED)).child((each.index() + 1).to_string()),
            });
        div()
            .id(("onboarding-step", each.index()))
            .h(px(40.0))
            .px_2()
            .flex()
            .items_center()
            .gap_3()
            .rounded(px(8.0))
            .text_color(hsla(if current || done { color::TEXT } else { color::MUTED }))
            .when(current, |row| row.bg(hsla(color::ACCENT).opacity(0.14)).font_weight(gpui_kit::FontWeight::SEMIBOLD))
            .when(done, |row| {
                row.cursor_pointer().hover(|style| style.bg(hsla(color::HOVER))).on_click(cx.listener(move |this, _, _, cx| {
                    this.ui.onboarding = Some(each);
                    cx.notify();
                }))
            })
            .child(badge)
            .child(each.name())
            .test_support()
    });
    div()
        .relative()
        .w(px(SIDE_WIDTH))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .px_5()
        .pt_10()
        .pb_6()
        .overflow_hidden()
        .bg(hsla(color::PANEL))
        .border_r_1()
        .border_color(hsla(color::HAIRLINE))
        .child(
            // A deeper wash of the accent at the top of the panel.
            div().absolute().top_0().left_0().right_0().h(px(260.0)).bg(gpui_kit::linear_gradient(
                180.0,
                gpui_kit::linear_color_stop(hsla(color::ACCENT).opacity(0.28), 0.0),
                gpui_kit::linear_color_stop(hsla(color::ACCENT).opacity(0.0), 1.0),
            )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(super::start_screen::brand_mark(40.0))
                .child(div().flex().flex_col().child(div().text_size(px(16.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Studio")).child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Setup"))),
        )
        .child(div().pt_8().flex().flex_col().gap_1().children(steps))
        .child(div().flex_1())
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("About a minute. Everything here can be changed later in Settings."))
}

/// "Step 2 of 4", with a dot for each page.
fn step_dots(step: Step) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .children(Step::ALL.iter().map(|each| {
            let reached = each.index() <= step.index();
            div().h(px(6.0)).w(px(if *each == step { 22.0 } else { 6.0 })).rounded_full().bg(if reached { hsla(color::ACCENT) } else { hsla(color::MUTED).opacity(0.35) })
        }))
        .child(div().pl_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("Step {} of {}", step.index() + 1, Step::ALL.len())))
}

/// Skip on the left; Back and Continue (or Start Creating, at the end) on the right.
fn footer(step: Step, cx: &mut Context<Workspace>) -> impl IntoElement {
    let next = match step.next() {
        Some(next) => Button::new("onboarding-next").primary().label("Continue").on_click(cx.listener(move |this, _, _, cx| {
            this.ui.onboarding = Some(next);
            cx.notify();
        })),
        None => Button::new("onboarding-next").primary().label("Start Creating").on_click(cx.listener(|this, _, _, cx| finish(this, cx))),
    };
    div()
        .flex_none()
        .py_4()
        .flex()
        .items_center()
        .justify_between()
        .border_t_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().when(step != Step::Done, |left| {
            left.child(Button::new("onboarding-skip").ghost().label("Skip Setup").tooltip("Keep what's picked so far. Settings can change it any time.").on_click(cx.listener(|this, _, _, cx| finish(this, cx))))
        }))
        .child(
            div()
                .flex()
                .gap_2()
                .children(step.previous().map(|previous| {
                    Button::new("onboarding-back").outline().label("Back").on_click(cx.listener(move |this, _, _, cx| {
                        this.ui.onboarding = Some(previous);
                        cx.notify();
                    }))
                }))
                .child(next),
        )
}

/// A small heading over a group of choices.
fn heading(text: &'static str) -> impl IntoElement {
    div().pt_4().pb_2().text_size(px(size::SMALL_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(hsla(color::MUTED)).child(text.to_uppercase())
}

/// A choice drawn as a card: an icon, a name and a line about it, outlined in the accent when
/// picked.
fn choice_card(id: impl Into<ElementId>, icon: Option<IconName>, name: &'static str, about: &'static str, picked: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .p_3()
        .flex()
        .flex_col()
        .gap_1()
        .rounded(px(10.0))
        .border_2()
        .border_color(hsla(if picked { color::ACCENT } else { color::HAIRLINE }))
        .bg(hsla(color::PANEL))
        .cursor_pointer()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .children(icon.map(|icon| Icon::new(icon).size(px(18.0)).text_color(hsla(if picked { color::ACCENT } else { color::MUTED }))))
        .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(name))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(about))
}

/// Page 1: the theme and the accent color, applied as they are picked.
fn look_page(settings: &AppSettings, cx: &mut Context<Workspace>) -> impl IntoElement {
    let themes = ThemeName::ALL.into_iter().map(|theme| {
        super::settings::theme_choice(theme, settings.accent, settings.theme == theme).on_click(cx.listener(move |this, _, _, cx| change(this, cx, |settings| settings.theme = theme))).test_support()
    });
    let accents = Accent::ALL.into_iter().map(|accent| {
        super::settings::accent_choice(accent, settings.accent == accent).on_click(cx.listener(move |this, _, _, cx| change(this, cx, |settings| settings.accent = accent))).test_support()
    });
    div()
        .child(heading("Theme"))
        .child(div().flex().gap_3().children(themes))
        .child(heading("Accent color"))
        .child(div().flex().items_center().gap_3().child(div().flex().gap_1().children(accents)).child(div().text_color(hsla(color::MUTED)).child(settings.accent.name())))
        .child(heading("Preview"))
        .child(editor_preview())
}

/// A small picture of the editor in the theme and accent picked so far (it is drawn in the live
/// palette, so it changes as they are picked): the top bar and rail, the viewer, the Properties
/// panel, and a timeline with a few clips.
fn editor_preview() -> impl IntoElement {
    let bar = |width: f32, tone: u32, alpha: f32| div().h(px(5.0)).w(px(width)).rounded(px(2.0)).bg(hsla(tone).opacity(alpha));
    let clip = |left: f32, width: f32, alpha: f32| div().absolute().top(px(3.0)).bottom(px(3.0)).left(relative(left)).w(relative(width)).rounded(px(3.0)).bg(hsla(color::ACCENT).opacity(alpha));
    let track = |clips: Vec<gpui_kit::Div>| div().relative().h(px(18.0)).rounded(px(3.0)).bg(hsla(color::INSET)).children(clips);
    div()
        .id("onboarding-preview")
        .h(px(232.0))
        .rounded(px(12.0))
        .overflow_hidden()
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(hsla(color::BACKGROUND))
        .flex()
        .flex_col()
        // The top bar: the mark and a few tool icons, the current one in the accent.
        .child(
            div()
                .h(px(24.0))
                .flex_none()
                .px_2()
                .flex()
                .items_center()
                .gap_1p5()
                .child(div().size(px(12.0)).rounded(px(3.0)).bg(hsla(color::ACCENT)))
                .child(div().size(px(10.0)).rounded(px(2.0)).bg(hsla(color::ACCENT).opacity(0.35)))
                .children((0..5).map(|_| div().size(px(10.0)).rounded(px(2.0)).bg(hsla(color::MUTED).opacity(0.35))))
                .child(div().flex_1())
                .child(bar(60.0, color::MUTED, 0.4)),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .gap_1()
                .px_1()
                .pb_1()
                // The rail.
                .child(div().w(px(18.0)).flex_none().flex().flex_col().items_center().gap_1p5().pt_1().children((0..5).map(|index| div().size(px(10.0)).rounded(px(3.0)).bg(if index == 0 { hsla(color::ACCENT) } else { hsla(color::MUTED).opacity(0.3) }))))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .flex()
                                .gap_1()
                                // The Project panel.
                                .child(div().w(px(90.0)).rounded(px(4.0)).bg(hsla(color::PANEL)).p_1p5().flex().flex_col().gap_1p5().child(bar(50.0, color::TEXT, 0.7)).children((0..4).map(|_| bar(64.0, color::MUTED, 0.35))))
                                // The viewer: a picture with a selected layer in it.
                                .child(
                                    div().flex_1().rounded(px(4.0)).bg(hsla(color::PANEL)).p_2().flex().items_center().justify_center().child(
                                        div()
                                            .relative()
                                            .w(px(150.0))
                                            .h(px(84.0))
                                            .rounded(px(3.0))
                                            .bg(gpui_kit::linear_gradient(150.0, gpui_kit::linear_color_stop(hsla(color::ACCENT).opacity(0.55), 0.0), gpui_kit::linear_color_stop(hsla(color::INSET), 1.0)))
                                            .child(div().absolute().left(px(30.0)).top(px(28.0)).w(px(80.0)).h(px(26.0)).border_1().border_color(hsla(color::ACCENT)).child(div().m(px(5.0)).h(px(5.0)).w(px(50.0)).rounded(px(2.0)).bg(gpui_kit::white().opacity(0.9)))),
                                    ),
                                )
                                // Properties.
                                .child(div().w(px(110.0)).rounded(px(4.0)).bg(hsla(color::PANEL)).p_1p5().flex().flex_col().gap_1p5().child(bar(40.0, color::TEXT, 0.7)).children((0..3).map(|_| div().flex().justify_between().child(bar(36.0, color::MUTED, 0.35)).child(bar(22.0, color::VALUE, 0.8))))),
                        )
                        // The timeline.
                        .child(
                            div()
                                .relative()
                                .h(px(70.0))
                                .flex_none()
                                .rounded(px(4.0))
                                .bg(hsla(color::PANEL))
                                .p_1p5()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(track(vec![clip(0.02, 0.3, 0.9), clip(0.36, 0.22, 0.6)]))
                                .child(track(vec![clip(0.1, 0.45, 0.45), clip(0.6, 0.3, 0.75)]))
                                .child(div().absolute().top(px(2.0)).bottom(px(2.0)).left(relative(0.42)).w(px(1.5)).bg(hsla(color::PLAYHEAD))),
                        ),
                ),
        )
        .test_support()
}

/// Page 2: a starting point, then every non-AI tool and feature as a checkbox, by group.
fn features_page(settings: &AppSettings, cx: &mut Context<Workspace>) -> impl IntoElement {
    let matching = Focus::matching(settings);
    let starting_points = Focus::ALL.into_iter().enumerate().map(|(index, focus)| {
        choice_card(("onboarding-focus", index), None, focus.name(), focus.about(), matching == Some(focus)).on_click(cx.listener(move |this, _, _, cx| change(this, cx, |settings| focus.apply(settings)))).test_support()
    });
    let mut page = div().child(heading("Start from")).child(div().flex().gap_2().children(starting_points));
    for (group, features) in GROUPS.iter().filter(|(group, _)| *group != AI_GROUP) {
        let boxes = features.iter().map(|info| {
            let feature = info.feature;
            div().w(px((COLUMN_WIDTH - 26.0) / 3.0)).py_1().child(
                Checkbox::new(ElementId::Name(format!("onboarding-feature-{}", feature.id()).into()))
                    .label(info.name)
                    .checked(settings.feature_on(feature))
                    .tooltip(info.about)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        let checked = *checked;
                        change(this, cx, |settings| settings.set_feature(feature, checked));
                    })),
            )
        });
        page = page.child(heading(group)).child(div().px_3().py_2().rounded(px(10.0)).bg(hsla(color::PANEL)).border_1().border_color(hsla(color::HAIRLINE)).flex().flex_wrap().children(boxes));
    }
    page.child(div().pt_4().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("New Comp, Import, Settings, Select, Hand and the Media panel are always there."))
}

/// Page 3: AI on or off, and which AI features.
fn ai_page(settings: &AppSettings, cx: &mut Context<Workspace>) -> impl IntoElement {
    let using = settings.ai.any();
    let cards = div()
        .flex()
        .gap_3()
        .child(
            choice_card("onboarding-ai-on", Some(IconName::Sparkles), "Use local AI", "Captions from speech, background removal, auto reframe and voice cleanup.", using)
                .on_click(cx.listener(|this, _, _, cx| change(this, cx, |settings| settings.ai = crate::app_settings::AiFeatures::all(true))))
                .test_support(),
        )
        .child(
            choice_card("onboarding-ai-off", Some(IconName::EyeOff), "No AI", "Hide every AI feature. Nothing is downloaded. Turn them on later in Settings ▸ AI Models.", !using)
                .on_click(cx.listener(|this, _, _, cx| change(this, cx, |settings| settings.ai = crate::app_settings::AiFeatures::all(false))))
                .test_support(),
        );
    let ai_features = GROUPS.iter().find(|(group, _)| *group == AI_GROUP).map_or(&[][..], |(_, features)| *features);
    let choices = ai_features.iter().map(|info| {
        let feature = info.feature;
        div().py_1p5().flex().flex_col().gap_0p5().child(
            Checkbox::new(ElementId::Name(format!("onboarding-feature-{}", feature.id()).into())).label(info.name).checked(settings.feature_on(feature)).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                let checked = *checked;
                change(this, cx, |settings| settings.set_feature(feature, checked));
            })),
        )
        .child(div().pl_6().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(info.about))
    });
    div().child(cards).when(using, |page| {
        page.child(heading("Which ones"))
            .children(choices)
            .child(
                div()
                    .mt_3()
                    .p_3()
                    .rounded(px(8.0))
                    .bg(hsla(color::PANEL))
                    .text_size(px(size::SMALL_TEXT))
                    .text_color(hsla(color::MUTED))
                    .child("Captions use Whisper (about 150 MB), and Remove Background and Auto Reframe use MobileSAM (about 40 MB). Each is downloaded once from huggingface.co, and Studio asks first, the first time it's needed. Enhance Speech's models come built in."),
            )
    })
}

/// Page 4: a summary of what was picked.
fn done_page(settings: &AppSettings) -> impl IntoElement {
    let total = non_ai_features().count();
    let on = non_ai_features().filter(|feature| settings.feature_on(*feature)).count();
    let ai = if settings.ai.any() {
        let names: Vec<&str> = GROUPS.iter().filter(|(group, _)| *group == AI_GROUP).flat_map(|(_, features)| features.iter()).filter(|info| settings.feature_on(info.feature)).map(|info| info.name).collect();
        format!("On: {}", names.join(", "))
    } else {
        "Off. Nothing will be downloaded.".to_owned()
    };
    let row = |name: &'static str, value: String, where_: &'static str| {
        div()
            .py_2p5()
            .flex()
            .items_center()
            .gap_4()
            .border_b_1()
            .border_color(hsla(color::HAIRLINE))
            .child(div().w(px(160.0)).flex_none().text_color(hsla(color::MUTED)).child(name))
            .child(div().flex_1().min_w_0().child(value))
            .child(div().flex_none().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(where_))
    };
    let tick = div()
        .size(px(56.0))
        .mb_4()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(color::ACCENT).opacity(0.18))
        .border_1()
        .border_color(hsla(color::ACCENT).opacity(0.5))
        .child(Icon::new(IconName::Check).size(px(26.0)).text_color(hsla(color::ACCENT)));
    div()
        .id("onboarding-summary")
        .pt_2()
        .child(tick)
        .child(row("Look", format!("{}, {}", settings.theme.name(), settings.accent.name()), "Settings ▸ Appearance"))
        .child(row("Tools & features", format!("{on} of {total} on{}", Focus::matching(settings).map(|focus| format!(" ({})", focus.name())).unwrap_or_default()), "Settings ▸ Tools & Features"))
        .child(row("AI", ai, "Settings ▸ AI Models"))
        .test_support()
}
