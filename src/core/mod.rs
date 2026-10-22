//! Core types and traits for the Bulk Renamer application.
//!
//! This module defines the fundamental data structures used throughout
//! the application, including file entries, rename rules, and results.

pub mod types;
pub mod rules;
pub mod error;

pub use types::*;
pub use rules::*;
pub use error::*;
