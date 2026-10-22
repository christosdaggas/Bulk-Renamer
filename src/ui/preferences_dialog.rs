//! Application preferences dialog.

use super::window::RenamerWindow;
use gettextrs::gettext;
use libadwaita as adw;
use adw::prelude::*;
use gtk4 as gtk;

/// Show the preferences window. Values are applied on Save.
pub fn show(window: &RenamerWindow) {
    let settings = window.settings_snapshot();
    let dialog = adw::Window::builder()
        .title(gettext("Preferences"))
        .default_width(520)
        .default_height(520)
        .modal(true)
        .transient_for(window)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let cancel_btn = gtk::Button::with_label(&gettext("Cancel"));
    cancel_btn.add_css_class("flat");
    let save_btn = gtk::Button::with_label(&gettext("Save"));
    save_btn.add_css_class("suggested-action");
    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);
    toolbar_view.add_top_bar(&header);

    let page = adw::PreferencesPage::new();

    let behavior = adw::PreferencesGroup::builder().title(gettext("Behavior")).build();
    let confirm_row = adw::SwitchRow::builder()
        .title(gettext("Confirm Before Rename"))
        .active(settings.confirm_before_rename)
        .build();
    let live_preview_row = adw::SwitchRow::builder()
        .title(gettext("Live Preview"))
        .subtitle(gettext("Recompute the preview automatically after every change"))
        .active(settings.live_preview)
        .build();
    let show_unchanged_row = adw::SwitchRow::builder()
        .title(gettext("Show Unchanged Files"))
        .active(settings.show_unchanged_files)
        .build();
    behavior.add(&confirm_row);
    behavior.add(&live_preview_row);
    behavior.add(&show_unchanged_row);
    page.add(&behavior);

    let files = adw::PreferencesGroup::builder().title(gettext("Files")).build();
    let hidden_row = adw::SwitchRow::builder()
        .title(gettext("Include Hidden Files"))
        .active(settings.show_hidden_files)
        .build();
    let symlink_row = adw::SwitchRow::builder()
        .title(gettext("Follow Symlinks"))
        .active(settings.follow_symlinks)
        .build();
    let metadata_row = adw::SwitchRow::builder()
        .title(gettext("Load Metadata"))
        .active(settings.metadata_loading_enabled)
        .build();
    let depth_row = adw::SpinRow::builder()
        .title(gettext("Folder Scan Depth"))
        .adjustment(&gtk::Adjustment::new(
            settings.recursive_folder_depth as f64,
            1.0,
            100.0,
            1.0,
            5.0,
            0.0,
        ))
        .build();
    files.add(&hidden_row);
    files.add(&symlink_row);
    files.add(&metadata_row);
    files.add(&depth_row);
    page.add(&files);

    let history = adw::PreferencesGroup::builder()
        .title(gettext("History and Logs"))
        .build();
    let undo_row = adw::SwitchRow::builder()
        .title(gettext("Persist Undo History"))
        .subtitle(gettext("Undo always works during a session; this keeps it across restarts"))
        .active(settings.undo_persistence_enabled)
        .build();
    let log_row = adw::SwitchRow::builder()
        .title(gettext("Log Rename Operations"))
        .active(settings.log_operations)
        .build();
    history.add(&undo_row);
    history.add(&log_row);
    page.add(&history);

    toolbar_view.set_content(Some(&page));
    dialog.set_content(Some(&toolbar_view));

    let dialog_clone = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        dialog_clone.close();
    });

    let dialog_clone = dialog.clone();
    let window = window.clone();
    save_btn.connect_clicked(move |_| {
        window.update_settings(|settings| {
            settings.confirm_before_rename = confirm_row.is_active();
            settings.live_preview = live_preview_row.is_active();
            settings.show_unchanged_files = show_unchanged_row.is_active();
            settings.show_hidden_files = hidden_row.is_active();
            settings.follow_symlinks = symlink_row.is_active();
            settings.metadata_loading_enabled = metadata_row.is_active();
            settings.recursive_folder_depth = depth_row.value() as usize;
            settings.undo_persistence_enabled = undo_row.is_active();
            settings.log_operations = log_row.is_active();
        });
        dialog_clone.close();
    });

    dialog.present();
}
