//! GObject item wrapping a `FileEntry` plus its live preview state.
//!
//! One `gio::ListStore` of these backs both the Files panel and the Preview
//! panel; `update_preview` writes preview results into the item properties and
//! the bound cells update in place — no widget rebuilds.

use crate::core::{FileEntry, RenamePreview, RenameStatus};
use gtk4 as gtk;
use gtk::glib;
use glib::prelude::*;
use glib::subclass::prelude::*;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::FileItem)]
    pub struct FileItem {
        #[property(get, set)]
        pub original_name: RefCell<String>,
        #[property(get, set)]
        pub new_name: RefCell<String>,
        #[property(get, set)]
        pub size_text: RefCell<String>,
        #[property(get, set)]
        pub icon_name: RefCell<String>,
        #[property(get, set)]
        pub status_icon: RefCell<String>,
        #[property(get, set)]
        pub status_css: RefCell<String>,
        #[property(get, set)]
        pub tooltip: RefCell<String>,
        #[property(get, set)]
        pub status_code: Cell<u8>,
        #[property(get, set)]
        pub included: Cell<bool>,
        /// User-typed name override; empty means "follow the rules".
        #[property(get, set)]
        pub manual_name: RefCell<String>,
        pub entry: RefCell<Option<FileEntry>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileItem {
        const NAME: &'static str = "BulkRenamerFileItem";
        type Type = super::FileItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for FileItem {}
}

glib::wrapper! {
    pub struct FileItem(ObjectSubclass<imp::FileItem>);
}

impl FileItem {
    pub const STATUS_WILL_RENAME: u8 = 0;
    pub const STATUS_UNCHANGED: u8 = 1;
    pub const STATUS_CONFLICT: u8 = 2;
    pub const STATUS_ERROR: u8 = 3;

    pub fn new(entry: FileEntry) -> Self {
        let item: Self = glib::Object::new();
        item.set_included(true);
        item.set_original_name(entry.original_name.clone());
        item.set_new_name(entry.original_name.clone());
        item.set_size_text(super::util::format_size(entry.size));
        item.set_icon_name(if entry.is_directory {
            "folder-symbolic"
        } else {
            super::util::get_icon_for_extension(entry.extension.as_deref())
        });
        item.set_status_code(Self::STATUS_UNCHANGED);
        item.imp().entry.replace(Some(entry));
        item
    }

    /// The file entry behind this row.
    pub fn entry(&self) -> FileEntry {
        self.imp()
            .entry
            .borrow()
            .clone()
            .expect("FileItem is always constructed with an entry")
    }

    /// Path of the file behind this row.
    pub fn path(&self) -> std::path::PathBuf {
        self.imp()
            .entry
            .borrow()
            .as_ref()
            .map(|entry| entry.path.clone())
            .unwrap_or_default()
    }

    /// The user-typed override for this file, if any.
    pub fn manual_override(&self) -> Option<String> {
        let manual = self.manual_name();
        if manual.is_empty() { None } else { Some(manual) }
    }

    /// Write a preview result into the bound properties.
    pub fn apply_preview(&self, preview: &RenamePreview) {
        let (code, icon, css) = match preview.status {
            RenameStatus::WillRename | RenameStatus::Completed => {
                (Self::STATUS_WILL_RENAME, "object-select-symbolic", "success")
            }
            RenameStatus::Unchanged | RenameStatus::Skipped => {
                (Self::STATUS_UNCHANGED, "action-unavailable-symbolic", "dim-label")
            }
            RenameStatus::Conflict | RenameStatus::InternalConflict => {
                (Self::STATUS_CONFLICT, "dialog-warning-symbolic", "warning")
            }
            RenameStatus::Error | RenameStatus::Failed => {
                (Self::STATUS_ERROR, "dialog-error-symbolic", "error")
            }
        };
        self.set_new_name(preview.new_name.clone());
        self.set_status_code(code);
        self.set_status_icon(icon);
        self.set_status_css(css);
        self.set_tooltip(preview.message.clone().unwrap_or_default());
    }
}
