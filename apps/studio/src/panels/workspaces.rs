//! Named workspaces (Window ▸ Workspace), as in After Effects: the panel layout on screen can be
//! saved under a name, switched back to later, updated with the current layout, or deleted. The
//! built-in Standard workspace is the standard layout (`dock::default_layout`) and can't be
//! changed or deleted. Saved workspaces live in the app settings (`saved_workspaces`), with the
//! name of the one picked (`current_workspace`).

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{AppContext as _, Context, InteractiveElement as _, ParentElement, Styled, TestSupportExt as _, Window, div, px};

use crate::app_settings::{self, SavedWorkspace};
use crate::theme::size;
use crate::workspace::{Workspace, refresh_menus};

/// The built-in workspace's name.
pub const STANDARD: &str = "Standard";

/// A name typed for a new workspace, tidied, or why it can't be used.
pub fn check_name(typed: &str, saved: &[SavedWorkspace]) -> Result<String, String> {
    let name = typed.trim();
    if name.is_empty() {
        return Err("Give the workspace a name.".into());
    }
    if name.eq_ignore_ascii_case(STANDARD) || saved.iter().any(|workspace| workspace.name.eq_ignore_ascii_case(name)) {
        return Err(format!("There is already a workspace called “{name}”. Pick another name, or switch to it and use Update Current Workspace."));
    }
    Ok(name.to_owned())
}

/// Window ▸ Workspace ▸ Save as New Workspace…: asks for a name, then saves the layout on screen
/// under it and makes it the current workspace. A name in use is refused, keeping the dialog open.
pub fn save_as_new(_: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("Workspace name"));
    let target = cx.weak_entity();
    window.open_dialog(cx, move |dialog, _, _| {
        let (input, target) = (input.clone(), target.clone());
        dialog
            .title("New Workspace")
            .w(px(420.0))
            .centered()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(size::TEXT + 1.0))
                    .child(div().flex_none().child("Name:"))
                    .child(div().id("workspace-name").flex_1().child(Input::new(&input)).test_support()),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("workspace-save-ok").primary().label("Save"))),
            )
            .on_ok(move |_, window, cx| {
                let typed = input.read(cx).value().to_string();
                let name = match check_name(&typed, &app_settings::load().saved_workspaces) {
                    Ok(name) => name,
                    Err(problem) => {
                        window.push_notification(problem.as_str(), cx);
                        return false;
                    }
                };
                let Some(layout) = target.upgrade().and_then(|workspace| workspace.read(cx).current_layout(cx)) else { return true };
                app_settings::update(|settings| {
                    settings.saved_workspaces.push(SavedWorkspace { name: name.clone(), layout: layout.clone() });
                    settings.current_workspace = Some(name.clone());
                });
                refresh_menus(cx);
                window.push_notification(format!("Saved the “{name}” workspace.").as_str(), cx);
                true
            })
    });
}

/// Window ▸ Workspace ▸ Update Current Workspace: saves the layout on screen over the current
/// workspace. Standard can't be changed (the menu item is off for it).
pub fn update_current(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(name) = app_settings::load().current_workspace else {
        window.push_notification("Standard can't be changed. Use Save as New Workspace… to keep this layout.", cx);
        return;
    };
    let Some(layout) = workspace.current_layout(cx) else { return };
    let mut found = false;
    app_settings::update(|settings| {
        if let Some(saved) = settings.saved_workspaces.iter_mut().find(|saved| saved.name == name) {
            saved.layout = layout.clone();
            found = true;
        }
    });
    let message = if found { format!("Updated the “{name}” workspace.") } else { format!("The “{name}” workspace is no longer saved.") };
    window.push_notification(message.as_str(), cx);
}

/// Window ▸ Workspace ▸ Delete Workspace…: asks first, then deletes the current workspace.
/// The panels stay where they are, and Standard becomes the current workspace.
pub fn delete_current(_: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(name) = app_settings::load().current_workspace else {
        window.push_notification("Standard is built in and can't be deleted.", cx);
        return;
    };
    window.open_alert_dialog(cx, move |dialog, _, _| {
        let name = name.clone();
        dialog
            .title(format!("Delete the “{name}” workspace?"))
            .description("The panels stay where they are now. This can't be undone.")
            .ok_text("Delete")
            .show_cancel(true)
            .on_ok(move |_, _, cx| {
                app_settings::update(|settings| {
                    settings.saved_workspaces.retain(|saved| saved.name != name);
                    if settings.current_workspace.as_deref() == Some(name.as_str()) {
                        settings.current_workspace = None;
                    }
                });
                refresh_menus(cx);
                true
            })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_workspace_needs_a_name_of_its_own() {
        let saved = vec![SavedWorkspace { name: "Editing".into(), layout: serde_json::Value::Null }];
        assert_eq!(check_name("  Color  ", &saved), Ok("Color".into()), "trimmed");
        assert!(check_name("   ", &saved).is_err(), "blank");
        assert!(check_name("standard", &saved).is_err(), "the built-in name");
        assert!(check_name("EDITING", &saved).is_err(), "one already saved");
    }
}
