//! The Settings dialog (Cmd+, or the gear at the bottom of the rail): a menu of pages down the
//! left, grouped (Studio, Project, Storage, Help), and the chosen page on the right. Each page
//! has a heading, a line about what it covers, and its settings in cards: a name and a short
//! description on the left, the control on the right. Every change applies at once and is
//! remembered.
//!
//! In memory, playback keeps only a few decoded frames around the playhead for each clip, which
//! needs no setting; the Media Cache page covers what is kept on disk.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, TestSupportExt as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px, rgb,
};

use crate::app_settings::{self, AppSettings, ImportFolders, PlaybackQuality, ProjectColumns, StartWith, Transparency};
use crate::theme::{Accent, ThemeName, color, hsla, size};
use crate::workspace::Workspace;

/// Disk limits offered for proxies, in gigabytes (`None` for no limit).
const CACHE_LIMITS: &[Option<u32>] = &[Some(5), Some(10), Some(25), Some(50), Some(100), None];
/// Recovery-copy intervals offered, in seconds (0 for never).
const RECOVERY_CHOICES: &[u32] = &[15, 30, 60, 120, 300, 0];
/// Sizes offered for new compositions: name, width, height.
const SIZES: &[(&str, u32, u32)] = &[
    ("HD 1080p", 1920, 1080),
    ("4K UHD", 3840, 2160),
    ("HD 720p", 1280, 720),
    ("Vertical 9:16", 1080, 1920),
    ("Portrait 4:5", 1080, 1350),
    ("Square 1:1", 1080, 1080),
];
const FRAME_RATES: &[u32] = &[24, 25, 30, 50, 60];
const LENGTHS: &[u32] = &[5, 10, 15, 30, 60, 120];

fn limit_label(limit: Option<u32>) -> String {
    limit.map_or_else(|| "No limit".to_owned(), |gb| format!("{gb} GB"))
}

fn recovery_label(seconds: u32) -> String {
    match seconds {
        0 => "Never".to_owned(),
        s if s < 60 => format!("Every {s} seconds"),
        60 => "Every minute".to_owned(),
        s => format!("Every {} minutes", s / 60),
    }
}

/// A byte count for people: "820 MB", "1.4 GB".
pub fn format_bytes(bytes: u64) -> String {
    let megabytes = bytes as f64 / 1_000_000.0;
    if megabytes < 1000.0 { format!("{megabytes:.0} MB") } else { format!("{:.1} GB", megabytes / 1000.0) }
}

/// The dialog's pages, in the order of its menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    General,
    Appearance,
    Playback,
    Timeline,
    Features,
    ProjectPanel,
    Folders,
    MediaCache,
    Models,
    Shortcuts,
    About,
}

impl Section {
    /// The menu, in groups.
    const GROUPS: [(&'static str, &'static [Section]); 4] = [
        ("Studio", &[Self::General, Self::Appearance, Self::Playback, Self::Timeline, Self::Features]),
        ("Project", &[Self::ProjectPanel, Self::Folders]),
        ("Storage", &[Self::MediaCache, Self::Models]),
        ("Help", &[Self::Shortcuts, Self::About]),
    ];

    fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Playback => "Playback",
            Self::Timeline => "Timeline & Viewer",
            Self::Features => "Tools & Features",
            Self::ProjectPanel => "Project Panel",
            Self::Folders => "Folders",
            Self::MediaCache => "Media Cache",
            Self::Models => "AI Models",
            Self::Shortcuts => "Keyboard Shortcuts",
            Self::About => "About",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::General => IconName::Settings,
            Self::Appearance => IconName::Palette,
            Self::Playback => IconName::Play,
            Self::Timeline => IconName::Film,
            Self::Features => IconName::ToggleRight,
            Self::ProjectPanel => IconName::Menu,
            Self::Folders => IconName::Folder,
            Self::MediaCache => IconName::Hourglass,
            Self::Models => IconName::Sparkles,
            Self::Shortcuts => IconName::Type,
            Self::About => IconName::Library,
        }
    }

    /// A line under the page's heading saying what it covers.
    fn about(self) -> &'static str {
        match self {
            Self::General => "Starting up, keeping unsaved work safe, and what new compositions start as.",
            Self::Appearance => "How Studio looks.",
            Self::Playback => "How the viewer plays.",
            Self::Timeline => "What the timeline and viewer show, and what snaps.",
            Self::Features => "Turn off what you don't use, and it's hidden everywhere: tools, panels, menus and shortcuts.",
            Self::ProjectPanel => "Which columns the Project panel's list shows.",
            Self::Folders => "The folders new projects start with, and where imports go.",
            Self::MediaCache => "Files Studio makes to keep playback smooth, and can always make again.",
            Self::Models => "Captions and background removal run on this computer with these models.",
            Self::Shortcuts => "Keys for the things you do most.",
            Self::About => "This copy of Studio, and where it keeps things.",
        }
    }
}

/// What the dialog shows; kept outside the dialog so its controls can change it.
struct Shown {
    section: Section,
    settings: AppSettings,
    /// Bytes on disk, measured when the dialog opened and after each change.
    proxy_bytes: u64,
    mask_bytes: u64,
    enhanced_bytes: u64,
    imported_bytes: u64,
    /// Models being downloaded from the AI Models page, by their row's id.
    downloading: Vec<&'static str>,
    /// The command waiting for its new shortcut on the Keyboard Shortcuts page.
    recording: Option<&'static str>,
    note: Option<String>,
}

type SharedShown = Rc<RefCell<Shown>>;

/// The total size of the files in a folder and everything inside it.
fn folder_size(folder: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else { return 0 };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(metadata) if metadata.is_dir() => folder_size(&entry.path()),
            Ok(metadata) => metadata.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Saves a settings change, shows it, hands it to the workspace (whose panels read it as they
/// draw), and redraws.
fn change(shown: &SharedShown, workspace: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App, edit: impl Fn(&mut AppSettings)) {
    let before = shown.borrow().settings.clone();
    app_settings::update(&edit);
    edit(&mut shown.borrow_mut().settings);
    let settings = shown.borrow().settings.clone();
    // Shortcuts changed, or a feature whose shortcut and menu items come and go with it.
    if settings.shortcuts != before.shortcuts || settings.features_off != before.features_off || settings.ai != before.ai {
        crate::keymap::apply(&settings, cx);
    }
    workspace
        .update(cx, |this, cx| {
            this.use_settings(settings);
            cx.notify();
        })
        .ok();
    window.refresh();
}

// Layout pieces ---------------------------------------------------------------------------

/// A page's heading and the line under it.
fn page(section: Section) -> gpui_kit::Div {
    div()
        .pb_3()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_size(px(18.0)).text_color(hsla(color::TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(section.name()))
        .child(div().text_size(px(size::TEXT)).text_color(hsla(color::MUTED)).child(section.about()))
}

/// A small heading above a card.
fn group_title(text: &'static str) -> gpui_kit::Div {
    div().pt_3().pb_1p5().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(text.to_uppercase())
}

/// A rounded card holding rows, with a hairline between each.
fn card(rows: Vec<AnyElement>) -> gpui_kit::Div {
    let count = rows.len();
    div()
        .rounded(px(10.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(hsla(color::RAISED))
        .children(rows.into_iter().enumerate().map(move |(index, row)| div().when(index + 1 < count, |row| row.border_b_1().border_color(hsla(color::HAIRLINE))).child(row)))
}

/// One setting: its name and a line about it on the left, its control on the right.
fn setting(name: impl Into<SharedString>, about: impl Into<SharedString>, control: impl IntoElement) -> AnyElement {
    div()
        .min_h(px(54.0))
        .px_3p5()
        .py_2()
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(div().text_size(px(size::TEXT + 1.0)).text_color(hsla(color::TEXT)).child(name.into()))
                .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(about.into())),
        )
        .child(div().flex_none().flex().items_center().gap_1p5().child(control))
        .into_any_element()
}

/// An on/off switch for a setting.
fn switch(id: &'static str, on: bool, shown: &SharedShown, workspace: &WeakEntity<Workspace>, set: fn(&mut AppSettings, bool)) -> impl IntoElement {
    let (shown, workspace) = (shown.clone(), workspace.clone());
    Switch::new(id).checked(on).on_click(move |checked: &bool, window, cx| {
        let checked = *checked;
        change(&shown, &workspace, window, cx, |settings| set(settings, checked));
    })
}

/// A dropdown of `choices` (label, value); picking one runs `set`.
fn menu<T: Clone + PartialEq + 'static>(id: &'static str, choices: Vec<(String, T)>, current: T, shown: &SharedShown, workspace: &WeakEntity<Workspace>, set: fn(&mut AppSettings, T)) -> impl IntoElement {
    let label = choices.iter().find(|(_, value)| *value == current).map_or_else(|| "Custom".to_owned(), |(label, _)| label.clone());
    let (shown, workspace) = (shown.clone(), workspace.clone());
    Button::new(id).outline().small().label(label).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
        choices.iter().fold(menu, |menu, (label, value)| {
            let (shown, workspace, value) = (shown.clone(), workspace.clone(), value.clone());
            let checked = value == current;
            menu.item(PopupMenuItem::new(label.clone()).checked(checked).on_click(move |_, window, cx| {
                let value = value.clone();
                change(&shown, &workspace, window, cx, |settings| set(settings, value.clone()));
            }))
        })
    })
}

/// Text shown at the end of a page after an action ("Freed 820 MB").
fn note(shown: &SharedShown) -> Option<AnyElement> {
    shown.borrow().note.clone().map(|note| div().pt_3().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).child(note).into_any_element())
}

// Pages -------------------------------------------------------------------------------------

fn general(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let now = shown.borrow().settings.clone();
    let size = now.new_composition;
    let sizes = SIZES.iter().map(|&(name, width, height)| (format!("{name} · {width}×{height}"), (width, height))).collect();
    let rates = FRAME_RATES.iter().map(|&fps| (format!("{fps} fps"), fps)).collect();
    let lengths = LENGTHS.iter().map(|&seconds| (if seconds < 60 { format!("{seconds} seconds") } else { format!("{} minutes", seconds / 60) }, seconds)).collect();
    let start_choices = vec![("Show the start screen".to_owned(), StartWith::StartScreen), ("Reopen the last project".to_owned(), StartWith::LastProject)];
    let recovery = RECOVERY_CHOICES.iter().map(|&seconds| (recovery_label(seconds), seconds)).collect();
    div()
        .child(page(Section::General))
        .child(group_title("Starting up"))
        .child(card(vec![
            setting("When Studio starts", "Show the start screen, or go straight back to the project you were working on.", menu("general-start", start_choices, now.start_with, shown, workspace, |settings, start| settings.start_with = start)),
            setting(
                "Welcome setup",
                "Pick a theme, the tools and features you want, and whether to use AI, step by step.",
                {
                    let workspace = workspace.clone();
                    Button::new("general-welcome").outline().small().label("Show Again").on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        workspace.update(cx, super::onboarding::show).ok();
                    })
                },
            ),
            setting("Recovery copy", "How often unsaved work is kept aside, so a crash loses little. Offered back on the next start.", menu("general-recovery", recovery, now.recovery_seconds, shown, workspace, |settings, seconds| settings.recovery_seconds = seconds)),
        ]))
        .child(group_title("Versions"))
        .child(card(vec![
            setting("Keep earlier versions", "A copy each time you save, and every few minutes while you work, to roll back to with File ▸ Browse Versions….", switch("general-versions", now.versions.keep, shown, workspace, |settings, on| settings.versions.keep = on)),
            setting("Every", "How often a version is kept while a project changes.", menu("general-versions-every", [5, 10, 15, 30].iter().map(|&minutes| (format!("{minutes} minutes"), minutes)).collect(), now.versions.every_minutes, shown, workspace, |settings, minutes| settings.versions.every_minutes = minutes)),
            setting("Keep", "How many each project keeps. The oldest go first.", menu("general-versions-count", [10, 30, 100].iter().map(|&count| (format!("{count} versions"), count)).collect(), now.versions.how_many, shown, workspace, |settings, count| settings.versions.how_many = count)),
        ]))
        .child(group_title("New compositions"))
        .child(card(vec![
            setting("Frame size", "What new compositions and new projects start at.", menu("general-size", sizes, (size.width, size.height), shown, workspace, |settings, (width, height)| (settings.new_composition.width, settings.new_composition.height) = (width, height))),
            setting("Frame rate", "Frames per second.", menu("general-fps", rates, size.fps, shown, workspace, |settings, fps| settings.new_composition.fps = fps)),
            setting("Length", "How long they run.", menu("general-length", lengths, size.seconds, shown, workspace, |settings, seconds| settings.new_composition.seconds = seconds)),
        ]))
        .child(div().pt_2().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("New compositions start {}×{}, {} fps, {} seconds, transparent.", size.width, size.height, size.fps, size.seconds)))
}

/// Applies a theme and accent everywhere.
fn set_look(theme: ThemeName, accent: Accent, cx: &mut App) {
    crate::theme::apply(theme, accent, cx);
    cx.refresh_windows();
}

fn appearance(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let now = shown.borrow().settings.clone();
    let themes = ThemeName::ALL.into_iter().map(|theme| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        theme_choice(theme, now.accent, now.theme == theme)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                let accent = shown.borrow().settings.accent;
                change(&shown, &workspace, window, cx, |settings| settings.theme = theme);
                set_look(theme, accent, cx);
            })
            .test_support()
    });
    let accents = Accent::ALL.into_iter().map(|accent| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        accent_choice(accent, now.accent == accent)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                let theme = shown.borrow().settings.theme;
                change(&shown, &workspace, window, cx, |settings| settings.accent = accent);
                set_look(theme, accent, cx);
            })
            .test_support()
    });
    div()
        .child(page(Section::Appearance))
        .child(group_title("Theme"))
        .child(div().flex().gap_3().children(themes))
        .child(group_title("Accent color"))
        .child(card(vec![setting(now.accent.name(), "Used for selections, buttons and highlights.", div().flex().gap_1().children(accents))]))
}

/// A theme to pick: a small picture of it (its background, a panel, a line of text and the
/// accent) with its name under it. Shared by Settings ▸ Appearance and the welcome setup.
pub(super) fn theme_choice(theme: ThemeName, accent: Accent, selected: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    let (background, panel, text) = match theme {
        ThemeName::Dark => (0x070708, 0x18181b, 0xededef),
        ThemeName::Black => (0x000000, 0x111111, 0xf2f2f2),
        ThemeName::Light => (0xe9e9ec, 0xffffff, 0x18181b),
    };
    let accent = accent.value();
    div()
        .id(("theme", theme as usize))
        .flex_1()
        .flex()
        .flex_col()
        .gap_2()
        .cursor_pointer()
        .child(
            div()
                .h(px(84.0))
                .rounded(px(10.0))
                .border_2()
                .border_color(hsla(if selected { color::ACCENT } else { color::HAIRLINE }))
                .bg(rgb(background))
                .p_2()
                .flex()
                .gap_1p5()
                .child(div().w(px(18.0)).rounded(px(4.0)).bg(rgb(panel)))
                .child(
                    div()
                        .flex_1()
                        .rounded(px(4.0))
                        .bg(rgb(panel))
                        .p_1p5()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().h(px(5.0)).w(px(46.0)).rounded(px(2.0)).bg(rgb(text)))
                        .child(div().h(px(4.0)).w(px(30.0)).rounded(px(2.0)).bg(rgb(text)).opacity(0.5))
                        .child(div().mt_auto().h(px(8.0)).w(px(26.0)).rounded(px(4.0)).bg(rgb(accent))),
                ),
        )
        .child(div().flex().items_center().gap_1p5().text_size(px(size::TEXT)).text_color(hsla(if selected { color::TEXT } else { color::MUTED })).child(theme.name()))
}

/// An accent color to pick: a round swatch, ringed when it is the one in use.
pub(super) fn accent_choice(accent: Accent, selected: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(("accent", accent as usize))
        .size(px(28.0))
        .rounded_full()
        .border_2()
        .border_color(hsla(if selected { color::TEXT } else { color::RAISED }))
        .p(px(3.0))
        .cursor_pointer()
        .child(div().size_full().rounded_full().bg(rgb(accent.value())))
}

fn playback(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let now = shown.borrow().settings.clone();
    let qualities = PlaybackQuality::ALL.iter().map(|&quality| (quality.name().to_owned(), quality)).collect();
    div()
        .child(page(Section::Playback))
        .child(group_title("Viewer"))
        .child(card(vec![
            setting("Quality while playing", "Lower plays heavy compositions more smoothly. Paused frames are always full quality. Auto halves it on Retina screens.", menu("playback-quality", qualities, now.playback_quality, shown, workspace, |settings, quality| settings.playback_quality = quality)),
            setting("Loop playback", "Start over at the end of the work area. Applies from the next start; the loop button changes it now.", switch("playback-loop", now.loop_playback, shown, workspace, |settings, on| settings.loop_playback = on)),
            setting(
                "Render ahead",
                "While you pause, the frames after the playhead are rendered in the background and kept, so heavy compositions play smoothly. A green line on the ruler marks them. More memory keeps more frames.",
                menu("playback-render-ahead", [(0, "Off"), (1024, "1 GB"), (2048, "2 GB"), (4096, "4 GB"), (8192, "8 GB")].iter().map(|&(mb, name)| (name.to_owned(), mb)).collect(), now.render_ahead_mb, shown, workspace, |settings, mb| settings.render_ahead_mb = mb),
            ),
        ]))
}

fn timeline(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let now = shown.borrow().settings.clone();
    let backgrounds = Transparency::ALL.iter().map(|&choice| (choice.name().to_owned(), choice)).collect();
    div()
        .child(page(Section::Timeline))
        .child(group_title("Timeline"))
        .child(card(vec![
            setting("Pictures on clips", "Frames from the footage along each video clip.", switch("timeline-thumbnails", now.clip_thumbnails, shown, workspace, |settings, on| settings.clip_thumbnails = on)),
            setting("Waveforms on clips", "The shape of the sound along clips with sound.", switch("timeline-waveforms", now.clip_waveforms, shown, workspace, |settings, on| settings.clip_waveforms = on)),
            setting("Compact tracks", "Short rows showing just each clip's name, so more tracks fit. The rows button in the timeline's bar changes it too.", switch("timeline-compact", now.compact_timeline, shown, workspace, |settings, on| settings.compact_timeline = on)),
            setting("Frame render time", "How long the last frame took to draw, under the timeline.", switch("timeline-render-time", now.show_render_time, shown, workspace, |settings, on| settings.show_render_time = on)),
            setting("Snapping starts on", "Moves and trims snap to edges, keyframes, markers and the playhead. Applies from the next start; S turns it on and off now.", switch("timeline-snapping", now.snapping, shown, workspace, |settings, on| settings.snapping = on)),
        ]))
        .child(group_title("Viewer"))
        .child(card(vec![
            setting("Behind transparency", "What shows where a composition has nothing.", menu("viewer-transparency", backgrounds, now.transparency, shown, workspace, |settings, choice| settings.transparency = choice)),
            setting("Anchor points snap", "Dragged anchor points snap to a layer's corners, edges and middle. Applies from the next start; the magnet under the viewer changes it now.", switch("viewer-anchor-snapping", now.anchor_snapping, shown, workspace, |settings, on| settings.anchor_snapping = on)),
        ]))
}

/// The Tools & Features page: a switch for every tool and feature that can be turned off, in
/// groups. New Composition, Import, Settings, Select, Hand and the Media panel are always on, so
/// they aren't listed.
fn features(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    use crate::features::{Feature, GROUPS};
    let now = shown.borrow().settings.clone();
    let any_off = GROUPS.iter().flat_map(|(_, features)| features.iter()).any(|info| !now.feature_on(info.feature));
    let switch_for = |feature: Feature| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Switch::new(gpui_kit::ElementId::Name(format!("feature-{}", feature.id()).into())).checked(now.feature_on(feature)).on_click(move |checked: &bool, window, cx| {
            let checked = *checked;
            change(&shown, &workspace, window, cx, |settings| settings.set_feature(feature, checked));
        })
    };
    let mut page_div = div().child(page(Section::Features));
    for (group, features) in GROUPS {
        page_div = page_div.child(group_title(group)).child(card(features.iter().map(|info| setting(info.name, info.about, switch_for(info.feature))).collect()));
    }
    let (reset_shown, reset_workspace) = (shown.clone(), workspace.clone());
    page_div
        .child(
            div()
                .pt_2()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                // The note wraps, so the button always stays in view beside it.
                .child(div().flex_1().min_w_0().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("New Comp, Import, Settings, Select, Hand and Media are always on. Turning something off keeps what it already made."))
                .child(Button::new("features-all-on").flex_none().outline().small().label("Turn All On").disabled(!any_off).on_click(move |_, window, cx| {
                    change(&reset_shown, &reset_workspace, window, cx, |settings| {
                        for (_, features) in GROUPS {
                            for info in *features {
                                settings.set_feature(info.feature, true);
                            }
                        }
                    });
                })),
        )
        .children(note(shown))
}

fn project_panel(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let columns = shown.borrow().settings.project_columns;
    let column = |id: &'static str, name: &'static str, about: &'static str, on: bool, set: fn(&mut ProjectColumns, bool)| -> AnyElement {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        setting(
            name,
            about,
            Switch::new(id).checked(on).on_click(move |checked: &bool, window, cx| {
                let checked = *checked;
                change(&shown, &workspace, window, cx, |settings| set(&mut settings.project_columns, checked));
            }),
        )
    };
    div()
        .child(page(Section::ProjectPanel))
        .child(group_title("Columns"))
        .child(card(vec![
            setting("Name", "Always shown.", Switch::new("column-name").checked(true).disabled(true)),
            column("column-label", "Label color", "The colored tag that groups items by eye.", columns.label, |columns, on| columns.label = on),
            column("column-type", "Type", "Footage, image, audio, composition… and proxy progress.", columns.kind, |columns, on| columns.kind = on),
            column("column-duration", "Duration", "How long footage, sound and compositions run.", columns.duration, |columns, on| columns.duration = on),
            column("column-dimensions", "Dimensions", "Width × height in pixels.", columns.dimensions, |columns, on| columns.dimensions = on),
            column("column-frame-rate", "Frame rate", "Frames per second of footage and compositions.", columns.frame_rate, |columns, on| columns.frame_rate = on),
        ]))
}

/// A dropdown choosing the folder one kind of import goes into.
fn import_folder_menu(id: &'static str, shown: &SharedShown, workspace: &WeakEntity<Workspace>, pick: fn(&mut ImportFolders) -> &mut Option<String>) -> impl IntoElement {
    let settings = shown.borrow().settings.clone();
    let mut copy = settings.import_folders.clone();
    let current = pick(&mut copy).clone();
    let label = current.clone().unwrap_or_else(|| "Top level".to_owned());
    let (shown, workspace) = (shown.clone(), workspace.clone());
    Button::new(id).outline().small().label(label).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
        let choices = std::iter::once(None).chain(settings.default_folders.iter().cloned().map(Some));
        choices.fold(menu, |menu, folder| {
            let (shown, workspace) = (shown.clone(), workspace.clone());
            let checked = folder == current;
            let name = folder.clone().unwrap_or_else(|| "Top level".to_owned());
            menu.item(PopupMenuItem::new(name).checked(checked).on_click(move |_, window, cx| {
                change(&shown, &workspace, window, cx, |settings| *pick(&mut settings.import_folders) = folder.clone());
            }))
        })
    })
}

fn folders(shown: &SharedShown, workspace: &WeakEntity<Workspace>, new_name: &Entity<InputState>) -> gpui_kit::Div {
    let names = shown.borrow().settings.default_folders.clone();
    let mut rows: Vec<AnyElement> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let (shown, workspace) = (shown.clone(), workspace.clone());
            div()
                .h(px(44.0))
                .px_3p5()
                .flex()
                .items_center()
                .gap_2()
                .child(Icon::new(IconName::Folder).size(px(15.0)).text_color(hsla(color::MUTED)))
                .child(div().flex_1().text_color(hsla(color::TEXT)).child(name.clone()))
                .child(Button::new(("remove-folder", index)).ghost().xsmall().icon(Icon::new(IconName::X)).tooltip("Remove").on_click(move |_, window, cx| {
                    change(&shown, &workspace, window, cx, |settings| {
                        if index < settings.default_folders.len() {
                            settings.default_folders.remove(index);
                        }
                    });
                }))
                .into_any_element()
        })
        .collect();
    let add = {
        let (shown, workspace, input) = (shown.clone(), workspace.clone(), new_name.clone());
        Button::new("add-folder").outline().small().icon(Icon::new(IconName::FolderPlus)).label("Add").on_click(move |_, window, cx| {
            let name = input.read(cx).value().trim().to_owned();
            let taken = shown.borrow().settings.default_folders.iter().any(|existing| existing.eq_ignore_ascii_case(&name));
            if name.is_empty() || taken {
                return;
            }
            change(&shown, &workspace, window, cx, |settings| settings.default_folders.push(name.clone()));
            input.update(cx, |input, cx| input.set_value("", window, cx));
        })
    };
    rows.push(div().px_3p5().py_2().flex().gap_2().child(div().flex_1().child(Input::new(new_name).small())).child(add).into_any_element());
    div()
        .child(page(Section::Folders))
        .child(group_title("New projects start with"))
        .child(card(rows))
        .child(group_title("Imported files go into"))
        .child(card(vec![
            setting("Video", "Footage, as it is imported.", import_folder_menu("import-video", shown, workspace, |folders| &mut folders.video)),
            setting("Images", "Photos and graphics.", import_folder_menu("import-image", shown, workspace, |folders| &mut folders.image)),
            setting("Audio", "Music, voice-over and sound effects.", import_folder_menu("import-audio", shown, workspace, |folders| &mut folders.audio)),
        ]))
        .child(div().pt_2().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("A folder the project doesn't have yet is made when something goes into it."))
}

/// Measures the caches again, after something changed them.
fn measure(shown: &SharedShown, workspace: &WeakEntity<Workspace>, cx: &App) {
    let Some(workspace) = workspace.upgrade() else { return };
    let workspace = workspace.read(cx);
    let (proxies, masks) = (workspace.render.proxies.cache_folder(), super::cutout::cutouts_folder(workspace));
    let mut shown = shown.borrow_mut();
    shown.proxy_bytes = studio_media::cache_size(&proxies);
    shown.mask_bytes = folder_size(&masks);
    shown.enhanced_bytes = folder_size(&super::enhance::enhanced_folder(workspace));
    shown.imported_bytes = folder_size(&workspace.imported_folder());
}

/// Moves the cache to `root` (or back to the platform's, with `None`), from now on.
fn move_cache(shown: &SharedShown, workspace: &WeakEntity<Workspace>, root: Option<PathBuf>, window: &mut Window, cx: &mut App) {
    change(shown, workspace, window, cx, |settings| settings.cache_folder = root.clone());
    let folder = root.unwrap_or_else(studio_media::default_cache_root);
    workspace
        .update(cx, |this, cx| {
            this.render.proxies.set_cache_folder(folder.join("Proxies"));
            this.request_proxies();
            cx.notify();
        })
        .ok();
    measure(shown, workspace, cx);
    shown.borrow_mut().note = Some(format!("The cache is now in {}. What was in the old folder is left there.", folder.display()));
}

fn media_cache(shown: &SharedShown, workspace: &WeakEntity<Workspace>, cx: &App) -> gpui_kit::Div {
    let now = shown.borrow();
    let Some(entity) = workspace.upgrade() else { return div() };
    let proxies = entity.read(cx).render.proxies.clone();
    let root = proxies.cache_folder().parent().map_or_else(|| proxies.cache_folder(), Path::to_path_buf);

    let choose = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("cache-choose").outline().small().label("Choose…").on_click(move |_, window, cx| {
            let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Use for Cache".into()) });
            let (shown, workspace) = (shown.clone(), workspace.clone());
            window
                .spawn(cx, async move |cx| {
                    let Ok(Ok(Some(paths))) = chosen.await else { return };
                    let Some(folder) = paths.into_iter().next() else { return };
                    cx.update(|window, cx| move_cache(&shown, &workspace, Some(folder.join("Studio")), window, cx)).ok();
                })
                .detach();
        })
    };
    let reset = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("cache-default").ghost().small().label("Use Default").disabled(now.settings.cache_folder.is_none()).on_click(move |_, window, cx| move_cache(&shown, &workspace, None, window, cx))
    };
    let reveal = {
        let root = root.clone();
        Button::new("cache-reveal").ghost().small().label("Show in Finder").on_click(move |_, _, cx| {
            let _ = std::fs::create_dir_all(&root);
            cx.reveal_path(&root);
        })
    };
    let limit_menu = {
        let (shown, proxies, workspace) = (shown.clone(), proxies.clone(), workspace.clone());
        let limit = now.settings.proxy_cache_limit_gb;
        Button::new("cache-limit").outline().small().label(limit_label(limit)).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
            CACHE_LIMITS.iter().fold(menu, |menu, &choice| {
                let (shown, proxies, workspace) = (shown.clone(), proxies.clone(), workspace.clone());
                menu.item(PopupMenuItem::new(limit_label(choice)).checked(choice == limit).on_click(move |_, window, cx| {
                    proxies.set_cache_limit(choice.map(|gb| u64::from(gb) << 30));
                    change(&shown, &workspace, window, cx, |settings| settings.proxy_cache_limit_gb = choice);
                    measure(&shown, &workspace, cx);
                }))
            })
        })
    };
    let build = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Switch::new("cache-build-proxies").checked(now.settings.build_proxies).on_click(move |checked: &bool, window, cx| {
            let checked = *checked;
            change(&shown, &workspace, window, cx, |settings| settings.build_proxies = checked);
            if checked {
                workspace.update(cx, |this, _| this.request_proxies()).ok();
            }
        })
    };
    let clear_proxies = {
        let (shown, proxies, workspace) = (shown.clone(), proxies.clone(), workspace.clone());
        Button::new("cache-clear").outline().small().label("Clear Proxies").on_click(move |_, window, cx| {
            let freed = proxies.clear_cache();
            measure(&shown, &workspace, cx);
            shown.borrow_mut().note = Some(format!("Freed {}. Proxies are made again when a project needs them.", format_bytes(freed)));
            window.refresh();
        })
    };
    let clear_masks = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("cache-clear-masks").outline().small().label("Clear Masks").on_click(move |_, window, cx| {
            let Some(entity) = workspace.upgrade() else { return };
            if entity.read(cx).ui.cutout_job.is_some() {
                shown.borrow_mut().note = Some("Background removal is running. Wait for it, or cancel it, first.".into());
                window.refresh();
                return;
            }
            let folder = super::cutout::cutouts_folder(entity.read(cx));
            let freed = folder_size(&folder);
            let _ = std::fs::remove_dir_all(&folder);
            measure(&shown, &workspace, cx);
            shown.borrow_mut().note = Some(format!("Freed {}. Cut-outs show the whole picture until their masks are made again.", format_bytes(freed)));
            window.refresh();
        })
    };
    let clear_enhanced = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("cache-clear-enhanced").outline().small().label("Clear Cleaned Sound").on_click(move |_, window, cx| {
            let Some(entity) = workspace.upgrade() else { return };
            if entity.read(cx).ui.enhance_job.is_some() {
                shown.borrow_mut().note = Some("Enhance Speech is running. Wait for it, or cancel it, first.".into());
                window.refresh();
                return;
            }
            let folder = super::enhance::enhanced_folder(entity.read(cx));
            let freed = folder_size(&folder);
            let _ = std::fs::remove_dir_all(&folder);
            measure(&shown, &workspace, cx);
            shown.borrow_mut().note = Some(format!("Freed {}. Enhanced clips are cleaned again the next time they are open.", format_bytes(freed)));
            window.refresh();
        })
    };
    let clear_imported = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("cache-clear-imported").outline().small().label("Clear Imported Images").on_click(move |_, window, cx| {
            let Some(entity) = workspace.upgrade() else { return };
            if entity.read(cx).ui.import_job.is_some() {
                shown.borrow_mut().note = Some("An import is running. Wait for it, or cancel it, first.".into());
                window.refresh();
                return;
            }
            let folder = entity.read(cx).imported_folder();
            let freed = folder_size(&folder);
            let _ = std::fs::remove_dir_all(&folder);
            measure(&shown, &workspace, cx);
            shown.borrow_mut().note = Some(format!("Freed {}. Images from imported animations show as missing until the animation is imported again.", format_bytes(freed)));
            window.refresh();
        })
    };
    let (proxy_bytes, mask_bytes, enhanced_bytes, imported_bytes) = (now.proxy_bytes, now.mask_bytes, now.enhanced_bytes, now.imported_bytes);
    drop(now);

    div()
        .child(page(Section::MediaCache))
        .child(group_title("Location"))
        .child(card(vec![setting("Cache folder", root.display().to_string(), div().flex().gap_1().child(choose).child(reset).child(reveal))]))
        .child(group_title("Proxies"))
        .child(card(vec![
            setting("Make proxies", "Small ProRes copies of imported footage, so scrubbing and playback stay smooth whatever the original codec.", build),
            setting("Size limit", "When proxies take more than this, the oldest are removed (never ones in use).", limit_menu),
            setting(format!("{} on disk", format_bytes(proxy_bytes)), "Clearing is safe: footage plays from the originals until its proxy is made again.", clear_proxies),
        ]))
        .child(group_title("Background removal"))
        .child(card(vec![setting(format!("{} of masks", format_bytes(mask_bytes)), "One picture per frame for each cut-out. Cleared cut-outs show the whole picture until made again.", clear_masks)]))
        .child(group_title("Enhance Speech"))
        .child(card(vec![setting(format!("{} of cleaned sound", format_bytes(enhanced_bytes)), "A cleaned copy of each enhanced clip's sound. Cleared clips play their original sound until cleaned again, which happens by itself.", clear_enhanced)]))
        .child(group_title("Imported animations"))
        .child(card(vec![setting(format!("{} of images", format_bytes(imported_bytes)), "Pictures that were inside imported Lottie animations. Projects use them, so clear only when you no longer need those animations.", clear_imported)]))
        .children(note(shown))
}

/// One downloadable model: what it is for, its size, and downloading or removing it.
/// Downloading runs in the background; the page shows it until it finishes.
fn model_row(id: &'static str, name: &'static str, purpose: &'static str, folder: PathBuf, present: bool, fetch: fn() -> Result<(), String>, shown: &SharedShown) -> AnyElement {
    let downloading = shown.borrow().downloading.contains(&id);
    let bytes = folder_size(&folder);
    let remove = {
        let (shown, folder) = (shown.clone(), folder.clone());
        Button::new(id).outline().small().label("Remove").disabled(!present || downloading).on_click(move |_, window, _| {
            let _ = std::fs::remove_dir_all(&folder);
            shown.borrow_mut().note = Some(format!("Removed {name}. Download it again here, or it is downloaded after asking the next time it is needed."));
            window.refresh();
        })
    };
    let download = {
        let shown = shown.clone();
        Button::new(gpui_kit::ElementId::Name(format!("{id}-download").into())).primary().small().label(if downloading { "Downloading…" } else { "Download" }).disabled(downloading).on_click(move |_, window, cx| {
            shown.borrow_mut().downloading.push(id);
            window.refresh();
            // The download blocks, so it runs on its own thread; the page checks on it.
            let result = std::sync::Arc::new(std::sync::Mutex::new(None));
            let thread_result = result.clone();
            std::thread::spawn(move || {
                let outcome = fetch();
                *thread_result.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(outcome);
            });
            let shown = shown.clone();
            cx.spawn(async move |cx| {
                let outcome = loop {
                    cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
                    if let Some(outcome) = result.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
                        break outcome;
                    }
                };
                {
                    let mut shown = shown.borrow_mut();
                    shown.downloading.retain(|&downloading| downloading != id);
                    shown.note = Some(match outcome {
                        Ok(()) => format!("Downloaded {name}."),
                        Err(message) => format!("{name} could not be downloaded: {message}"),
                    });
                }
                cx.update(|cx| cx.refresh_windows());
            })
            .detach();
        })
    };
    let size = div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(if present { format_bytes(bytes) } else { "Not downloaded".to_owned() });
    let action = if present { remove.into_any_element() } else { download.into_any_element() };
    setting(name, purpose, div().flex().items_center().gap_3().child(size).child(action))
}

/// A model that ships inside the app: nothing to download or remove.
fn built_in_row(name: &'static str, purpose: &'static str) -> AnyElement {
    setting(name, purpose, div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Built in"))
}

fn models(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let folder = crate::app_settings::folder().join("Models");
    let now = shown.borrow().settings.clone();
    let ai = now.ai;
    let feature = |id: &'static str, name: &'static str, purpose: &'static str, on: bool, set: fn(&mut AppSettings, bool)| setting(name, purpose, switch(id, on, shown, workspace, set));
    div()
        .child(page(Section::Models))
        .child(group_title("AI features"))
        .child(card(vec![
            feature("ai-all", "Show AI features", "Turn off to hide everything below at once. Hidden features keep what they already made (captions, cut-outs, cleaned sound).", ai.any(), |settings, on| settings.ai = crate::app_settings::AiFeatures::all(on)),
            feature("ai-captions", "Captions", "Generate Captions, and the Captions panel on the rail.", ai.captions, |settings, on| settings.ai.captions = on),
            feature("ai-cutout", "Object Matte", "Painting an object out of footage and images, like Roto Brush.", ai.remove_background, |settings, on| settings.ai.remove_background = on),
            feature("ai-reframe", "Auto Reframe", "New shapes of a clip that follow its subject.", ai.auto_reframe, |settings, on| settings.ai.auto_reframe = on),
            feature("ai-enhance", "Enhance Speech", "Cleaning up voice recordings.", ai.enhance_speech, |settings, on| settings.ai.enhance_speech = on),
        ]))
        .child(group_title("Background removal"))
        .child(card(vec![setting(
            "Run cooler",
            "Rests the graphics chip between frames, so long clips don't heat the Mac or spin up its fans. Tracking takes about twice as long.",
            switch("cutout-cooler", now.cooler_background_removal, shown, workspace, |settings, on| settings.cooler_background_removal = on),
        )]))
        .child(group_title("Models"))
        .child(card(vec![
            model_row("model-whisper", "Whisper (tiny)", "Speech to text, for captions. About 150 MB, from huggingface.co.", super::captions::model_folder(), studio_captions::model_present(&super::captions::model_folder()), super::captions::fetch_model, shown),
            model_row("model-sam", "MobileSAM", "Finding objects, for Object Matte and Auto Reframe. About 40 MB, from huggingface.co; checked before use.", super::cutout::model_folder(), studio_cutout::model_present(&super::cutout::model_folder()), || super::cutout::fetch_model(&super::cutout::model_folder(), &|| false), shown),
            built_in_row("DeepFilterNet3", "Studio noise removal, for Enhance Speech."),
            built_in_row("RNNoise", "Light noise removal, for Enhance Speech."),
        ]))
        .child(
            div()
                .pt_2()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Models run on this Mac. Nothing you make is uploaded."))
                .child(Button::new("models-reveal").ghost().small().label("Show in Finder").on_click(move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&folder);
                    cx.reveal_path(&folder);
                })),
        )
        .children(note(shown))
}

/// The Keyboard Shortcuts page: a starting set (Studio's, After Effects' or Premiere Pro's), and
/// every command with its keys. Change waits for the next key pressed (Esc cancels); a key
/// another command had moves to this one, and the page says so.
fn shortcuts(shown: &SharedShown, workspace: &WeakEntity<Workspace>, capture: &gpui_kit::FocusHandle, search: &gpui_kit::Entity<InputState>, cx: &App) -> gpui_kit::Div {
    use crate::keymap::{self, COMMANDS, KeyPreset};
    let now = shown.borrow().settings.shortcuts.clone();
    let recording = shown.borrow().recording;
    let keys = keymap::keys_in_use(&now);
    let presets = KeyPreset::ALL.iter().map(|&preset| (preset.name().to_owned(), preset)).collect();
    let reset = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("shortcuts-reset").outline().small().label("Undo My Changes").disabled(now.custom.is_empty()).on_click(move |_, window, cx| {
            change(&shown, &workspace, window, cx, |settings| settings.shortcuts.custom.clear());
        })
    };
    let mut page_div = div().child(page(Section::Shortcuts)).child(card(vec![
        setting(
            "Start from",
            "Studio's keys, or the ones you know from After Effects or Premiere Pro. Choosing one sets aside your own changes.",
            menu("shortcuts-preset", presets, now.preset, shown, workspace, |settings, preset| {
                settings.shortcuts.preset = preset;
                settings.shortcuts.custom.clear();
            }),
        ),
        setting(format!("{} changed", now.custom.len()), "Commands you gave keys of your own.", reset),
    ]));
    // The search narrows the list by command name or key.
    let wanted = search.read(cx).value().trim().to_lowercase();
    let matches = |command: &keymap::Command| {
        wanted.is_empty() || command.name.to_lowercase().contains(&wanted) || keys.get(command.id).is_some_and(|keys| keys.iter().any(|key| keymap::display(key).to_lowercase() == wanted || key == &wanted))
    };
    page_div = page_div.child(div().pt_3().child(Input::new(search).small().prefix(Icon::new(IconName::Search).size(px(14.0)))));
    let mut groups: Vec<&str> = Vec::new();
    for command in COMMANDS.iter().filter(|command| matches(command)) {
        if !groups.contains(&command.group) {
            groups.push(command.group);
        }
    }
    for group in groups {
        let rows = COMMANDS
            .iter()
            .filter(|command| command.group == group && matches(command))
            .map(|command| {
                let id = command.id;
                let own = keys.get(id).cloned().unwrap_or_default();
                let waiting = recording == Some(id);
                let chips = own.iter().map(|key| div().px_2().py_0p5().rounded(px(5.0)).border_1().border_color(hsla(color::HAIRLINE)).bg(hsla(color::INSET)).text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).font_family("Menlo").child(keymap::display(key)));
                let change_button = {
                    let (shown, capture) = (shown.clone(), capture.clone());
                    Button::new(gpui_kit::ElementId::Name(format!("shortcut-change:{id}").into()))
                        .small()
                        .map(|button| if waiting { button.primary().label("Press a key…") } else { button.ghost().label("Change") })
                        .on_click(move |_, window, cx| {
                            let mut now = shown.borrow_mut();
                            now.recording = if now.recording == Some(id) { None } else { Some(id) };
                            now.note = None;
                            drop(now);
                            window.focus(&capture, cx);
                            window.refresh();
                        })
                };
                let clear = {
                    let (shown, workspace) = (shown.clone(), workspace.clone());
                    Button::new(gpui_kit::ElementId::Name(format!("shortcut-clear:{id}").into())).ghost().small().icon(Icon::new(IconName::X)).tooltip("No shortcut").disabled(own.is_empty()).on_click(move |_, window, cx| {
                        change(&shown, &workspace, window, cx, move |settings| {
                            keymap::assign(&mut settings.shortcuts, id, None);
                        });
                    })
                };
                div()
                    .h(px(38.0))
                    .px_3p5()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(div().flex_1().text_color(hsla(color::TEXT)).child(command.name))
                    .children(chips)
                    .when(own.is_empty(), |row| row.child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("None")))
                    .child(change_button)
                    .child(clear)
                    .into_any_element()
            })
            .collect();
        page_div = page_div.child(group_title(group)).child(card(rows));
    }
    page_div.children(note(shown))
}

/// The next key pressed while a command waits for one (Change): it becomes the command's
/// shortcut. Esc cancels. Keys pressed now don't run their own commands.
fn capture_key(shown: &SharedShown, workspace: &WeakEntity<Workspace>, event: &gpui_kit::KeyDownEvent, window: &mut Window, cx: &mut App) {
    let Some(id) = shown.borrow().recording else { return };
    cx.stop_propagation();
    let key = event.keystroke.unparse(gpui_kit::OperatingSystem::Mac);
    shown.borrow_mut().recording = None;
    if key == "escape" {
        window.refresh();
        return;
    }
    let taken = crate::keymap::used_by(&shown.borrow().settings.shortcuts, &key, id);
    let key_for_change = key.clone();
    change(shown, workspace, window, cx, move |settings| {
        crate::keymap::assign(&mut settings.shortcuts, id, Some(&key_for_change));
    });
    let name = crate::keymap::command(id).map_or("", |command| command.name);
    shown.borrow_mut().note = Some(match taken {
        Some(other) => format!("{} is now {name}. It was {}, which has no shortcut now.", crate::keymap::display(&key), other.name),
        None => format!("{} is now {name}.", crate::keymap::display(&key)),
    });
    window.refresh();
}

/// A document that ships with Studio (`LICENSE`, `THIRD_PARTY_NOTICES.md`): inside the app's
/// Resources folder in a packaged app, or at the top of the source tree when run from it.
fn bundled_document(name: &str) -> Option<PathBuf> {
    let packaged = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|macos| macos.join("../Resources").join(name)));
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(name);
    [packaged, Some(source)].into_iter().flatten().find(|path| path.is_file())
}

/// A button that opens a document that ships with Studio, or says it can't be found.
fn document_button(id: &'static str, label: &'static str, name: &'static str) -> Button {
    Button::new(id).outline().small().label(label).disabled(bundled_document(name).is_none()).on_click(move |_, _, cx| {
        if let Some(path) = bundled_document(name) {
            cx.open_with_system(&path);
        }
    })
}

fn about(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let folder = crate::app_settings::folder();
    let reveal = {
        let folder = folder.clone();
        Button::new("about-reveal").ghost().small().label("Show in Finder").on_click(move |_, _, cx| {
            let _ = std::fs::create_dir_all(&folder);
            cx.reveal_path(&folder);
        })
    };
    let reset = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("about-reset").outline().small().label("Reset Settings").on_click(move |_, window, cx| {
            // Everything back to how Studio comes, except what it remembers about your work.
            let kept = shown.borrow().settings.clone();
            change(&shown, &workspace, window, cx, |settings| {
                *settings = AppSettings { last_project: kept.last_project.clone(), window: kept.window, recovery_of: kept.recovery_of.clone(), ..AppSettings::default() }
            });
            let defaults = AppSettings::default();
            set_look(defaults.theme, defaults.accent, cx);
            // The proxy cache keeps its own copy of these, so it is told too. A cache moved
            // elsewhere goes back to the standard folder (what was in the other one stays there).
            workspace
                .update(cx, |this, cx| {
                    let proxies = &this.render.proxies;
                    proxies.set_cache_limit(defaults.proxy_cache_limit_gb.map(|gb| u64::from(gb) << 30));
                    if kept.cache_folder.is_some() {
                        proxies.set_cache_folder(studio_media::default_cache_folder());
                        this.request_proxies();
                    }
                    cx.notify();
                })
                .ok();
            measure(&shown, &workspace, cx);
            shown.borrow_mut().note = Some("Settings are back to how Studio comes. Your projects, libraries and caches are untouched.".into());
            window.refresh();
        })
    };
    div()
        .child(page(Section::About))
        .child(
            div()
                .p_4()
                .rounded(px(10.0))
                .border_1()
                .border_color(hsla(color::HAIRLINE))
                .bg(hsla(color::RAISED))
                .flex()
                .items_center()
                .gap_3()
                .child(div().size(px(44.0)).rounded(px(10.0)).bg(hsla(color::ACCENT)).flex().items_center().justify_center().child(Icon::new(IconName::Clapperboard).size(px(22.0)).text_color(rgb(0xffffff))))
                .child(div().flex().flex_col().gap_0p5().child(div().text_size(px(16.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Studio")).child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("Version {}", env!("CARGO_PKG_VERSION"))))),
        )
        .child(group_title("Where Studio keeps things"))
        .child(card(vec![
            setting("Settings, libraries and models", folder.display().to_string(), reveal),
            setting("Reset settings", "Puts every setting back to how Studio comes. Projects, libraries and caches are not touched.", reset),
        ]))
        // The legal notices the GPL asks an interactive program to show.
        .child(group_title("License"))
        .child(card(vec![
            setting(
                "Free software",
                "Copyright © 2026 Odd Omens LLC. Studio is free software under the GNU General Public License, version 3 or later: you may share and change it under its terms. It comes with no warranty.",
                document_button("about-license", "License", "LICENSE"),
            ),
            setting(
                "Built with",
                "FFmpeg, x264, the Lucide icons, RNNoise, DeepFilterNet and the Rust crates listed in the notices, each under its own license.",
                document_button("about-notices", "Third-Party Notices", "THIRD_PARTY_NOTICES.md"),
            ),
        ]))
        .children(note(shown))
}

/// The menu of pages down the left, in groups.
fn section_menu(shown: &SharedShown) -> gpui_kit::Div {
    let current = shown.borrow().section;
    let mut menu = div().w(px(196.0)).flex_none().flex().flex_col().pr_3().border_r_1().border_color(hsla(color::HAIRLINE));
    let mut index: usize = 0;
    for (group, sections) in Section::GROUPS {
        menu = menu.child(div().pt_2().pb_1().px_2().text_size(px(10.5)).text_color(hsla(color::MUTED)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(group.to_uppercase()));
        for &section in sections {
            let shown = shown.clone();
            let selected = section == current;
            menu = menu.child(
                div()
                    .id(("settings-section", index))
                    .h(px(30.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_size(px(size::TEXT + 0.5))
                    .text_color(hsla(if selected { color::TEXT } else { color::MUTED }))
                    .when(selected, |item| item.bg(hsla(color::ROW_SELECTED)))
                    .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, _| {
                        let mut now = shown.borrow_mut();
                        now.section = section;
                        now.note = None;
                        drop(now);
                        window.refresh();
                    })
                    .child(Icon::new(section.icon()).size(px(14.0)).text_color(hsla(if selected { color::ACCENT } else { color::MUTED })))
                    .child(section.name())
                    .test_support(),
            );
            index += 1;
        }
    }
    menu
}

pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    open_at(Section::General, workspace, window, cx);
}

/// Studio ▸ About Studio: Settings, on its About page.
pub fn open_about(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    open_at(Section::About, workspace, window, cx);
}

fn open_at(section: Section, workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let shown: SharedShown = Rc::new(RefCell::new(Shown {
        section,
        settings: app_settings::load(),
        // Measured from the workspace directly: it is being updated, so it can't be read back.
        proxy_bytes: studio_media::cache_size(&workspace.render.proxies.cache_folder()),
        mask_bytes: folder_size(&super::cutout::cutouts_folder(workspace)),
        enhanced_bytes: folder_size(&super::enhance::enhanced_folder(workspace)),
        imported_bytes: folder_size(&workspace.imported_folder()),
        downloading: Vec::new(),
        recording: None,
        note: None,
    }));
    // Holds the keyboard while a shortcut is being changed, so the key pressed is captured.
    let capture = cx.focus_handle();
    let shortcut_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search commands or keys"));
    let redraw = cx.subscribe_in(&shortcut_search, window, |_, _, _: &gpui_kit::component::input::InputEvent, window, _| window.refresh());
    workspace.ui.dialog_subscriptions.push(redraw);
    let target = cx.weak_entity();
    let new_folder_name = cx.new(|cx| InputState::new(window, cx).placeholder("New folder name"));

    window.open_dialog(cx, move |dialog, _, cx| {
        let section = shown.borrow().section;
        let content = match section {
            Section::General => general(&shown, &target),
            Section::Appearance => appearance(&shown, &target),
            Section::Playback => playback(&shown, &target),
            Section::Timeline => timeline(&shown, &target),
            Section::Features => features(&shown, &target),
            Section::ProjectPanel => project_panel(&shown, &target),
            Section::Folders => folders(&shown, &target, &new_folder_name),
            Section::MediaCache => media_cache(&shown, &target, cx),
            Section::Models => models(&shown, &target),
            Section::Shortcuts => shortcuts(&shown, &target, &capture, &shortcut_search, cx),
            Section::About => about(&shown, &target),
        };
        dialog
            .title("Settings")
            .w(px(880.0))
            .centered()
            .child(
                div()
                    .h(px(540.0))
                    .flex()
                    .gap_5()
                    .text_size(px(size::TEXT))
                    .child(section_menu(&shown))
                    .child({
                        let (shown, target) = (shown.clone(), target.clone());
                        div()
                            .id("settings-content")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .pr_2()
                            .pb_2()
                            .track_focus(&capture)
                            .capture_key_down(move |event, window, cx| capture_key(&shown, &target, event, window, cx))
                            .child(content)
                    }),
            )
            // A compact Done at the bottom right (the dialog's own close button stretches).
            .footer(div().w_full().flex().justify_end().child(Button::new("settings-done").primary().small().px_4().label("Done").on_click(|_, window, cx| window.close_dialog(cx))))
    });
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_license_and_notices_ship_with_studio() {
        for name in ["LICENSE", "THIRD_PARTY_NOTICES.md"] {
            let found = super::bundled_document(name).unwrap_or_else(|| panic!("{name} is missing"));
            assert!(std::fs::metadata(&found).unwrap().len() > 1000, "{name} has its text");
        }
        assert!(super::bundled_document("NOT_A_FILE").is_none());
    }

    use super::*;

    #[test]
    fn sizes_and_intervals_read_naturally() {
        assert_eq!(format_bytes(820_000_000), "820 MB");
        assert_eq!(format_bytes(1_400_000_000), "1.4 GB");
        assert_eq!(limit_label(None), "No limit");
        assert_eq!(recovery_label(30), "Every 30 seconds");
        assert_eq!(recovery_label(300), "Every 5 minutes");
        assert_eq!(recovery_label(0), "Never");
    }

    #[test]
    fn the_menu_lists_every_page() {
        let listed: Vec<Section> = Section::GROUPS.iter().flat_map(|(_, sections)| sections.iter().copied()).collect();
        assert_eq!(listed.len(), 11);
    }
}
