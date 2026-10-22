//! Dialog components.

use gettextrs::gettext;
use libadwaita as adw;
use adw::prelude::*;
use gtk4 as gtk;

/// Application ID following reverse-DNS convention
const APP_ID: &str = "com.chrisdaggas.bulk-renamer";

/// Create the about dialog.
pub fn create_about_dialog() -> adw::AboutWindow {
    adw::AboutWindow::builder()
        .application_name(gettext("Bulk Renamer"))
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Christos A. Daggas")
        .license_type(gtk::License::MitX11)
        .website("https://chrisdaggas.com")
        .issue_url("https://github.com/christosdaggas/Bulk-Renamer/issues")
        .copyright("© 2024-2026 Christos A. Daggas")
        .developers(vec!["Christos A. Daggas"])
        .designers(vec!["Christos A. Daggas"])
        .build()
}

/// Create a progress dialog for long-running operations.
pub fn create_progress_dialog(parent: &impl IsA<gtk::Window>, title: &str) -> adw::MessageDialog {
    let dialog = adw::MessageDialog::new(Some(parent), Some(title), None);

    let box_ = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(24)
        .margin_end(24)
        .margin_top(12)
        .margin_bottom(12)
        .build();

    let progress = gtk::ProgressBar::builder()
        .show_text(true)
        .build();
    box_.append(&progress);

    let status = gtk::Label::builder()
        .label(gettext("Processing..."))
        .css_classes(vec!["dim-label"])
        .build();
    box_.append(&status);

    dialog.set_extra_child(Some(&box_));
    dialog.add_response("cancel", &gettext("Cancel"));

    dialog
}

