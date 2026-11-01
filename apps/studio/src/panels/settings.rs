//! The Settings dialog (Cmd+, or the gear at the bottom of the rail): a menu of sections down
//! the left, and the chosen section's settings on the right. Every change applies at once and
//! is remembered.
//!
//! - **Appearance:** the theme and accent color.
//! - **Project Panel:** which columns it shows.
//! - **Folders:** the folders new projects start with, and which folder imports go into.
//! - **Media Cache:** where Studio keeps what it can rebuild (proxies of footage, background-
//!   removal masks), how big the proxies may grow, whether footage gets proxies at all, and
//!   clearing each. In memory, playback keeps only a few decoded frames around the playhead for
//!   each clip, which needs no setting.
//! - **AI Models:** the models downloaded for captions and background removal, which can be
//!   removed to free space (they are downloaded again, after asking, when next needed).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::assets::IconName;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled,
    TestSupportExt as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px, rgb,
};

use crate::app_settings::{self, AppSettings, ImportFolders, ProjectColumns};
use crate::theme::{Accent, ThemeName, color, hsla, size};
use crate::workspace::Workspace;

/// Disk limits offered for proxies, in gigabytes (`None` for no limit).
const CACHE_LIMITS: &[Option<u32>] = &[Some(5), Some(10), Some(25), Some(50), Some(100), None];

fn limit_label(limit: Option<u32>) -> String {
    limit.map_or_else(|| "No limit".to_owned(), |gb| format!("{gb} GB"))
}

/// A byte count for people: "820 MB", "1.4 GB".
pub fn format_bytes(bytes: u64) -> String {
    let megabytes = bytes as f64 / 1_000_000.0;
    if megabytes < 1000.0 { format!("{megabytes:.0} MB") } else { format!("{:.1} GB", megabytes / 1000.0) }
}

/// The dialog's sections, in the order of its menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Appearance,
    ProjectPanel,
    Folders,
    MediaCache,
    Models,
}

impl Section {
    const ALL: [Self; 5] = [Self::Appearance, Self::ProjectPanel, Self::Folders, Self::MediaCache, Self::Models];

    fn name(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::ProjectPanel => "Project Panel",
            Self::Folders => "Folders",
            Self::MediaCache => "Media Cache",
            Self::Models => "AI Models",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Appearance => IconName::Palette,
            Self::ProjectPanel => IconName::Menu,
            Self::Folders => IconName::Folder,
            Self::MediaCache => IconName::Hourglass,
            Self::Models => IconName::Sparkles,
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

/// Saves a settings change, shows it, and redraws the window.
fn change(shown: &SharedShown, window: &mut Window, edit: impl Fn(&mut AppSettings)) {
    app_settings::update(&edit);
    edit(&mut shown.borrow_mut().settings);
    window.refresh();
}

fn heading(text: &'static str) -> gpui_kit::Div {
    div().pt_1().text_color(hsla(color::TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(text)
}

fn explain(text: impl Into<gpui_kit::SharedString>) -> gpui_kit::Div {
    div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(text.into())
}

fn row(label: &'static str, control: impl IntoElement) -> gpui_kit::Div {
    div().min_h(px(34.0)).flex().items_center().gap_3().child(div().w(px(130.0)).flex_none().text_color(hsla(color::MUTED)).child(label)).child(div().flex_1().min_w_0().child(control))
}

fn rule() -> gpui_kit::Div {
    div().h(px(1.0)).my_2().bg(hsla(color::HAIRLINE))
}

/// Applies a theme and accent everywhere.
fn set_look(workspace: &WeakEntity<Workspace>, theme: ThemeName, accent: Accent, cx: &mut App) {
    crate::theme::apply(theme, accent, cx);
    // Every panel redraws when the workspace does.
    workspace.update(cx, |_, cx| cx.notify()).ok();
    cx.refresh_windows();
}

fn appearance(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let now = shown.borrow();
    let themes = ThemeName::ALL.into_iter().map(|theme| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        let selected = now.settings.theme == theme;
        // A little preview of the theme's panel and text colors.
        let (panel, text) = match theme {
            ThemeName::Dark => (0x111112, 0xededef),
            ThemeName::Black => (0x000000, 0xf2f2f2),
            ThemeName::Light => (0xf6f6f7, 0x18181b),
        };
        div()
            .id(("theme", theme as usize))
            .flex_1()
            .flex()
            .flex_col()
            .gap_1p5()
            .p_1p5()
            .rounded(px(8.0))
            .border_2()
            .border_color(hsla(if selected { color::ACCENT } else { color::HAIRLINE }))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                let accent = shown.borrow().settings.accent;
                change(&shown, window, |settings| settings.theme = theme);
                set_look(&workspace, theme, accent, cx);
            })
            .child(div().h(px(36.0)).rounded(px(5.0)).bg(rgb(panel)).border_1().border_color(hsla(color::HAIRLINE)).p_1p5().child(div().text_color(rgb(text)).text_size(px(size::SMALL_TEXT)).child("Aa")))
            .child(div().text_size(px(size::SMALL_TEXT)).child(theme.name()))
            .test_support()
    });
    let accents = Accent::ALL.into_iter().map(|accent| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        let selected = now.settings.accent == accent;
        div()
            .id(("accent", accent as usize))
            .size(px(26.0))
            .rounded_full()
            .border_2()
            .border_color(hsla(if selected { color::TEXT } else { color::PANEL }))
            .p(px(2.0))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                let theme = shown.borrow().settings.theme;
                change(&shown, window, |settings| settings.accent = accent);
                set_look(&workspace, theme, accent, cx);
            })
            .child(div().size_full().rounded_full().bg(rgb(accent.value())))
            .test_support()
    });
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(heading("Appearance"))
        .child(row("Theme", div().flex().gap_2().children(themes)))
        .child(row("Accent color", div().flex().flex_wrap().items_center().gap_1p5().children(accents).child(div().pl_1().text_color(hsla(color::MUTED)).child(now.settings.accent.name()))))
}

fn project_panel(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let columns = shown.borrow().settings.project_columns;
    let toggle = |key: &'static str, label: &'static str, on: bool, set: fn(&mut ProjectColumns, bool)| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Checkbox::new(key).label(label).checked(on).on_click(move |checked: &bool, window, cx| {
            let checked = *checked;
            change(&shown, window, |settings| set(&mut settings.project_columns, checked));
            let columns = shown.borrow().settings.project_columns;
            workspace
                .update(cx, |this, cx| {
                    this.ui.project_columns = columns;
                    cx.notify();
                })
                .ok();
        })
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(heading("Project Panel"))
        .child(explain("Choose the columns the Project panel shows. The name is always shown."))
        .child(div().flex().flex_col().gap_2().pt_1()
            .child(Checkbox::new("column-name").label("Name").checked(true).disabled(true))
            .child(toggle("column-label", "Label color", columns.label, |columns, on| columns.label = on))
            .child(toggle("column-type", "Type", columns.kind, |columns, on| columns.kind = on))
            .child(toggle("column-duration", "Duration", columns.duration, |columns, on| columns.duration = on))
            .child(toggle("column-dimensions", "Dimensions", columns.dimensions, |columns, on| columns.dimensions = on))
            .child(toggle("column-frame-rate", "Frame rate", columns.frame_rate, |columns, on| columns.frame_rate = on)))
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
                change(&shown, window, |settings| *pick(&mut settings.import_folders) = folder.clone());
                let folders = shown.borrow().settings.import_folders.clone();
                workspace.update(cx, |this, _| this.state.import_folders = folders).ok();
            }))
        })
    })
}

fn folders(shown: &SharedShown, workspace: &WeakEntity<Workspace>, new_name: &Entity<InputState>) -> gpui_kit::Div {
    let names = shown.borrow().settings.default_folders.clone();
    let list = names.iter().enumerate().map(|(index, name)| {
        let shown = shown.clone();
        div()
            .h(px(30.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .bg(hsla(color::INSET))
            .child(Icon::new(IconName::Folder).size(px(14.0)).text_color(hsla(color::MUTED)))
            .child(div().flex_1().child(name.clone()))
            .child(Button::new(("remove-folder", index)).ghost().xsmall().icon(Icon::new(IconName::X)).tooltip("Remove").on_click(move |_, window, _| {
                change(&shown, window, |settings| {
                    if index < settings.default_folders.len() {
                        settings.default_folders.remove(index);
                    }
                });
            }))
    });
    let add = {
        let (shown, input) = (shown.clone(), new_name.clone());
        Button::new("add-folder").outline().small().icon(Icon::new(IconName::FolderPlus)).label("Add").on_click(move |_, window, cx| {
            let name = input.read(cx).value().trim().to_owned();
            let taken = shown.borrow().settings.default_folders.iter().any(|existing| existing.eq_ignore_ascii_case(&name));
            if name.is_empty() || taken {
                return;
            }
            change(&shown, window, |settings| settings.default_folders.push(name.clone()));
            input.update(cx, |input, cx| input.set_value("", window, cx));
        })
    };
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(heading("Folders"))
        .child(explain("New projects start with these folders in the Project panel (File > New Project, ⌥⌘N)."))
        .child(div().flex().flex_col().gap_1().children(list))
        .child(div().flex().gap_2().child(div().flex_1().child(Input::new(new_name).small())).child(add))
        .child(rule())
        .child(heading("Sorting imports"))
        .child(explain("Imported files go into the folder for their kind. A folder that isn't in the project yet is made."))
        .child(row("Video", import_folder_menu("import-video", shown, workspace, |folders| &mut folders.video)))
        .child(row("Images", import_folder_menu("import-image", shown, workspace, |folders| &mut folders.image)))
        .child(row("Audio", import_folder_menu("import-audio", shown, workspace, |folders| &mut folders.audio)))
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
    change(shown, window, |settings| settings.cache_folder = root.clone());
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
                    change(&shown, window, |settings| settings.proxy_cache_limit_gb = choice);
                    measure(&shown, &workspace, cx);
                }))
            })
        })
    };
    let build = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Checkbox::new("cache-build-proxies").label("Make proxies of imported footage").checked(now.settings.build_proxies).on_click(move |checked: &bool, window, cx| {
            let checked = *checked;
            change(&shown, window, |settings| settings.build_proxies = checked);
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

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(heading("Media Cache"))
        .child(explain("Files Studio makes and can always make again: small ProRes proxies of footage, so scrubbing and playback stay smooth, and background-removal masks."))
        .child(row("Cache folder", div().flex().flex_col().gap_1().child(div().text_size(px(size::SMALL_TEXT)).truncate().child(root.display().to_string())).child(div().flex().gap_1().child(choose).child(reset).child(reveal))))
        .child(rule())
        .child(heading("Proxies"))
        .child(row("On disk", div().child(format_bytes(now.proxy_bytes))))
        .child(row("Size limit", div().flex().items_center().gap_2().child(limit_menu).child(explain("oldest go first"))))
        .child(row("", build))
        .child(row("", clear_proxies))
        .child(rule())
        .child(heading("Background-removal masks"))
        .child(row("On disk", div().child(format_bytes(now.mask_bytes))))
        .child(row("", clear_masks))
        .when_some(now.note.clone(), |column, note| column.child(div().pt_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).child(note)))
}

/// One downloaded model: what it is for, its size, and removing it.
fn model_row(id: &'static str, name: &'static str, purpose: &'static str, folder: PathBuf, shown: &SharedShown) -> gpui_kit::Div {
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
    div()
        .py_1()
        .flex()
        .items_center()
        .gap_3()
        .child(div().flex_1().flex().flex_col().child(name).child(explain(purpose)))
        .child(div().text_color(hsla(color::MUTED)).child(if present { format_bytes(bytes) } else { "Not downloaded".to_owned() }))
        .child(remove)
}

fn models(shown: &SharedShown) -> gpui_kit::Div {
    let note = shown.borrow().note.clone();
    let folder = crate::app_settings::folder().join("Models");
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(heading("AI Models"))
        .child(explain("Captions and background removal run on this computer with these models. Each is downloaded once, after asking, and kept in Studio's settings folder."))
        .child(model_row("model-whisper", "Whisper (tiny)", "Speech to text, for Generate Captions", super::captions::model_folder(), shown))
        .child(model_row("model-sam", "MobileSAM", "Picking out subjects, for Remove Background", super::cutout::model_folder(), shown))
        .child(div().pt_1().child(Button::new("models-reveal").ghost().small().label("Show in Finder").on_click(move |_, _, cx| {
            let _ = std::fs::create_dir_all(&folder);
            cx.reveal_path(&folder);
        })))
        .when_some(note, |column, note| column.child(div().pt_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).child(note)))
}

/// The menu of sections down the left.
fn section_menu(shown: &SharedShown) -> gpui_kit::Div {
    let current = shown.borrow().section;
    div().w(px(170.0)).flex_none().flex().flex_col().gap_0p5().pr_3().border_r_1().border_color(hsla(color::HAIRLINE)).children(Section::ALL.into_iter().enumerate().map(|(index, section)| {
        let shown = shown.clone();
        div()
            .id(("settings-section", index))
            .h(px(32.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_color(hsla(if section == current { color::TEXT } else { color::MUTED }))
            .when(section == current, |item| item.bg(hsla(color::ROW_SELECTED)))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, _| {
                let mut now = shown.borrow_mut();
                now.section = section;
                now.note = None;
                drop(now);
                window.refresh();
            })
            .child(Icon::new(section.icon()).size(px(15.0)))
            .child(section.name())
            .test_support()
    }))
}

pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let shown: SharedShown = Rc::new(RefCell::new(Shown { section: Section::Appearance, settings: app_settings::load(), proxy_bytes: 0, mask_bytes: 0, note: None }));
    let target = cx.weak_entity();
    {
        // Measured from the workspace directly: it is being updated, so it can't be read back.
        let mut now = shown.borrow_mut();
        now.proxy_bytes = studio_media::cache_size(&workspace.render.proxies.cache_folder());
        now.mask_bytes = folder_size(&super::cutout::cutouts_folder(workspace));
    }
    let new_folder_name = cx.new(|cx| InputState::new(window, cx).placeholder("New folder name"));

    window.open_dialog(cx, move |dialog, _, cx| {
        let section = shown.borrow().section;
        let content = match section {
            Section::Appearance => appearance(&shown, &target),
            Section::ProjectPanel => project_panel(&shown, &target),
            Section::Folders => folders(&shown, &target, &new_folder_name),
            Section::MediaCache => media_cache(&shown, &target, cx),
            Section::Models => models(&shown),
        };
        dialog
            .title("Settings")
            .w(px(780.0))
            .centered()
            .child(
                div()
                    .h(px(460.0))
                    .flex()
                    .gap_4()
                    .text_size(px(size::TEXT + 1.0))
                    .child(section_menu(&shown))
                    .child(div().id("settings-content").flex_1().min_w_0().overflow_y_scroll().pr_1().child(content)),
            )
            .footer(DialogFooter::new().child(DialogClose::new().trigger(|button| button.primary().label("Done"))))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(format_bytes(820_000_000), "820 MB");
        assert_eq!(format_bytes(1_400_000_000), "1.4 GB");
        assert_eq!(limit_label(None), "No limit");
    }
}
