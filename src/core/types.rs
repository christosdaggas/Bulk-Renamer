//! Core type definitions for the Bulk Renamer application.

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Represents a file entry in the rename queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Unique identifier for this entry.
    pub id: Uuid,
    /// Full path to the file.
    pub path: PathBuf,
    /// Original filename (without path).
    pub original_name: String,
    /// File extension (if any).
    pub extension: Option<String>,
    /// Whether this is a directory.
    pub is_directory: bool,
    /// File size in bytes.
    pub size: u64,
    /// Last modified time.
    pub modified: Option<DateTime<Local>>,
    /// Created time.
    pub created: Option<DateTime<Local>>,
    /// Accessed time.
    pub accessed: Option<DateTime<Local>>,
    /// Depth in directory tree (0 for root level).
    pub depth: usize,
    /// Parent folder name.
    pub parent_name: Option<String>,
    /// Cached metadata for expressions.
    #[serde(skip)]
    pub metadata_cache: Option<MetadataCache>,
}

impl FileEntry {
    /// Create a new FileEntry from a path.
    pub fn from_path(path: PathBuf, depth: usize) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(&path)?;
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let extension = if metadata.is_file() {
            path.extension().map(|e| e.to_string_lossy().to_string())
        } else {
            None
        };

        let parent_name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string());

        let modified = metadata.modified().ok().map(|t| DateTime::from(t));
        let created = metadata.created().ok().map(|t| DateTime::from(t));
        let accessed = metadata.accessed().ok().map(|t| DateTime::from(t));

        Ok(Self {
            id: Uuid::new_v4(),
            path,
            original_name: file_name,
            extension,
            is_directory: metadata.is_dir(),
            size: metadata.len(),
            modified,
            created,
            accessed,
            depth,
            parent_name,
            metadata_cache: None,
        })
    }

    /// Get the stem (filename without extension).
    pub fn stem(&self) -> String {
        if let Some(ext) = &self.extension {
            if self.original_name.ends_with(&format!(".{}", ext)) {
                return self.original_name[..self.original_name.len() - ext.len() - 1].to_string();
            }
        }
        self.original_name.clone()
    }

    /// Get the full filename with extension.
    pub fn full_name(&self) -> &str {
        &self.original_name
    }
}

/// Cached metadata for a file.
#[derive(Debug, Clone, Default)]
pub struct MetadataCache {
    /// EXIF data for images.
    pub exif: Option<ExifData>,
    /// ID3 tags for audio files.
    pub id3: Option<Id3Data>,
    /// Image dimensions.
    pub dimensions: Option<(u32, u32)>,
    /// Media duration in seconds.
    pub duration: Option<f64>,
    /// Bitrate in kbps.
    pub bitrate: Option<u32>,
}

/// EXIF metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExifData {
    pub date_taken: Option<DateTime<Local>>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub focal_length: Option<f64>,
    pub aperture: Option<f64>,
    pub iso: Option<u32>,
    pub exposure_time: Option<String>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
    pub orientation: Option<u16>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// ID3 metadata for audio files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Id3Data {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i32>,
    pub track: Option<u32>,
    pub genre: Option<String>,
    pub duration: Option<u32>,
    pub bitrate: Option<u32>,
}

/// Result of a rename preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePreview {
    /// The file entry being renamed.
    pub file_id: Uuid,
    /// Original filename.
    pub original_name: String,
    /// New computed filename.
    pub new_name: String,
    /// Full new path.
    pub new_path: PathBuf,
    /// Status of this rename operation.
    pub status: RenameStatus,
    /// Any error or warning message.
    pub message: Option<String>,
}

/// Status of a rename operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenameStatus {
    /// Name will be changed.
    WillRename,
    /// Name unchanged (same as original).
    Unchanged,
    /// Conflict with existing file.
    Conflict,
    /// Conflict with another file being renamed.
    InternalConflict,
    /// Error (invalid characters, path too long, etc.).
    Error,
    /// Operation completed successfully.
    Completed,
    /// Operation failed.
    Failed,
    /// Skipped by user or filter.
    Skipped,
}

/// Target type for rename operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RenameTarget {
    /// Only rename files.
    #[default]
    FilesOnly,
    /// Only rename folders.
    FoldersOnly,
    /// Rename both files and folders.
    Both,
}

/// Sort column for the preview list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortColumn {
    #[default]
    OriginalName,
    NewName,
    Status,
    Size,
    Modified,
    Extension,
    Path,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

/// Theme preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

/// Current schema version for the settings file.
///
/// Bump this when the on-disk format changes in a way that requires migration.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

fn default_settings_version() -> u32 {
    SETTINGS_SCHEMA_VERSION
}

/// Application settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Schema version of the settings file (for future migrations).
    #[serde(default = "default_settings_version")]
    pub version: u32,
    pub theme: ThemePreference,
    pub accent_color: Option<String>,
    pub confirm_before_rename: bool,
    pub create_undo_script: bool,
    pub undo_persistence_enabled: bool,
    pub log_operations: bool,
    pub log_file_path: Option<PathBuf>,
    pub default_preset: Option<String>,
    pub live_preview: bool,
    pub show_unchanged_files: bool,
    pub show_hidden_files: bool,
    pub follow_symlinks: bool,
    pub metadata_loading_enabled: bool,
    pub recursive_folder_depth: usize,
    pub max_path_length: usize,
    pub window_width: i32,
    pub window_height: i32,
    pub window_maximized: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_SCHEMA_VERSION,
            theme: ThemePreference::System,
            accent_color: None,
            confirm_before_rename: true,
            create_undo_script: true,
            undo_persistence_enabled: true,
            log_operations: true,
            log_file_path: None,
            default_preset: None,
            live_preview: true,
            show_unchanged_files: true,
            show_hidden_files: false,
            follow_symlinks: false,
            metadata_loading_enabled: true,
            recursive_folder_depth: 10,
            max_path_length: 4096,
            window_width: 1200,
            window_height: 800,
            window_maximized: false,
        }
    }
}

impl AppSettings {
    /// Get the path to the config file
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("bulk-renamer").join("settings.toml"))
    }

    /// Load settings from config file, or return defaults if not found
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|path| {
                fs::read_to_string(&path).ok()
            })
            .and_then(|content| {
                toml::from_str(&content).ok()
            })
            .unwrap_or_default()
    }

    /// Save settings to config file
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path()
            .ok_or_else(|| "Could not determine config directory".to_string())?;

        self.save_to_path(&path)
    }

    /// Save settings to a specific path, atomically.
    ///
    /// The data is first written to a temporary file in the same directory
    /// and then renamed over the target, so a crash mid-write can never
    /// leave a truncated or corrupt settings file behind.
    pub fn save_to_path(&self, path: &std::path::Path) -> Result<(), String> {
        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        let content = toml::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("settings.toml");
        let tmp_path = dir.join(format!("{}.tmp-{}", file_name, Uuid::new_v4().simple()));

        let result = (|| {
            use std::io::Write;
            let mut file = fs::File::create(&tmp_path)
                .map_err(|e| format!("Failed to create temp config file: {}", e))?;
            file.write_all(content.as_bytes())
                .map_err(|e| format!("Failed to write temp config file: {}", e))?;
            // Make sure the bytes hit the disk before the rename makes them visible.
            file.sync_all()
                .map_err(|e| format!("Failed to sync temp config file: {}", e))?;
            fs::rename(&tmp_path, path)
                .map_err(|e| format!("Failed to write config file: {}", e))?;
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }

        result
    }
}

/// Record of a completed rename operation (for undo).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameRecord {
    pub id: Uuid,
    pub timestamp: DateTime<Local>,
    pub original_path: PathBuf,
    pub new_path: PathBuf,
    pub was_directory: bool,
}

/// A batch of rename operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameBatch {
    pub id: Uuid,
    pub timestamp: DateTime<Local>,
    pub records: Vec<RenameRecord>,
    pub description: Option<String>,
}

impl RenameBatch {
    pub fn new(records: Vec<RenameRecord>) -> Self {
        Self {
            id: Uuid::new_v4(),
            timestamp: Local::now(),
            records,
            description: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_save_to_path_is_atomic_and_valid() {
        let dir = std::env::temp_dir().join(format!("bulk-renamer-settings-test-{}", Uuid::new_v4()));
        let path = dir.join("settings.toml");

        let mut settings = AppSettings::default();
        settings.window_width = 1024;
        settings.save_to_path(&path).unwrap();
        // Save again to exercise the overwrite path (rename over existing target).
        settings.save_to_path(&path).unwrap();

        assert!(path.exists(), "settings file must exist after save");

        // The saved file must be complete, valid TOML.
        let content = fs::read_to_string(&path).unwrap();
        let loaded: AppSettings = toml::from_str(&content).unwrap();
        assert_eq!(loaded.window_width, 1024);
        assert_eq!(loaded.version, SETTINGS_SCHEMA_VERSION);

        // No temporary files may be left behind.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.contains(".tmp-"))
                    .unwrap_or(false)
            })
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {:?}", leftovers);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_toml_without_version_loads() {
        // Settings written by builds that predate the `version` field.
        let toml_str = r#"
            theme = "Dark"
            confirm_before_rename = false
            window_width = 900
        "#;

        let settings: AppSettings = toml::from_str(toml_str).unwrap();
        assert_eq!(settings.version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.theme, ThemePreference::Dark);
        assert!(!settings.confirm_before_rename);
        assert_eq!(settings.window_width, 900);
        // Missing fields fall back to defaults.
        assert!(settings.live_preview);
        assert_eq!(settings.max_path_length, 4096);
    }
}
