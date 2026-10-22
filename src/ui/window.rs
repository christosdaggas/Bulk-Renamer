//! Main application window with three-panel layout.

use crate::core::{AppSettings, FileEntry, RenameBatch, RenameConfig, RenamePreview, RenameStatus, RenameTarget};
use crate::core::types::ThemePreference;
use crate::engine::{RenameEngine, RenameValidator};
use crate::presets::Preset;
use crate::undo::{RenameLogEntry, RenameLogger, UndoManager, UndoResult};
use super::file_item::FileItem;
use gettextrs::gettext;
use glib::closure;
use async_channel;
use libadwaita as adw;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk::{gio, glib};
use glib::clone;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RenamerWindow {
        pub previews: RefCell<Vec<RenamePreview>>,
        pub config: RefCell<RenameConfig>,
        pub target: RefCell<RenameTarget>,
        pub settings: RefCell<AppSettings>,
        pub store: RefCell<Option<gio::ListStore>>,
        pub file_selection: RefCell<Option<gtk::MultiSelection>>,
        pub file_list: RefCell<Option<gtk::ListView>>,
        pub files_stack: RefCell<Option<gtk::Stack>>,
        pub preview_filter: RefCell<Option<gtk::CustomFilter>>,
        pub rules_list: RefCell<Option<gtk::ListBox>>,
        pub files_count_label: RefCell<Option<gtk::Label>>,
        pub selected_count_label: RefCell<Option<gtk::Label>>,
        pub preview_count_label: RefCell<Option<gtk::Label>>,
        pub rename_button: RefCell<Option<gtk::Button>>,
        pub undo_manager: RefCell<UndoManager>,
        pub logger: RefCell<RenameLogger>,
        pub preview_pending: RefCell<Option<glib::SourceId>>,
        pub busy: Cell<bool>,
        pub toast_overlay: RefCell<Option<adw::ToastOverlay>>,
        pub conflict_banner: RefCell<Option<adw::Banner>>,
        pub auto_resolve_conflicts: Cell<bool>,
        pub search_text: RefCell<String>,
        pub extension_filter: RefCell<String>,
        /// Bumped by Clear All; a folder scan started under an older value is
        /// stale and its result is dropped.
        pub scan_generation: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RenamerWindow {
        const NAME: &'static str = "GnomeRenamerWindow";
        type Type = super::RenamerWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for RenamerWindow {}
    impl WidgetImpl for RenamerWindow {}
    impl WindowImpl for RenamerWindow {}
    impl ApplicationWindowImpl for RenamerWindow {}
    impl AdwApplicationWindowImpl for RenamerWindow {}
}
glib::wrapper! {
    pub struct RenamerWindow(ObjectSubclass<imp::RenamerWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl RenamerWindow {
    pub fn new(app: &impl IsA<gtk::Application>) -> Self {
        let window: Self = glib::Object::builder()
            .property("application", app)
            .property("default-width", 1400)
            .property("default-height", 800)
            .property("title", gettext("Bulk Renamer"))
            .build();

        window.setup_ui();
        window.setup_actions();
        window.setup_help_overlay();
        window.load_settings();
        window.check_interrupted_renames();

        window
    }

    /// Keyboard shortcuts window, reachable via Ctrl+? and the main menu.
    fn setup_help_overlay(&self) {
        const SHORTCUTS_UI: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<interface>
  <object class="GtkShortcutsWindow" id="shortcuts">
    <child>
      <object class="GtkShortcutsSection">
        <property name="section-name">shortcuts</property>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">Files</property>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Add Files</property>
                <property name="accelerator">&lt;Control&gt;o</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Add Folder</property>
                <property name="accelerator">&lt;Control&gt;&lt;Shift&gt;o</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Clear File List</property>
                <property name="accelerator">&lt;Control&gt;&lt;Shift&gt;Delete</property>
              </object>
            </child>
          </object>
        </child>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">Renaming</property>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Rename Files</property>
                <property name="accelerator">&lt;Control&gt;Return</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Undo Last Rename</property>
                <property name="accelerator">&lt;Control&gt;z</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Redo Rename</property>
                <property name="accelerator">&lt;Control&gt;&lt;Shift&gt;z</property>
              </object>
            </child>
          </object>
        </child>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">Quick Rules</property>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Lowercase</property>
                <property name="accelerator">&lt;Control&gt;1</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Uppercase</property>
                <property name="accelerator">&lt;Control&gt;2</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Title Case</property>
                <property name="accelerator">&lt;Control&gt;3</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Numbering</property>
                <property name="accelerator">&lt;Control&gt;4</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Tidy Up Names</property>
                <property name="accelerator">&lt;Control&gt;5</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Clear All Rules</property>
                <property name="accelerator">&lt;Control&gt;&lt;Shift&gt;k</property>
              </object>
            </child>
          </object>
        </child>
        <child>
          <object class="GtkShortcutsGroup">
            <property name="title">Presets and Tools</property>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Save Preset</property>
                <property name="accelerator">&lt;Control&gt;s</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Load Preset</property>
                <property name="accelerator">&lt;Control&gt;l</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Preferences</property>
                <property name="accelerator">&lt;Control&gt;comma</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Keyboard Shortcuts</property>
                <property name="accelerator">&lt;Control&gt;question</property>
              </object>
            </child>
            <child>
              <object class="GtkShortcutsShortcut">
                <property name="title">Quit</property>
                <property name="accelerator">&lt;Control&gt;q</property>
              </object>
            </child>
          </object>
        </child>
      </object>
    </child>
  </object>
</interface>"##;

        let builder = gtk::Builder::from_string(SHORTCUTS_UI);
        if let Some(shortcuts) = builder.object::<gtk::ShortcutsWindow>("shortcuts") {
            self.set_help_overlay(Some(&shortcuts));
        }
    }

    /// Offer to restore files an interrupted batch left under staging names.
    fn check_interrupted_renames(&self) {
        let data_dir = crate::undo::default_data_dir();
        let count = crate::engine::pending_recovery_count(&data_dir);
        if count == 0 {
            return;
        }

        let dialog = adw::MessageDialog::new(
            Some(self),
            Some(gettext("Interrupted Rename Detected").as_str()),
            Some(
                gettext(
                    "A previous rename was interrupted and left {} file(s) under temporary names. \
                     Restore them to their original names now?",
                )
                .replacen("{}", &count.to_string(), 1)
                .as_str(),
            ),
        );
        dialog.add_response("later", &gettext("Later"));
        dialog.add_response("recover", &gettext("Restore"));
        dialog.set_response_appearance("recover", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("recover"));

        dialog.connect_response(None, clone!(
            #[weak(rename_to = window)]
            self,
            move |dialog, response| {
                if response == "recover" {
                    let outcome =
                        crate::engine::recover_interrupted(&crate::undo::default_data_dir());
                    let mut message = gettext("Restored {} file(s).")
                        .replacen("{}", &outcome.restored.len().to_string(), 1);
                    for (path, reason) in outcome.failed.iter().take(5) {
                        message.push_str(&format!("\n\n{}: {}", path.display(), reason));
                    }
                    if outcome.failed.len() > 5 {
                        message.push_str("\n\n");
                        message.push_str(
                            &gettext("…and {} more.")
                                .replacen("{}", &(outcome.failed.len() - 5).to_string(), 1),
                        );
                    }
                    let title = if outcome.failed.is_empty() {
                        gettext("Recovery Complete")
                    } else {
                        gettext("Recovery Incomplete")
                    };
                    window.show_info_dialog(&title, &message);
                }
                dialog.close();
            }
        ));
        dialog.present();
    }

    fn setup_ui(&self) {
        // Create header bar
        let header = self.create_header_bar();

        // Create main content with three panels
        let content = self.create_main_content();

        // Create toolbar view
        // The header bar is flat and the toolbar view behind it gets the panels'
        // background (style.css), so title area and content read as one surface.
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.set_top_bar_style(adw::ToolbarStyle::Flat);
        toolbar_view.add_css_class("main-view");
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content));

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&toolbar_view));
        self.imp().toast_overlay.replace(Some(toast_overlay.clone()));

        self.set_content(Some(&toast_overlay));

        // Closing mid-batch would end the process with files still under their
        // temporary names, so the batch finishes first.
        self.connect_close_request(|window| {
            if window.is_busy() {
                window.show_toast(&gettext("Wait until the current operation finishes"));
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }

    fn create_header_bar(&self) -> adw::HeaderBar {
        let header = adw::HeaderBar::new();

        // Title
        let title = adw::WindowTitle::new(&gettext("Bulk Renamer"), "");
        header.set_title_widget(Some(&title));

        // Right side: Rename button and menu
        let rename_btn = gtk::Button::builder()
            .label(gettext("Rename"))
            .sensitive(false)
            .build();
        rename_btn.add_css_class("suggested-action");

        rename_btn.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.execute_rename();
            }
        ));

        // Menu button with custom popover
        let menu_btn = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text(gettext("Menu"))
            .build();
        menu_btn.add_css_class("flat");

        // Create custom popover with theme selector
        let popover = super::menu::build(self);
        menu_btn.set_popover(Some(&popover));

        header.pack_end(&menu_btn);
        header.pack_end(&rename_btn);
        self.imp().rename_button.replace(Some(rename_btn));

        header
    }

    fn show_about_dialog(&self) {
        let about = adw::AboutWindow::builder()
            .transient_for(self)
            .application_name(gettext("Bulk Renamer"))
            .application_icon("com.chrisdaggas.bulk-renamer")
            .version(env!("CARGO_PKG_VERSION"))
            .developer_name("Christos A. Daggas")
            .license_type(gtk::License::MitX11)
            .website("https://chrisdaggas.com")
            .issue_url("https://github.com/christosdaggas/Bulk-Renamer/issues")
            .copyright("© 2024-2026 Christos A. Daggas")
            .comments(gettext("A powerful bulk file renaming application for Linux"))
            .build();

        about.add_credit_section(Some(gettext("Created by").as_str()), &["Christos A. Daggas"]);
        about.present();
    }

    fn create_main_content(&self) -> gtk::Widget {
        // Main horizontal box with three panels
        let main_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(0)
            .build();

        // Left panel: File browser
        let files_panel = self.create_files_panel();
        files_panel.set_width_request(320);
        
        // Center panel: Rules
        let rules_panel = self.create_rules_panel();
        rules_panel.set_width_request(300);
        
        // Right panel: Preview
        let preview_panel = self.create_preview_panel();
        preview_panel.set_hexpand(true);

        // Add separators
        let sep1 = gtk::Separator::new(gtk::Orientation::Vertical);
        let sep2 = gtk::Separator::new(gtk::Orientation::Vertical);

        main_box.append(&files_panel);
        main_box.append(&sep1);
        main_box.append(&rules_panel);
        main_box.append(&sep2);
        main_box.append(&preview_panel);

        main_box.into()
    }

    fn create_files_panel(&self) -> gtk::Widget {
        let panel = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .build();
        panel.add_css_class("view");

        // Header with title and buttons
        let header_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(6)
            .build();

        let title_label = gtk::Label::builder()
            .label(gettext("Files"))
            .css_classes(vec!["title-4"])
            .hexpand(true)
            .xalign(0.0)
            .build();

        let add_files_btn = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text(gettext("Add Files"))
            .build();
        add_files_btn.add_css_class("flat");
        add_files_btn.add_css_class("circular");

        let add_folder_btn = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text(gettext("Add Folder"))
            .build();
        add_folder_btn.add_css_class("flat");
        add_folder_btn.add_css_class("circular");

        let clear_btn = gtk::Button::builder()
            .icon_name("edit-clear-all-symbolic")
            .tooltip_text(gettext("Clear All"))
            .build();
        clear_btn.add_css_class("flat");
        clear_btn.add_css_class("circular");

        // Sorting reorders the store itself: batch order drives numbering.
        let sort_menu = gio::Menu::new();
        for (label, target) in [
            ("Name (A to Z)", "name-asc"),
            ("Name (Z to A)", "name-desc"),
            ("Smallest First", "size-asc"),
            ("Largest First", "size-desc"),
            ("Oldest First", "modified-asc"),
            ("Newest First", "modified-desc"),
        ] {
            let item = gio::MenuItem::new(Some(gettext(label).as_str()), None);
            item.set_action_and_target_value(
                Some("win.sort-files"),
                Some(&target.to_variant()),
            );
            sort_menu.append_item(&item);
        }
        let sort_btn = gtk::MenuButton::builder()
            .icon_name("view-sort-descending-symbolic")
            .tooltip_text(gettext("Sort Files"))
            .menu_model(&sort_menu)
            .build();
        sort_btn.add_css_class("flat");
        sort_btn.add_css_class("circular");

        header_box.append(&title_label);
        header_box.append(&add_files_btn);
        header_box.append(&add_folder_btn);
        header_box.append(&sort_btn);
        header_box.append(&clear_btn);
        panel.append(&header_box);

        // Status bar
        let status_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(6)
            .spacing(12)
            .build();

        let files_count = gtk::Label::builder()
            .label(gettext("{} files").replacen("{}", "0", 1))
            .css_classes(vec!["dim-label", "caption"])
            .build();

        let selected_count = gtk::Label::builder()
            .label(gettext("{} selected").replacen("{}", "0", 1))
            .css_classes(vec!["dim-label", "caption"])
            .build();

        status_box.append(&files_count);
        status_box.append(&selected_count);
        panel.append(&status_box);

        self.imp().files_count_label.replace(Some(files_count));
        self.imp().selected_count_label.replace(Some(selected_count));

        // Search + extension filter: both feed the engine's FilterConfig, so
        // non-matching files simply stay unchanged in the batch.
        let filter_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(6)
            .spacing(6)
            .build();

        let search_entry = gtk::SearchEntry::builder()
            .placeholder_text(gettext("Filter by name (use * and ? for globs)"))
            .hexpand(true)
            .build();
        search_entry.connect_search_changed(clone!(
            #[weak(rename_to = window)]
            self,
            move |entry| {
                window.imp().search_text.replace(entry.text().to_string());
                window.rebuild_filter();
            }
        ));

        let ext_entry = gtk::Entry::builder()
            .placeholder_text(gettext("Extensions: jpg, png"))
            .width_chars(14)
            .tooltip_text(gettext("Only rename files with these extensions (comma-separated)"))
            .build();
        ext_entry.connect_changed(clone!(
            #[weak(rename_to = window)]
            self,
            move |entry| {
                window.imp().extension_filter.replace(entry.text().to_string());
                window.rebuild_filter();
            }
        ));

        filter_box.append(&search_entry);
        filter_box.append(&ext_entry);
        panel.append(&filter_box);

        // Virtualized file list over the shared FileItem store.
        let store = gio::ListStore::new::<FileItem>();
        let selection = gtk::MultiSelection::new(Some(store.clone()));

        let factory = gtk::SignalListItemFactory::new();
        let window_for_factory = self.downgrade();
        factory.connect_setup(move |_, list_item| {
            let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let row_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_start(12)
                .margin_end(12)
                .margin_top(8)
                .margin_bottom(8)
                .build();

            // Per-file inclusion: unchecked files are skipped by the batch.
            let include_check = gtk::CheckButton::builder()
                .tooltip_text(gettext("Include in the rename"))
                .build();
            let icon = gtk::Image::new();
            icon.add_css_class("dim-label");
            let name_label = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();
            let size_label = gtk::Label::builder()
                .css_classes(vec!["dim-label", "caption"])
                .build();

            row_box.append(&include_check);
            row_box.append(&icon);
            row_box.append(&name_label);
            row_box.append(&size_label);
            list_item.set_child(Some(&row_box));

            let item_expr = list_item.property_expression("item");
            item_expr
                .chain_property::<FileItem>("included")
                .bind(&include_check, "active", None::<&gtk::Widget>);
            item_expr
                .chain_property::<FileItem>("icon-name")
                .bind(&icon, "icon-name", None::<&gtk::Widget>);
            item_expr
                .chain_property::<FileItem>("original-name")
                .bind(&name_label, "label", None::<&gtk::Widget>);
            item_expr
                .chain_property::<FileItem>("size-text")
                .bind(&size_label, "label", None::<&gtk::Widget>);

            let list_item_weak = glib::clone::Downgrade::downgrade(list_item);
            let window_weak = window_for_factory.clone();
            include_check.connect_toggled(move |check| {
                let Some(list_item) = list_item_weak.upgrade() else { return };
                let Some(item) = list_item.item().and_downcast::<FileItem>() else {
                    return;
                };
                if item.included() != check.is_active() {
                    item.set_included(check.is_active());
                    if let Some(window) = window_weak.upgrade() {
                        window.request_preview();
                    }
                }
            });
        });

        let file_list = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .show_separators(true)
            .css_classes(vec!["card-list"])
            .build();

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&file_list)
            .build();

        // Empty state and list share a stack; refresh_file_count switches them.
        let placeholder = adw::StatusPage::builder()
            .icon_name("folder-documents-symbolic")
            .title(gettext("No Files"))
            .description(gettext("Drop files here or click + to add"))
            .vexpand(true)
            .build();
        placeholder.add_css_class("compact");

        // The list and its empty state sit on a card, like the Preview list.
        let stack = gtk::Stack::builder()
            .vexpand(true)
            .margin_start(12)
            .margin_end(12)
            .css_classes(vec!["card"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        stack.add_named(&placeholder, Some("empty"));
        stack.add_named(&scroll, Some("list"));
        stack.set_visible_child_name("empty");
        panel.append(&stack);

        selection.connect_selection_changed(clone!(
            #[weak(rename_to = window)]
            self,
            move |sel, _, _| {
                window.update_selected_count(sel.selection().size() as usize);
            }
        ));

        self.imp().store.replace(Some(store));
        self.imp().file_selection.replace(Some(selection.clone()));
        self.imp().file_list.replace(Some(file_list.clone()));
        self.imp().files_stack.replace(Some(stack));

        // Bottom action bar
        let action_bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(12)
            .spacing(6)
            .build();

        let select_all_btn = gtk::Button::builder()
            .label(gettext("Select All"))
            .hexpand(true)
            .build();
        select_all_btn.add_css_class("flat");

        let remove_selected_btn = gtk::Button::builder()
            .label(gettext("Remove Selected"))
            .hexpand(true)
            .build();
        remove_selected_btn.add_css_class("flat");

        action_bar.append(&select_all_btn);
        action_bar.append(&remove_selected_btn);
        panel.append(&action_bar);

        // Connect button signals
        add_files_btn.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.show_add_files_dialog();
            }
        ));

        add_folder_btn.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.show_add_folder_dialog();
            }
        ));

        clear_btn.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.clear_files();
            }
        ));

        select_all_btn.connect_clicked(clone!(
            #[weak]
            selection,
            move |_| {
                selection.select_all();
            }
        ));

        remove_selected_btn.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.remove_selected_files();
            }
        ));

        // Setup drag and drop
        // Accept gdk::FileList first: text/uri-list also deserializes to a single
        // gio::File, which would silently drop every file but the first.
        let drop_target = gtk::DropTarget::new(glib::Type::INVALID, gdk4::DragAction::COPY);
        drop_target.set_types(&[gdk4::FileList::static_type(), gio::File::static_type()]);
        drop_target.connect_drop(clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                let paths = Self::paths_from_drop_value(value);
                if paths.is_empty() {
                    return false;
                }
                window.add_paths(paths);
                true
            }
        ));
        // Highlight the whole panel while a drag hovers so the drop zone is visible.
        let panel_weak = panel.downgrade();
        drop_target.connect_enter(move |_, _, _| {
            if let Some(panel) = panel_weak.upgrade() {
                panel.add_css_class("drop-zone");
            }
            gdk4::DragAction::COPY
        });
        let panel_weak = panel.downgrade();
        drop_target.connect_leave(move |_| {
            if let Some(panel) = panel_weak.upgrade() {
                panel.remove_css_class("drop-zone");
            }
        });
        // On the whole panel, not the list: while the list is empty the panel shows
        // the "drop files here" page instead, and the list cannot receive a drop.
        panel.add_controller(drop_target);

        panel.into()
    }

    fn create_rules_panel(&self) -> gtk::Widget {
        let panel = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .build();
        panel.add_css_class("view");

        // Header
        let header_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(6)
            .build();

        let title_label = gtk::Label::builder()
            .label(gettext("Rules"))
            .css_classes(vec!["title-4"])
            .hexpand(true)
            .xalign(0.0)
            .build();

        let add_rule_btn = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text(gettext("Add Rule"))
            .build();
        add_rule_btn.add_css_class("flat");
        add_rule_btn.add_css_class("circular");

        let clear_rules_btn = gtk::Button::builder()
            .icon_name("edit-clear-all-symbolic")
            .tooltip_text(gettext("Clear All Rules"))
            .build();
        clear_rules_btn.add_css_class("flat");
        clear_rules_btn.add_css_class("circular");
        clear_rules_btn.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.clear_all_rules();
            }
        ));

        header_box.append(&title_label);
        header_box.append(&add_rule_btn);
        header_box.append(&clear_rules_btn);
        panel.append(&header_box);

        // Target type selector
        let target_group = adw::PreferencesGroup::new();
        target_group.set_margin_start(12);
        target_group.set_margin_end(12);
        target_group.set_margin_bottom(6);

        let target_row = adw::ComboRow::builder()
            .title(gettext("Apply to"))
            .model(&gtk::StringList::new(&[
                gettext("Files only").as_str(),
                gettext("Folders only").as_str(),
                gettext("Both").as_str(),
            ]))
            .build();
        target_group.add(&target_row);
        panel.append(&target_group);

        // Rules list. Never scroll sideways: a row that is too wide must shrink
        // its labels instead of pushing its buttons past the box edge.
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();

        let rules_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(vec!["boxed-list"])
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();

        // Enable drag and drop reordering
        rules_list.set_show_separators(false);

        rules_list.set_placeholder(Some(&gtk::Label::builder()
            .label(gettext("No rules added\nClick + to add a rule"))
            .css_classes(vec!["dim-label"])
            .margin_top(24)
            .margin_bottom(24)
            .justify(gtk::Justification::Center)
            .build()));

        scroll.set_child(Some(&rules_list));
        panel.append(&scroll);

        // Store rules_list for later reference
        self.imp().rules_list.replace(Some(rules_list.clone()));

        // Options
        let options_group = adw::PreferencesGroup::new();
        options_group.set_margin_start(12);
        options_group.set_margin_end(12);
        options_group.set_margin_bottom(12);

        let ext_switch = adw::SwitchRow::builder()
            .title(gettext("Process extension separately"))
            .active(true)
            .build();
        options_group.add(&ext_switch);

        panel.append(&options_group);

        // Connect add rule button
        add_rule_btn.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            rules_list,
            move |_| {
                window.show_add_rule_dialog(&rules_list);
            }
        ));

        target_row.connect_selected_notify(clone!(
            #[weak(rename_to = window)]
            self,
            move |row| {
                let target = match row.selected() {
                    1 => RenameTarget::FoldersOnly,
                    2 => RenameTarget::Both,
                    _ => RenameTarget::FilesOnly,
                };
                *window.imp().target.borrow_mut() = target;
                window.request_preview();
            }
        ));

        ext_switch.connect_active_notify(clone!(
            #[weak(rename_to = window)]
            self,
            move |row| {
                window.imp().config.borrow_mut().separate_extension = row.is_active();
                window.request_preview();
            }
        ));

        panel.into()
    }

    fn create_preview_panel(&self) -> gtk::Widget {
        let panel = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .build();
        panel.add_css_class("view");

        // Header
        let header_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(6)
            .build();

        let title_label = gtk::Label::builder()
            .label(gettext("Preview"))
            .css_classes(vec!["title-4"])
            .hexpand(true)
            .xalign(0.0)
            .build();

        let refresh_btn = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text(gettext("Refresh Preview"))
            .build();
        refresh_btn.add_css_class("flat");
        refresh_btn.add_css_class("circular");

        header_box.append(&title_label);
        header_box.append(&refresh_btn);
        panel.append(&header_box);

        // Stats bar
        let stats_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(6)
            .spacing(12)
            .build();

        let preview_count = gtk::Label::builder()
            .label(gettext("0 will be renamed"))
            .css_classes(vec!["dim-label", "caption"])
            .build();

        stats_box.append(&preview_count);
        panel.append(&stats_box);

        self.imp().preview_count_label.replace(Some(preview_count));

        // Conflict banner: one click renames collisions to "name (n)".
        let banner = adw::Banner::new("");
        banner.set_button_label(Some(gettext("Add Numbers").as_str()));
        banner.connect_button_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.imp().auto_resolve_conflicts.set(true);
                window.update_preview();
                window.show_toast(&gettext("Collisions are renamed with (n) suffixes"));
            }
        ));
        panel.append(&banner);
        self.imp().conflict_banner.replace(Some(banner));

        // The preview is the same FileItem store seen through a filter; cells
        // bind to item properties, so preview refreshes never rebuild widgets.
        let store = self
            .imp()
            .store
            .borrow()
            .clone()
            .expect("files panel builds the store before the preview panel");

        let window_weak = self.downgrade();
        let filter = gtk::CustomFilter::new(move |obj| {
            let Some(window) = window_weak.upgrade() else {
                return true;
            };
            let Some(item) = obj.downcast_ref::<FileItem>() else {
                return true;
            };
            window.imp().settings.borrow().show_unchanged_files
                || item.status_code() != FileItem::STATUS_UNCHANGED
        });
        let filter_model = gtk::FilterListModel::new(Some(store), Some(filter.clone()));
        self.imp().preview_filter.replace(Some(filter));

        let selection_model = gtk::NoSelection::new(Some(filter_model.clone()));

        // Captions above the list.
        let captions = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .homogeneous(true)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(6)
            .build();
        for caption in [gettext("Original"), gettext("New Name")] {
            captions.append(
                &gtk::Label::builder()
                    .label(caption)
                    .xalign(0.0)
                    .css_classes(vec!["dim-label", "caption"])
                    .build(),
            );
        }
        panel.append(&captions);

        // One row per file on a card: status icon, the original name, and the new
        // name under it. Names wrap instead of being cut off, so the list never
        // scrolls sideways and keeps the same 12px gutters as the other panels.
        let factory = gtk::SignalListItemFactory::new();
        let window_for_names = self.downgrade();
        factory.connect_setup(move |_, list_item| {
            let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            list_item.set_activatable(false);

            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .margin_start(12)
                .margin_end(12)
                .margin_top(8)
                .margin_bottom(8)
                .build();

            let status = gtk::Image::builder().valign(gtk::Align::Start).build();
            row.append(&status);

            let names = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(2)
                .hexpand(true)
                .build();
            let original = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .build();
            names.append(&original);
            names.append(&new_name_editor(list_item, window_for_names.clone()));
            row.append(&names);
            list_item.set_child(Some(&row));

            let item_expr = list_item.property_expression("item");
            item_expr
                .chain_property::<FileItem>("original-name")
                .bind(&original, "label", None::<&gtk::Widget>);
            item_expr
                .chain_property::<FileItem>("status-icon")
                .bind(&status, "icon-name", None::<&gtk::Widget>);
            item_expr
                .chain_property::<FileItem>("tooltip")
                .bind(&status, "tooltip-text", None::<&gtk::Widget>);
            item_expr
                .chain_property::<FileItem>("status-css")
                .chain_closure::<Vec<String>>(closure!(
                    |_: Option<glib::Object>, css: String| vec![css]
                ))
                .bind(&status, "css-classes", None::<&gtk::Widget>);
        });

        let list_view = gtk::ListView::builder()
            .model(&selection_model)
            .factory(&factory)
            .show_separators(true)
            .css_classes(vec!["card-list"])
            .build();

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list_view)
            .css_classes(vec!["card", "preview-card"])
            .overflow(gtk::Overflow::Hidden)
            .build();

        // An empty card says why it is empty, like the rules list does.
        let placeholder = gtk::Label::builder()
            .label(gettext("No files to preview"))
            .css_classes(vec!["dim-label"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .can_target(false)
            .build();
        placeholder.set_visible(filter_model.n_items() == 0);
        filter_model.connect_items_changed(clone!(
            #[weak]
            placeholder,
            move |model, _, _, _| placeholder.set_visible(model.n_items() == 0)
        ));

        let overlay = gtk::Overlay::builder()
            .child(&scroll)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(12)
            .build();
        overlay.add_overlay(&placeholder);
        panel.append(&overlay);

        // Refresh button handler
        refresh_btn.connect_clicked(clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.update_preview();
            }
        ));

        panel.into()
    }

    fn setup_actions(&self) {
        // Execute rename action
        let execute_action = gio::SimpleAction::new("execute-rename", None);
        execute_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.execute_rename();
            }
        ));
        self.add_action(&execute_action);

        // Add files/folders actions
        let add_files_action = gio::SimpleAction::new("add-files", None);
        add_files_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.show_add_files_dialog();
            }
        ));
        self.add_action(&add_files_action);

        let add_folder_action = gio::SimpleAction::new("add-folder", None);
        add_folder_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.show_add_folder_dialog();
            }
        ));
        self.add_action(&add_folder_action);

        let clear_files_action = gio::SimpleAction::new("clear-files", None);
        clear_files_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.clear_files();
            }
        ));
        self.add_action(&clear_files_action);

        // Undo/redo actions
        let undo_action = gio::SimpleAction::new("undo", None);
        undo_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.undo_last_batch();
            }
        ));
        self.add_action(&undo_action);

        let redo_action = gio::SimpleAction::new("redo", None);
        redo_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.redo_last_batch();
            }
        ));
        self.add_action(&redo_action);

        // Preferences action
        let preferences_action = gio::SimpleAction::new("preferences", None);
        preferences_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.show_preferences_dialog();
            }
        ));
        self.add_action(&preferences_action);

        // Save preset action
        let save_preset_action = gio::SimpleAction::new("save-preset", None);
        save_preset_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.show_save_preset_dialog();
            }
        ));
        self.add_action(&save_preset_action);

        // Load preset action
        let load_preset_action = gio::SimpleAction::new("load-preset", None);
        load_preset_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.show_load_preset_dialog();
            }
        ));
        self.add_action(&load_preset_action);

        // Import CSV action
        let import_csv_action = gio::SimpleAction::new("import-csv", None);
        import_csv_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.show_import_csv_dialog();
            }
        ));
        self.add_action(&import_csv_action);

        // Export log action
        let export_log_action = gio::SimpleAction::new("export-log", None);
        export_log_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.show_export_log_dialog();
            }
        ));
        self.add_action(&export_log_action);

        let about_action = gio::SimpleAction::new("about", None);
        about_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.show_about_dialog();
            }
        ));
        self.add_action(&about_action);

        self.add_quick_rule_action("quick-lowercase", crate::core::CaseType::Lower);
        self.add_quick_rule_action("quick-uppercase", crate::core::CaseType::Upper);
        self.add_quick_rule_action("quick-titlecase", crate::core::CaseType::Title);

        let quick_cleanup_action = gio::SimpleAction::new("quick-cleanup", None);
        quick_cleanup_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.commit_rule(
                    crate::core::RuleType::Cleanup(crate::core::CleanupRule::default()),
                    None,
                );
            }
        ));
        self.add_action(&quick_cleanup_action);

        let history_action = gio::SimpleAction::new("history", None);
        history_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                super::history_dialog::show(&window);
            }
        ));
        self.add_action(&history_action);

        let export_preview_action = gio::SimpleAction::new("export-preview", None);
        export_preview_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                super::csv_io::show_export_preview_dialog(&window);
            }
        ));
        self.add_action(&export_preview_action);

        let export_script_action = gio::SimpleAction::new("export-undo-script", None);
        export_script_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                super::csv_io::show_export_undo_script_dialog(&window);
            }
        ));
        self.add_action(&export_script_action);

        let sort_action =
            gio::SimpleAction::new("sort-files", Some(glib::VariantTy::STRING));
        sort_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, param| {
                use crate::core::{SortColumn, SortDirection};
                let key = param.and_then(|v| v.str()).unwrap_or_default();
                let (column, direction) = match key {
                    "name-desc" => (SortColumn::OriginalName, SortDirection::Descending),
                    "size-asc" => (SortColumn::Size, SortDirection::Ascending),
                    "size-desc" => (SortColumn::Size, SortDirection::Descending),
                    "modified-asc" => (SortColumn::Modified, SortDirection::Ascending),
                    "modified-desc" => (SortColumn::Modified, SortDirection::Descending),
                    _ => (SortColumn::OriginalName, SortDirection::Ascending),
                };
                window.sort_store(column, direction);
            }
        ));
        self.add_action(&sort_action);

        let clear_rules_action = gio::SimpleAction::new("clear-rules", None);
        clear_rules_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                window.clear_all_rules();
            }
        ));
        self.add_action(&clear_rules_action);

        let quick_number_action = gio::SimpleAction::new("quick-number", None);
        quick_number_action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(rules_list) = window.imp().rules_list.borrow().as_ref() {
                    window.add_numbering_rule_at(rules_list, 1, 1, 2, 1, "_".to_string(), None);
                }
            }
        ));
        self.add_action(&quick_number_action);
    }

    fn add_quick_rule_action(&self, name: &str, case_type: crate::core::CaseType) {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| {
                if let Some(rules_list) = window.imp().rules_list.borrow().as_ref() {
                    window.add_case_rule_at(rules_list, case_type as usize, None);
                }
            }
        ));
        self.add_action(&action);
    }

    fn load_settings(&self) {
        // Load settings from disk
        let settings = AppSettings::load();
        
        // Apply theme preference
        let style_manager = adw::StyleManager::default();
        match settings.theme {
            ThemePreference::System => style_manager.set_color_scheme(adw::ColorScheme::Default),
            ThemePreference::Light => style_manager.set_color_scheme(adw::ColorScheme::ForceLight),
            ThemePreference::Dark => style_manager.set_color_scheme(adw::ColorScheme::ForceDark),
        }
        self.imp().logger.borrow_mut().set_enabled(settings.log_operations);
        // The toggle only controls disk persistence; in-session undo always works.
        {
            let mut manager = self.imp().undo_manager.borrow_mut();
            manager.set_persistence(settings.undo_persistence_enabled);
            if settings.undo_persistence_enabled {
                if let Err(err) = manager.load_from_disk() {
                    tracing::error!("Failed to load undo history: {}", err);
                }
            }
        }

        self.imp().settings.replace(settings);
        self.update_history_actions();
    }

    fn save_settings(&self) {
        let settings = self.imp().settings.borrow();
        if let Err(e) = settings.save() {
            tracing::error!("Failed to save settings: {}", e);
        }
    }

    /// Called during application shutdown to save any pending state
    pub fn save_on_shutdown(&self) {
        tracing::debug!("Saving window state on shutdown");
        self.save_settings();
    }

    pub(crate) fn set_theme(&self, theme: ThemePreference) {
        let style_manager = adw::StyleManager::default();
        match theme {
            ThemePreference::System => style_manager.set_color_scheme(adw::ColorScheme::Default),
            ThemePreference::Light => style_manager.set_color_scheme(adw::ColorScheme::ForceLight),
            ThemePreference::Dark => style_manager.set_color_scheme(adw::ColorScheme::ForceDark),
        }
        
        // Update settings and save
        self.imp().settings.borrow_mut().theme = theme;
        self.save_settings();
    }

    // ============ File Operations ============

    pub fn show_add_files_dialog(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Select Files"))
            // Not modal: GNOME attaches modal dialogs to their parent window, so
            // the chooser could only be dragged together with the whole app.
            .modal(false)
            .build();

        dialog.open_multiple(Some(self), gio::Cancellable::NONE, clone!(
            #[weak(rename_to = window)]
            self,
            move |result| {
                if let Ok(files) = result {
                    let paths: Vec<PathBuf> = (0..files.n_items())
                        .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                        .filter_map(|file| file.path())
                        .collect();
                    window.add_paths(paths);
                }
            }
        ));
    }

    pub fn show_add_folder_dialog(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Select Folder"))
            // Not modal: GNOME attaches modal dialogs to their parent window, so
            // the chooser could only be dragged together with the whole app.
            .modal(false)
            .build();

        dialog.select_folder(Some(self), gio::Cancellable::NONE, clone!(
            #[weak(rename_to = window)]
            self,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        window.add_path(path);
                    }
                }
            }
        ));
    }

    /// Extract every dropped path. A multi-file drop arrives as a gdk::FileList;
    /// the single gio::File form is kept as a fallback for sources that offer it.
    fn paths_from_drop_value(value: &glib::Value) -> Vec<PathBuf> {
        if let Ok(list) = value.get::<gdk4::FileList>() {
            list.files().iter().filter_map(|file| file.path()).collect()
        } else if let Ok(file) = value.get::<gio::File>() {
            file.path().into_iter().collect()
        } else {
            Vec::new()
        }
    }

    pub fn add_path(&self, path: PathBuf) {
        self.add_paths(vec![path]);
    }

    /// Add several paths in a single pass. Plain files are collected and appended
    /// once, so the list and preview are refreshed one time instead of per path.
    pub fn add_paths(&self, paths: Vec<PathBuf>) {
        let mut entries = Vec::new();

        for path in paths {
            if path.is_dir() {
                // Use gio::spawn_blocking for directory traversal to avoid blocking the UI
                let (sender, receiver) = async_channel::bounded::<Vec<FileEntry>>(1);
                let settings = self.imp().settings.borrow().clone();
                let generation = self.imp().scan_generation.get();

                gio::spawn_blocking(move || {
                    let _ = sender.send_blocking(scan_folder(&path, &settings));
                });

                // Receive results on main thread
                glib::spawn_future_local(clone!(
                    #[weak(rename_to = window)]
                    self,
                    async move {
                        if let Ok(entries) = receiver.recv().await {
                            if window.imp().scan_generation.get() == generation {
                                window.append_file_entries(entries);
                            }
                        }
                    }
                ));
            } else {
                entries.push(path);
            }
        }

        // Plain files also load off the main thread: metadata parsing (EXIF,
        // ID3, image probing) is file I/O and a large drop used to freeze the UI.
        if !entries.is_empty() {
            let settings = self.imp().settings.borrow().clone();
            let generation = self.imp().scan_generation.get();
            let (sender, receiver) = async_channel::bounded::<Vec<FileEntry>>(1);

            gio::spawn_blocking(move || {
                let mut loaded = Vec::new();
                for path in entries {
                    if let Ok(mut file_entry) = FileEntry::from_path(path, 0) {
                        if settings.metadata_loading_enabled && file_entry.metadata_cache.is_none() {
                            let _ = crate::metadata::load_metadata(&mut file_entry);
                        }
                        loaded.push(file_entry);
                    }
                }
                let _ = sender.send_blocking(loaded);
            });

            glib::spawn_future_local(clone!(
                #[weak(rename_to = window)]
                self,
                async move {
                    if let Ok(loaded) = receiver.recv().await {
                        if window.imp().scan_generation.get() == generation {
                            window.append_file_entries(loaded);
                        }
                    }
                }
            ));
        }
    }

    /// Append file entries to the list model in one splice (called from async context)
    fn append_file_entries(&self, entries: Vec<FileEntry>) {
        {
            let store_ref = self.imp().store.borrow();
            let Some(store) = store_ref.as_ref() else {
                return;
            };
            // A file that is already listed (the same folder added twice, or a file
            // dropped again) is not added a second time: the duplicate would collide
            // with itself in the preview and block the rename.
            let mut listed: std::collections::HashSet<PathBuf> = (0..store.n_items())
                .filter_map(|position| store.item(position).and_downcast::<FileItem>())
                .map(|item| item.path())
                .collect();
            let items: Vec<FileItem> = entries
                .into_iter()
                .filter(|entry| listed.insert(entry.path.clone()))
                .map(FileItem::new)
                .collect();
            store.splice(store.n_items(), 0, &items);
        }
        self.refresh_file_count();
        self.request_preview_debounced();
    }

    pub fn clear_files(&self) {
        // Results of scans still running belong to the list being cleared.
        self.imp().scan_generation.set(self.imp().scan_generation.get() + 1);
        if let Some(store) = self.imp().store.borrow().as_ref() {
            store.remove_all();
        }
        self.imp().previews.borrow_mut().clear();
        self.refresh_file_count();
        self.request_preview();
    }

    fn remove_selected_files(&self) {
        let Some(selection) = self.imp().file_selection.borrow().clone() else {
            return;
        };
        let Some(store) = self.imp().store.borrow().clone() else {
            return;
        };
        let bitset = selection.selection();
        let mut indices = Vec::new();
        if let Some((iter, first)) = gtk::BitsetIter::init_first(&bitset) {
            indices.push(first);
            indices.extend(iter);
        }
        for idx in indices.into_iter().rev() {
            store.remove(idx);
        }
        self.refresh_file_count();
        self.request_preview();
    }

    /// Every FileItem currently in the list model.
    fn file_items(&self) -> Vec<FileItem> {
        match self.imp().store.borrow().as_ref() {
            Some(store) => (0..store.n_items())
                .filter_map(|i| store.item(i).and_downcast::<FileItem>())
                .collect(),
            None => Vec::new(),
        }
    }

    /// Snapshot of the file entries behind the list model.
    pub(crate) fn file_entries(&self) -> Vec<FileEntry> {
        self.file_items().iter().map(|item| item.entry()).collect()
    }

    fn refresh_file_count(&self) {
        let count = self.imp().store.borrow().as_ref().map_or(0, |s| s.n_items());
        if let Some(label) = self.imp().files_count_label.borrow().as_ref() {
            label.set_label(&gettext("{} files").replacen("{}", &count.to_string(), 1));
        }
        if let Some(stack) = self.imp().files_stack.borrow().as_ref() {
            stack.set_visible_child_name(if count == 0 { "empty" } else { "list" });
        }
        let selected = self
            .imp()
            .file_selection
            .borrow()
            .as_ref()
            .map_or(0, |sel| sel.selection().size() as usize);
        self.update_selected_count(selected);
    }

    fn update_selected_count(&self, count: usize) {
        if let Some(label) = self.imp().selected_count_label.borrow().as_ref() {
            label.set_label(&gettext("{} selected").replacen("{}", &count.to_string(), 1));
        }
    }

    /// History summaries for the browser, newest first: (id, title, sample).
    pub(crate) fn undo_history(&self) -> Vec<(Uuid, String, String)> {
        let manager = self.imp().undo_manager.borrow();
        manager
            .history()
            .iter()
            .rev()
            .map(|batch| {
                let count = batch.records.len();
                let template = if count == 1 {
                    gettext("{} file — {}")
                } else {
                    gettext("{} files — {}")
                };
                let title = template
                    .replacen("{}", &count.to_string(), 1)
                    .replacen("{}", &batch.timestamp.format("%Y-%m-%d %H:%M").to_string(), 1);
                let mut sample = batch
                    .records
                    .iter()
                    .take(2)
                    .map(|record| {
                        format!(
                            "{} → {}",
                            record
                                .original_path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            record
                                .new_path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                if count > 2 {
                    sample.push_str(", …");
                }
                (batch.id, title, sample)
            })
            .collect()
    }

    /// Shell script reverting the most recent batch, if any.
    pub(crate) fn latest_undo_script(&self) -> Option<String> {
        let manager = self.imp().undo_manager.borrow();
        manager
            .peek_undo()
            .map(|batch| manager.generate_undo_script(batch))
    }

    /// Current previews, for exports.
    pub(crate) fn previews_snapshot(&self) -> Vec<RenamePreview> {
        self.imp().previews.borrow().clone()
    }

    /// Reorder the list model itself; batch order is what numbering follows.
    pub(crate) fn sort_store(&self, column: crate::core::SortColumn, direction: crate::core::SortDirection) {
        use crate::core::{SortColumn, SortDirection};
        let Some(store) = self.imp().store.borrow().clone() else {
            return;
        };
        let mut items = self.file_items();
        items.sort_by(|a, b| {
            let ea = a.entry();
            let eb = b.entry();
            let cmp = match column {
                SortColumn::Size => ea.size.cmp(&eb.size),
                SortColumn::Modified => ea.modified.cmp(&eb.modified),
                _ => crate::engine::natural_cmp(&ea.original_name, &eb.original_name),
            };
            match direction {
                SortDirection::Ascending => cmp,
                SortDirection::Descending => cmp.reverse(),
            }
        });
        store.splice(0, store.n_items(), &items);
        self.request_preview();
    }

    /// Rebuild the engine filter from the search box and extension entry.
    pub(crate) fn rebuild_filter(&self) {
        use crate::core::{FilterConfig, FilterField, FilterMode, FilterOperator, FilterRule};

        let search = self.imp().search_text.borrow().trim().to_string();
        let extensions = self.imp().extension_filter.borrow().clone();
        let mut rules = Vec::new();

        if !search.is_empty() {
            let operator = if search.contains('*') || search.contains('?') {
                FilterOperator::MatchesGlob
            } else {
                FilterOperator::Contains
            };
            rules.push(FilterRule {
                field: FilterField::Name,
                operator,
                value: search,
            });
        }

        let ext_list: Vec<String> = extensions
            .split(',')
            .map(|ext| ext.trim().trim_start_matches('.').to_lowercase())
            .filter(|ext| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
            .collect();
        if !ext_list.is_empty() {
            // Single regex rule, because filter rules combine with AND.
            rules.push(FilterRule {
                field: FilterField::Extension,
                operator: FilterOperator::Matches,
                value: format!("^(?i)({})$", ext_list.join("|")),
            });
        }

        self.imp().config.borrow_mut().filter = if rules.is_empty() {
            None
        } else {
            Some(FilterConfig {
                mode: FilterMode::Include,
                rules,
            })
        };
        self.request_preview();
    }

    /// Recompute the preview if Live Preview is enabled. Explicit refreshes
    /// (the refresh button, executing, saving preferences) call
    /// `update_preview` directly instead.
    pub(crate) fn request_preview(&self) {
        if !self.imp().settings.borrow().live_preview {
            return;
        }
        self.update_preview();
    }

    /// Debounced variant for bursty sources (directory scans arriving in
    /// waves): coalesces every request inside an 80 ms window into one pass.
    fn request_preview_debounced(&self) {
        if !self.imp().settings.borrow().live_preview {
            return;
        }
        if self.imp().preview_pending.borrow().is_some() {
            return;
        }
        let weak = self.downgrade();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(80), move || {
            if let Some(window) = weak.upgrade() {
                window.imp().preview_pending.replace(None);
                window.update_preview();
            }
        });
        self.imp().preview_pending.replace(Some(id));
    }

    /// A blocking operation (rename, undo, redo) is in flight; used to guard
    /// re-entrancy from accelerators while the progress dialog is up.
    pub(crate) fn is_busy(&self) -> bool {
        self.imp().busy.get()
    }

    pub(crate) fn set_busy(&self, busy: bool) {
        self.imp().busy.set(busy);
    }

    /// Move the undo manager to a worker thread; always pair with
    /// `restore_undo_manager`.
    pub(crate) fn take_undo_manager(&self) -> UndoManager {
        self.imp().undo_manager.take()
    }

    pub(crate) fn restore_undo_manager(&self, manager: UndoManager) {
        self.imp().undo_manager.replace(manager);
    }

    // ============ Preview ============

    pub fn update_preview(&self) {
        let items = self.file_items();
        let files: Vec<FileEntry> = items.iter().map(|item| item.entry()).collect();
        let config = self.imp().config.borrow().clone();

        // Only included files run through the rules, so numbering and counters
        // never spend a value on an excluded row.
        let included_entries: Vec<FileEntry> = items
            .iter()
            .filter(|item| item.included())
            .map(|item| item.entry())
            .collect();

        let mut engine = RenameEngine::new(config);
        engine.set_target(*self.imp().target.borrow());
        let engine_previews = engine.generate_previews(&included_entries);

        // Merge back into item order, keyed by file id: the engine's filter can
        // drop entries entirely, excluded rows become Skipped entries, and
        // manual overrides replace the engine's name before validation.
        let mut by_id: HashMap<Uuid, RenamePreview> = engine_previews
            .into_iter()
            .map(|preview| (preview.file_id, preview))
            .collect();
        let mut previews: Vec<RenamePreview> = Vec::with_capacity(items.len());
        for (item, entry) in items.iter().zip(files.iter()) {
            if !item.included() {
                previews.push(RenamePreview {
                    file_id: entry.id,
                    original_name: entry.original_name.clone(),
                    new_name: entry.original_name.clone(),
                    new_path: entry.path.clone(),
                    status: RenameStatus::Skipped,
                    message: Some(gettext("Excluded from this batch")),
                });
                continue;
            }
            let mut preview = by_id.remove(&entry.id).unwrap_or_else(|| RenamePreview {
                file_id: entry.id,
                original_name: entry.original_name.clone(),
                new_name: entry.original_name.clone(),
                new_path: entry.path.clone(),
                status: RenameStatus::Unchanged,
                message: Some(gettext("Filtered out")),
            });
            if let Some(manual) = item.manual_override() {
                preview.new_path = entry
                    .path
                    .parent()
                    .map(|p| p.join(&manual))
                    .unwrap_or_else(|| PathBuf::from(&manual));
                preview.status = if manual == entry.original_name {
                    RenameStatus::Unchanged
                } else {
                    RenameStatus::WillRename
                };
                preview.new_name = manual;
                preview.message = Some(gettext("Manually renamed"));
            }
            previews.push(preview);
        }
        let mut previews = previews;

        let files_by_id: HashMap<Uuid, FileEntry> = files
            .iter()
            .map(|entry| (entry.id, entry.clone()))
            .collect();
        let validator = RenameValidator::new();
        for error in validator.validate_batch_with_files(&previews, &files_by_id) {
            if let Some(preview) = previews.get_mut(error.file_index) {
                preview.status = match error.error_type {
                    crate::core::ValidationErrorType::Conflict => RenameStatus::InternalConflict,
                    _ => RenameStatus::Error,
                };
                preview.message = Some(error.message);
            }
        }

        // Once the user opts in, collisions keep resolving on every refresh.
        if self.imp().auto_resolve_conflicts.get() {
            crate::engine::resolve_preview_conflicts(
                &mut previews,
                crate::engine::ConflictResolution::AppendCounter,
            );
        }

        // Count stats
        let will_rename = previews.iter()
            .filter(|p| matches!(p.status, RenameStatus::WillRename))
            .count();
        let unchanged = previews.iter()
            .filter(|p| matches!(p.status, RenameStatus::Unchanged))
            .count();
        let conflicts = previews.iter()
            .filter(|p| matches!(p.status, RenameStatus::Conflict | RenameStatus::InternalConflict))
            .count();
        let errors = previews.iter()
            .filter(|p| matches!(p.status, RenameStatus::Error | RenameStatus::Failed))
            .count();

        if let Some(label) = self.imp().preview_count_label.borrow().as_ref() {
            label.set_label(
                &gettext("{} rename, {} unchanged, {} conflicts, {} errors")
                    .replacen("{}", &will_rename.to_string(), 1)
                    .replacen("{}", &unchanged.to_string(), 1)
                    .replacen("{}", &conflicts.to_string(), 1)
                    .replacen("{}", &errors.to_string(), 1),
            );
        }

        if let Some(button) = self.imp().rename_button.borrow().as_ref() {
            button.set_sensitive(will_rename > 0 && conflicts == 0 && errors == 0);
        }

        if let Some(banner) = self.imp().conflict_banner.borrow().as_ref() {
            if conflicts > 0 {
                let template = if conflicts == 1 {
                    gettext("{} name collide with existing files")
                } else {
                    gettext("{} names collide with existing files")
                };
                banner.set_title(&template.replacen("{}", &conflicts.to_string(), 1));
                banner.set_revealed(true);
            } else {
                banner.set_revealed(false);
            }
        }

        // Push results into the bound item properties; the views update in place.
        for (item, preview) in items.iter().zip(previews.iter()) {
            item.apply_preview(preview);
        }

        self.imp().previews.replace(previews);

        if let Some(filter) = self.imp().preview_filter.borrow().as_ref() {
            filter.changed(gtk::FilterChange::Different);
        }
    }

    // ============ Rename Operations ============

    pub fn execute_rename(&self) {
        if self.is_busy() {
            return;
        }
        // Ctrl+Return is an application shortcut and runs before a text field sees
        // it, so a new name still being typed in the preview is committed first.
        gtk::prelude::GtkWindowExt::set_focus(self, None::<&gtk::Widget>);
        // Recompute so the accelerator cannot execute a stale or conflicting
        // plan the Rename button would have refused.
        self.update_preview();

        let (to_rename_count, blocked) = {
            let previews = self.imp().previews.borrow();
            let renames = previews
                .iter()
                .filter(|p| matches!(p.status, RenameStatus::WillRename))
                .count();
            let blocked = previews.iter().any(|p| {
                matches!(
                    p.status,
                    RenameStatus::Conflict
                        | RenameStatus::InternalConflict
                        | RenameStatus::Error
                        | RenameStatus::Failed
                )
            });
            (renames, blocked)
        };

        if to_rename_count == 0 {
            self.show_info_dialog(
                &gettext("Nothing to Rename"),
                &gettext("No files will be renamed with the current rules."),
            );
            return;
        }
        if blocked {
            self.show_info_dialog(
                &gettext("Rename Blocked"),
                &gettext("Resolve the conflicts and errors shown in the preview first."),
            );
            return;
        }

        // Run exactly what the user saw and confirmed: a folder scan that finishes
        // while the dialog is open must not add its files to this batch.
        let previews = self.imp().previews.borrow().clone();
        let files = self.file_entries_by_id();

        if !self.imp().settings.borrow().confirm_before_rename {
            self.perform_rename(previews, files);
            return;
        }

        let dialog = adw::MessageDialog::new(
            Some(self),
            Some(gettext("Confirm Rename").as_str()),
            Some(
                gettext("Rename {} files?")
                    .replacen("{}", &to_rename_count.to_string(), 1)
                    .as_str(),
            ),
        );

        dialog.add_response("cancel", &gettext("Cancel"));
        dialog.add_response("rename", &gettext("Rename"));
        dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("rename"));

        let confirmed = RefCell::new(Some((previews, files)));
        dialog.connect_response(None, clone!(
            #[weak(rename_to = window)]
            self,
            move |_, response| {
                if response == "rename" {
                    if let Some((previews, files)) = confirmed.borrow_mut().take() {
                        window.perform_rename(previews, files);
                    }
                }
            }
        ));
        dialog.present();
    }

    fn perform_rename(&self, previews: Vec<RenamePreview>, files: HashMap<Uuid, FileEntry>) {
        super::execution::run_rename(self, previews, files);
    }

    /// The listed files by id, as the rename engine takes them.
    fn file_entries_by_id(&self) -> HashMap<Uuid, FileEntry> {
        self.file_entries().into_iter().map(|f| (f.id, f)).collect()
    }

    pub(crate) fn handle_rename_result(&self, result: crate::engine::RenameBatchResult) {
        let success_count = result.success_count();
        let error_count = result.failure_count();

        if let Some(batch) = result.batch.clone() {
            // Always record: the persistence setting only controls whether the
            // manager writes the record to disk, never whether undo works.
            if let Err(err) = self.imp().undo_manager.borrow_mut().record_batch(batch.clone()) {
                tracing::error!("Failed to record undo batch: {}", err);
            }
            self.log_rename_batch(&batch);
        }

        if error_count == 0 {
            self.show_toast_with_undo(
                &gettext("Renamed {} files").replacen("{}", &success_count.to_string(), 1),
            );
        } else {
            let details = result
                .failures
                .iter()
                .take(8)
                .map(|failure| {
                    format!(
                        "{}: {}",
                        failure.target_path.display(),
                        failure.error
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let summary = gettext("Renamed {} files successfully, {} failed.")
                .replacen("{}", &success_count.to_string(), 1)
                .replacen("{}", &error_count.to_string(), 1);
            self.show_info_dialog(
                &gettext("Rename Completed with Errors"),
                &format!("{}\n{}", summary, details),
            );
        }

        if success_count > 0 {
            // Drop only the entries that were actually renamed; anything that failed or
            // was skipped stays in the list so the user can retry it.
            let renamed: std::collections::HashSet<&PathBuf> = result
                .successes
                .iter()
                .map(|record| &record.original_path)
                .collect();
            if let Some(store) = self.imp().store.borrow().as_ref() {
                let mut idx = 0;
                while idx < store.n_items() {
                    let was_renamed = store
                        .item(idx)
                        .and_downcast::<FileItem>()
                        .is_some_and(|item| renamed.contains(&item.entry().path));
                    if was_renamed {
                        store.remove(idx);
                    } else {
                        idx += 1;
                    }
                }
            }
            self.refresh_file_count();
        }
        self.update_history_actions();
        self.update_preview();
    }

    fn log_rename_batch(&self, batch: &RenameBatch) {
        let settings = self.imp().settings.borrow();
        if !settings.log_operations {
            return;
        }
        drop(settings);

        let statuses = batch
            .records
            .iter()
            .map(|record| (record.id, RenameStatus::Completed, None))
            .collect::<Vec<_>>();

        let logger = self.imp().logger.borrow();
        if let Err(err) = logger.log_batch(batch, &statuses) {
            tracing::error!("Failed to write rename log: {}", err);
        }

        for record in &batch.records {
            let entry = RenameLogEntry {
                id: Uuid::new_v4(),
                timestamp: record.timestamp,
                batch_id: batch.id,
                original_path: record.original_path.clone(),
                new_path: record.new_path.clone(),
                is_directory: record.was_directory,
                status: RenameStatus::Completed,
                error: None,
            };
            if let Err(err) = logger.log_jsonl(&entry) {
                tracing::error!("Failed to write rename JSONL log: {}", err);
            }
        }
    }

    /// Non-blocking confirmation for routine outcomes; dialogs stay for
    /// errors and anything the user must act on.
    pub(crate) fn show_toast(&self, message: &str) {
        if let Some(overlay) = self.imp().toast_overlay.borrow().as_ref() {
            overlay.add_toast(adw::Toast::new(message));
        }
    }

    /// Toast with a one-click Undo button wired to win.undo.
    pub(crate) fn show_toast_with_undo(&self, message: &str) {
        if let Some(overlay) = self.imp().toast_overlay.borrow().as_ref() {
            let toast = adw::Toast::new(message);
            toast.set_button_label(Some(gettext("Undo").as_str()));
            toast.set_action_name(Some("win.undo"));
            overlay.add_toast(toast);
        }
    }

    /// Keep the Undo/Redo menu items and accelerators in sync with the stacks.
    pub(crate) fn update_history_actions(&self) {
        let (can_undo, can_redo) = {
            let manager = self.imp().undo_manager.borrow();
            (manager.can_undo(), manager.can_redo())
        };
        for (name, enabled) in [("undo", can_undo), ("redo", can_redo)] {
            if let Some(action) = self.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                action.set_enabled(enabled);
            }
        }
    }

    pub(crate) fn show_info_dialog(&self, title: &str, message: &str) {
        let dialog = adw::MessageDialog::new(Some(self), Some(title), Some(message));
        dialog.add_response("ok", &gettext("OK"));
        dialog.set_default_response(Some("ok"));
        dialog.present();
    }

    /// Undo and redo refuse a move when the destination is occupied or the file no
    /// longer matches what was recorded. That refusal is the one outcome a user has
    /// to act on, so the per-record reason has to reach the dialog rather than be
    /// implied by a shortfall in the count.
    pub(crate) fn undo_result_message(summary: String, result: &UndoResult) -> String {
        const MAX_REASONS: usize = 5;

        let reasons: Vec<&str> = result
            .results
            .iter()
            .filter(|record| !record.success)
            .filter_map(|record| record.error.as_deref())
            .collect();

        if reasons.is_empty() {
            return summary;
        }

        let mut message = summary;
        for reason in reasons.iter().take(MAX_REASONS) {
            message.push_str(&format!("\n\n{}", reason));
        }
        if reasons.len() > MAX_REASONS {
            message.push_str("\n\n");
            message.push_str(
                &gettext("…and {} more.")
                    .replacen("{}", &(reasons.len() - MAX_REASONS).to_string(), 1),
            );
        }
        message
    }

    /// The text field that has the keyboard focus, if any. Ctrl+Z and
    /// Ctrl+Shift+Z are application shortcuts, which GTK runs before the focused
    /// widget sees the key; inside a text field they belong to its own typing
    /// history, not to the renames on disk.
    fn focused_text(&self) -> Option<gtk::Widget> {
        gtk::prelude::GtkWindowExt::focus(self).filter(|widget| widget.is::<gtk::Text>())
    }

    fn undo_last_batch(&self) {
        if let Some(text) = self.focused_text() {
            let _ = WidgetExt::activate_action(&text, "text.undo", None);
            return;
        }
        super::execution::run_undo(self);
    }

    fn redo_last_batch(&self) {
        if let Some(text) = self.focused_text() {
            let _ = WidgetExt::activate_action(&text, "text.redo", None);
            return;
        }
        super::execution::run_redo(self);
    }

    fn show_preferences_dialog(&self) {
        super::preferences_dialog::show(self);
    }

    /// Snapshot of the current settings, for dialogs.
    pub(crate) fn settings_snapshot(&self) -> AppSettings {
        self.imp().settings.borrow().clone()
    }

    /// Mutate the settings, then persist them and refresh everything they affect.
    pub(crate) fn update_settings(&self, apply: impl FnOnce(&mut AppSettings)) {
        {
            let mut settings = self.imp().settings.borrow_mut();
            apply(&mut settings);
        }
        let (log_enabled, persist_undo) = {
            let settings = self.imp().settings.borrow();
            (settings.log_operations, settings.undo_persistence_enabled)
        };
        self.imp().logger.borrow_mut().set_enabled(log_enabled);
        self.imp().undo_manager.borrow_mut().set_persistence(persist_undo);
        self.save_settings();
        self.update_preview();
    }

    // ============ Rule Dialogs ============

    fn show_add_rule_dialog(&self, _rules_list: &gtk::ListBox) {
        use super::rule_dialogs::RuleKind;

        let dialog = adw::MessageDialog::new(
            Some(self),
            Some(gettext("Add Rule").as_str()),
            Some(gettext("Select the type of rule to add:").as_str()),
        );

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(vec!["boxed-list"])
            .build();

        for (_, icon, name, desc) in RuleKind::catalog() {
            let row = adw::ActionRow::builder()
                .title(gettext(*name))
                .subtitle(gettext(*desc))
                .activatable(true)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name(icon));
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            list.append(&row);
        }

        let scroll = gtk::ScrolledWindow::builder()
            .max_content_height(420)
            .propagate_natural_height(true)
            .child(&list)
            .build();
        dialog.set_extra_child(Some(&scroll));
        dialog.add_response("cancel", &gettext("Cancel"));
        dialog.present();

        let dialog_clone = dialog.clone();
        list.connect_row_activated(clone!(
            #[weak(rename_to = window)]
            self,
            move |_, row| {
                let index = row.index() as usize;
                dialog_clone.close();
                if let Some((kind, ..)) = RuleKind::catalog().get(index) {
                    super::rule_dialogs::open(&window, *kind, None);
                }
            }
        ));
    }

    /// Read a rule back for the edit dialogs.
    pub(crate) fn rule_at(&self, index: usize) -> Option<crate::core::RenameRule> {
        self.imp().config.borrow().rules.get(index).cloned()
    }

    /// Insert or replace a rule, rebuild the rules list, refresh the preview.
    /// Editing preserves the rule's id and enabled flag.
    pub(crate) fn commit_rule(&self, rule_type: crate::core::RuleType, edit_index: Option<usize>) {
        {
            let mut config = self.imp().config.borrow_mut();
            match edit_index {
                Some(idx) if idx < config.rules.len() => {
                    config.rules[idx].rule_type = rule_type;
                }
                _ => config.rules.push(crate::core::RenameRule::new(rule_type)),
            }
        }
        if let Some(rules_list) = self.imp().rules_list.borrow().as_ref() {
            self.rebuild_rules_list(rules_list);
        }
        self.request_preview();
    }

    /// Remove every rule at once.
    pub(crate) fn clear_all_rules(&self) {
        self.imp().config.borrow_mut().rules.clear();
        if let Some(rules_list) = self.imp().rules_list.borrow().as_ref() {
            self.rebuild_rules_list(rules_list);
        }
        self.request_preview();
    }

    // Thin rule builders kept for the quick-rule actions and the widget tests;
    // the dialogs go through rule_dialogs::open and commit_rule directly.

    #[cfg(test)]
    fn add_replace_rule(
        &self,
        _rules_list: &gtk::ListBox,
        find: String,
        replace: String,
        case_sensitive: bool,
        use_regex: bool,
        replace_all: bool,
    ) {
        use crate::core::{ReplaceRule, RuleType};
        self.commit_rule(
            RuleType::Replace(ReplaceRule {
                find,
                replace,
                use_regex,
                case_sensitive,
                replace_all,
                include_extension: false,
            }),
            None,
        );
    }

    fn add_case_rule_at(&self, _rules_list: &gtk::ListBox, case_type_idx: usize, edit_index: Option<usize>) {
        use crate::core::{CaseRule, CaseType, RuleType};
        // Index order matches the CaseType enum, which the quick actions cast.
        let case_type = match case_type_idx {
            0 => CaseType::Lower,
            1 => CaseType::Upper,
            2 => CaseType::Title,
            3 => CaseType::Sentence,
            _ => CaseType::Lower,
        };
        self.commit_rule(
            RuleType::ChangeCase(CaseRule {
                case_type,
                include_extension: false,
            }),
            edit_index,
        );
    }

    fn add_numbering_rule_at(
        &self,
        _rules_list: &gtk::ListBox,
        start: i64,
        increment: i64,
        padding: usize,
        position: usize,
        separator: String,
        edit_index: Option<usize>,
    ) {
        use crate::core::{InsertPosition, NumberFormat, NumberingRule, RuleType};
        let (insert_pos, prefix, suffix) = if position == 0 {
            (InsertPosition::Prefix, String::new(), separator)
        } else {
            (InsertPosition::Suffix, separator, String::new())
        };
        self.commit_rule(
            RuleType::Numbering(NumberingRule {
                start,
                increment,
                padding,
                position: insert_pos,
                prefix,
                suffix,
                reset_per_folder: false,
                format: NumberFormat::Decimal,
            }),
            edit_index,
        );
    }

    #[cfg(test)]
    fn add_datetime_rule(&self, rules_list: &gtk::ListBox, source: usize, format: usize, position: usize) {
        self.add_datetime_rule_at(rules_list, source, format, position, None);
    }

    #[cfg(test)]
    fn add_datetime_rule_at(
        &self,
        _rules_list: &gtk::ListBox,
        source: usize,
        format: usize,
        position: usize,
        edit_index: Option<usize>,
    ) {
        use crate::core::{DateSource, InsertPosition, InsertRule, InsertText, RuleType};

        let formats = ["%Y-%m-%d", "%Y%m%d", "%d-%m-%Y", "%b %d, %Y", "%Y-%m-%d_%H-%M-%S"];
        let format_str = formats.get(format).unwrap_or(&"%Y-%m-%d").to_string();
        let date_source = match source {
            0 => DateSource::Modified,
            1 => DateSource::Created,
            2 => DateSource::Now,
            3 => DateSource::ExifDateTaken,
            _ => DateSource::Modified,
        };
        let insert_pos = if position == 0 {
            InsertPosition::Prefix
        } else {
            InsertPosition::Suffix
        };
        self.commit_rule(
            RuleType::Insert(InsertRule {
                text: InsertText::FileDate {
                    source: date_source,
                    format: format_str,
                },
                position: insert_pos,
            }),
            edit_index,
        );
    }

    fn create_rule_row(
        &self,
        title: &str,
        subtitle: &str,
        icon_name: &str,
        rule_index: usize,
        enabled: bool,
        rules_list: &gtk::ListBox,
    ) -> gtk::ListBoxRow {
        let row_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_start(12)
            .margin_end(6)
            .margin_top(8)
            .margin_bottom(8)
            .build();

        // Drag handle
        let drag_handle = gtk::Image::from_icon_name("list-drag-handle-symbolic");
        drag_handle.add_css_class("dim-label");
        drag_handle.add_css_class("drag-handle");
        drag_handle.set_tooltip_text(Some(gettext("Drag to reorder").as_str()));
        row_box.append(&drag_handle);

        // Icon
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.add_css_class("dim-label");
        row_box.append(&icon);

        // Labels
        let label_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .spacing(2)
            .build();

        let title_label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(vec!["heading"])
            .build();
        label_box.append(&title_label);

        let subtitle_label = gtk::Label::builder()
            .label(subtitle)
            .xalign(0.0)
            .css_classes(vec!["dim-label", "caption"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        label_box.append(&subtitle_label);

        row_box.append(&label_box);

        // Enable toggle: the engine already skips disabled rules, this is the
        // switch that was never exposed. Its handler is wired after the row
        // exists, further down, so it can read its live index.
        let enable_switch = gtk::Switch::builder()
            .active(enabled)
            .valign(gtk::Align::Center)
            .tooltip_text(gettext("Enable this rule"))
            .build();
        row_box.append(&enable_switch);

        // Edit button
        let edit_btn = gtk::Button::from_icon_name("document-edit-symbolic");
        edit_btn.add_css_class("flat");
        edit_btn.add_css_class("circular");
        edit_btn.set_valign(gtk::Align::Center);
        edit_btn.set_tooltip_text(Some(gettext("Edit rule").as_str()));

        // Delete button
        let delete_btn = gtk::Button::from_icon_name("edit-delete-symbolic");
        delete_btn.add_css_class("flat");
        delete_btn.add_css_class("circular");
        delete_btn.set_valign(gtk::Align::Center);
        delete_btn.set_tooltip_text(Some(gettext("Remove rule").as_str()));

        row_box.append(&edit_btn);
        row_box.append(&delete_btn);

        let row = gtk::ListBoxRow::builder()
            .child(&row_box)
            .build();
        row.add_css_class("rule-row");

        // Store rule index as widget name for easy access
        row.set_widget_name(&format!("rule_{}", rule_index));

        // Setup drag source
        let drag_source = gtk::DragSource::new();
        drag_source.set_actions(gtk::gdk::DragAction::MOVE);

        let row_weak = row.downgrade();
        drag_source.connect_prepare(move |_source, _x, _y| {
            if let Some(row) = row_weak.upgrade() {
                let idx = row.index();
                Some(gtk::gdk::ContentProvider::for_value(&idx.to_value()))
            } else {
                None
            }
        });

        let row_weak = row.downgrade();
        drag_source.connect_drag_begin(move |_source, _drag| {
            if let Some(row) = row_weak.upgrade() {
                row.add_css_class("dragging");
            }
        });

        let row_weak = row.downgrade();
        drag_source.connect_drag_end(move |_source, _drag, _delete| {
            if let Some(row) = row_weak.upgrade() {
                row.remove_css_class("dragging");
            }
        });

        row.add_controller(drag_source);

        // Setup drop target
        let drop_target = gtk::DropTarget::new(i32::static_type(), gtk::gdk::DragAction::MOVE);

        let rules_list_weak = rules_list.downgrade();
        let window_weak = self.downgrade();
        drop_target.connect_drop(move |_target, value, _x, _y| {
            if let (Some(rules_list), Some(window)) = (rules_list_weak.upgrade(), window_weak.upgrade()) {
                if let Ok(source_idx) = value.get::<i32>() {
                    let source_idx = source_idx as usize;
                    // Get target index from current row position
                    if let Some(target_row) = _target.widget().and_downcast::<gtk::ListBoxRow>() {
                        let target_idx = target_row.index() as usize;
                        if source_idx != target_idx {
                            window.reorder_rule(source_idx, target_idx, &rules_list);
                            return true;
                        }
                    }
                }
            }
            false
        });

        row.add_controller(drop_target);

        // Connect enable switch
        let window = self.clone();
        let row_weak = row.downgrade();
        enable_switch.connect_active_notify(move |switch| {
            let Some(r) = row_weak.upgrade() else { return };
            let idx = r.index();
            if idx < 0 {
                return;
            }
            {
                let mut config = window.imp().config.borrow_mut();
                if let Some(rule) = config.rules.get_mut(idx as usize) {
                    rule.enabled = switch.is_active();
                }
            }
            window.request_preview();
        });

        // Connect edit button
        let window = self.clone();
        let rules_list_clone = rules_list.clone();
        let row_weak = row.downgrade();
        edit_btn.connect_clicked(move |_| {
            if let Some(r) = row_weak.upgrade() {
                let idx = r.index() as usize;
                window.edit_rule_at_index(idx, &rules_list_clone);
            }
        });

        // Connect delete button
        let window = self.clone();
        let rules_list_clone = rules_list.clone();
        let row_weak = row.downgrade();
        delete_btn.connect_clicked(move |_| {
            if let Some(r) = row_weak.upgrade() {
                let idx = r.index() as usize;
                rules_list_clone.remove(&r);
                // Guard the model: a detached row reports index -1, which would wrap
                // to a huge usize and panic inside Vec::remove.
                let mut config = window.imp().config.borrow_mut();
                if idx < config.rules.len() {
                    config.rules.remove(idx);
                }
                drop(config);
                window.request_preview();
            }
        });

        row
    }

    fn reorder_rule(&self, from: usize, to: usize, rules_list: &gtk::ListBox) {
        // Reorder in data model
        let mut rules = self.imp().config.borrow_mut();
        if from < rules.rules.len() {
            let rule = rules.rules.remove(from);
            let insert_at = if to > from { to - 1 } else { to };
            let insert_at = insert_at.min(rules.rules.len());
            rules.rules.insert(insert_at, rule);
        }
        drop(rules);

        // Rebuild the rules list UI
        self.rebuild_rules_list(rules_list);
        self.request_preview();
    }

    fn rebuild_rules_list(&self, rules_list: &gtk::ListBox) {
        // Remove all rows
        while let Some(row) = rules_list.row_at_index(0) {
            rules_list.remove(&row);
        }

        // Re-add all rules
        let rules = self.imp().config.borrow().rules.clone();
        for (idx, rule) in rules.iter().enumerate() {
            let (title, subtitle, icon) = self.get_rule_display_info(rule);
            let row = self.create_rule_row(&title, &subtitle, &icon, idx, rule.enabled, rules_list);
            rules_list.append(&row);
        }
    }

    fn get_rule_display_info(&self, rule: &crate::core::RenameRule) -> (String, String, String) {
        super::rule_dialogs::rule_summary(&rule.rule_type)
    }

    fn edit_rule_at_index(&self, index: usize, _rules_list: &gtk::ListBox) {
        use super::rule_dialogs::RuleKind;
        let Some(rule) = self.rule_at(index) else { return };
        super::rule_dialogs::open(self, RuleKind::of(&rule.rule_type), Some(index));
    }

    /// Subtitle shared with rule_dialogs so a rebuilt row is identical to the
    /// one created when the rule was added.
    #[cfg(test)]
    fn datetime_subtitle(rule: &crate::core::InsertRule) -> String {
        super::rule_dialogs::datetime_subtitle_for(rule)
    }
    // ============ Preset/Import/Export Dialogs ============

    fn show_save_preset_dialog(&self) {
        super::presets_dialog::show_save(self);
    }

    fn show_load_preset_dialog(&self) {
        super::presets_dialog::show_load(self);
    }

    /// Clone of the active rule configuration, for preset dialogs.
    pub(crate) fn config_snapshot(&self) -> RenameConfig {
        self.imp().config.borrow().clone()
    }

    pub(crate) fn apply_preset(&self, preset: Preset) {
        self.imp().config.replace(preset.config);
        if let Some(rules_list) = self.imp().rules_list.borrow().as_ref() {
            self.rebuild_rules_list(rules_list);
        }
        self.request_preview();
        self.show_toast(&gettext("Loaded preset '{}'").replacen("{}", &preset.name, 1));
    }

    fn show_import_csv_dialog(&self) {
        super::csv_io::show_import_dialog(self);
    }

    fn show_export_log_dialog(&self) {
        super::csv_io::show_export_dialog(self);
    }

    pub(crate) fn export_log_csv(&self, path: &std::path::Path) -> crate::core::RenamerResult<()> {
        self.imp().logger.borrow().export_csv(path)
    }
}

/// Every entry under `root` that adding the folder puts in the list. Unless the
/// settings include hidden files, a hidden entry is pruned together with
/// everything below it: skipping only the dot-entry itself still walked into
/// `.git/` and listed its contents, ready to be renamed by any rule.
fn scan_folder(root: &std::path::Path, settings: &AppSettings) -> Vec<FileEntry> {
    let show_hidden = settings.show_hidden_files;
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .min_depth(1)
        .max_depth(settings.recursive_folder_depth)
        .follow_links(settings.follow_symlinks)
        .into_iter()
        // Depth 0 is the folder the user picked, which is walked even if hidden.
        .filter_entry(|entry| {
            show_hidden || entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.')
        })
        .filter_map(|e| e.ok())
    {
        if let Ok(mut file_entry) = FileEntry::from_path(entry.path().to_path_buf(), entry.depth()) {
            if settings.metadata_loading_enabled && file_entry.metadata_cache.is_none() {
                let _ = crate::metadata::load_metadata(&mut file_entry);
            }
            entries.push(file_entry);
        }
    }
    entries
}

/// The new-name line of a preview row: a small dim label that becomes a text
/// field when clicked. Typing a name overrides the rules for that file; clearing
/// it hands the name back to the rules. GtkEditableLabel is not used because its
/// label never wraps, so a long name would push the list wider than the panel.
fn new_name_editor(list_item: &gtk::ListItem, window: glib::WeakRef<RenamerWindow>) -> gtk::Stack {
    let label = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(vec!["dim-label", "caption"])
        .focusable(true)
        .tooltip_text(gettext(
            "Click to type a name for this file; clear it to follow the rules again",
        ))
        .build();
    let entry = gtk::Entry::builder().css_classes(vec!["caption"]).build();

    let stack = gtk::Stack::builder()
        .hhomogeneous(false)
        .vhomogeneous(false)
        .build();
    stack.add_named(&label, Some("label"));
    stack.add_named(&entry, Some("entry"));
    stack.set_visible_child_name("label");

    list_item
        .property_expression("item")
        .chain_property::<FileItem>("new-name")
        .bind(&label, "label", None::<&gtk::Widget>);

    let start_editing = clone!(
        #[weak]
        stack,
        #[weak]
        entry,
        #[weak]
        label,
        move || {
            entry.set_text(&label.label());
            stack.set_visible_child_name("entry");
            entry.grab_focus();
        }
    );
    // Claim the click so the list row does not handle it too: the row takes the
    // focus on release, which would end the edit as soon as it started. The
    // focus moves to the entry once the click is over, for the same reason.
    let click = gtk::GestureClick::new();
    click.connect_pressed(|gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    let start = start_editing.clone();
    click.connect_released(move |_, _, _, _| {
        glib::idle_add_local_once(start.clone());
    });
    label.add_controller(click);
    let label_keys = gtk::EventControllerKey::new();
    label_keys.connect_key_pressed(move |_, key, _, _| {
        if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::space) {
            start_editing();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    label.add_controller(label_keys);

    let list_item_weak = list_item.downgrade();
    let finish = move |stack: &gtk::Stack, entry: &gtk::Entry, save: bool| {
        if stack.visible_child_name().as_deref() != Some("entry") {
            return;
        }
        stack.set_visible_child_name("label");
        if !save {
            return;
        }
        let Some(item) = list_item_weak
            .upgrade()
            .and_then(|list_item| list_item.item())
            .and_downcast::<FileItem>()
        else {
            return;
        };
        let typed = entry.text().to_string();
        if typed == item.new_name() {
            return;
        }
        item.set_manual_name(typed);
        if let Some(window) = window.upgrade() {
            window.request_preview();
        }
    };
    let finish = std::rc::Rc::new(finish);

    entry.connect_activate(clone!(
        #[weak]
        stack,
        #[strong]
        finish,
        move |entry| finish(&stack, entry, true)
    ));
    let entry_keys = gtk::EventControllerKey::new();
    entry_keys.connect_key_pressed(clone!(
        #[weak]
        stack,
        #[weak]
        entry,
        #[strong]
        finish,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                finish(&stack, &entry, false);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    ));
    entry.add_controller(entry_keys);
    let entry_focus = gtk::EventControllerFocus::new();
    entry_focus.connect_leave(clone!(
        #[weak]
        stack,
        #[weak]
        entry,
        #[strong]
        finish,
        move |_| finish(&stack, &entry, true)
    ));
    entry.add_controller(entry_focus);

    // A recycled row must not carry a half-typed name over to another file.
    list_item.connect_notify_local(
        Some("item"),
        clone!(
            #[weak]
            stack,
            move |_, _| stack.set_visible_child_name("label")
        ),
    );

    stack
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{DateSource, InsertPosition, InsertText};
    use std::sync::Once;

    static GTK_INIT: Once = Once::new();

    /// Paths `scan_folder` returns, relative to `root` and sorted.
    fn scanned(root: &std::path::Path, show_hidden: bool) -> Vec<String> {
        let settings = AppSettings {
            show_hidden_files: show_hidden,
            recursive_folder_depth: 10,
            metadata_loading_enabled: false,
            ..AppSettings::default()
        };
        let mut names: Vec<String> = scan_folder(root, &settings)
            .iter()
            .map(|entry| {
                entry
                    .path
                    .strip_prefix(root)
                    .expect("entries lie under the root")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_folder_scan_leaves_hidden_folders_and_their_contents_out() {
        let root = std::env::temp_dir().join(format!("bulk-renamer-scan-{}", Uuid::new_v4()));
        for dir in [".git/objects", "sub"] {
            std::fs::create_dir_all(root.join(dir)).expect("create test dirs");
        }
        for file in ["visible.txt", ".hidden.txt", ".git/HEAD", ".git/objects/ab", "sub/inner.txt", "sub/.secret"] {
            std::fs::write(root.join(file), b"x").expect("write test file");
        }

        assert_eq!(scanned(&root, false), ["sub", "sub/inner.txt", "visible.txt"]);
        assert!(scanned(&root, true).contains(&".git/HEAD".to_string()));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// These tests drive real widgets, so they need a display; run them with
    /// `cargo test -- --ignored --test-threads=1`.
    fn test_window() -> RenamerWindow {
        GTK_INIT.call_once(|| {
            adw::init().expect("libadwaita must initialise for widget tests");
        });
        let app = adw::Application::builder()
            .application_id("com.chrisdaggas.bulk-renamer.Tests")
            .build();
        RenamerWindow::new(&app)
    }

    fn rules_list_of(window: &RenamerWindow) -> gtk::ListBox {
        window
            .imp()
            .rules_list
            .borrow()
            .clone()
            .expect("window builds a rules list")
    }

    /// Last child of the row box is the delete button built by create_rule_row.
    fn delete_button(row: &gtk::ListBoxRow) -> gtk::Button {
        row.child()
            .and_downcast::<gtk::Box>()
            .and_then(|row_box| row_box.last_child())
            .and_downcast::<gtk::Button>()
            .expect("rule row ends with a delete button")
    }

    /// Third child of the row box is the label box; its first label is the title.
    fn row_title(row: &gtk::ListBoxRow) -> String {
        let label_box = row
            .child()
            .and_downcast::<gtk::Box>()
            .and_then(|row_box| row_box.first_child())
            .and_then(|drag_handle| drag_handle.next_sibling())
            .and_then(|icon| icon.next_sibling())
            .and_downcast::<gtk::Box>()
            .expect("rule row has a label box");
        label_box
            .first_child()
            .and_downcast::<gtk::Label>()
            .expect("label box starts with the title label")
            .label()
            .to_string()
    }

    fn row_titles(rules_list: &gtk::ListBox) -> Vec<String> {
        let mut titles = Vec::new();
        let mut idx = 0;
        while let Some(row) = rules_list.row_at_index(idx) {
            titles.push(row_title(&row));
            idx += 1;
        }
        titles
    }

    fn rule_titles(window: &RenamerWindow) -> Vec<String> {
        window
            .imp()
            .config
            .borrow()
            .rules
            .iter()
            .map(|rule| window.get_rule_display_info(rule).0)
            .collect()
    }

    fn add_replace(window: &RenamerWindow, rules_list: &gtk::ListBox, find: &str) {
        window.add_replace_rule(
            rules_list,
            find.to_string(),
            "x".to_string(),
            true,
            false,
            true,
        );
    }

    fn scenario_datetime_row_delete_removes_the_datetime_rule() {
        let window = test_window();
        let rules_list = rules_list_of(&window);

        add_replace(&window, &rules_list, "a");
        window.add_datetime_rule(&rules_list, 0, 0, 0);
        add_replace(&window, &rules_list, "b");

        // Delete the first Replace rule, which shifts the Date/Time rule down.
        delete_button(&rules_list.row_at_index(0).expect("row 0")).emit_clicked();
        assert_eq!(rule_titles(&window), vec!["Date/Time", "Replace"]);

        // Now delete the Date/Time row: it must remove the Date/Time rule.
        delete_button(&rules_list.row_at_index(0).expect("row 0")).emit_clicked();

        assert_eq!(rule_titles(&window), vec!["Replace"]);
        assert_eq!(row_titles(&rules_list), vec!["Replace"]);
    }

    fn scenario_datetime_row_delete_does_not_panic_when_it_is_the_last_rule() {
        let window = test_window();
        let rules_list = rules_list_of(&window);

        add_replace(&window, &rules_list, "a");
        window.add_datetime_rule(&rules_list, 0, 0, 0);

        delete_button(&rules_list.row_at_index(0).expect("row 0")).emit_clicked();
        // Used to panic with "removal index (is 1) should be < len (is 1)".
        delete_button(&rules_list.row_at_index(0).expect("row 0")).emit_clicked();

        assert!(window.imp().config.borrow().rules.is_empty());
        assert!(rules_list.row_at_index(0).is_none());
    }

    fn scenario_datetime_row_survives_a_rules_list_rebuild() {
        let window = test_window();
        let rules_list = rules_list_of(&window);

        window.add_datetime_rule(&rules_list, 3, 0, 1);
        let before = row_titles(&rules_list);

        window.rebuild_rules_list(&rules_list);

        assert_eq!(before, vec!["Date/Time"]);
        assert_eq!(row_titles(&rules_list), before);
    }

    #[test]
    fn datetime_rules_are_labelled_as_date_time() {
        let rule = crate::core::InsertRule {
            text: InsertText::FileDate {
                source: DateSource::ExifDateTaken,
                format: "%Y-%m-%d".to_string(),
            },
            position: InsertPosition::Suffix,
        };
        assert_eq!(
            RenamerWindow::datetime_subtitle(&rule),
            "EXIF date as suffix"
        );
    }

    fn scenario_drop_value_yields_every_file_of_a_multi_file_drop() {
        let _window = test_window();
        let files: Vec<gio::File> = ["/tmp/one.txt", "/tmp/two.txt", "/tmp/three.txt"]
            .iter()
            .map(gio::File::for_path)
            .collect();
        let value = gdk4::FileList::from_array(&files).to_value();

        let paths = RenamerWindow::paths_from_drop_value(&value);

        assert_eq!(paths.len(), 3);
        assert_eq!(paths[2], PathBuf::from("/tmp/three.txt"));
    }

    fn scenario_drop_value_still_accepts_a_single_file() {
        let _window = test_window();
        let value = gio::File::for_path("/tmp/one.txt").to_value();

        assert_eq!(
            RenamerWindow::paths_from_drop_value(&value),
            vec![PathBuf::from("/tmp/one.txt")]
        );
    }

    fn scenario_drop_target_advertises_the_file_list_type() {
        let window = test_window();
        let stack = window
            .imp()
            .files_stack
            .borrow()
            .clone()
            .expect("window builds the files stack");

        // The target must sit above the stack that holds both the list and the
        // empty page: on the list alone, a drop failed until a file was listed.
        let mut types = Vec::new();
        let mut widget = stack.parent();
        while let Some(current) = widget {
            let controllers = current.observe_controllers();
            for i in 0..controllers.n_items() {
                if let Some(target) = controllers.item(i).and_downcast::<gtk::DropTarget>() {
                    types = target.types().to_vec();
                }
            }
            if !types.is_empty() {
                break;
            }
            widget = current.parent();
        }

        assert!(types.contains(&gdk4::FileList::static_type()));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bulk-renamer-{}-{}", name, Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// Plain-file adds load entries on a worker thread; pump the main context
    /// until they land in the store (or fail loudly).
    fn wait_for_files(window: &RenamerWindow, count: usize) {
        let context = glib::MainContext::default();
        let start = std::time::Instant::now();
        while window.file_entries().len() < count {
            context.iteration(true);
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "timed out waiting for {} files, have {}",
                count,
                window.file_entries().len()
            );
        }
    }

    fn scenario_adding_many_paths_rebuilds_the_list_once() {
        let window = test_window();
        let dir = temp_dir("batch-add");
        for name in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(dir.join(name), b"x").expect("write file");
        }

        // Seed the list so the count label already has a value, then count how many
        // times refresh_file_count rewrites it while adding the rest.
        window.add_path(dir.join("a.txt"));
        wait_for_files(&window, 1);
        let label = window
            .imp()
            .files_count_label
            .borrow()
            .clone()
            .expect("window builds a count label");
        let rebuilds = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let rebuilds_clone = rebuilds.clone();
        label.connect_label_notify(move |_| {
            rebuilds_clone.set(rebuilds_clone.get() + 1);
        });

        window.add_paths(vec![dir.join("b.txt"), dir.join("c.txt")]);
        wait_for_files(&window, 3);

        assert_eq!(window.file_entries().len(), 3);
        assert_eq!(rebuilds.get(), 1, "two paths must cost one list rebuild");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scenario_a_failed_rename_unwinds_the_whole_batch() {
        let window = test_window();
        let rules_list = rules_list_of(&window);
        let dir = temp_dir("partial-rename");
        for name in ["keep_a.txt", "keep_b.txt"] {
            std::fs::write(dir.join(name), b"x").expect("write file");
        }

        window.add_paths(vec![dir.join("keep_a.txt"), dir.join("keep_b.txt")]);
        wait_for_files(&window, 2);
        add_replace(&window, &rules_list, "keep");

        // Real pipeline: update_preview generated and validated the previews above.
        let files: HashMap<Uuid, FileEntry> = window
            .file_entries()
            .into_iter()
            .map(|entry| (entry.id, entry))
            .collect();
        let previews = window.imp().previews.borrow().clone();
        let plan = crate::engine::plan_renames(&previews, &files).expect("batch plans cleanly");

        // Occupy the second target with a directory between planning and execution, so
        // its rename fails. Phase 1 has already vacated every source, so the failed item
        // has nowhere safe to return to while the finished moves stand: the batch unwinds
        // rather than landing half-applied.
        std::fs::create_dir(dir.join("x_b.txt")).expect("block b's target");
        let result = crate::engine::execute_rename_plan(plan);
        assert_eq!(result.success_count(), 0, "an unwound batch renames nothing");
        assert!(result.failure_count() >= 1);

        window.handle_rename_result(result);

        let remaining: Vec<PathBuf> = window
            .file_entries()
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        assert_eq!(
            remaining,
            vec![dir.join("keep_a.txt"), dir.join("keep_b.txt")],
            "nothing was renamed, so both files stay in the list"
        );
        assert!(
            dir.join("keep_a.txt").exists() && dir.join("keep_b.txt").exists(),
            "both originals must be back in place after the unwind"
        );
        assert!(!dir.join("x_a.txt").exists(), "no target may survive an unwind");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A refused undo is reported as a plain shortfall in the count, which leaves the
    /// user with no idea that something is sitting on the destination. Drives a real
    /// refusal rather than a hand-built result, so it stays honest about the reason
    /// string the undo manager actually produces.
    #[test]
    fn a_refused_undo_tells_the_user_why() {
        use crate::core::{RenameRule, ReplaceRule, RuleType};
        use crate::engine::execute_renames;

        let dir = std::env::temp_dir().join(format!("bulk-renamer-undo-msg-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let data = dir.join("undo-data");
        std::fs::create_dir_all(&data).expect("create data dir");

        let a = dir.join("a.txt");
        std::fs::write(&a, "original").expect("write a");

        let config = RenameConfig {
            rules: vec![RenameRule::new(RuleType::Replace(ReplaceRule {
                find: "a".to_string(),
                replace: "z".to_string(),
                ..Default::default()
            }))],
            ..Default::default()
        };

        let entry = FileEntry::from_path(a.clone(), 0).expect("file entry");
        let mut engine = RenameEngine::new(config);
        let previews = engine.generate_previews(std::slice::from_ref(&entry));
        let map: HashMap<Uuid, FileEntry> = std::iter::once((entry.id, entry)).collect();
        let batch = execute_renames(&previews, &map)
            .expect("execute renames")
            .batch
            .expect("batch");

        let mut manager = UndoManager::new(data, true);
        manager.record_batch(batch).expect("record");

        // Put something back at the original path so the undo has to refuse.
        std::fs::write(&a, "precious").expect("write precious");
        let result = manager.undo().expect("undo");
        assert_eq!(result.success_count, 0, "the undo must have been refused");

        let summary = format!(
            "Restored {} of {} renamed files.",
            result.success_count, result.total_records
        );
        let message = RenamerWindow::undo_result_message(summary, &result);

        let reason = result.results[0]
            .error
            .as_deref()
            .expect("a refused record carries a reason");
        assert!(
            message.contains(reason),
            "the dialog must repeat the refusal reason {:?}, got {:?}",
            reason,
            message
        );
        assert!(
            !result.all_successful(),
            "a refused undo is not a complete one"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Rust harness runs every test on its own thread, but libadwaita may only
    /// be used from the thread that initialised it, so all widget scenarios share
    /// one test. Run with `cargo test --lib -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a display"]
    fn gtk_widget_regressions() {
        let scenarios: Vec<(&str, fn())> = vec![
            ("datetime_row_delete_removes_the_datetime_rule", scenario_datetime_row_delete_removes_the_datetime_rule),
            ("datetime_row_delete_does_not_panic_when_it_is_the_last_rule", scenario_datetime_row_delete_does_not_panic_when_it_is_the_last_rule),
            ("datetime_row_survives_a_rules_list_rebuild", scenario_datetime_row_survives_a_rules_list_rebuild),
            ("drop_value_yields_every_file_of_a_multi_file_drop", scenario_drop_value_yields_every_file_of_a_multi_file_drop),
            ("drop_value_still_accepts_a_single_file", scenario_drop_value_still_accepts_a_single_file),
            ("drop_target_advertises_the_file_list_type", scenario_drop_target_advertises_the_file_list_type),
            ("adding_many_paths_rebuilds_the_list_once", scenario_adding_many_paths_rebuilds_the_list_once),
            ("a_failed_rename_unwinds_the_whole_batch", scenario_a_failed_rename_unwinds_the_whole_batch),
        ];

        for (name, scenario) in scenarios {
            scenario();
            eprintln!("scenario ok: {}", name);
        }
    }
}
