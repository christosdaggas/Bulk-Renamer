//! CSV import (rename plans) and log export.

use super::window::RenamerWindow;
use crate::core::{FileEntry, RenamePreview, RenameStatus};
use gettextrs::gettext;
use libadwaita as adw;
use adw::prelude::*;
use gtk4 as gtk;
use gtk::{gio, glib};
use glib::clone;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

pub fn show_import_dialog(window: &RenamerWindow) {
    let dialog = gtk::FileDialog::builder()
        .title(gettext("Import from CSV"))
        // Not modal: GNOME attaches modal dialogs to their parent window, so
        // the chooser could only be dragged together with the whole app.
        .modal(false)
        .build();

    dialog.open(
        Some(window),
        gio::Cancellable::NONE,
        clone!(
            #[weak(rename_to = window)]
            window,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        import_csv_file(&window, path);
                    }
                }
            }
        ),
    );
}

pub fn show_export_dialog(window: &RenamerWindow) {
    let dialog = gtk::FileDialog::builder()
        .title(gettext("Export Log"))
        // Not modal: GNOME attaches modal dialogs to their parent window, so
        // the chooser could only be dragged together with the whole app.
        .modal(false)
        .build();

    dialog.save(
        Some(window),
        gio::Cancellable::NONE,
        clone!(
            #[weak(rename_to = window)]
            window,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        match window.export_log_csv(&path) {
                            Ok(()) => window.show_toast(&gettext("Rename log exported to CSV")),
                            Err(err) => window
                                .show_info_dialog(&gettext("Export Failed"), &err.to_string()),
                        }
                    }
                }
            }
        ),
    );
}

fn import_csv_file(window: &RenamerWindow, path: PathBuf) {
    match read_csv_rename_plan(path) {
        Ok((previews, files)) => {
            let count = previews.len();
            let dialog = adw::MessageDialog::new(
                Some(window),
                Some(gettext("Import CSV Renames").as_str()),
                Some(
                    gettext("Apply {} renames from the CSV file?")
                        .replacen("{}", &count.to_string(), 1)
                        .as_str(),
                ),
            );
            dialog.add_response("cancel", &gettext("Cancel"));
            dialog.add_response("rename", &gettext("Rename"));
            dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
            dialog.connect_response(
                None,
                clone!(
                    #[weak(rename_to = window)]
                    window,
                    move |dialog, response| {
                        if response == "rename" {
                            super::execution::run_rename(&window, previews.clone(), files.clone());
                        }
                        dialog.close();
                    }
                ),
            );
            dialog.present();
        }
        Err(err) => window.show_info_dialog(&gettext("CSV Import Failed"), &err.to_string()),
    }
}

fn read_csv_rename_plan(
    path: PathBuf,
) -> crate::core::RenamerResult<(Vec<RenamePreview>, HashMap<Uuid, FileEntry>)> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let path_idx = headers
        .iter()
        .position(|header| header == "original_path")
        .ok_or_else(|| {
            crate::core::RenamerError::Internal(gettext("CSV must include an original_path column"))
        })?;
    let name_idx = headers
        .iter()
        .position(|header| header == "new_name")
        .ok_or_else(|| {
            crate::core::RenamerError::Internal(gettext("CSV must include a new_name column"))
        })?;

    let mut previews = Vec::new();
    let mut files = HashMap::new();

    for record in reader.records() {
        let record = record?;
        let original_path = PathBuf::from(record.get(path_idx).unwrap_or_default());
        let new_name = record.get(name_idx).unwrap_or_default().trim().to_string();
        let entry = FileEntry::from_path(original_path.clone(), 0)?;
        let new_path = original_path
            .parent()
            .map(|parent| parent.join(&new_name))
            .unwrap_or_else(|| PathBuf::from(&new_name));
        let status = if new_name == entry.original_name {
            RenameStatus::Unchanged
        } else {
            RenameStatus::WillRename
        };
        previews.push(RenamePreview {
            file_id: entry.id,
            original_name: entry.original_name.clone(),
            new_name,
            new_path,
            status,
            message: None,
        });
        files.insert(entry.id, entry);
    }

    Ok((previews, files))
}

/// Export the current preview as a CSV that round-trips with Import from CSV
/// (original_path,new_name columns).
pub fn show_export_preview_dialog(window: &RenamerWindow) {
    let previews = window.previews_snapshot();
    if previews.is_empty() {
        window.show_info_dialog(
            &gettext("Nothing to Export"),
            &gettext("Add files to build a preview first."),
        );
        return;
    }

    let dialog = gtk::FileDialog::builder()
        .title(gettext("Export Preview as CSV"))
        .initial_name("rename-plan.csv")
        // Not modal: GNOME attaches modal dialogs to their parent window, so
        // the chooser could only be dragged together with the whole app.
        .modal(false)
        .build();

    dialog.save(
        Some(window),
        gio::Cancellable::NONE,
        clone!(
            #[weak(rename_to = window)]
            window,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        match write_preview_csv(&previews, &path) {
                            Ok(()) => window.show_toast(&gettext("Preview exported as CSV")),
                            Err(err) => {
                                window.show_info_dialog(&gettext("Export Failed"), &err.to_string())
                            }
                        }
                    }
                }
            }
        ),
    );
}

fn write_preview_csv(previews: &[RenamePreview], path: &std::path::Path) -> crate::core::RenamerResult<()> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record(["original_path", "new_name"])?;
    for preview in previews {
        // Renames stay within the parent directory, so the original path is
        // the target's parent joined with the original name.
        let original_path = preview
            .new_path
            .parent()
            .map(|parent| parent.join(&preview.original_name))
            .unwrap_or_else(|| PathBuf::from(&preview.original_name));
        writer.write_record([
            original_path.to_string_lossy().as_ref(),
            preview.new_name.as_str(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

/// Save a shell script that reverts the most recent rename batch.
pub fn show_export_undo_script_dialog(window: &RenamerWindow) {
    let Some(script) = window.latest_undo_script() else {
        window.show_info_dialog(
            &gettext("No Undo Script"),
            &gettext("No renames have been recorded yet."),
        );
        return;
    };

    let dialog = gtk::FileDialog::builder()
        .title(gettext("Export Undo Script"))
        .initial_name("undo-rename.sh")
        // Not modal: GNOME attaches modal dialogs to their parent window, so
        // the chooser could only be dragged together with the whole app.
        .modal(false)
        .build();

    dialog.save(
        Some(window),
        gio::Cancellable::NONE,
        clone!(
            #[weak(rename_to = window)]
            window,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let outcome = std::fs::write(&path, &script).and_then(|()| {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                std::fs::set_permissions(
                                    &path,
                                    std::fs::Permissions::from_mode(0o755),
                                )
                            }
                            #[cfg(not(unix))]
                            Ok(())
                        });
                        match outcome {
                            Ok(()) => window.show_toast(&gettext("Undo script exported")),
                            Err(err) => {
                                window.show_info_dialog(&gettext("Export Failed"), &err.to_string())
                            }
                        }
                    }
                }
            }
        ),
    );
}
