//! Bulk Renamer - A bulk file renaming application for GNOME.
//!
//! This is the library crate that exposes all modules for the application.

pub mod app;
pub mod cli;
pub mod core;
pub mod engine;
pub mod expression;
pub mod metadata;
pub mod presets;
pub mod ui;
pub mod undo;

pub use app::RenamerApplication;
