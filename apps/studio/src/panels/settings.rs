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

use crate::app_settings::{self, AppSettings, ImportFolders, PlaybackQuality, ProjectColumns, Transparency};
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
        ("Studio", &[Self::General, Self::Appearance, Self::Playback, Self::Timeline]),
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
    app_settings::update(&edit);
    edit(&mut shown.borrow_mut().settings);
    let settings = shown.borrow().settings.clone();
    workspace
        .update(cx, |this, cx| {
            this.ui.project_columns = settings.project_columns;
            this.state.import_folders = settings.import_folders.clone();
            this.ui.settings = settings;
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
    let recovery = RECOVERY_CHOICES.iter().map(|&seconds| (recovery_label(seconds), seconds)).collect();
    div()
        .child(page(Section::General))
        .child(group_title("Starting up"))
        .child(card(vec![
            setting("Reopen the last project", "Studio opens the project you were working on when it starts.", switch("general-reopen", now.reopen_last_project, shown, workspace, |settings, on| settings.reopen_last_project = on)),
            setting("Recovery copy", "How often unsaved work is kept aside, so a crash loses little. Offered back on the next start.", menu("general-recovery", recovery, now.recovery_seconds, shown, workspace, |settings, seconds| settings.recovery_seconds = seconds)),
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
        let selected = now.theme == theme;
        // A small picture of the theme: its background, a panel, a line of text and an accent.
        let (background, panel, text) = match theme {
            ThemeName::Dark => (0x070708, 0x18181b, 0xededef),
            ThemeName::Black => (0x000000, 0x111111, 0xf2f2f2),
            ThemeName::Light => (0xe9e9ec, 0xffffff, 0x18181b),
        };
        let accent = now.accent.value();
        div()
            .id(("theme", theme as usize))
            .flex_1()
            .flex()
            .flex_col()
            .gap_2()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                let accent = shown.borrow().settings.accent;
                change(&shown, &workspace, window, cx, |settings| settings.theme = theme);
                set_look(theme, accent, cx);
            })
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
            .test_support()
    });
    let accents = Accent::ALL.into_iter().map(|accent| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        let selected = now.accent == accent;
        div()
            .id(("accent", accent as usize))
            .size(px(28.0))
            .rounded_full()
            .border_2()
            .border_color(hsla(if selected { color::TEXT } else { color::RAISED }))
            .p(px(3.0))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                let theme = shown.borrow().settings.theme;
                change(&shown, &workspace, window, cx, |settings| settings.accent = accent);
                set_look(theme, accent, cx);
            })
            .child(div().size_full().rounded_full().bg(rgb(accent.value())))
            .test_support()
    });
    div()
        .child(page(Section::Appearance))
        .child(group_title("Theme"))
        .child(div().flex().gap_3().children(themes))
        .child(group_title("Accent color"))
        .child(card(vec![setting(now.accent.name(), "Used for selections, buttons and highlights.", div().flex().gap_1().children(accents))]))
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
    let (proxy_bytes, mask_bytes) = (now.proxy_bytes, now.mask_bytes);
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
        .children(note(shown))
}

/// One downloaded model: what it is for, its size, and removing it.
fn model_row(id: &'static str, name: &'static str, purpose: &'static str, folder: PathBuf, shown: &SharedShown) -> AnyElement {
    let bytes = folder_size(&folder);
    let present = bytes > 0;
    let remove = {
        let (shown, folder) = (shown.clone(), folder.clone());
        Button::new(id).outline().small().label("Remove").disabled(!present).on_click(move |_, window, _| {
            let _ = std::fs::remove_dir_all(&folder);
            shown.borrow_mut().note = Some(format!("Removed {name}. It is downloaded again, after asking, the next time it is needed."));
            window.refresh();
        })
    };
    let size = div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(if present { format_bytes(bytes) } else { "Not downloaded".to_owned() });
    setting(name, purpose, div().flex().items_center().gap_3().child(size).child(remove))
}

fn models(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let folder = crate::app_settings::folder().join("Models");
    let now = shown.borrow().settings.clone();
    div()
        .child(page(Section::Models))
        .child(group_title("Background removal"))
        .child(card(vec![setting(
            "Run cooler",
            "Rests the graphics chip between frames, so long clips don't heat the Mac or spin up its fans. Tracking takes about twice as long.",
            switch("cutout-cooler", now.cooler_background_removal, shown, workspace, |settings, on| settings.cooler_background_removal = on),
        )]))
        .child(group_title("Downloaded"))
        .child(card(vec![
            model_row("model-whisper", "Whisper (tiny)", "Speech to text, for captions. About 150 MB.", super::captions::model_folder(), shown),
            model_row("model-sam", "MobileSAM", "Finding subjects, for Remove Background and Auto Reframe. About 40 MB.", super::cutout::model_folder(), shown),
        ]))
        .child(
            div()
                .pt_2()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Each is downloaded once, after asking. Nothing you make is uploaded."))
                .child(Button::new("models-reveal").ghost().small().label("Show in Finder").on_click(move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&folder);
                    cx.reveal_path(&folder);
                })),
        )
        .children(note(shown))
}

/// The keys listed on the Keyboard Shortcuts page, by group (they match `workspace::bind_keys`).
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    ("Playback", &[("Play or pause", "Space"), ("Next frame / previous frame", "Page Down / Page Up"), ("Ten frames on / back", "⇧ Page Down / ⇧ Page Up"), ("First frame / last frame", "Home / End")]),
    ("Tools", &[("Select", "V"), ("Track Select Forward", "A"), ("Text", "⌘T"), ("Shape", "Q"), ("Pen (Enter ends a path)", "G"), ("Rotate", "W"), ("Hand / Zoom", "H / Z"), ("Cut", "C"), ("Ripple / Roll / Slip / Slide", "B / N / Y / U")]),
    ("Editing", &[("Undo / redo", "⌘Z / ⇧⌘Z"), ("Copy / paste / duplicate", "⌘C / ⌘V / ⌘D"), ("Split at the playhead", "⇧⌘D"), ("Trim in / out to the playhead", "⌥[ / ⌥]"), ("Delete / ripple delete", "⌫ / ⇧⌫"), ("Pre-compose", "⇧⌘C"), ("Easy ease", "F9"), ("Snapping on or off", "S")]),
    ("Project", &[("New project / composition", "⌥⌘N / ⌘N"), ("Composition settings", "⌘K"), ("Open / save / save as", "⌘O / ⌘S / ⇧⌘S"), ("Import", "⌘I"), ("Export", "⌘M"), ("Settings", "⌘,")]),
];

fn shortcuts() -> gpui_kit::Div {
    let mut page_div = div().child(page(Section::Shortcuts));
    for (group, keys) in SHORTCUTS {
        let rows = keys
            .iter()
            .map(|(what, key)| {
                div()
                    .h(px(38.0))
                    .px_3p5()
                    .flex()
                    .items_center()
                    .child(div().flex_1().text_color(hsla(color::TEXT)).child(*what))
                    .child(div().px_2().py_0p5().rounded(px(5.0)).border_1().border_color(hsla(color::HAIRLINE)).bg(hsla(color::INSET)).text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).font_family("Menlo").child(*key))
                    .into_any_element()
            })
            .collect();
        page_div = page_div.child(div().pt_3().pb_1p5().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(group.to_uppercase())).child(card(rows));
    }
    page_div
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
    let shown: SharedShown = Rc::new(RefCell::new(Shown {
        section: Section::General,
        settings: app_settings::load(),
        // Measured from the workspace directly: it is being updated, so it can't be read back.
        proxy_bytes: studio_media::cache_size(&workspace.render.proxies.cache_folder()),
        mask_bytes: folder_size(&super::cutout::cutouts_folder(workspace)),
        note: None,
    }));
    let target = cx.weak_entity();
    let new_folder_name = cx.new(|cx| InputState::new(window, cx).placeholder("New folder name"));

    window.open_dialog(cx, move |dialog, _, cx| {
        let section = shown.borrow().section;
        let content = match section {
            Section::General => general(&shown, &target),
            Section::Appearance => appearance(&shown, &target),
            Section::Playback => playback(&shown, &target),
            Section::Timeline => timeline(&shown, &target),
            Section::ProjectPanel => project_panel(&shown, &target),
            Section::Folders => folders(&shown, &target, &new_folder_name),
            Section::MediaCache => media_cache(&shown, &target, cx),
            Section::Models => models(&shown, &target),
            Section::Shortcuts => shortcuts(),
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
                    .child(div().id("settings-content").flex_1().min_w_0().overflow_y_scroll().pr_2().pb_2().child(content)),
            )
            // A compact Done at the bottom right (the dialog's own close button stretches).
            .footer(div().w_full().flex().justify_end().child(Button::new("settings-done").primary().small().px_4().label("Done").on_click(|_, window, cx| window.close_dialog(cx))))
    });
}

#[cfg(test)]
mod tests {
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
        assert_eq!(listed.len(), 10);
    }
}
