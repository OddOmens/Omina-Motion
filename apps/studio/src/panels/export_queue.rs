//! The export queue (File ▸ Export Queue…), like Adobe Media Encoder's: exports added from the
//! Export dialog's Add to Queue… wait here, each with the project as it was when added, its
//! settings and its file, then Start Queue runs them one after another in the background.
//! Each item shows whether it is waiting, exporting, done, cancelled or failed.

use std::path::PathBuf;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::{Context, ParentElement, Styled, WeakEntity, Window, div, prelude::FluentBuilder as _, px};
use studio_core::model::Project;
use studio_export::{ExportRequest, Outcome};

use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Where a queued export has got to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueStatus {
    Waiting,
    Exporting,
    Done,
    Cancelled,
    Failed(String),
}

/// One export waiting in (or done by) the queue.
pub struct QueuedExport {
    /// What the list shows: the composition's name.
    pub composition: String,
    /// The project as it was when the export was added, so later edits don't change it.
    pub project: Project,
    pub request: ExportRequest,
    pub status: QueueStatus,
    /// Whether Iris was asked for it in her conversation, where she then says how it ended.
    pub from_assistant: bool,
}

/// The queue: its exports in order, and whether it is working through them.
#[derive(Default)]
pub struct ExportQueue {
    pub items: Vec<QueuedExport>,
    pub running: bool,
}

impl ExportQueue {
    /// Whether one of the exports is running now.
    fn exporting(&self) -> bool {
        self.items.iter().any(|item| item.status == QueueStatus::Exporting)
    }

    /// Takes the finished exports (done, cancelled or failed) out of the list. Nothing is taken
    /// out while one is running, because it reports back by its place in the list: moving it
    /// would mark another export as the one that finished.
    fn clear_finished(&mut self) {
        if self.exporting() {
            return;
        }
        self.items.retain(|item| item.status == QueueStatus::Waiting);
    }
}

/// Adds an export of the open composition, with the Export dialog's settings, saving to `path`.
pub fn add(workspace: &mut Workspace, path: PathBuf, window: &mut Window, cx: &mut Context<Workspace>) {
    // The queued export keeps the project as it is now, so recipes set to run before each
    // export run as it is added.
    super::export::before_export(workspace, window, cx);
    let request = super::export::request_for(workspace, path);
    let composition = workspace.state.composition().name.clone();
    let project = workspace.state.project.clone();
    workspace.ui.export_queue.items.push(QueuedExport { composition, project, request, status: QueueStatus::Waiting, from_assistant: false });
    let waiting = workspace.ui.export_queue.items.iter().filter(|item| item.status == QueueStatus::Waiting).count();
    window.push_notification(format!("Added to the export queue ({waiting} waiting). File ▸ Export Queue… starts it.").as_str(), cx);
    cx.notify();
}

/// Starts working through the waiting exports, unless an export is already running.
pub fn start(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.export_job.is_some() {
        window.push_notification("An export is already running. Start the queue once it finishes.", cx);
        return;
    }
    workspace.ui.export_queue.running = true;
    start_next(workspace, window, cx);
}

/// Starts the first waiting export, or, with none left, stops and says how it went.
fn start_next(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let queue = &mut workspace.ui.export_queue;
    let Some(index) = queue.items.iter().position(|item| item.status == QueueStatus::Waiting).filter(|_| queue.running) else {
        if std::mem::take(&mut queue.running) {
            let done = queue.items.iter().filter(|item| item.status == QueueStatus::Done).count();
            let failed = queue.items.iter().filter(|item| matches!(item.status, QueueStatus::Failed(_))).count();
            let summary = if failed == 0 { format!("Export queue finished: {done} exported.") } else { format!("Export queue finished: {done} exported, {failed} failed.") };
            window.push_notification(summary.as_str(), cx);
        }
        cx.notify();
        return;
    };
    let item = &mut queue.items[index];
    item.status = QueueStatus::Exporting;
    let (project, request) = (item.project.clone(), item.request.clone());
    super::export::start_queued(workspace, index, project, request, window, cx);
}

/// Called when queued export `index` ends: notes how, then starts the next. A cancelled one
/// stops the queue (Cancel means stop), leaving the rest waiting.
pub(crate) fn finished(workspace: &mut Workspace, index: usize, outcome: Result<Outcome, String>, window: &mut Window, cx: &mut Context<Workspace>) {
    // An export Iris was asked for is reported where it was asked for.
    if let Some(path) = workspace.ui.export_queue.items.get(index).filter(|item| item.from_assistant).map(|item| item.request.path.clone()) {
        super::assistant::export_finished(workspace, &path, &outcome, window, cx);
    }
    let status = match outcome {
        Ok(Outcome::Finished { .. }) => QueueStatus::Done,
        Ok(Outcome::Cancelled) => QueueStatus::Cancelled,
        Err(error) => QueueStatus::Failed(error),
    };
    if status == QueueStatus::Cancelled {
        workspace.ui.export_queue.running = false;
    }
    if let Some(item) = workspace.ui.export_queue.items.get_mut(index) {
        item.status = status;
    }
    start_next(workspace, window, cx);
}

/// Opens the queue's window: each export with its status, Remove for those not running,
/// Clear Finished, and Start Queue.
pub fn open(_workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let target = cx.weak_entity();
    crate::panels::chrome::open_dialog(window, cx, move |dialog, _, cx| {
        let Some(workspace) = target.upgrade() else { return dialog };
        let workspace = workspace.read(cx);
        let queue = &workspace.ui.export_queue;
        let rows: Vec<_> = queue.items.iter().enumerate().map(|(index, item)| row(index, item, &target)).collect();
        let waiting = queue.items.iter().any(|item| item.status == QueueStatus::Waiting);
        let finished = !queue.exporting() && queue.items.iter().any(|item| !matches!(item.status, QueueStatus::Waiting | QueueStatus::Exporting));
        let can_start = waiting && !queue.running && workspace.ui.export_job.is_none();
        let (start_target, clear_target) = (target.clone(), target.clone());
        dialog
            .title("Export Queue")
            .w(px(560.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_size(px(size::LARGE_TEXT))
                    .children(rows)
                    .when(queue.items.is_empty(), |list| {
                        list.child(div().py_2().text_color(hsla(color::MUTED)).child("Nothing queued. In the Export dialog (⌘M), Add to Queue… saves an export to run here later."))
                    }),
            )
            .footer(
                div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(Button::new("export-queue-clear").outline().small().label("Clear Finished").disabled(!finished).on_click(move |_, window, cx| {
                        clear_target
                            .update(cx, |this, cx| {
                                this.ui.export_queue.clear_finished();
                                cx.notify();
                            })
                            .ok();
                        window.refresh();
                    }))
                    .child(Button::new("export-queue-start").primary().small().label("Start Queue").disabled(!can_start).on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        start_target.update(cx, |this, cx| start(this, window, cx)).ok();
                    }))
                    .child(Button::new("export-queue-done").outline().small().label("Done").on_click(|_, window, cx| window.close_dialog(cx))),
            )
    });
}

/// One export in the list: composition, file, status, and Remove while it isn't running.
fn row(index: usize, item: &QueuedExport, target: &WeakEntity<Workspace>) -> impl gpui_kit::IntoElement + use<> {
    let file = item.request.path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let (status, tint) = match &item.status {
        QueueStatus::Waiting => ("Waiting".to_owned(), color::MUTED),
        QueueStatus::Exporting => ("Exporting…".to_owned(), color::ACCENT),
        QueueStatus::Done => ("Done".to_owned(), color::TEXT),
        QueueStatus::Cancelled => ("Cancelled".to_owned(), color::MUTED),
        QueueStatus::Failed(error) => (format!("Failed: {error}"), color::MUTED),
    };
    let remove = target.clone();
    div()
        .flex()
        .items_center()
        .gap_3()
        .py_1()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().flex_1().min_w_0().flex().flex_col().child(div().truncate().child(item.composition.clone())).child(div().truncate().text_color(hsla(color::MUTED)).child(file)))
        .child(div().flex_none().max_w(px(200.0)).truncate().text_color(hsla(tint)).child(status))
        .child(Button::new(("export-queue-remove", index)).ghost().small().label("Remove").disabled(item.status == QueueStatus::Exporting).on_click(move |_, window, cx| {
            remove
                .update(cx, |this, cx| {
                    remove_item(this, index);
                    cx.notify();
                })
                .ok();
            window.refresh();
        }))
}

/// Takes export `index` out of the queue, unless it is the one running (whose place in the
/// list the running export still reports to).
fn remove_item(workspace: &mut Workspace, index: usize) {
    let queue = &mut workspace.ui.export_queue;
    if queue.exporting() || queue.items.get(index).is_none() {
        return;
    }
    queue.items.remove(index);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(status: QueueStatus) -> QueuedExport {
        let project = studio_core::sample::sample_project();
        let request = ExportRequest::new(project.compositions[0].id, 0..10, studio_export::ExportFormat::Gif, PathBuf::from("out.gif"));
        QueuedExport { composition: "Comp".to_owned(), project, request, status, from_assistant: false }
    }

    #[test]
    fn clear_finished_waits_while_an_export_runs() {
        // A finished, then the one running (which reports back as item 1), then one waiting.
        let mut queue = ExportQueue { items: vec![queued(QueueStatus::Done), queued(QueueStatus::Exporting), queued(QueueStatus::Waiting)], running: true };
        queue.clear_finished();
        assert_eq!(queue.items.len(), 3, "the running export keeps its place in the list");
        queue.items[1].status = QueueStatus::Done;
        queue.clear_finished();
        assert_eq!(queue.items.iter().map(|item| item.status.clone()).collect::<Vec<_>>(), vec![QueueStatus::Waiting]);
    }
}
