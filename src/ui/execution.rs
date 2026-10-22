//! Background execution of renames and undo/redo with progress dialogs.
//!
//! Every filesystem-heavy operation runs in `gio::spawn_blocking`, so the GTK
//! main loop keeps painting; progress and results come back over channels.

use super::window::RenamerWindow;
use crate::core::{FileEntry, RenamePreview};
use crate::engine::RenameBatchResult;
use crate::undo::UndoResult;
use gettextrs::gettext;
use libadwaita as adw;
use adw::prelude::*;
use gtk4 as gtk;
use gtk::{gio, glib};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

/// Modal progress window with a cancel button. Returns the window, the bar,
/// and the cancel button so callers can wire them up.
fn progress_window(
    parent: &RenamerWindow,
    title: &str,
) -> (adw::Window, gtk::ProgressBar, gtk::Button) {
    let dialog = adw::Window::builder()
        .title(title)
        .modal(true)
        .transient_for(parent)
        .deletable(false)
        .default_width(360)
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(24)
        .margin_end(24)
        .margin_top(24)
        .margin_bottom(24)
        .build();

    let heading = gtk::Label::builder()
        .label(title)
        .css_classes(vec!["title-4"])
        .build();
    content.append(&heading);

    let bar = gtk::ProgressBar::builder().show_text(true).build();
    content.append(&bar);

    let cancel_btn = gtk::Button::with_label(&gettext("Cancel"));
    cancel_btn.set_halign(gtk::Align::Center);
    content.append(&cancel_btn);

    dialog.set_content(Some(&content));
    (dialog, bar, cancel_btn)
}

/// Whether a batch result is a clean user cancellation: everything failed with
/// the cancel reason and the rollback put every file back.
fn is_clean_cancellation(result: &RenameBatchResult) -> bool {
    result.success_count() == 0
        && !result.failures.is_empty()
        && result
            .failures
            .iter()
            .all(|failure| failure.error == crate::engine::CANCELLED)
}

/// Execute a rename plan off the main thread with live progress and a working
/// Cancel button. All-or-nothing semantics are preserved by the engine.
pub fn run_rename(
    window: &RenamerWindow,
    previews: Vec<RenamePreview>,
    files: HashMap<Uuid, FileEntry>,
) {
    if window.is_busy() {
        return;
    }
    window.set_busy(true);

    let cancel = Arc::new(AtomicBool::new(false));
    let (dialog, bar, cancel_btn) = progress_window(window, &gettext("Renaming Files…"));
    bar.set_text(Some(gettext("Preparing…").as_str()));

    {
        let cancel = cancel.clone();
        let bar = bar.clone();
        cancel_btn.connect_clicked(move |btn| {
            cancel.store(true, Ordering::Relaxed);
            btn.set_sensitive(false);
            bar.set_text(Some(gettext("Cancelling…").as_str()));
        });
    }

    let (progress_tx, progress_rx) = async_channel::unbounded::<(usize, usize)>();
    let (result_tx, result_rx) =
        async_channel::bounded::<crate::core::RenamerResult<RenameBatchResult>>(1);

    let worker_cancel = cancel.clone();
    let journal_dir = crate::undo::default_data_dir();
    gio::spawn_blocking(move || {
        let result = crate::engine::execute_renames_with(
            &previews,
            &files,
            Some(&journal_dir),
            move |done, total| {
                let _ = progress_tx.send_blocking((done, total));
            },
            &worker_cancel,
        );
        let _ = result_tx.send_blocking(result);
    });

    // Progress updates until the sender is dropped.
    let bar_clone = bar.clone();
    let cancel_for_bar = cancel.clone();
    glib::spawn_future_local(async move {
        while let Ok((done, total)) = progress_rx.recv().await {
            if cancel_for_bar.load(Ordering::Relaxed) {
                continue;
            }
            if total > 0 {
                bar_clone.set_fraction(done as f64 / total as f64);
                let text = gettext("{} of {} steps")
                    .replacen("{}", &done.to_string(), 1)
                    .replacen("{}", &total.to_string(), 1);
                bar_clone.set_text(Some(&text));
            }
        }
    });

    let window_weak = window.downgrade();
    let dialog_clone = dialog.clone();
    glib::spawn_future_local(async move {
        let outcome = result_rx.recv().await;
        dialog_clone.close();
        let Some(window) = window_weak.upgrade() else {
            return;
        };
        window.set_busy(false);
        match outcome {
            Ok(Ok(result)) if is_clean_cancellation(&result) => {
                window.show_toast(&gettext("Rename cancelled — no files were changed"));
                window.update_preview();
            }
            Ok(Ok(result)) => window.handle_rename_result(result),
            Ok(Err(err)) => window.show_info_dialog(&gettext("Rename Blocked"), &err.to_string()),
            Err(_) => window.set_busy(false),
        }
    });

    dialog.present();
}

enum HistoryOp {
    Undo,
    Redo,
    UndoBatch(Uuid),
}

/// Run undo/redo on a worker thread. The UndoManager is moved out of the
/// window for the duration and always put back, so the busy flag is the only
/// thing guarding re-entrancy.
fn run_history(window: &RenamerWindow, op: HistoryOp) {
    if window.is_busy() {
        return;
    }
    window.set_busy(true);

    let title = match op {
        HistoryOp::Undo | HistoryOp::UndoBatch(_) => gettext("Undoing Rename…"),
        HistoryOp::Redo => gettext("Redoing Rename…"),
    };
    let (dialog, bar, cancel_btn) = progress_window(window, &title);
    cancel_btn.set_visible(false);
    bar.set_text(Some(gettext("Working…").as_str()));

    // Pulse while the dialog is up.
    let bar_weak = bar.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        match bar_weak.upgrade() {
            Some(bar) if bar.is_visible() => {
                bar.pulse();
                glib::ControlFlow::Continue
            }
            _ => glib::ControlFlow::Break,
        }
    });

    let mut manager = window.take_undo_manager();
    let (result_tx, result_rx) = async_channel::bounded::<(
        crate::undo::UndoManager,
        crate::core::RenamerResult<UndoResult>,
    )>(1);

    let is_undo = !matches!(op, HistoryOp::Redo);
    gio::spawn_blocking(move || {
        let result = match op {
            HistoryOp::Undo => manager.undo(),
            HistoryOp::Redo => manager.redo(),
            HistoryOp::UndoBatch(batch_id) => manager.undo_batch(batch_id),
        };
        let _ = result_tx.send_blocking((manager, result));
    });

    let window_weak = window.downgrade();
    let dialog_clone = dialog.clone();
    glib::spawn_future_local(async move {
        let received = result_rx.recv().await;
        dialog_clone.close();
        let Some(window) = window_weak.upgrade() else {
            return;
        };
        let Ok((manager, result)) = received else {
            window.set_busy(false);
            return;
        };
        window.restore_undo_manager(manager);
        window.set_busy(false);

        let (done_title, partial_title, unavailable_title) = if is_undo {
            (
                gettext("Undo Complete"),
                gettext("Undo Incomplete"),
                gettext("Undo Unavailable"),
            )
        } else {
            (
                gettext("Redo Complete"),
                gettext("Redo Incomplete"),
                gettext("Redo Unavailable"),
            )
        };
        match result {
            Ok(result) => {
                let template = if is_undo {
                    gettext("Restored {} of {} renamed files.")
                } else {
                    gettext("Renamed {} of {} files again.")
                };
                let summary = template
                    .replacen("{}", &result.success_count.to_string(), 1)
                    .replacen("{}", &result.total_records.to_string(), 1);
                if result.all_successful() {
                    window.show_toast(&summary);
                } else {
                    let _ = done_title;
                    window.show_info_dialog(
                        &partial_title,
                        &RenamerWindow::undo_result_message(summary, &result),
                    );
                }
                window.update_history_actions();
                window.update_preview();
            }
            Err(err) => window.show_info_dialog(&unavailable_title, &err.to_string()),
        }
    });

    dialog.present();
}

pub fn run_undo(window: &RenamerWindow) {
    run_history(window, HistoryOp::Undo);
}

pub fn run_redo(window: &RenamerWindow) {
    run_history(window, HistoryOp::Redo);
}

/// Undo one specific batch from the history browser.
pub fn run_undo_batch(window: &RenamerWindow, batch_id: Uuid) {
    run_history(window, HistoryOp::UndoBatch(batch_id));
}
