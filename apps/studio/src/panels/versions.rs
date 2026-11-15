//! File ▸ Browse Versions…: the earlier versions kept of the open project (see `crate::versions`),
//! newest first, each with when it was kept and its size. Restore opens one as unsaved changes,
//! keeping the work as it is now as a version too, so nothing is lost either way.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::{AnyElement, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px};

use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let file = workspace.state.file().map(std::path::Path::to_path_buf);
    let versions = crate::versions::list(file.as_deref());
    let folder = crate::versions::folder_for(file.as_deref());
    let keeping = workspace.ui.settings.versions;
    let now = crate::app_settings::now_seconds();
    let target = cx.weak_entity();
    crate::panels::chrome::open_dialog(window, cx, move |dialog, _, _| {
        let rows: Vec<AnyElement> = versions
            .iter()
            .enumerate()
            .map(|(index, version)| {
                let (target, version) = (target.clone(), version.clone());
                let when = super::start_screen::time_ago(version.kept_at, now);
                div()
                    .h(px(40.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(hsla(color::HAIRLINE))
                    .child(div().flex_1().text_color(hsla(color::TEXT)).child(if index == 0 { format!("{when} (newest)") } else { when }))
                    .child(div().w(px(80.0)).text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("{:.1} KB", version.bytes as f64 / 1024.0)))
                    .child(Button::new(("version-restore", index)).outline().small().label("Restore").on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        target.update(cx, |this, cx| this.restore_version(&version, window, cx)).ok();
                        window.push_notification("Restored that version as unsaved changes. Save to keep it, or restore the newest version to go back.", cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let empty = if keeping.keep {
            format!("No versions yet. One is kept each time you save, and every {} minutes while you work.", keeping.every_minutes)
        } else {
            "Versions are turned off in Settings ▸ General.".to_owned()
        };
        let folder = folder.clone();
        dialog
            .title("Earlier Versions")
            .w(px(520.0))
            .centered()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_color(hsla(color::MUTED)).child("Restoring keeps your work as it is now as a version too, so you can always come back."))
                    .child(div().id("versions-list").max_h(px(360.0)).overflow_y_scroll().rounded(px(8.0)).border_1().border_color(hsla(color::HAIRLINE)).when(rows.is_empty(), |list| list.child(div().p_3().text_color(hsla(color::MUTED)).child(empty))).children(rows)),
            )
            .footer(
                div().w_full().flex().justify_between().child(Button::new("versions-reveal").ghost().small().label("Show in Finder").on_click(move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&folder);
                    cx.reveal_path(&folder);
                })).child(Button::new("versions-done").primary().small().px_4().label("Done").on_click(|_, window, cx| window.close_dialog(cx))),
            )
    });
}
