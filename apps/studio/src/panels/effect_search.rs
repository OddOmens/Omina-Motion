//! Quick effect search (⌘Return): a box in the middle of the window that finds an effect, or
//! one of the user's presets, by typing part of its name, and puts it on the selected clips.
//! Up and Down move through the results, Return applies the highlighted one, Escape (or a
//! click outside) closes it.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::{
    AnyElement, AppContext as _, Context, ElementId, Entity, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Subscription, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};
use studio_core::model::LinkId;

use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// The open search: its text field, the highlighted result, and the field's event listener.
pub struct EffectSearch {
    input: Entity<InputState>,
    highlighted: usize,
    _events: Subscription,
}

/// How many results show at once.
const SHOWN: usize = 12;

/// What a result applies.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Apply {
    Effect(&'static str),
    Preset(LinkId),
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
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search effects and presets"));
    input.update(cx, |state, cx| state.focus(window, cx));
    let events = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| match event {
        InputEvent::PressEnter { .. } => apply_highlighted(this, window, cx),
        // A new search starts at its best result.
        InputEvent::Change => {
            if let Some(search) = this.ui.effect_search.as_mut() {
                search.highlighted = 0;
            }
            cx.notify();
        }
        _ => {}
    });
    workspace.ui.effect_search = Some(EffectSearch { input, highlighted: 0, _events: events });
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
/// equals: they are the user's own.
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
    // A stable sort keeps presets, then catalog order, within each rank.
    found.sort_by_key(|(score, _)| *score);
    found.into_iter().map(|(_, found)| found).collect()
}

/// The current search's results.
fn current_results(workspace: &Workspace, cx: &gpui_kit::App) -> Vec<Found> {
    let query = workspace.ui.effect_search.as_ref().map(|search| search.input.read(cx).value().to_string()).unwrap_or_default();
    results(workspace, &query)
}

/// Applies the highlighted result to the selected clips and closes the search.
fn apply_highlighted(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let results = current_results(workspace, cx);
    let highlighted = workspace.ui.effect_search.as_ref().map_or(0, |search| search.highlighted);
    if let Some(found) = results.get(highlighted).cloned() {
        apply(workspace, &found, window, cx);
    }
}

/// Puts `found` on every selected clip, shows it where it is edited, and closes the search.
fn apply(workspace: &mut Workspace, found: &Found, window: &mut Window, cx: &mut Context<Workspace>) {
    use gpui_kit::component::WindowExt as _;
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
    };
    workspace.ui.tabs.insert(PanelId::Properties, tab);
    close(workspace, window, cx);
}

/// Moves the highlight up or down through the results.
fn step(workspace: &mut Workspace, down: bool, cx: &mut Context<Workspace>) {
    let count = current_results(workspace, cx).len().min(SHOWN);
    if let Some(search) = workspace.ui.effect_search.as_mut().filter(|_| count > 0) {
        search.highlighted = if down { (search.highlighted + 1).min(count - 1) } else { search.highlighted.saturating_sub(1) };
        cx.notify();
    }
}

/// The search box over a dimmed window, while it is open.
pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let search = workspace.ui.effect_search.as_ref()?;
    let results = current_results(workspace, cx);
    let highlighted = search.highlighted.min(results.len().saturating_sub(1));
    let target = match workspace.state.selected_layers.len() {
        0 => "Select a clip to put an effect on it".to_owned(),
        1 => {
            let composition = workspace.state.composition();
            let name = workspace.state.selected_layers.first().and_then(|id| composition.layer(*id)).map_or_else(String::new, |layer| layer.name.clone());
            format!("For {name}")
        }
        count => format!("For {count} clips"),
    };
    let rows: Vec<AnyElement> = results
        .iter()
        .take(SHOWN)
        .enumerate()
        .map(|(index, found)| {
            let found = found.clone();
            let on = index == highlighted;
            let icon = match found.apply {
                Apply::Preset(_) => IconName::Bookmark,
                Apply::Effect(name) => super::effect_controls::category_icon(name),
            };
            div()
                .id(ElementId::NamedInteger("effect-search-result".into(), index as u64))
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1p5()
                .rounded(px(6.0))
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
    let card = div()
        .id("effect-search")
        .w(px(520.0))
        .mt(px(90.0))
        .p_2()
        .flex()
        .flex_col()
        .gap_1()
        .rounded(px(12.0))
        .bg(hsla(color::POPOVER))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .shadow_lg()
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
        .children(rows)
        .when(empty, |card| card.child(div().px_3().py_2().text_color(hsla(color::MUTED)).child("No effect matches. Try another word.")))
        .test_support();
    Some(
        div()
            .id("effect-search-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .items_start()
            .bg(gpui_kit::rgba(0x00000066))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| close(this, window, cx)))
            .child(card)
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
