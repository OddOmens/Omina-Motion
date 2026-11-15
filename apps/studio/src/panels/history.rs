//! The History panel, opened from the rail: every change made since the project was opened,
//! oldest at the top, each in a few words. Clicking one goes back (or forward) to the project
//! as it was just after that change; the changes after it are dimmed until they are redone or
//! replaced by a new change. It is the same history Undo and Redo step through.

use gpui_kit::{Context, ElementId, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, div, prelude::FluentBuilder as _, px};

use super::chrome::panel;
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// One line of the history. `made` is how many changes are applied after going to it; `now`
/// marks where the project stands, and `undone` the changes after that.
fn row(made: usize, label: String, now: bool, undone: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id(ElementId::NamedInteger("history-step".into(), made as u64))
        .flex()
        .items_center()
        .gap_2()
        .px_2p5()
        .h(px(26.0))
        .cursor_pointer()
        .when(now, |row| row.bg(hsla(color::ROW_SELECTED)))
        .when(undone, |row| row.text_color(hsla(color::MUTED)))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.go_to_history(made);
            cx.notify();
        }))
        // The step's number, then what it did.
        .child(div().w(px(22.0)).flex_none().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(if made == 0 { String::new() } else { made.to_string() }))
        .child(div().flex_1().min_w_0().truncate().child(label))
        .test_support()
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let (made, undone) = workspace.state.history_changes();
    let made: Vec<String> = made.into_iter().map(str::to_owned).collect();
    let undone: Vec<String> = undone.into_iter().map(str::to_owned).collect();
    let now = made.len();
    let mut rows = vec![row(0, "Project opened".to_owned(), now == 0, false, cx).into_any_element()];
    rows.extend(made.into_iter().enumerate().map(|(index, label)| row(index + 1, label, index + 1 == now, false, cx).into_any_element()));
    rows.extend(undone.into_iter().enumerate().map(|(index, label)| row(now + index + 1, label, false, true, cx).into_any_element()));
    let note = div().px_2p5().py_2().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Click a step to go back to it. A new change replaces the dimmed steps.");
    let body = div().id("history-list").size_full().overflow_y_scroll().flex().flex_col().children(rows).child(note);
    panel(PanelId::Side, &[], None, body, workspace, cx)
}
