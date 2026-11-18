//! The Stock panel, opened from the rail: free video, pictures and sound, searched online (see
//! `crate::stock`). Everything it lists is in the public domain (CC0), so it can be used for
//! anything without a licence or a credit. Clicking a result downloads it into a Stock folder
//! and imports it into the project; clicking it again while it downloads cancels.
//!
//! A result can be tried before it is added: a sound's play button plays it, and a video's
//! fetches it so that moving the pointer across its picture scrubs through it.
//!
//! Searching and downloading happen in the background, so the editor stays usable meanwhile.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _};
use gpui_kit::{
    AnyElement, AppContext as _, Context, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, Subscription, TestSupportExt as _, Window, div, img, prelude::FluentBuilder as _, px,
};

use super::chrome::{MeasuredBounds, panel};
use crate::stock::{Kind, Page, Source, StockItem};
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// Width of a video or picture tile, in points. Its picture is 16:9.
const TILE_WIDTH: f32 = 132.0;
const TILE_PICTURE_HEIGHT: f32 = 74.0;

/// Searches offered before anything has been typed, for each kind.
fn suggestions(kind: Kind) -> [&'static str; 6] {
    match kind {
        Kind::Video => ["ocean", "city", "clouds", "forest", "fire", "traffic"],
        Kind::Image => ["mountain", "texture", "sky", "flowers", "paper", "space"],
        Kind::Audio => ["rain", "whoosh", "applause", "click", "ambient", "wind"],
    }
}

/// Where a search stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    /// Nothing is being fetched.
    #[default]
    Idle,
    Searching,
    /// The last search failed, and why.
    Failed(String),
}

/// The Stock panel's state: its search field, what was searched for and found, the preview
/// pictures fetched so far, and the downloads in progress.
#[derive(Default)]
pub struct StockUi {
    /// The search field, made the first time the panel shows (it needs the window).
    input: Option<(Entity<InputState>, Subscription)>,
    /// The kind and the words of the search whose results are showing or on their way.
    pub kind: Kind,
    pub query: String,
    pub items: Vec<StockItem>,
    /// Whether any service has more results after the pages fetched so far.
    pub more: bool,
    /// A line for each service the last page could not be fetched from.
    pub problems: Vec<String>,
    /// How many pages of results have been fetched.
    pages: usize,
    pub status: Status,
    /// Counts searches, so a reply that arrives after a newer search began is dropped.
    search_number: u64,
    /// Preview pictures on disk, by the address they were fetched from.
    thumbnails: HashMap<String, PathBuf>,
    /// Addresses whose preview picture is being fetched or could not be, so each is tried once.
    thumbnails_asked: HashSet<String>,
    /// Downloads in progress, by item id, each with the flag that cancels it.
    downloads: HashMap<String, Arc<AtomicBool>>,
    /// Items already added to the project in this session, by id.
    pub added: HashSet<String>,
    /// Downloaded files waiting to be imported (an import can't start while another runs).
    waiting: Vec<PathBuf>,
    /// Files fetched to preview, by item id, each with its length in seconds.
    pub previews: HashMap<String, (PathBuf, f64)>,
    /// Items whose preview file is being fetched.
    previews_loading: HashSet<String>,
    /// The sound being played as a preview: its item id, and a number that tells this play
    /// from later ones of the same sound.
    pub playing: Option<(String, u64)>,
    /// Counts previews played, for `playing`.
    plays: u64,
    /// The video being scrubbed and how far across its tile the pointer is, from 0 to 1.
    pub scrub: Option<(String, f32)>,
    /// Where each previewed video's picture is on screen, by item id, to tell how far across
    /// it the pointer is.
    tile_bounds: HashMap<String, MeasuredBounds>,
    /// The starred and recently added items, read from disk the first time the panel shows.
    shelf: Option<crate::stock::Shelf>,
}

impl StockUi {
    /// The starred and recently added items (empty until the panel has shown once).
    pub fn shelf(&self) -> Option<&crate::stock::Shelf> {
        self.shelf.as_ref()
    }
}

/// Stars `item`, or takes its star away, and saves the change.
pub fn toggle_favorite(workspace: &mut Workspace, item: &StockItem) {
    let shelf = workspace.ui.stock.shelf.get_or_insert_with(crate::stock::Shelf::load);
    shelf.toggle_favorite(item);
    shelf.save();
}

/// The kind the panel's selected tab stands for.
fn tab_kind(workspace: &Workspace) -> Kind {
    Kind::ALL.get(workspace.ui.tab(PanelId::Stock)).copied().unwrap_or_default()
}

/// Keeps the panel up to date while it shows: makes its search field the first time, and when
/// another tab has been picked, searches that kind for the same words. Called from the
/// workspace's render.
pub fn keep_up(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.stock.input.is_none() {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search free stock"));
        let events = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let query = input.read(cx).value().to_string();
                search(this, query, window, cx);
            }
        });
        workspace.ui.stock.input = Some((input, events));
    }
    if workspace.ui.stock.shelf.is_none() {
        workspace.ui.stock.shelf = Some(crate::stock::Shelf::load());
    }
    let kind = tab_kind(workspace);
    if workspace.ui.stock.kind != kind {
        let query = workspace.ui.stock.query.clone();
        workspace.ui.stock.kind = kind;
        search(workspace, query, window, cx);
    }
}

/// Starts a new search of the selected tab's kind for `query`, replacing what was found
/// before. Empty words clear the results.
pub fn search(workspace: &mut Workspace, query: String, window: &mut Window, cx: &mut Context<Workspace>) {
    let stock = &mut workspace.ui.stock;
    stock.query = query.trim().to_owned();
    stock.items.clear();
    stock.problems.clear();
    stock.more = false;
    stock.pages = 0;
    stock.search_number += 1;
    stock.status = Status::Idle;
    if !stock.query.is_empty() {
        fetch_page(workspace, window, cx);
    }
    cx.notify();
}

/// Fetches the next page of the current search in the background and adds it to the results.
fn fetch_page(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let stock = &mut workspace.ui.stock;
    if stock.status == Status::Searching {
        return;
    }
    stock.status = Status::Searching;
    let (kind, query, page, number) = (stock.kind, stock.query.clone(), stock.pages, stock.search_number);
    // The services switched on in Settings ▸ Stock that have this kind (and their key).
    let access = workspace.ui.settings.stock.searchable(kind);
    if access.is_empty() {
        workspace.ui.stock.status = Status::Idle;
        cx.notify();
        return;
    }
    cx.notify();
    cx.spawn_in(window, async move |this, cx| {
        let found = cx.background_executor().spawn(async move { crate::stock::search(kind, &query, page, &access) }).await;
        this.update_in(cx, |this, window, cx| show_page(this, number, found, window, cx)).ok();
    })
    .detach();
}

/// Adds a fetched page to the results, unless a newer search has begun since it was asked for.
pub(crate) fn show_page(workspace: &mut Workspace, number: u64, found: Page, window: &mut Window, cx: &mut Context<Workspace>) {
    let stock = &mut workspace.ui.stock;
    if stock.search_number != number {
        return;
    }
    // A page where every service failed is a failed search; one where some answered shows
    // what they found, with a line about the others.
    if found.items.is_empty() && !found.problems.is_empty() {
        stock.status = Status::Failed(found.problems.join(" "));
        cx.notify();
        return;
    }
    // A later page can repeat an item when a service's results shift between requests.
    let known: HashSet<String> = stock.items.iter().map(|item| item.id.clone()).collect();
    stock.items.extend(found.items.into_iter().filter(|item| !known.contains(&item.id)));
    stock.more = found.more;
    stock.problems = found.problems;
    stock.pages += 1;
    stock.status = Status::Idle;
    fetch_thumbnails(workspace, window, cx);
    cx.notify();
}

/// Fetches the preview picture of every result that has one and hasn't been asked for yet,
/// each in the background, showing each as it arrives.
fn fetch_thumbnails(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let stock = &mut workspace.ui.stock;
    let wanted: Vec<String> = stock.items.iter().filter_map(|item| item.thumbnail_url.clone()).filter(|url| !stock.thumbnails_asked.contains(url)).collect();
    for url in wanted {
        stock.thumbnails_asked.insert(url.clone());
        cx.spawn_in(window, async move |this, cx| {
            let address = url.clone();
            // A picture that can't be fetched just leaves its tile plain.
            let Ok(path) = cx.background_executor().spawn(async move { crate::stock::fetch_thumbnail(&address) }).await else { return };
            this.update(cx, |this, cx| {
                this.ui.stock.thumbnails.insert(url, path);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Downloads `item` in the background and imports it into the project, or, when it is
/// already downloading, cancels that.
pub fn add(workspace: &mut Workspace, item: StockItem, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some(cancel) = workspace.ui.stock.downloads.get(&item.id) {
        cancel.store(true, Ordering::Relaxed);
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    workspace.ui.stock.downloads.insert(item.id.clone(), cancel.clone());
    let folder = crate::stock::folder(workspace.state.file());
    cx.notify();
    cx.spawn_in(window, async move |this, cx| {
        let wanted = item.clone();
        let downloaded = cx.background_executor().spawn(async move { crate::stock::download(&wanted, &folder, &|| cancel.load(Ordering::Relaxed)) }).await;
        this.update_in(cx, |this, window, cx| {
            this.ui.stock.downloads.remove(&item.id);
            match downloaded {
                Ok(path) => {
                    this.ui.stock.added.insert(item.id.clone());
                    // Remembered among the recently added, on disk.
                    let shelf = this.ui.stock.shelf.get_or_insert_with(crate::stock::Shelf::load);
                    shelf.note_added(&item);
                    shelf.save();
                    this.ui.stock.waiting.push(path);
                    import_downloaded(this, window, cx);
                }
                // A download the person cancelled needs no notice.
                Err(error) if error == "cancelled" => {}
                Err(error) => window.push_notification(error.as_str(), cx),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Tries out `item` before adding it. A sound plays (or stops, when it is the one playing); a
/// video is fetched so that moving the pointer across its tile scrubs it. The file is fetched
/// in the background the first time and kept, so adding the item afterwards is instant.
pub fn preview(workspace: &mut Workspace, item: StockItem, window: &mut Window, cx: &mut Context<Workspace>) {
    let is_sound = item.kind == Kind::Audio;
    if is_sound && workspace.ui.stock.playing.as_ref().is_some_and(|(id, _)| *id == item.id) {
        stop_playing(workspace);
        cx.notify();
        return;
    }
    if let Some((path, seconds)) = workspace.ui.stock.previews.get(&item.id).cloned() {
        if is_sound {
            play(workspace, &item.id, &path, seconds, window, cx);
        }
        return;
    }
    if !workspace.ui.stock.previews_loading.insert(item.id.clone()) {
        return; // Already on its way.
    }
    cx.notify();
    cx.spawn_in(window, async move |this, cx| {
        let wanted = item.clone();
        let fetched = cx
            .background_executor()
            .spawn(async move {
                let path = crate::stock::fetch_preview(&wanted, &|| false)?;
                // Its length, read from the file: the services don't always say.
                let seconds = if wanted.kind == Kind::Audio {
                    studio_media::probe_audio(&path).map(|info| info.duration_seconds)
                } else {
                    studio_media::probe(&path).map(|info| info.duration_seconds)
                };
                seconds.map(|seconds| (path, seconds)).map_err(|error| format!("“{}” can't be previewed ({error}).", wanted.title))
            })
            .await;
        this.update_in(cx, |this, window, cx| {
            this.ui.stock.previews_loading.remove(&item.id);
            match fetched {
                Ok((path, seconds)) => {
                    this.ui.stock.previews.insert(item.id.clone(), (path.clone(), seconds));
                    this.ui.stock.tile_bounds.entry(item.id.clone()).or_default();
                    if is_sound {
                        play(this, &item.id, &path, seconds, window, cx);
                    }
                }
                Err(error) => window.push_notification(error.as_str(), cx),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Plays a fetched sound from its start, and marks it as no longer playing once its length
/// has passed.
fn play(workspace: &mut Workspace, id: &str, path: &std::path::Path, seconds: f64, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.play_sound_file(path, seconds);
    workspace.ui.stock.plays += 1;
    let playing = (id.to_owned(), workspace.ui.stock.plays);
    workspace.ui.stock.playing = Some(playing.clone());
    cx.notify();
    cx.spawn_in(window, async move |this, cx| {
        cx.background_executor().timer(std::time::Duration::from_secs_f64(seconds.max(0.0))).await;
        this.update(cx, |this, cx| {
            // Only if nothing else was played or stopped meanwhile.
            if this.ui.stock.playing.as_ref() == Some(&playing) {
                stop_playing(this);
                cx.notify();
            }
        })
        .ok();
    })
    .detach();
}

/// Stops the sound being previewed, if one is.
fn stop_playing(workspace: &mut Workspace) {
    if workspace.ui.stock.playing.take().is_some() {
        workspace.stop_sound_file();
    }
}

/// The small round button that previews a result: a play triangle, a pause sign while its
/// sound plays, or a spinner while its file is on its way. A click on it doesn't add the item.
fn preview_button(workspace: &Workspace, index: usize, item: &StockItem, cx: &mut Context<Workspace>) -> AnyElement {
    let stock = &workspace.ui.stock;
    let loading = stock.previews_loading.contains(&item.id);
    let playing = stock.playing.as_ref().is_some_and(|(id, _)| *id == item.id);
    let tooltip = match (item.kind, playing) {
        (Kind::Audio, true) => "Stop",
        (Kind::Audio, false) => "Play this sound",
        _ => "Preview: fetch this video, then move the pointer across it to scrub through it",
    };
    let clicked = item.clone();
    div()
        .id(ElementId::NamedInteger("stock-preview".into(), index as u64))
        .size(px(20.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(gpui_kit::black().opacity(0.65))
        .hover(|style| style.bg(hsla(color::ACCENT)))
        .cursor_pointer()
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip).build(window, cx))
        .on_click(cx.listener(move |this, _, window, cx| {
            // The button handles the click, so the result behind it isn't added as well.
            cx.stop_propagation();
            preview(this, clicked.clone(), window, cx);
        }))
        .child(if loading {
            Spinner::new().xsmall().into_any_element()
        } else {
            Icon::new(if playing { IconName::Pause } else { IconName::Play }).size(px(10.0)).text_color(gpui_kit::white()).into_any_element()
        })
        .test_support()
        .into_any_element()
}

/// The small star on a result that keeps it among the favourites (lit when it is one). A click
/// on it doesn't add the item.
fn star_button(workspace: &Workspace, index: usize, item: &StockItem, cx: &mut Context<Workspace>) -> AnyElement {
    let starred = workspace.ui.stock.shelf.as_ref().is_some_and(|shelf| shelf.is_favorite(&item.id));
    let clicked = item.clone();
    div()
        .id(ElementId::NamedInteger("stock-star".into(), index as u64))
        .size(px(20.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(gpui_kit::black().opacity(0.65))
        .hover(|style| style.bg(gpui_kit::black().opacity(0.9)))
        .cursor_pointer()
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(if starred { "Remove from Favourites" } else { "Add to Favourites" }).build(window, cx))
        .on_click(cx.listener(move |this, _, _, cx| {
            // The star handles the click, so the result behind it isn't added as well.
            cx.stop_propagation();
            toggle_favorite(this, &clicked);
            cx.notify();
        }))
        .child(Icon::new(IconName::Star).size(px(11.0)).text_color(if starred { hsla(color::ACCENT) } else { gpui_kit::white().opacity(0.8) }))
        .test_support()
        .into_any_element()
}

/// Imports the downloaded files that are waiting, once no other import is running. Called
/// when a download finishes and from the workspace's render, so files that had to wait go in
/// as soon as the import before them ends.
pub fn import_downloaded(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.stock.waiting.is_empty() || workspace.ui.import_job.is_some() {
        return;
    }
    let paths = std::mem::take(&mut workspace.ui.stock.waiting);
    workspace.import_paths(paths, None, window, cx);
}

/// A result's state for drawing: being downloaded, already added, or neither.
fn badge(workspace: &Workspace, item: &StockItem) -> Option<AnyElement> {
    let stock = &workspace.ui.stock;
    if stock.downloads.contains_key(&item.id) {
        return Some(div().flex().items_center().gap_1().child(Spinner::new().xsmall()).child("Cancel").into_any_element());
    }
    stock.added.contains(&item.id).then(|| div().flex().items_center().gap_1().child(Icon::new(IconName::Check).size(px(11.0))).child("Added").into_any_element())
}

/// What a result's tooltip says: its full title, who made it, where it is from, and what a
/// click does.
fn tooltip_text(workspace: &Workspace, item: &StockItem) -> SharedString {
    let by = item.creator.as_ref().map_or_else(String::new, |creator| format!(" by {creator}"));
    let action = if workspace.ui.stock.downloads.contains_key(&item.id) { "Click to cancel the download." } else { "Click to download it and add it to the project." };
    format!("{}{by}, from {}. {action}", item.title, item.origin).into()
}

/// The line of terms under the search field, for the services being searched: public domain
/// when they all are, and the sites' own licences named when Pexels or Pixabay are among them.
fn terms_note(sources: &[Source]) -> String {
    let names = |sources: &[Source]| sources.iter().map(|source| source.name()).collect::<Vec<_>>().join(" and ");
    let (free, licensed): (Vec<Source>, Vec<Source>) = sources.iter().copied().partition(|source| source.public_domain_only());
    match (free.is_empty(), licensed.is_empty()) {
        (false, true) => format!("Public domain (CC0) from {}: free for any use, no credit needed.", names(&free)),
        (true, false) => format!("From {}: free for commercial use with no credit needed, under the site's own licence.", names(&licensed)),
        (false, false) => format!("{} results are public domain (CC0). {} results are free for commercial use with no credit needed, under the site's own licence.", names(&free), names(&licensed)),
        (true, true) => String::new(),
    }
}

/// A video or picture result: its preview picture with the title and facts under it.
fn tile(workspace: &Workspace, index: usize, item: &StockItem, cx: &mut Context<Workspace>) -> AnyElement {
    let stock = &workspace.ui.stock;
    let picture = item.thumbnail_url.as_ref().and_then(|url| stock.thumbnails.get(url)).cloned();
    let tooltip = tooltip_text(workspace, item);
    let clicked = item.clone();
    // A video that has been fetched to preview: its file, its length, and where its picture
    // is on screen. Moving the pointer across the picture scrubs through it.
    let previewed = (item.kind == Kind::Video).then(|| stock.previews.get(&item.id).cloned()).flatten();
    let bounds = stock.tile_bounds.get(&item.id).cloned().unwrap_or_default();
    let scrubbed = stock.scrub.as_ref().filter(|(id, _)| *id == item.id).map(|(_, fraction)| *fraction);
    // The frame under the pointer, once it has been decoded; the still picture until then.
    let frame = previewed.as_ref().zip(scrubbed).and_then(|((path, seconds), fraction)| workspace.ui.timeline.media.thumbnails.thumbnail(path, f64::from(fraction) * seconds)).map(|frame| workspace.ui.timeline.media.image(&frame));
    let (scrub_id, leave_id, probe) = (item.id.clone(), item.id.clone(), bounds.clone());
    div()
        .id(ElementId::NamedInteger("stock-item".into(), index as u64))
        .w(px(TILE_WIDTH))
        .flex()
        .flex_col()
        .gap_0p5()
        .cursor_pointer()
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(cx.listener(move |this, _, window, cx| add(this, clicked.clone(), window, cx)))
        // Leaving the tile goes back to its still picture.
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            if !hovered && this.ui.stock.scrub.as_ref().is_some_and(|(id, _)| *id == leave_id) {
                this.ui.stock.scrub = None;
                cx.notify();
            }
        }))
        .child(
            div()
                .relative()
                .w_full()
                .h(px(TILE_PICTURE_HEIGHT))
                .rounded(px(size::POPUP_RADIUS))
                .overflow_hidden()
                .bg(hsla(color::INSET))
                .border_1()
                .border_color(hsla(color::HAIRLINE))
                .hover(|style| style.border_color(hsla(color::ACCENT)))
                .when(previewed.is_some(), |picture| {
                    picture.child(bounds.probe()).on_mouse_move(cx.listener(move |this, event: &gpui_kit::MouseMoveEvent, _, cx| {
                        let rect = probe.get();
                        if rect.width <= 0.0 {
                            return;
                        }
                        let fraction = ((f32::from(event.position.x) - rect.left) / rect.width).clamp(0.0, 1.0);
                        let scrub = Some((scrub_id.clone(), fraction));
                        if this.ui.stock.scrub != scrub {
                            this.ui.stock.scrub = scrub;
                            cx.notify();
                        }
                    }))
                })
                .map(|tile_picture| match frame {
                    Some(frame) => tile_picture.child(gpui_kit::StyledImage::object_fit(img(gpui_kit::ImageSource::Render(frame)).size_full(), gpui_kit::ObjectFit::Cover)),
                    None => tile_picture.children(picture.map(|path| gpui_kit::StyledImage::object_fit(img(path).size_full(), gpui_kit::ObjectFit::Cover))),
                })
                // How far through the video the pointer is, as a line along the bottom.
                .children(scrubbed.filter(|_| previewed.is_some()).map(|fraction| div().absolute().left_0().bottom_0().h(px(2.0)).w(gpui_kit::relative(fraction)).bg(hsla(color::ACCENT))))
                // A video not fetched yet offers to be, in the picture's top right corner.
                // In the picture's top right corner: the star, and for a video not fetched yet
                // the button that fetches it to preview.
                .child(
                    div()
                        .absolute()
                        .right(px(3.0))
                        .top(px(3.0))
                        .flex()
                        .gap_1()
                        .children((item.kind == Kind::Video && previewed.is_none()).then(|| preview_button(workspace, index, item, cx)))
                        .child(star_button(workspace, index, item, cx)),
                )
                // Which site it came from, in the picture's top corner.
                .child(div().absolute().left(px(3.0)).top(px(3.0)).max_w(px(TILE_WIDTH - 8.0)).px_1().rounded(px(3.0)).bg(gpui_kit::black().opacity(0.65)).text_size(px(size::CAPTION_TEXT)).text_color(gpui_kit::white()).truncate().child(item.origin.clone()))
                // Downloading or added, said over the picture's bottom edge.
                .children(badge(workspace, item).map(|badge| {
                    div().absolute().left_0().right_0().bottom_0().px_1().py_0p5().bg(gpui_kit::black().opacity(0.65)).text_size(px(size::CAPTION_TEXT)).text_color(gpui_kit::white()).child(badge)
                })),
        )
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).truncate().child(item.title.clone()))
        .child(div().text_size(px(size::CAPTION_TEXT)).text_color(hsla(color::MUTED)).truncate().child(item.summary()))
        .test_support()
        .into_any_element()
}

/// A sound result: one line with its title, its facts, and whether it is downloading or added.
fn sound_row(workspace: &Workspace, index: usize, item: &StockItem, cx: &mut Context<Workspace>) -> AnyElement {
    let tooltip = tooltip_text(workspace, item);
    let clicked = item.clone();
    div()
        .id(ElementId::NamedInteger("stock-item".into(), index as u64))
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(px(size::POPUP_RADIUS))
        .cursor_pointer()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx))
        .on_click(cx.listener(move |this, _, window, cx| add(this, clicked.clone(), window, cx)))
        .child(preview_button(workspace, index, item, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().truncate().text_color(hsla(color::TEXT)).child(item.title.clone()))
                // Which site it came from, then its facts.
                .child(div().truncate().text_size(px(size::CAPTION_TEXT)).text_color(hsla(color::MUTED)).child(format!("{} · {}", item.origin, item.summary()))),
        )
        .children(badge(workspace, item).map(|badge| div().flex_none().text_size(px(size::CAPTION_TEXT)).text_color(hsla(color::MUTED)).child(badge)))
        .child(star_button(workspace, index, item, cx))
        .test_support()
        .into_any_element()
}

/// Where the favourites' and the recents' numbering starts in their elements' ids, well past
/// any search result's, so the three lists never share an id.
const SHELF_FAVORITES: usize = 1_000_000;
const SHELF_RECENTS: usize = 2_000_000;

/// A titled list of kept items (favourites, or recently added), drawn like search results.
/// `None` when there are none of this kind. `first` is where its ids start.
fn shelf_section(title: &'static str, first: usize, items: &[StockItem], kind: Kind, workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if items.is_empty() {
        return None;
    }
    let numbered = items.iter().enumerate().map(|(index, item)| (first + index, item));
    let list = match kind {
        Kind::Audio => div().flex().flex_col().children(numbered.map(|(index, item)| sound_row(workspace, index, item, cx))),
        Kind::Video | Kind::Image => div().flex().flex_wrap().gap_2().children(numbered.map(|(index, item)| tile(workspace, index, item, cx))),
    };
    Some(div().flex().flex_col().gap_1p5().child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(title)).child(list).into_any_element())
}

/// The words offered to search for before anything has been typed. Clicking one searches it.
fn suggestion_chips(kind: Kind, cx: &mut Context<Workspace>) -> AnyElement {
    let chips = suggestions(kind).into_iter().map(|word| {
        div()
            .id(ElementId::Name(format!("stock-suggestion:{word}").into()))
            .px_2()
            .py_1()
            .rounded(px(size::POPUP_RADIUS))
            .bg(hsla(color::INSET))
            .text_color(hsla(color::MUTED))
            .cursor_pointer()
            .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some((input, _)) = &this.ui.stock.input {
                    input.update(cx, |input, cx| input.set_value(word, window, cx));
                }
                search(this, word.to_owned(), window, cx);
            }))
            .child(word)
            .test_support()
    });
    div().flex().flex_col().gap_2().child(div().text_color(hsla(color::MUTED)).child("Try one of these:")).child(div().flex().flex_wrap().gap_1p5().children(chips)).into_any_element()
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let stock = &workspace.ui.stock;
    let kind = tab_kind(workspace);
    let search_row = stock.input.as_ref().map(|(input, _)| {
        let field = input.clone();
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl_2()
                    .rounded(px(size::POPUP_RADIUS))
                    .bg(hsla(color::INSET))
                    .child(Icon::new(IconName::Search).size(px(13.0)).text_color(hsla(color::MUTED)))
                    .child(div().flex_1().min_w_0().child(Input::new(input).appearance(false).small())),
            )
            .child(Button::new("stock-search").outline().small().label("Search").on_click(cx.listener(move |this, _, window, cx| {
                let query = field.read(cx).value().to_string();
                search(this, query, window, cx);
            })))
    });
    let sources: Vec<Source> = workspace.ui.settings.stock.searchable(kind).into_iter().map(|(source, _)| source).collect();
    let settings_button = || Button::new("stock-open-settings").outline().small().label("Stock Settings…").on_click(cx.listener(|this, _, window, cx| super::settings::open_stock(this, window, cx)));
    // With every service for this kind switched off, there is nothing to search: say so.
    if sources.is_empty() {
        let body = div()
            .id("stock-list")
            .size_full()
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_color(hsla(color::MUTED)).child(format!("No stock source is turned on for {}. Turn one on in Settings.", kind.name().to_lowercase())))
            .child(div().child(settings_button()));
        let tabs = Kind::ALL.map(Kind::name);
        return panel(PanelId::Stock, &tabs, None, body, workspace, cx);
    }
    let note = terms_note(&sources);

    let results: AnyElement = if stock.query.is_empty() {
        // Before a search: words to try, then the starred and the recently added of this kind.
        let shelf = stock.shelf();
        let of_kind = |items: &[StockItem]| -> Vec<StockItem> { items.iter().filter(|item| item.kind == kind).cloned().collect() };
        let favorites = shelf.map(|shelf| of_kind(&shelf.favorites)).unwrap_or_default();
        let recents = shelf.map(|shelf| of_kind(&shelf.recents)).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(suggestion_chips(kind, cx))
            .children(shelf_section("Favourites", SHELF_FAVORITES, &favorites, kind, workspace, cx))
            .children(shelf_section("Recently Added", SHELF_RECENTS, &recents, kind, workspace, cx))
            .into_any_element()
    } else if stock.items.is_empty() {
        match &stock.status {
            Status::Searching => div().flex().items_center().gap_2().text_color(hsla(color::MUTED)).child(Spinner::new().small()).child("Searching…").into_any_element(),
            Status::Failed(error) => div().text_color(hsla(color::MUTED)).child(error.clone()).into_any_element(),
            Status::Idle => div().text_color(hsla(color::MUTED)).child(format!("Nothing free was found for “{}”. Try another word.", stock.query)).into_any_element(),
        }
    } else {
        let found = stock.items.iter().enumerate();
        let list = match kind {
            Kind::Audio => div().flex().flex_col().children(found.map(|(index, item)| sound_row(workspace, index, item, cx))),
            Kind::Video | Kind::Image => div().flex().flex_wrap().gap_2().children(found.map(|(index, item)| tile(workspace, index, item, cx))),
        };
        // Under the results: more of them, or why the next page didn't come.
        let after = match &stock.status {
            Status::Searching => Some(div().flex().items_center().gap_2().text_color(hsla(color::MUTED)).child(Spinner::new().small()).child("Loading more…").into_any_element()),
            Status::Failed(error) => Some(div().text_color(hsla(color::MUTED)).child(error.clone()).into_any_element()),
            // Some services answered and others didn't: say which didn't.
            Status::Idle => (!stock.problems.is_empty()).then(|| div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(stock.problems.join(" ")).into_any_element()),
        };
        let load_more = (stock.more && stock.status != Status::Searching).then(|| Button::new("stock-more").outline().small().label("Load More").on_click(cx.listener(|this, _, window, cx| fetch_page(this, window, cx))));
        div().flex().flex_col().gap_2().child(list).children(after).children(load_more.map(|button| div().flex().justify_center().py_1().child(button))).into_any_element()
    };

    let body = div()
        .id("stock-list")
        .size_full()
        .overflow_y_scroll()
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .children(search_row)
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(note))
        .child(results);
    let tabs = Kind::ALL.map(Kind::name);
    panel(PanelId::Stock, &tabs, None, body, workspace, cx)
}
