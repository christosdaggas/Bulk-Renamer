//! Main menu built from a gio::MenuModel.
//!
//! Menu items are wired to window actions, so their sensitivity follows the
//! action's enabled state. The theme selector is a custom child at the top.

use super::window::RenamerWindow;
use crate::core::types::ThemePreference;
use gettextrs::gettext;
use libadwaita as adw;
use adw::prelude::*;
use gtk4 as gtk;
use gtk::gio;
use gtk::glib;
use glib::prelude::ToVariant;

/// Build the primary menu popover for the header bar.
pub fn build(window: &RenamerWindow) -> gtk::PopoverMenu {
    let menu = gio::Menu::new();

    let theme_section = gio::Menu::new();
    let theme_item = gio::MenuItem::new(None, None);
    theme_item.set_attribute_value("custom", Some(&"theme-selector".to_variant()));
    theme_section.append_item(&theme_item);
    menu.append_section(None, &theme_section);

    let presets_section = gio::Menu::new();
    presets_section.append(Some(gettext("Save Preset…").as_str()), Some("win.save-preset"));
    presets_section.append(Some(gettext("Load Preset…").as_str()), Some("win.load-preset"));
    menu.append_section(None, &presets_section);

    let tools_section = gio::Menu::new();
    tools_section.append(Some(gettext("Import from CSV…").as_str()), Some("win.import-csv"));
    tools_section.append(Some(gettext("Export Preview as CSV…").as_str()), Some("win.export-preview"));
    tools_section.append(Some(gettext("Export Log…").as_str()), Some("win.export-log"));
    tools_section.append(Some(gettext("Export Undo Script…").as_str()), Some("win.export-undo-script"));
    menu.append_section(None, &tools_section);

    let history_section = gio::Menu::new();
    history_section.append(Some(gettext("Undo Last Rename").as_str()), Some("win.undo"));
    history_section.append(Some(gettext("Redo Rename").as_str()), Some("win.redo"));
    history_section.append(Some(gettext("Rename History…").as_str()), Some("win.history"));
    menu.append_section(None, &history_section);

    let app_section = gio::Menu::new();
    app_section.append(Some(gettext("Preferences").as_str()), Some("win.preferences"));
    app_section.append(Some(gettext("Keyboard Shortcuts").as_str()), Some("win.show-help-overlay"));
    app_section.append(Some(gettext("About Bulk Renamer").as_str()), Some("win.about"));
    menu.append_section(None, &app_section);

    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.add_child(&theme_selector(window), "theme-selector");
    popover
}

/// Three-way theme toggle (system / light / dark) with a checkmark overlay on
/// the active choice.
fn theme_selector(window: &RenamerWindow) -> gtk::Widget {
    let theme_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(18)
        .halign(gtk::Align::Center)
        .margin_top(6)
        .margin_bottom(6)
        .build();

    fn theme_content(css_class: &str, is_selected: bool) -> gtk::Overlay {
        let overlay = gtk::Overlay::new();
        let icon = gtk::Box::builder()
            .width_request(44)
            .height_request(44)
            .build();
        icon.add_css_class("theme-selector");
        icon.add_css_class(css_class);
        overlay.set_child(Some(&icon));
        if is_selected {
            let check = gtk::Image::from_icon_name("object-select-symbolic");
            check.add_css_class("theme-check");
            check.set_halign(gtk::Align::Center);
            check.set_valign(gtk::Align::Center);
            overlay.add_overlay(&check);
        }
        overlay
    }

    let choices: [(ThemePreference, &str, &str); 3] = [
        (ThemePreference::System, "theme-default", "System"),
        (ThemePreference::Light, "theme-light", "Light"),
        (ThemePreference::Dark, "theme-dark", "Dark"),
    ];

    let style_manager = adw::StyleManager::default();
    let active = match style_manager.color_scheme() {
        adw::ColorScheme::ForceLight => 1,
        adw::ColorScheme::ForceDark => 2,
        _ => 0,
    };

    let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (idx, (_, css_class, tooltip)) in choices.iter().enumerate() {
        let button = gtk::ToggleButton::new();
        button.set_child(Some(&theme_content(css_class, idx == active)));
        button.set_tooltip_text(Some(gettext(*tooltip).as_str()));
        button.add_css_class("flat");
        button.add_css_class("circular");
        button.add_css_class("theme-button");
        if let Some(first) = buttons.first() {
            button.set_group(Some(first));
        }
        buttons.push(button);
    }
    buttons[active].set_active(true);

    for (idx, button) in buttons.iter().enumerate() {
        let (preference, _, _) = choices[idx];
        let window = window.clone();
        let all_buttons = buttons.clone();
        button.connect_toggled(move |btn| {
            if btn.is_active() {
                window.set_theme(preference);
                for (b_idx, other) in all_buttons.iter().enumerate() {
                    let (_, css_class, _) = choices[b_idx];
                    other.set_child(Some(&theme_content(css_class, b_idx == idx)));
                }
            }
        });
    }

    for button in &buttons {
        theme_box.append(button);
    }
    theme_box.into()
}
