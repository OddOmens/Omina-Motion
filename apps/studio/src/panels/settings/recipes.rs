//! Settings ▸ Recipes: every recipe Iris knows in Motion, the ones taught and the ones Motion
//! comes with, each with its steps, its shortcut, its button at the top of the chat and the
//! moments it runs by itself. A taught recipe can be renamed, its steps changed (written as in
//! a message to Iris, joined with "then" or a semicolon),
//! moved up or down the list and deleted (with Undo); any recipe can be given a shortcut, a
//! button and moments. Export and Import write and read all of them as a file, through the
//! usual save and open dialogs.
//!
//! The changes themselves are made by `assistant_automation`, which saves the recipes and
//! binds their shortcuts again after each one.

use gpui_kit::ElementId;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use iris::recipes::{Recipe, Trigger};

use super::*;
use crate::panels::assistant_automation as automation;

/// The fields for changing a recipe, made when Settings opens and filled when Edit is clicked.
pub(super) struct Fields {
    pub name: Entity<InputState>,
    pub steps: Entity<InputState>,
    pub key: Entity<InputState>,
}

impl Fields {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        Self {
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Name")),
            steps: cx.new(|cx| InputState::new(window, cx).placeholder("fade in; then pulse")),
            key: cx.new(|cx| InputState::new(window, cx).placeholder("⌘⇧T, or empty for none")),
        }
    }
}

/// How a moment reads beside its checkbox.
fn moment_label(trigger: Trigger) -> &'static str {
    match trigger {
        Trigger::BeforeExport => "Before export",
        Trigger::OnOpen => "On open",
        Trigger::BeforeSave => "Before save",
        Trigger::AfterImport => "After import",
    }
}

/// Every recipe Iris knows, the taught ones first, each with whether Motion comes with it.
fn listed(workspace: &WeakEntity<Workspace>, cx: &mut App) -> Vec<(Recipe, bool)> {
    workspace
        .update(cx, |this, _| {
            let memory = crate::panels::assistant::memory(this);
            let own: Vec<String> = memory.built_in.iter().map(|recipe| recipe.name.clone()).collect();
            memory.known().into_iter().map(|recipe| {
                let is_own = own.contains(&recipe.name);
                (recipe, is_own)
            }).collect()
        })
        .unwrap_or_default()
}

/// Says `said` at the foot of the page.
fn tell(shown: &SharedShown, said: impl Into<String>, window: &mut Window) {
    shown.borrow_mut().note = Some(said.into());
    window.refresh();
}

/// One recipe's card: its name and what it is set up with, its steps, and its controls.
fn recipe_card(index: usize, recipe: &Recipe, built_in: bool, taught_count: usize, shown: &SharedShown, workspace: &WeakEntity<Workspace>, fields: &Fields) -> AnyElement {
    let name = recipe.name.clone();
    let editing = shown.borrow().recipe_editing.as_deref() == Some(name.as_str());
    let id = |part: &str| ElementId::Name(format!("recipe-{part}:{index}").into());
    let extras = recipe.extras();
    let heading = div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().text_size(px(size::TEXT + 1.0)).text_color(hsla(color::TEXT)).font_weight(gpui_kit::FontWeight::MEDIUM).child(recipe.name.clone()))
        .when(built_in, |row| row.child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("comes with Omina Motion")))
        .when(!extras.is_empty(), |row| row.child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).child(extras)))
        .child(div().flex_1());
    // Moving and deleting are for the taught ones; a built-in copy can only be reset.
    let position = if built_in { None } else { Some(index) };
    let mover = |up: bool| {
        let (workspace, name) = (workspace.clone(), name.clone());
        let disabled = position.is_none_or(|at| if up { at == 0 } else { at + 1 >= taught_count });
        Button::new(id(if up { "up" } else { "down" })).ghost().xsmall().label(if up { "Up" } else { "Down" }).tooltip(if up { "Move up the list" } else { "Move down the list" }).disabled(disabled).on_click(
            move |_, window, cx| {
                workspace.update(cx, |this, cx| automation::move_recipe(this, &name, up, cx)).ok();
                window.refresh();
            },
        )
    };
    let edit = {
        let (shown, name, fields) = (shown.clone(), name.clone(), (fields.name.clone(), fields.steps.clone(), fields.key.clone()));
        let (steps, key) = (recipe.steps.join("; then "), recipe.key.as_deref().map(iris::recipes::key_label).unwrap_or_default());
        Button::new(id("edit")).ghost().xsmall().label(if editing { "Close" } else { "Edit" }).on_click(move |_, window, cx| {
            let opening = shown.borrow().recipe_editing.as_deref() != Some(name.as_str());
            shown.borrow_mut().recipe_editing = opening.then(|| name.clone());
            shown.borrow_mut().note = None;
            if opening {
                fields.0.update(cx, |field, cx| field.set_value(name.clone(), window, cx));
                fields.1.update(cx, |field, cx| field.set_value(steps.clone(), window, cx));
                fields.2.update(cx, |field, cx| field.set_value(key.clone(), window, cx));
            }
            window.refresh();
        })
    };
    let delete = {
        let (shown, workspace, name) = (shown.clone(), workspace.clone(), name.clone());
        let label = if built_in { "Reset" } else { "Delete" };
        let in_taught = !built_in || !extras_empty(recipe);
        Button::new(id("delete")).ghost().xsmall().label(label).disabled(!in_taught).on_click(move |_, window, cx| {
            let removed = workspace.update(cx, |this, cx| automation::delete_recipe(this, &name, cx)).ok().flatten();
            let said = if built_in { format!("“{name}” has no shortcut, button or moments now.") } else { format!("Deleted “{name}”.") };
            shown.borrow_mut().recipe_undo = removed;
            shown.borrow_mut().recipe_editing = None;
            tell(&shown, said, window);
        })
    };
    let heading = heading.child(mover(true)).child(mover(false)).child(edit).child(delete);
    let steps = div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(recipe.steps.join(" → "));
    // The button and the moments are a click each.
    let button_switch = {
        let (workspace, name) = (workspace.clone(), name.clone());
        Checkbox::new(id("button")).label("Button in the chat").checked(recipe.button).on_click(move |checked: &bool, window, cx| {
            let on = *checked;
            workspace.update(cx, |this, cx| automation::set_button(this, &name, on, cx)).ok();
            window.refresh();
        })
    };
    let moments = Trigger::ALL.into_iter().enumerate().map(|(number, trigger)| {
        let (workspace, name) = (workspace.clone(), name.clone());
        Checkbox::new(ElementId::Name(format!("recipe-when-{number}:{index}").into())).label(moment_label(trigger)).checked(recipe.when.contains(&trigger)).on_click(move |checked: &bool, window, cx| {
            let on = *checked;
            workspace.update(cx, |this, cx| automation::set_trigger(this, &name, trigger, on, cx)).ok();
            window.refresh();
        })
    });
    let controls = div().flex().flex_wrap().items_center().gap_3().child(button_switch).children(moments);
    let form = editing.then(|| {
        let save = {
            let (shown, workspace, name) = (shown.clone(), workspace.clone(), name.clone());
            let (name_field, steps_field, key_field) = (fields.name.clone(), fields.steps.clone(), fields.key.clone());
            Button::new(id("save")).primary().small().label("Save").on_click(move |_, window, cx| {
                let (new_name, steps, key) = (name_field.read(cx).value().to_string(), steps_field.read(cx).value().to_string(), key_field.read(cx).value().to_string());
                let result = workspace.update(cx, |this, cx| automation::edit_recipe(this, &name, &new_name, &steps, &key, cx)).unwrap_or_else(|_| Err("Omina Motion's window has closed.".to_owned()));
                match result {
                    Ok(said) => {
                        shown.borrow_mut().recipe_editing = None;
                        tell(&shown, said, window);
                    }
                    Err(why) => tell(&shown, why, window),
                }
            })
        };
        let field = |label: &'static str, control: AnyElement| div().flex().items_start().gap_3().child(div().w(px(70.0)).pt_1().flex_none().text_color(hsla(color::MUTED)).child(label)).child(div().flex_1().min_w_0().child(control));
        div()
            .id(id("form"))
            .pt_2()
            .flex()
            .flex_col()
            .gap_2()
            .when(!built_in, |form| {
                form.child(field("Name", div().id("recipe-name-field").child(Input::new(&fields.name).small()).test_support().into_any_element()))
                    .child(field("Steps", div().id("recipe-steps-field").child(Input::new(&fields.steps).small()).test_support().into_any_element()))
            })
            .child(field("Shortcut", div().id("recipe-key-field").w(px(200.0)).child(Input::new(&fields.key).small()).test_support().into_any_element()))
            .child(div().flex().justify_end().child(save))
            .test_support()
    });
    div().id(id("card")).px_3p5().py_2p5().flex().flex_col().gap_1p5().child(heading).child(steps).child(controls).children(form).test_support().into_any_element()
}

/// Whether a recipe has no shortcut, button or moments.
fn extras_empty(recipe: &Recipe) -> bool {
    recipe.key.is_none() && !recipe.button && recipe.when.is_empty()
}

/// The page itself.
pub(super) fn recipes_page(shown: &SharedShown, workspace: &WeakEntity<Workspace>, fields: &Fields, cx: &mut App) -> gpui_kit::Div {
    let all = listed(workspace, cx);
    // A copy of a recipe Motion comes with is listed among Motion's, where people look for it.
    let own_names: Vec<String> = all.iter().filter(|(_, own)| *own).map(|(recipe, _)| recipe.name.clone()).collect();
    let taught: Vec<&Recipe> = all.iter().filter(|(recipe, own)| !own && !own_names.contains(&recipe.name)).map(|(recipe, _)| recipe).collect();
    let built_in: Vec<&Recipe> = all.iter().filter(|(_, own)| *own).map(|(recipe, _)| recipe).collect();
    let taught_cards: Vec<AnyElement> = taught.iter().enumerate().map(|(index, recipe)| recipe_card(index, recipe, false, taught.len(), shown, workspace, fields)).collect();
    let own_cards: Vec<AnyElement> = built_in.iter().enumerate().map(|(index, recipe)| recipe_card(taught.len() + index, recipe, true, taught.len(), shown, workspace, fields)).collect();
    let undo = shown.borrow().recipe_undo.clone().map(|(at, recipe)| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        let name = recipe.name.clone();
        Button::new("recipe-undo").outline().small().label(format!("Undo Deleting “{name}”")).on_click(move |_, window, cx| {
            let recipe = recipe.clone();
            workspace.update(cx, |this, cx| automation::undelete_recipe(this, at, recipe, cx)).ok();
            shown.borrow_mut().recipe_undo = None;
            tell(&shown, format!("“{name}” is back."), window);
        })
    });
    let export = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("recipes-export").outline().small().label("Export All…").on_click(move |_, window, cx| {
            let Some(text) = workspace.update(cx, |this, _| automation::recipes_file(this)).ok() else { return };
            let folder = crate::panels::export::starting_folder(None);
            let chosen = cx.prompt_for_new_path(&folder, Some("Omina Motion recipes.json"));
            let shown = shown.clone();
            window
                .spawn(cx, async move |cx| {
                    let Ok(Ok(Some(path))) = chosen.await else { return };
                    let said = match crate::app_settings::write_atomically(&path, &text) {
                        Ok(()) => format!("Saved every recipe to {}.", path.display()),
                        Err(error) => format!("The recipes couldn't be saved: {error}"),
                    };
                    cx.update(|window, _| tell(&shown, said, window)).ok();
                })
                .detach();
        })
    };
    let import = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("recipes-import").outline().small().label("Import…").on_click(move |_, window, cx| {
            let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Import Recipes".into()) });
            let (shown, workspace) = (shown.clone(), workspace.clone());
            window
                .spawn(cx, async move |cx| {
                    let Ok(Ok(Some(paths))) = chosen.await else { return };
                    let Some(path) = paths.into_iter().next() else { return };
                    let text = std::fs::read_to_string(&path).map_err(|error| format!("{} couldn't be read: {error}", path.display()));
                    cx.update(|window, cx| {
                        let said = text.and_then(|text| workspace.update(cx, |this, cx| automation::import_recipes(this, &text, cx)).unwrap_or_else(|_| Err("Omina Motion's window has closed.".to_owned())));
                        tell(&shown, said.unwrap_or_else(|why| why), window);
                    })
                    .ok();
                })
                .detach();
        })
    };
    let empty = taught.is_empty().then(|| div().px_3p5().py_3().text_color(hsla(color::MUTED)).child("None yet. Teach one in Iris's chat: “teach tidy: close the gaps, then fix everything”.").into_any_element());
    div()
        .child(page(Section::Recipes))
        .child(div().flex().gap_2().child(export).child(import).children(undo))
        .child(group_title("Taught"))
        .child(card(taught_cards.into_iter().chain(empty).collect()))
        .child(group_title("Comes with Omina Motion"))
        .child(card(own_cards))
        .children(note(shown))
}
