//! The quick menu (⌘Return): a box in the middle of the window with a row of one-click
//! actions (save the frame, export, import and so on) and a search field. Typing part of a
//! name finds an effect or one of the user's presets, which is put on the selected clips, or
//! any of Studio's commands, which is run. Up and Down move through the results (the list
//! scrolls to follow), Return picks the highlighted one, Escape (or a click outside) closes it.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AnyElement, AppContext as _, Context, ElementId, Entity, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    ScrollHandle, StatefulInteractiveElement, Styled, Subscription, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};
use studio_core::model::LinkId;

use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// The open menu: its text field, the highlighted result, where the result list is scrolled
/// to, and the field's event listener.
pub struct EffectSearch {
    input: Entity<InputState>,
    highlighted: usize,
    scroll: ScrollHandle,
    _events: Subscription,
}

/// The tallest the result list grows before it scrolls.
const LIST_HEIGHT: f32 = 380.0;

/// What picking a result does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Apply {
    Effect(&'static str),
    Preset(LinkId),
    /// Runs a command, by its id in `keymap::COMMANDS`.
    Command(&'static str),
}

/// The one-click actions along the top of the menu: an icon, a short label, and the id of the
/// command each runs (see `keymap::COMMANDS`).
const QUICK_ACTIONS: [(IconName, &str, &str); 11] = [
    (IconName::Camera, "Save Frame", "save-frame"),
    (IconName::Images, "Copy Frame", "copy-frame"),
    (IconName::SquarePlay, "Render & Play", "render-and-play"),
    (IconName::Flag, "Marker", "toggle-marker"),
    (IconName::Mic, "Voiceover", "record-voiceover"),
    (IconName::Download, "Export", "export"),
    (IconName::LayoutList, "Export Queue", "export-queue"),
    (IconName::FileInput, "Import", "import"),
    (IconName::Clapperboard, "New Comp", "new-composition"),
    (IconName::Save, "Save", "save"),
    (IconName::Settings, "Settings", "open-settings"),
];

/// Commands the menu leaves out: itself, and closing a popup (which Escape already does here).
const HIDDEN_COMMANDS: [&str; 2] = ["search-effects", "dismiss"];

/// Whether a command can be run from the menu: its feature is turned on in Settings.
fn command_on(workspace: &Workspace, id: &str) -> bool {
    crate::features::Feature::of_command(id).is_none_or(|feature| workspace.has(feature))
}

/// One search result: what it is called, a note under it, and what it applies.
#[derive(Clone, Debug)]
struct Found {
    name: String,
    detail: String,
    apply: Apply,
}

/// Opens the search with an empty field, ready to type.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search effects, presets and commands"));
    input.update(cx, |state, cx| state.focus(window, cx));
    let events = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
        InputEvent::PressEnter { .. } => apply_highlighted(this, window, cx),
        // A new search starts at its best result.
        InputEvent::Change => {
            if let Some(search) = this.ui.effect_search.as_mut() {
                search.highlighted = 0;
                search.scroll.scroll_to_item(0);
            }
            cx.notify();
        }
        _ => {}
    });
    workspace.ui.effect_search = Some(EffectSearch { input, highlighted: 0, scroll: ScrollHandle::new(), _events: events });
    cx.notify();
}

/// Closes the search (Escape, a click outside, or once something is applied).
pub fn close(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.ui.effect_search = None;
    window.focus(&workspace.focus, cx);
    cx.notify();
}

/// Whether `found` matches every word of the search, and how well: 0 when the name starts with
/// the search, 1 when a word of it does, 2 otherwise. `None` when it doesn't match.
fn rank(name: &str, detail: &str, query: &str) -> Option<u8> {
    let (name, detail, query) = (name.to_lowercase(), detail.to_lowercase(), query.trim().to_lowercase());
    let haystack = format!("{name} {detail}");
    if !query.split_whitespace().all(|word| haystack.contains(word)) {
        return None;
    }
    Some(if name.starts_with(&query) {
        0
    } else if name.split([' ', '-', '/', '&']).any(|word| word.starts_with(&query)) {
        1
    } else {
        2
    })
}

/// The effects and presets matching `query` that suit every selected clip (or all of them,
/// with none selected), best matches first, then in catalog order. Presets come first among
/// equals: they are the user's own. Once something is typed, Studio's commands are searched
/// too; an empty search lists only effects and presets, which is what the menu is opened for
/// most.
fn results(workspace: &Workspace, query: &str) -> Vec<Found> {
    let composition = workspace.state.composition();
    let selected: Vec<_> = workspace.state.selected_layers.iter().filter_map(|id| composition.layer(*id)).collect();
    let mut found: Vec<(u8, Found)> = Vec::new();
    for preset in &workspace.state.libraries.presets {
        let detail = format!("My Preset · {}", preset.summary());
        if let Some(score) = rank(&preset.name, &detail, query) {
            found.push((score, Found { name: preset.name.clone(), detail, apply: Apply::Preset(preset.id) }));
        }
    }
    for category in studio_core::effects::CATEGORIES {
        if !selected.iter().all(|layer| studio_core::effects::suits(category, &layer.content)) {
            continue;
        }
        for definition in category.effects.iter().filter(|definition| super::effect_controls::is_usable(definition.name)) {
            if let Some(score) = rank(definition.name, category.name, query) {
                found.push((score, Found { name: definition.name.to_owned(), detail: category.name.to_owned(), apply: Apply::Effect(definition.name) }));
            }
        }
    }
    if !query.trim().is_empty() {
        let shortcuts = &workspace.ui.settings.shortcuts;
        for command in crate::keymap::COMMANDS.iter().filter(|command| !HIDDEN_COMMANDS.contains(&command.id) && command_on(workspace, command.id)) {
            // The note names the command's group and, when it has one, its shortcut.
            let detail = match crate::keymap::key_for(shortcuts, command.id) {
                Some(key) => format!("{} · {key}", command.group),
                None => command.group.to_owned(),
            };
            if let Some(score) = rank(command.name, command.group, query) {
                found.push((score, Found { name: command.name.to_owned(), detail, apply: Apply::Command(command.id) }));
            }
        }
    }
    // A stable sort keeps presets, then catalog order, then commands, within each rank.
    found.sort_by_key(|(score, _)| *score);
    found.into_iter().map(|(_, found)| found).collect()
}

/// The current search's results.
fn current_results(workspace: &Workspace, cx: &gpui_kit::App) -> Vec<Found> {
    let query = workspace.ui.effect_search.as_ref().map(|search| search.input.read(cx).value().to_string()).unwrap_or_default();
    results(workspace, &query)
}

/// Picks the highlighted result and closes the menu.
fn apply_highlighted(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let results = current_results(workspace, cx);
    let highlighted = workspace.ui.effect_search.as_ref().map_or(0, |search| search.highlighted);
    if let Some(found) = results.get(highlighted).cloned() {
        apply(workspace, &found, window, cx);
    }
}

/// Closes the menu and runs the command with this id, as its shortcut would.
fn run_command(workspace: &mut Workspace, id: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    close(workspace, window, cx);
    if let Some(command) = crate::keymap::command(id).filter(|command| command_on(workspace, command.id)) {
        // The editor has the keyboard again, so the command reaches it like a key press would,
        // once this click or key press has been dealt with.
        window.dispatch_action(command.action(), cx);
    }
}

/// Picks `found` and closes the menu: a command is run; an effect or preset is put on every
/// selected clip and shown where it is edited.
fn apply(workspace: &mut Workspace, found: &Found, window: &mut Window, cx: &mut Context<Workspace>) {
    use gpui_kit::component::WindowExt as _;
    if let Apply::Command(id) = found.apply {
        return run_command(workspace, id, window, cx);
    }
    let layers = workspace.state.selected_layers.clone();
    if layers.is_empty() {
        window.push_notification("Select a clip first, then search for an effect to put on it.", cx);
        return;
    }
    let tab = match found.apply {
        Apply::Effect(name) => {
            for &layer in &layers {
                workspace.state.add_effect(layer, name);
            }
            super::color_grade::tab_for_effect(name)
        }
        Apply::Preset(id) => {
            workspace.state.apply_effect_preset(id, &layers);
            1
        }
        Apply::Command(_) => unreachable!("commands are run above"),
    };
    workspace.ui.tabs.insert(PanelId::Properties, tab);
    close(workspace, window, cx);
}

/// Moves the highlight up or down through the results, scrolling the list to keep it in view.
fn step(workspace: &mut Workspace, down: bool, cx: &mut Context<Workspace>) {
    let count = current_results(workspace, cx).len();
    if let Some(search) = workspace.ui.effect_search.as_mut().filter(|_| count > 0) {
        search.highlighted = if down { (search.highlighted + 1).min(count - 1) } else { search.highlighted.saturating_sub(1) };
        search.scroll.scroll_to_item(search.highlighted);
        cx.notify();
    }
}

/// One of the buttons along the top of the menu: an icon over a short label. Its tooltip names
/// the command in full, with its shortcut.
fn quick_action(workspace: &Workspace, (icon, label, id): (IconName, &'static str, &'static str), cx: &mut Context<Workspace>) -> AnyElement {
    let name = crate::keymap::command(id).map_or(label, |command| command.name);
    let tooltip: gpui_kit::SharedString = crate::keymap::with_key(&workspace.ui.settings.shortcuts, name, id).into();
    div()
        .id(ElementId::Name(format!("quick-action:{id}").into()))
        .flex_1()
        .min_w_0()
        .py_2()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .rounded(px(size::POPUP_RADIUS))
        .bg(hsla(color::INSET))
        .cursor_pointer()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx))
        .child(Icon::new(icon).size(px(16.0)))
        .child(div().truncate().child(label))
        .on_click(cx.listener(move |this, _, window, cx| run_command(this, id, window, cx)))
        .test_support()
        .into_any_element()
}

/// The menu over a dimmed window, while it is open.
pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let search = workspace.ui.effect_search.as_ref()?;
    let results = current_results(workspace, cx);
    let highlighted = search.highlighted.min(results.len().saturating_sub(1));
    let target = match workspace.state.selected_layers.len() {
        0 => "Select a clip to put an effect on it, or type to find a command".to_owned(),
        1 => {
            let composition = workspace.state.composition();
            let name = workspace.state.selected_layers.first().and_then(|id| composition.layer(*id)).map_or_else(String::new, |layer| layer.name.clone());
            format!("For {name}")
        }
        count => format!("For {count} clips"),
    };
    let rows: Vec<AnyElement> = results
        .iter()
        .enumerate()
        .map(|(index, found)| {
            let found = found.clone();
            let on = index == highlighted;
            let icon = match found.apply {
                Apply::Preset(_) => IconName::Bookmark,
                Apply::Effect(name) => super::effect_controls::category_icon(name),
                Apply::Command(_) => IconName::Keyboard,
            };
            div()
                .id(ElementId::NamedInteger("effect-search-result".into(), index as u64))
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .flex_none()
                .rounded(px(size::POPUP_RADIUS))
                .cursor_pointer()
                .when(on, |row| row.bg(hsla(color::ROW_SELECTED)))
                .hover(|style| style.bg(hsla(color::HOVER)))
                .child(Icon::new(icon).size(px(14.0)).text_color(hsla(color::MUTED)))
                .child(div().flex_1().min_w_0().truncate().text_color(hsla(color::TEXT)).child(found.name.clone()))
                .child(div().flex_none().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(found.detail.clone()))
                .on_click(cx.listener(move |this, _, window, cx| apply(this, &found, window, cx)))
                .test_support()
                .into_any_element()
        })
        .collect();
    let empty = rows.is_empty();
    let actions: Vec<AnyElement> = QUICK_ACTIONS.into_iter().filter(|(_, _, id)| command_on(workspace, id)).map(|action| quick_action(workspace, action, cx)).collect();
    // The results scroll inside the box once there are more than fit.
    let list = div()
        .id("effect-search-results")
        .max_h(px(LIST_HEIGHT))
        .overflow_y_scroll()
        .track_scroll(&search.scroll)
        .flex()
        .flex_col()
        .children(rows)
        .when(empty, |list| list.child(div().px_3().py_2().text_color(hsla(color::MUTED)).child("Nothing matches. Try another word.")))
        .test_support();
    let card = super::chrome::popup_surface(div().id("effect-search"))
        .w(px(520.0))
        .p_2()
        .flex()
        .flex_col()
        .gap_1()
        .text_size(px(size::TEXT))
        // Clicks inside the box stay inside it.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // The field has the keyboard, so the box handles its own keys: Escape closes it (the
        // editor's Escape is off while typing), Up and Down move the highlight.
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| match event.keystroke.key.as_str() {
            "escape" => close(this, window, cx),
            "down" => step(this, true, cx),
            "up" => step(this, false, cx),
            _ => {}
        }))
        .child(div().flex().items_center().gap_2().px_1().child(Icon::new(IconName::Search).size(px(14.0)).text_color(hsla(color::MUTED))).child(div().flex_1().child(Input::new(&search.input).appearance(false).large())))
        .child(div().px_3().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(target))
        // Six to a row, so the buttons stay wide enough for their names.
        .child(div().grid().grid_cols(6).gap_1p5().px_1().pb_1().children(actions))
        .child(div().h(px(1.0)).bg(hsla(color::HAIRLINE)))
        .child(list)
        .test_support();
    Some(
        super::chrome::popup_backdrop("effect-search-backdrop", card)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| close(this, window, cx)))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::rank;

    #[test]
    fn names_that_start_with_the_search_rank_first_and_every_word_must_match() {
        assert_eq!(rank("Corner Pin", "Distort", "corner"), Some(0));
        assert_eq!(rank("Corner Pin", "Distort", "pin"), Some(1));
        assert_eq!(rank("Lens Flare", "Generate", "lare"), Some(2));
        assert_eq!(rank("Gaussian Blur", "Blur & Sharpen", "blur sharpen"), Some(2), "words can match the category");
        assert_eq!(rank("Gaussian Blur", "Blur & Sharpen", "glow"), None);
        assert_eq!(rank("Glow", "Stylize", ""), Some(0), "an empty search shows everything");
    }
}
