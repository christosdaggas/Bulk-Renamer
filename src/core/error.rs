//! Error types for the GNOME Renamer application.

use std::path::PathBuf;
use thiserror::Error;

/// Main error type for the renamer application.
#[derive(Error, Debug)]
pub enum RenamerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("File not found: {path}")]
    FileNotFound { path: PathBuf },

    #[error("Permission denied: {path}")]
    PermissionDenied { path: PathBuf },

    #[error("Invalid filename: {reason}")]
    InvalidFilename { reason: String },

    #[error("Filename conflict: '{new_name}' already exists")]
    FilenameConflict { new_name: String },

    #[error("Duplicate target name: '{name}' would be used for multiple files")]
    DuplicateTarget { name: String },

    #[error("Path too long: {path} ({length} characters, max {max})")]
    PathTooLong { path: String, length: usize, max: usize },

    #[error("Invalid regex pattern: {0}")]
    InvalidRegex(#[from] regex::Error),

    #[error("Invalid expression: {0}")]
    InvalidExpression(String),

    #[error("Metadata error: {0}")]
    MetadataError(String),

    #[error("EXIF error: {0}")]
    ExifError(String),

    #[error("ID3 error: {0}")]
    Id3Error(String),

    #[error("CSV parse error: {0}")]
    CsvError(#[from] csv::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    TomlError(String),

    #[error("Preset not found: {name}")]
    PresetNotFound { name: String },

    #[error("Operation cancelled by user")]
    Cancelled,

    #[error("Undo not available: {reason}")]
    UndoNotAvailable { reason: String },

    #[error("Clipboard error: {0}")]
    ClipboardError(String),

    #[error("Walkdir error: {0}")]
    WalkdirError(#[from] walkdir::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type alias for renamer operations.
pub type RenamerResult<T> = Result<T, RenamerError>;

/// Validation error for a specific file entry.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub file_index: usize,
    pub original_name: String,
    pub error_type: ValidationErrorType,
    pub message: String,
}

/// Types of validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationErrorType {
    Conflict,
    InvalidCharacters,
    PathTooLong,
    NameEmpty,
    NameUnchanged,
    PermissionDenied,
    FileNotFound,
    ReservedName,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Validation error for '{}': {} - {}",
            self.original_name,
            match self.error_type {
                ValidationErrorType::Conflict => "Conflict",
                ValidationErrorType::InvalidCharacters => "Invalid characters",
                ValidationErrorType::PathTooLong => "Path too long",
                ValidationErrorType::NameEmpty => "Name empty",
                ValidationErrorType::NameUnchanged => "Name unchanged",
                ValidationErrorType::PermissionDenied => "Permission denied",
                ValidationErrorType::FileNotFound => "File not found",
                ValidationErrorType::ReservedName => "Reserved name",
            },
            self.message
        )
    }
}
