//! UI components module.
//!
//! This module contains all GTK4/libadwaita UI components for the application.

pub mod csv_io;
pub mod dialogs;
pub mod execution;
pub mod file_item;
pub mod history_dialog;
pub mod menu;
pub mod preferences_dialog;
pub mod presets_dialog;
pub mod rule_dialogs;
pub mod util;
pub mod window;

pub use window::RenamerWindow;
