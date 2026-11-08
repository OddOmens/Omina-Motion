//! The start screen, shown over the editor when Studio opens (and from File ▸ Start Screen or
//! by clicking Studio's name in the top bar, after offering to save unsaved changes):
//! recent projects to pick up again, a new project, opening another one, or starting from one
//! of the templates. Like After Effects' Home screen.

use std::path::{Path, PathBuf};

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
    workspace.ui.start_screen = Some(StartScreen { tab: StartTab::Home, filter, changes_declined: false, _filter_changes: changes });
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
    let page = match screen.tab {
        StartTab::Home => home_page(workspace, screen, cx).into_any_element(),
        StartTab::Templates => templates_page(workspace, cx).into_any_element(),
    };
    let main = div().relative().flex_1().min_w_0().flex().flex_col().child(accent_glow(320.0)).child(page);
    let body = div().flex_1().min_h_0().flex().child(sidebar(screen.tab, cx)).child(main);
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

/// Studio's mark: a clapperboard on an accent tile, `side` points square.
pub(crate) fn brand_mark(side: f32) -> gpui_kit::Div {
    div()
        .size(px(side))
        .flex_none()
        .rounded(px(side * 0.24))
        .flex()
        .items_center()
        .justify_center()
        .bg(linear_gradient(135.0, linear_color_stop(hsla(color::ACCENT), 0.0), linear_color_stop(hsla(color::ACCENT).opacity(0.7), 1.0)))
        .child(Icon::new(IconName::Clapperboard).size(px(side * 0.56)).text_color(gpui_kit::white()))
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
                .child(div().text_size(px(16.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Studio")),
        )
        .child(close)
}

/// The left column: New Project and Open Project, then the pages.
fn sidebar(tab: StartTab, cx: &mut Context<Workspace>) -> impl IntoElement {
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    let new = Button::new("start-new").primary().icon(Icon::new(IconName::Plus)).label("New Project").on_click(cx.listener(|this, _, window, cx| new_project(this, window, cx)));
    let open = Button::new("start-open").ghost().icon(Icon::new(IconName::FolderOpen)).label("Open Project…").on_click(cx.listener(|this, _, window, cx| this.open(window, cx)));
    let page = |id: &'static str, label: &'static str, icon: IconName, which: StartTab| {
        let active = tab == which;
        div()
            .id(id)
            .h(px(34.0))
            .px_3()
            .flex()
            .items_center()
            .gap_2p5()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_color(hsla(if active { color::TEXT } else { color::MUTED }))
            .when(active, |row| row.bg(hsla(color::ROW_SELECTED)).font_weight(gpui_kit::FontWeight::SEMIBOLD))
            .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(screen) = this.ui.start_screen.as_mut() {
                    screen.tab = which;
                }
                cx.notify();
            }))
            .child(Icon::new(icon).size(px(16.0)))
            .child(label)
            .test_support()
    };
    div()
        .w(px(240.0))
        .flex_none()
        .flex()
        .flex_col()
        .gap_1()
        .p_5()
        .bg(hsla(color::PANEL))
        .border_r_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().flex().flex_col().gap_1p5().child(new).child(open))
        .child(div().my_3().h(px(1.0)).bg(hsla(color::HAIRLINE)))
        .child(page("start-home", "Home", IconName::House, StartTab::Home))
        .child(page("start-templates", "Templates", IconName::LayoutTemplate, StartTab::Templates))
}

/// The Home page: a welcome, and the recent projects with a filter.
fn home_page(workspace: &Workspace, screen: &StartScreen, cx: &mut Context<Workspace>) -> impl IntoElement {
    let filter = screen.filter.read(cx).value().to_string();
    let recent = app_settings::load().recent_projects;
    let shown = shown_recent(&recent, &filter);
    let now = app_settings::now_seconds();
    let heading = |label: &'static str| div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(label);
    let header = div()
        .flex()
        .items_center()
        .h(px(30.0))
        .px_3()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().flex_1().min_w_0().child(heading("NAME")))
        .child(div().w(px(150.0)).child(heading("OPENED")))
        .child(div().w(px(80.0)).flex().justify_end().child(heading("SIZE")));
    let rows: Vec<AnyElement> = shown.iter().enumerate().map(|(index, project)| recent_row(index, project, now, cx).into_any_element()).collect();
    let empty = if recent.iter().any(|project| project.path.exists()) {
        "No recent projects match the filter."
    } else {
        "Projects you open or save will be listed here. Start a new one, or pick a template."
    };
    let list = div()
        .id("start-recent")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(header)
        .when(rows.is_empty(), |list| {
            list.child(
                div()
                    .id("start-recent-empty")
                    .m_3()
                    .p_5()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(hsla(color::HAIRLINE))
                    .bg(hsla(color::PANEL))
                    .child(div().size(px(34.0)).flex_none().rounded_full().flex().items_center().justify_center().bg(hsla(color::ACCENT).opacity(0.15)).child(Icon::new(IconName::Film).size(px(16.0)).text_color(hsla(color::ACCENT))))
                    .child(div().text_color(hsla(color::MUTED)).child(empty))
                    .test_support(),
            )
        })
        .children(rows);
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .gap_4()
        .px(px(48.0))
        .pt(px(28.0))
        .pb_6()
        .child(hero(recent.iter().filter(|project| project.path.exists()).count()))
        .child(div().text_size(px(16.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("New project"))
        .child(presets_row(cx))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_size(px(16.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Start from a template"))
                .child(
                    div()
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
                        .test_support(),
                ),
        )
        .child(super::templates::start_screen_strip(workspace, HOME_TEMPLATES, cx))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_size(px(16.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Recent"))
                .child(div().w(px(260.0)).child(Input::new(&screen.filter).prefix(Icon::new(IconName::Search).size(px(14.0)).text_color(hsla(color::MUTED))).cleanable(true))),
        )
        .child(list)
}

/// How many templates the Home page shows in its strip.
const HOME_TEMPLATES: usize = 16;

/// The banner across the top of Home: a welcome over a wash of the accent color.
fn hero(recent: usize) -> impl IntoElement {
    let (title, about) = if recent == 0 {
        ("Welcome to Studio", "Start a project at the size you need, or from a ready-made template.".to_owned())
    } else {
        ("Welcome back", format!("Pick up one of your {recent} recent project{}, or start something new.", if recent == 1 { "" } else { "s" }))
    };
    div()
        .id("start-hero")
        .relative()
        .flex_none()
        .h(px(108.0))
        .px_6()
        .flex()
        .items_center()
        .gap_4()
        .rounded(px(14.0))
        .overflow_hidden()
        .border_1()
        .border_color(hsla(color::ACCENT).opacity(0.35))
        .bg(linear_gradient(100.0, linear_color_stop(hsla(color::ACCENT).opacity(0.42), 0.0), linear_color_stop(hsla(color::ACCENT).opacity(0.06), 1.0)))
        .child(brand_mark(52.0))
        .child(div().flex().flex_col().gap_1().child(div().text_size(px(24.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(title)).child(div().text_color(hsla(color::TEXT)).opacity(0.75).child(about)))
        .test_support()
}

/// One recent project: its name and folder, when it was opened, and its size. A click opens it;
/// a right-click offers to show it in the Finder or take it off the list.
fn recent_row(index: usize, project: &RecentProject, now: u64, cx: &mut Context<Workspace>) -> impl IntoElement {
    use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
    let path = project.path.clone();
    let bytes = std::fs::metadata(&path).map_or(0, |metadata| metadata.len());
    let target = cx.weak_entity();
    let menu_path = path.clone();
    div()
        .id(("start-recent-row", index))
        .h(px(52.0))
        .px_3()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .cursor_pointer()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_click(cx.listener({
            let path = path.clone();
            move |this, _, window, cx| open_recent(this, path.clone(), window, cx)
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_3()
                .child(Icon::new(IconName::Film).size(px(18.0)).text_color(hsla(color::MUTED)))
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().truncate().child(display_name(&path)))
                        .child(div().truncate().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(display_folder(&path))),
                ),
        )
        .child(div().w(px(150.0)).text_color(hsla(color::MUTED)).child(time_ago(project.opened, now)))
        .child(div().w(px(80.0)).flex().justify_end().text_color(hsla(color::MUTED)).child(super::settings::format_bytes(bytes)))
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
                crate::workspace::refresh_menus(cx);
                target.update(cx, |_, cx| cx.notify()).ok();
            }))
        })
}

/// The Templates page: the template shapes and categories, and a card for each; a click starts
/// a new project from it.
fn templates_page(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let cards = super::templates::start_screen_cards(workspace, cx);
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
                .items_baseline()
                .gap_3()
                .child(div().text_size(px(22.0)).child("Start from a template"))
                .child(div().text_color(hsla(color::MUTED)).child(format!("{} templates · point at one to play it, + starts a new project with it", TEMPLATES.len()))),
        )
        .child(cards)
}

#[cfg(test)]
mod tests {
    use super::*;

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
