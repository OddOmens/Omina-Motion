//! The start screen, shown over the editor when Studio opens (and from File ▸ Start Screen or
//! by clicking Studio's name in the top bar, after offering to save unsaved changes):
//! recent projects to pick up again, a new project, opening another one, or starting from one
//! of the templates. Like After Effects' Home screen.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Subscription, TestSupportExt as _, Window, div,
    linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};
use studio_core::templates::TEMPLATES;

use crate::app_settings::{self, RecentProject};
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Which page of the start screen is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartTab {
    /// Recent projects.
    Home,
    /// Every template, to start a new project from one.
    Templates,
}

/// The start screen while it is showing.
pub struct StartScreen {
    pub tab: StartTab,
    /// Narrows the recent list by name or folder.
    pub filter: Entity<InputState>,
    /// The person was asked about unsaved changes on the way here and chose not to save, so
    /// opening or starting a project from here goes ahead without asking again.
    pub changes_declined: bool,
    /// Whether Home lists every recent project, not just the latest few.
    pub all_recent: bool,
    _filter_changes: Subscription,
}

/// Shows the start screen on its Home page.
pub fn show(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter recent projects"));
    // Typing in the filter redraws the list.
    let changes = cx.subscribe_in(&filter, window, |_, _, event: &InputEvent, _, cx| {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    });
    workspace.ui.start_screen = Some(StartScreen { tab: StartTab::Home, filter, changes_declined: false, all_recent: false, _filter_changes: changes });
    // The editor's own floating panels would draw over the start screen, so they close.
    workspace.ui.template_gallery = false;
    workspace.ui.color_picker = None;
    cx.notify();
}

/// Goes to the start screen from the editor (the Studio name in the top bar, or File ▸ Start
/// Screen). Unsaved changes are offered for saving first; Cancel stays in the editor.
pub fn go_to(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.unless_cancelled(window, cx, Box::new(|this, window, cx| {
        let declined = this.state.has_unsaved_changes();
        show(this, window, cx);
        // Still unsaved here means "Don't Save" was picked: the work stays open behind the
        // start screen (Esc goes back to it), but the buttons here don't ask about it again.
        if let Some(screen) = this.ui.start_screen.as_mut() {
            screen.changes_declined = declined;
        }
    }));
}

/// Whether the start screen is showing after the person chose not to save their changes.
pub fn changes_declined(workspace: &Workspace) -> bool {
    workspace.ui.start_screen.as_ref().is_some_and(|screen| screen.changes_declined)
}

/// Goes from the start screen to the editor.
pub fn hide(workspace: &mut Workspace) {
    workspace.ui.start_screen = None;
}

/// Starts a new, empty project (after offering to save unsaved changes) and goes to it.
fn new_project(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.unless_cancelled(window, cx, Box::new(|this, _, cx| {
        this.begin_new_project();
        hide(this);
        cx.notify();
    }));
}

/// Sizes a new project can start at from the start screen: name, what it suits, width, height,
/// frames per second.
pub const PRESETS: [(&str, &str, u32, u32, u32); 6] = [
    ("YouTube", "1080p 16:9", 1920, 1080, 30),
    ("4K", "UHD 16:9", 3840, 2160, 30),
    ("Vertical", "Reels, TikTok, Shorts", 1080, 1920, 30),
    ("Square", "1:1 feed post", 1080, 1080, 30),
    ("Portrait", "4:5 feed post", 1080, 1350, 30),
    ("Film", "1080p at 24 fps", 1920, 1080, 24),
];

/// Starts a new project at preset `index`'s size (after offering to save unsaved changes).
fn new_from_preset(workspace: &mut Workspace, index: usize, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(&(_, _, width, height, fps)) = PRESETS.get(index) else { return };
    workspace.unless_cancelled(window, cx, Box::new(move |this, _, cx| {
        let seconds = app_settings::load().new_composition.seconds;
        this.begin_new_project_sized(Some(app_settings::NewComposition { width, height, fps, seconds }));
        hide(this);
        cx.notify();
    }));
}

/// The row of preset cards: each shows its shape, name and size.
fn presets_row(cx: &mut Context<Workspace>) -> impl IntoElement {
    let cards = PRESETS.iter().enumerate().map(|(index, &(name, suits, width, height, fps))| {
        // The shape, at most 36 points either way.
        let scale = 36.0 / width.max(height) as f32;
        let shape = div()
            .w(px(width as f32 * scale))
            .h(px(height as f32 * scale))
            .rounded(px(3.0))
            .border_1()
            .border_color(hsla(color::ACCENT).opacity(0.7))
            .bg(linear_gradient(135.0, linear_color_stop(hsla(color::ACCENT).opacity(0.45), 0.0), linear_color_stop(hsla(color::ACCENT).opacity(0.1), 1.0)));
        div()
            .id(("start-preset", index))
            .flex_1()
            .min_w_0()
            .h(px(112.0))
            .p_2()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .rounded(px(10.0))
            .bg(hsla(color::PANEL))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .cursor_pointer()
            .hover(|style| style.border_color(hsla(color::ACCENT)).bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, _, window, cx| new_from_preset(this, index, window, cx)))
            .child(div().h(px(40.0)).flex().items_center().child(shape))
            .child(div().text_color(hsla(color::TEXT)).child(name))
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).truncate().child(format!("{width}×{height} · {fps} fps")))
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).truncate().child(suits))
            .test_support()
    });
    div().flex().gap_2().children(cards)
}

/// Starts a new project made from template `index` (after offering to save unsaved changes),
/// in the shape and colors chosen on the Templates page, and goes to it.
pub(crate) fn new_from_template(workspace: &mut Workspace, index: usize, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.unless_cancelled(window, cx, Box::new(move |this, _, cx| {
        let settings = app_settings::load();
        let (format, look) = (this.ui.template_format, this.ui.template_look);
        if this.state.new_project_from_template(&settings.default_folders, settings.new_composition, index, format, look).is_some() {
            app_settings::update(|app| app.last_project = None);
            hide(this);
        }
        cx.notify();
    }));
}

/// Opens a recent project (after offering to save unsaved changes). One that has gone missing
/// is taken off the list, with a note saying so.
pub(crate) fn open_recent(workspace: &mut Workspace, path: PathBuf, window: &mut Window, cx: &mut Context<Workspace>) {
    use gpui_kit::component::WindowExt as _;
    if !path.exists() {
        app_settings::forget_recent(&path);
        crate::workspace::refresh_menus(cx);
        window.push_notification(format!("“{}” is no longer there, so it was taken off the list.", display_name(&path)).as_str(), cx);
        cx.notify();
        return;
    }
    workspace.unless_cancelled(window, cx, Box::new(move |this, window, cx| this.open_file(path, window, cx)));
}

/// A project's name: its file name without `.studio`.
fn display_name(path: &Path) -> String {
    path.file_stem().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The folder a project is in, with the home folder written as `~`.
fn display_folder(path: &Path) -> String {
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if folder.starts_with(&home) => format!("~/{}", folder.strip_prefix(&home).unwrap_or(&folder).display()).trim_end_matches('/').to_owned(),
        _ => folder.display().to_string(),
    }
}

/// How long ago `then` was, from `now` (both in seconds since 1970), in words: "just now",
/// "5 minutes ago", "9 hours ago", "2 days ago" and so on.
pub fn time_ago(then: u64, now: u64) -> String {
    let seconds = now.saturating_sub(then);
    let (count, unit) = match seconds {
        0..60 => return "just now".into(),
        60..3_600 => (seconds / 60, "minute"),
        3_600..86_400 => (seconds / 3_600, "hour"),
        86_400..604_800 => (seconds / 86_400, "day"),
        604_800..2_592_000 => (seconds / 604_800, "week"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    if count == 1 { format!("1 {unit} ago") } else { format!("{count} {unit}s ago") }
}

/// The recent projects still on disk whose name or folder contains `filter` (any case).
pub fn shown_recent(recent: &[RecentProject], filter: &str) -> Vec<RecentProject> {
    let filter = filter.trim().to_lowercase();
    recent
        .iter()
        .filter(|project| project.path.exists())
        .filter(|project| filter.is_empty() || project.path.to_string_lossy().to_lowercase().contains(&filter))
        .cloned()
        .collect()
}

/// The start screen over the whole window, while it is showing.
pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let screen = workspace.ui.start_screen.as_ref()?;
    // With Templates switched off in Settings ▸ Tools & Features, its page is gone too.
    let templates_on = workspace.has(crate::features::Feature::Templates);
    let page = match screen.tab {
        StartTab::Templates if templates_on => templates_page(workspace, cx).into_any_element(),
        StartTab::Home | StartTab::Templates => home_page(workspace, screen, cx).into_any_element(),
    };
    let body = div().relative().flex_1().min_h_0().flex().flex_col().child(accent_glow(320.0)).child(page);
    let screen = div()
        .id("start-screen")
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .bg(hsla(color::BACKGROUND))
        .text_color(hsla(color::TEXT))
        .text_size(px(size::TEXT))
        .occlude()
        .child(top_bar(cx))
        .child(body)
        .test_support();
    // Drawn in the window's normal order, as the last thing over the editor, rather than as a
    // deferred overlay: deferred overlays paint above the window's notifications, which would
    // then be hidden under the start screen and couldn't be clicked.
    Some(screen.into_any_element())
}

/// A soft wash of the accent color fading down from the top of a screen, drawn behind its
/// content so a full-window page isn't a flat field of one color. Shared with the welcome setup.
pub(crate) fn accent_glow(height: f32) -> gpui_kit::Div {
    div().absolute().top_0().left_0().right_0().h(px(height)).bg(linear_gradient(
        180.0,
        linear_color_stop(hsla(color::ACCENT).opacity(0.14), 0.0),
        linear_color_stop(hsla(color::ACCENT).opacity(0.0), 1.0),
    ))
}

/// The app icon's artwork, built into the app so the mark never has to be loaded from disk.
static BRAND_MARK: std::sync::LazyLock<Arc<gpui_kit::Image>> = std::sync::LazyLock::new(|| {
    Arc::new(gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, include_bytes!("../../../../assets/icon/mark.png").to_vec()))
});

/// Omina Motion's mark, the same artwork as the app icon, `side` points square.
pub(crate) fn brand_mark(side: f32) -> gpui_kit::Div {
    div().size(px(side)).flex_none().child(gpui_kit::img(BRAND_MARK.clone()).size_full())
}

/// Studio's name on the left, and a button to go to the editor on the right.
fn top_bar(cx: &mut Context<Workspace>) -> impl IntoElement {
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    let close = Button::new("start-close").ghost().icon(Icon::new(IconName::X)).tooltip("Go to the editor (Esc)").on_click(cx.listener(|this, _, _, cx| {
        hide(this);
        cx.notify();
    }));
    div()
        .h(px(56.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .px_5()
        .bg(hsla(color::PANEL))
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(brand_mark(30.0))
                .child(div().text_size(px(size::TITLE_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Omina Motion")),
        )
        .child(close)
}

/// How many recent projects Home lists until "Show all" is pressed (a filter searches all).
const RECENT_SHOWN: usize = 8;

/// A section's title on the Home page.
fn section_title(title: &'static str) -> gpui_kit::Div {
    div().text_size(px(size::PAGE_TITLE_TEXT)).font_weight(gpui_kit::FontWeight::BOLD).child(title)
}

/// The Home page, which scrolls as a whole: a welcome, the recent projects (first, for someone
/// coming back to their work), a row of sample projects, and sizes to start a new project at.
/// Before any project has been opened, the recent list is a note at the bottom instead.
fn home_page(workspace: &Workspace, screen: &StartScreen, cx: &mut Context<Workspace>) -> impl IntoElement {
    // The sample projects and the way to every template come and go with the Templates feature.
    let templates_on = workspace.has(crate::features::Feature::Templates);
    let filter = screen.filter.read(cx).value().to_string();
    let recent = app_settings::load().recent_projects;
    let on_disk = recent.iter().filter(|project| project.path.exists()).count();
    let matching = shown_recent(&recent, &filter);
    // The latest few, unless a filter is narrowing the list or all were asked for.
    let limited = filter.trim().is_empty() && !screen.all_recent && matching.len() > RECENT_SHOWN;
    let shown = if limited { &matching[..RECENT_SHOWN] } else { &matching[..] };
    let now = app_settings::now_seconds();
    let cards: Vec<AnyElement> = shown.iter().enumerate().map(|(index, project)| recent_row(index, project, now, cx).into_any_element()).collect();
    // "View all", beside the heading, while only the latest few are showing.
    let view_all = limited.then(|| {
        div()
            .id("start-recent-all")
            .text_color(hsla(color::MUTED))
            .cursor_pointer()
            .hover(|style| style.text_color(hsla(color::TEXT)))
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(screen) = this.ui.start_screen.as_mut() {
                    screen.all_recent = true;
                }
                cx.notify();
            }))
            .child(format!("View all {}", matching.len()))
            .test_support()
    });
    let list = div()
        .id("start-recent")
        .flex_none()
        .flex()
        .flex_wrap()
        .gap_4()
        .when(cards.is_empty(), |list| list.child(div().py_4().text_color(hsla(color::MUTED)).child("No recent projects match the filter.")))
        .children(cards);
    let recent_section = div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(section_title("Recent projects"))
                .child(div().flex().items_center().gap_4().children(view_all).child(div().w(px(240.0)).child(Input::new(&screen.filter).prefix(Icon::new(IconName::Search).size(px(14.0)).text_color(hsla(color::MUTED))).cleanable(true)))),
        )
        .child(list);
    // With nothing opened yet, a note says where projects will be listed.
    let first_run_note = div()
        .id("start-recent-empty")
        .flex_none()
        .p_5()
        .flex()
        .items_center()
        .gap_3()
        .rounded(px(10.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(hsla(color::PANEL))
        .child(div().size(px(34.0)).flex_none().rounded_full().flex().items_center().justify_center().bg(hsla(color::ACCENT).opacity(0.15)).child(Icon::new(IconName::Film).size(px(16.0)).text_color(hsla(color::ACCENT))))
        .child(div().text_color(hsla(color::MUTED)).child(if templates_on { "Projects you open or save will be listed here. Start a new one, or pick a template." } else { "Projects you open or save will be listed here. Start a new one to begin." }))
        .test_support();
    let new_project_section = div().flex().flex_col().gap_3().child(section_title("New project")).child(presets_row(cx));
    let see_all = div()
        .id("start-see-templates")
        .flex()
        .items_center()
        .gap_1()
        .text_color(hsla(color::ACCENT))
        .cursor_pointer()
        .hover(|style| style.text_color(hsla(color::TEXT)))
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(screen) = this.ui.start_screen.as_mut() {
                screen.tab = StartTab::Templates;
            }
            cx.notify();
        }))
        .child(format!("All {} templates", TEMPLATES.len()))
        .child(Icon::new(IconName::ChevronRight).size(px(14.0)))
        .test_support();
    let samples_section = div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(section_title("Sample projects"))
                        .child(div().text_color(hsla(color::MUTED)).child("Start from a sample project and make it your own. Every template is on the Templates page too.")),
                )
                .child(see_all),
        )
        .child(super::templates::start_screen_strip(workspace, cx));
    div()
        .id("start-home-page")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_6()
        .px(px(48.0))
        .pt(px(28.0))
        .pb_8()
        .child(hero(on_disk, templates_on, cx))
        .when(on_disk > 0, |page| page.child(recent_section))
        .when(templates_on, |page| page.child(samples_section))
        .child(new_project_section)
        .when(on_disk == 0, |page| page.child(first_run_note))
}

/// The height of the Home page's hero, in points.
const HERO_HEIGHT: f32 = 320.0;

/// A faint grid behind the hero, like the ruled background of a design canvas: thin lines every
/// 40 points, across more width than any window has (the hero clips what it doesn't need).
fn hero_grid() -> gpui_kit::Div {
    const STEP: f32 = 40.0;
    let line = || div().absolute().bg(gpui_kit::white().opacity(0.035));
    let uprights = (1..80).map(|index| line().top_0().bottom_0().left(px(index as f32 * STEP)).w(px(1.0)));
    let levels = (1..(HERO_HEIGHT / STEP) as usize + 1).map(|index| line().left_0().right_0().top(px(index as f32 * STEP)).h(px(1.0)));
    div().absolute().inset_0().children(uprights).children(levels)
}

/// The artwork at the hero's left edge: a selected layer (a shape inside a selection box with
/// its four corner handles) and, under it, a property's keyframes along a line.
fn hero_layer_art() -> gpui_kit::Div {
    let accent = hsla(color::ACCENT);
    let handle = || div().absolute().size(px(9.0)).rounded_full().bg(accent).border_1().border_color(hsla(color::BACKGROUND));
    let selection = div()
        .relative()
        .w(px(178.0))
        .h(px(122.0))
        .border_1()
        .border_color(accent)
        // The layer itself: a soft block of color with a brighter disc on it.
        .child(div().absolute().left(px(22.0)).top(px(20.0)).w(px(134.0)).h(px(82.0)).rounded(px(12.0)).bg(linear_gradient(135.0, linear_color_stop(hsla(color::VALUE).opacity(0.55), 0.0), linear_color_stop(accent.opacity(0.18), 1.0))))
        .child(div().absolute().left(px(40.0)).top(px(34.0)).size(px(54.0)).rounded_full().bg(linear_gradient(135.0, linear_color_stop(accent.opacity(0.95), 0.0), linear_color_stop(hsla(color::VALUE).opacity(0.7), 1.0))))
        .child(handle().left(px(-5.0)).top(px(-5.0)))
        .child(handle().right(px(-5.0)).top(px(-5.0)))
        .child(handle().left(px(-5.0)).bottom(px(-5.0)))
        .child(handle().right(px(-5.0)).bottom(px(-5.0)));
    let keyframe = |left: f32, lit: bool| div().absolute().left(px(left)).top(px(-6.0)).child(Icon::new(IconName::Diamond).size(px(13.0)).text_color(if lit { accent } else { gpui_kit::white().opacity(0.45) }));
    let keyframes = div().relative().w(px(210.0)).h(px(1.0)).bg(gpui_kit::white().opacity(0.18)).child(keyframe(8.0, true)).child(keyframe(84.0, false)).child(keyframe(170.0, true));
    div().absolute().left(px(44.0)).top(px(52.0)).flex().flex_col().gap(px(34.0)).child(selection).child(keyframes)
}

/// The artwork at the hero's right edge: a few tracks of clips with a playhead across them,
/// and under them a color swatch and a type tile.
fn hero_timeline_art() -> gpui_kit::Div {
    let accent = hsla(color::ACCENT);
    let clip = |left: f32, width: f32, fill: gpui_kit::Hsla| div().absolute().left(px(left)).top_0().bottom_0().w(px(width)).rounded(px(6.0)).bg(fill).border_1().border_color(gpui_kit::white().opacity(0.12));
    let track = || div().relative().w(px(250.0)).h(px(26.0));
    let tracks = div()
        .relative()
        .flex()
        .flex_col()
        .gap(px(7.0))
        .child(track().child(clip(0.0, 150.0, hsla(color::VALUE).opacity(0.6))).child(clip(158.0, 92.0, hsla(color::VALUE).opacity(0.3))))
        .child(track().child(clip(34.0, 96.0, accent.opacity(0.75))).child(clip(138.0, 78.0, accent.opacity(0.35))))
        .child(track().child(clip(12.0, 206.0, gpui_kit::white().opacity(0.14))))
        // The playhead: a line down through the tracks with its handle on top.
        .child(div().absolute().left(px(112.0)).top(px(-10.0)).bottom(px(-8.0)).w(px(2.0)).rounded_full().bg(gpui_kit::white()))
        .child(div().absolute().left(px(107.0)).top(px(-16.0)).w(px(12.0)).h(px(9.0)).rounded(px(3.0)).bg(gpui_kit::white()));
    let tile = || div().size(px(54.0)).rounded(px(10.0)).flex().items_center().justify_center();
    let tiles = div()
        .flex()
        .gap_2()
        .child(tile().bg(linear_gradient(135.0, linear_color_stop(accent, 0.0), linear_color_stop(hsla(color::VALUE), 1.0))))
        .child(tile().bg(gpui_kit::white().opacity(0.88)).text_color(hsla(color::BACKGROUND)).text_size(px(22.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Aa"));
    div().absolute().right(px(40.0)).top(px(64.0)).flex().flex_col().items_end().gap(px(30.0)).child(tracks).child(tiles)
}

/// One of the hero's large actions: an icon over its name, on a dark tile that lights up in
/// the accent color under the pointer.
fn hero_action(id: &'static str, icon: IconName, label: &'static str, on_click: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id(id)
        .w(px(184.0))
        .h(px(92.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(px(16.0))
        .bg(hsla(color::BACKGROUND).opacity(0.72))
        .border_1()
        .border_color(gpui_kit::white().opacity(0.1))
        .cursor_pointer()
        .hover(|style| style.bg(hsla(color::ACCENT).opacity(0.28)).border_color(hsla(color::ACCENT)))
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
        .child(Icon::new(icon).size(px(20.0)))
        .child(div().text_size(px(size::HEADING_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(label))
        .test_support()
}

/// The hero across the top of Home: a welcome in large type over a ruled canvas with a glow of
/// the accent color rising from its foot, the three ways to begin (a new project, opening one,
/// the templates) on a tray under it, and artwork of the editor's own pieces at either side.
fn hero(recent: usize, templates_on: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let accent = hsla(color::ACCENT);
    let (title, about) = if recent == 0 { ("Welcome to Omina Motion", "Layers, keyframes and editing, all on one timeline.") } else { ("Welcome back", "Pick up where you left off, or start something new.") };
    let tray = div()
        .flex()
        .gap_3()
        .p_3()
        .rounded(px(24.0))
        .bg(accent.opacity(0.1))
        .border_1()
        .border_color(accent.opacity(0.4))
        .child(hero_action("start-new", IconName::Plus, "New project", new_project, cx))
        .child(hero_action("start-open", IconName::FolderOpen, "Open project…", |this, window, cx| this.open(window, cx), cx))
        .when(templates_on, |tray| {
            tray.child(hero_action("start-templates", IconName::LayoutTemplate, "Browse templates", |this, _, cx| {
                if let Some(screen) = this.ui.start_screen.as_mut() {
                    screen.tab = StartTab::Templates;
                }
                cx.notify();
            }, cx))
        });
    div()
        .id("start-hero")
        .relative()
        .flex_none()
        .h(px(HERO_HEIGHT))
        .rounded(px(18.0))
        .overflow_hidden()
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(hsla(color::PANEL))
        .child(hero_grid())
        // The glow, rounded at the foot like the hero so it doesn't square off its corners.
        .child(div().absolute().left_0().right_0().bottom_0().h(px(HERO_HEIGHT * 0.8)).rounded_b(px(18.0)).bg(linear_gradient(0.0, linear_color_stop(accent.opacity(0.5), 0.0), linear_color_stop(accent.opacity(0.0), 1.0))))
        .child(hero_layer_art())
        .child(hero_timeline_art())
        // The words and the tray, centered over everything else.
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().text_size(px(size::HERO_TEXT)).line_height(px(60.0)).font_weight(gpui_kit::FontWeight::BOLD).child(title))
                .child(div().text_size(px(size::TITLE_TEXT)).text_color(hsla(color::TEXT)).opacity(0.8).child(about))
                .child(div().pt_5().child(tray)),
        )
        .test_support()
}

/// A project file's size in words. Project files hold no media, so most are well under a
/// megabyte: those are given in kilobytes rather than as "0 MB".
fn file_size(bytes: u64) -> String {
    if bytes < 1_000_000 { format!("{} KB", bytes.div_ceil(1000).max(1)) } else { super::settings::format_bytes(bytes) }
}

/// The size of a recent project's card picture, in points: 16:9.
const RECENT_PICTURE: (f32, f32) = (232.0, 130.5);

/// One recent project, as a card: its picture from the last save (a film icon until it has
/// one), then its name, when it was opened and its size. Its folder shows in a tooltip. A
/// click opens it; a right-click offers to show it in the Finder or take it off the list.
fn recent_row(index: usize, project: &RecentProject, now: u64, cx: &mut Context<Workspace>) -> impl IntoElement {
    use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
    let path = project.path.clone();
    let bytes = std::fs::metadata(&path).map_or(0, |metadata| metadata.len());
    let target = cx.weak_entity();
    let menu_path = path.clone();
    let folder: gpui_kit::SharedString = display_folder(&path).into();
    let picture = div()
        .w(px(RECENT_PICTURE.0))
        .h(px(RECENT_PICTURE.1))
        .flex_none()
        .rounded_t(px(11.0))
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
        .bg(linear_gradient(135.0, linear_color_stop(hsla(color::INSET), 0.0), linear_color_stop(hsla(color::ACCENT).opacity(0.16), 1.0)))
        .child(match crate::project_thumbnails::find(&path) {
            Some(picture) => gpui_kit::StyledImage::object_fit(gpui_kit::img(picture).size_full(), gpui_kit::ObjectFit::Cover).into_any_element(),
            None => Icon::new(IconName::Film).size(px(28.0)).text_color(hsla(color::MUTED)).into_any_element(),
        });
    div()
        .id(("start-recent-row", index))
        .w(px(RECENT_PICTURE.0))
        .flex_none()
        .flex()
        .flex_col()
        .rounded(px(12.0))
        .bg(hsla(color::PANEL))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .cursor_pointer()
        .hover(|style| style.border_color(hsla(color::ACCENT)).bg(hsla(color::HOVER)))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(folder.clone()).build(window, cx))
        .on_click(cx.listener({
            let path = path.clone();
            move |this, _, window, cx| open_recent(this, path.clone(), window, cx)
        }))
        .child(picture)
        .child(
            div()
                .px_3()
                .py_2p5()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(div().truncate().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(display_name(&path)))
                .child(div().truncate().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("{} · {}", time_ago(project.opened, now), file_size(bytes)))),
        )
        .test_support()
        .context_menu(move |menu, _, _| {
            let (reveal, forget, target) = (menu_path.clone(), menu_path.clone(), target.clone());
            menu.item(PopupMenuItem::new("Show in Finder").on_click(move |_, window, cx| {
                use gpui_kit::component::WindowExt as _;
                if let Err(error) = super::project::reveal_in_finder(&reveal) {
                    window.push_notification(format!("Couldn't show the project in the Finder: {error}").as_str(), cx);
                }
            }))
            .item(PopupMenuItem::new("Remove from Recent").on_click(move |_, _, cx| {
                app_settings::forget_recent(&forget);
                crate::project_thumbnails::forget(&forget);
                crate::workspace::refresh_menus(cx);
                target.update(cx, |_, cx| cx.notify()).ok();
            }))
        })
}

/// The Templates page: a button back to Home, the template shapes and categories, and a card
/// for each; a click starts a new project from it.
fn templates_page(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    let cards = super::templates::start_screen_cards(workspace, cx);
    // The way back to Home, which is where this page is reached from.
    let back = Button::new("start-home").ghost().icon(Icon::new(IconName::ChevronLeft)).tooltip("Back to Home").on_click(cx.listener(|this, _, _, cx| {
        if let Some(screen) = this.ui.start_screen.as_mut() {
            screen.tab = StartTab::Home;
        }
        cx.notify();
    }));
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .gap_3()
        .px(px(48.0))
        .pt(px(32.0))
        .pb_4()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(back)
                .child(div().text_size(px(size::PAGE_TITLE_TEXT)).child("Start from a template"))
                .child(div().text_color(hsla(color::MUTED)).child(format!("{} templates · point at one to play it, + starts a new project with it", TEMPLATES.len()))),
        )
        .child(cards)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_project_files_are_sized_in_kilobytes() {
        assert_eq!(file_size(0), "1 KB");
        assert_eq!(file_size(41_200), "42 KB");
        assert_eq!(file_size(3_400_000), "3 MB");
    }

    #[test]
    fn times_read_like_after_effects() {
        assert_eq!(time_ago(1_000, 1_030), "just now");
        assert_eq!(time_ago(0, 60), "1 minute ago");
        assert_eq!(time_ago(0, 9 * 3_600 + 59), "9 hours ago");
        assert_eq!(time_ago(0, 2 * 86_400), "2 days ago");
        assert_eq!(time_ago(0, 3 * 604_800), "3 weeks ago");
        assert_eq!(time_ago(0, 40_000_000), "1 year ago");
        assert_eq!(time_ago(100, 50), "just now", "a clock that went back");
    }

    #[test]
    fn the_filter_matches_names_and_folders_of_files_still_there() {
        let folder = std::env::temp_dir().join(format!("studio-recent-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (promo, launch) = (folder.join("Promo.studio"), folder.join("Launch Video.studio"));
        std::fs::write(&promo, "{}").unwrap();
        std::fs::write(&launch, "{}").unwrap();
        let recent = [&promo, &launch, &folder.join("Gone.studio")].map(|path| RecentProject { path: path.clone(), opened: 0 });
        assert_eq!(shown_recent(&recent, "").len(), 2, "the missing one is left out");
        let found = shown_recent(&recent, "  launch ");
        assert_eq!(found.iter().map(|project| project.path.clone()).collect::<Vec<_>>(), [launch]);
        assert_eq!(shown_recent(&recent, "studio-recent-test").len(), 2, "by folder too");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
