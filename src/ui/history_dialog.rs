//! Rename history browser with per-batch undo.

use super::window::RenamerWindow;
use gettextrs::gettext;
use libadwaita as adw;
use adw::prelude::*;
use gtk4 as gtk;

pub fn show(window: &RenamerWindow) {
    let entries = window.undo_history();
    if entries.is_empty() {
        window.show_info_dialog(
            &gettext("No History"),
            &gettext("No renames have been recorded yet."),
        );
        return;
    }

    let dialog = adw::Window::builder()
        .title(gettext("Rename History"))
        .default_width(520)
        .default_height(480)
        .modal(true)
        .transient_for(window)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(vec!["boxed-list"])
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();

    for (batch_id, title, subtitle) in entries {
        let row = adw::ActionRow::builder()
            .title(&title)
            .subtitle(&subtitle)
            .build();

        let undo_btn = gtk::Button::with_label(&gettext("Undo"));
        undo_btn.set_valign(gtk::Align::Center);
        undo_btn.add_css_class("flat");

        let window_clone = window.clone();
        let dialog_clone = dialog.clone();
        undo_btn.connect_clicked(move |_| {
            dialog_clone.close();
            super::execution::run_undo_batch(&window_clone, batch_id);
        });

        row.add_suffix(&undo_btn);
        list.append(&row);
    }

    let scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&list)
        .build();
    toolbar_view.set_content(Some(&scroll));
    dialog.set_content(Some(&toolbar_view));
    dialog.present();
}
