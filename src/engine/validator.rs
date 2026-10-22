//! Filename validation utilities.

use crate::core::{
    FileEntry, RenamePreview, RenameStatus, RenamerError, RenamerResult, ValidationError,
    ValidationErrorType,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Reserved filenames on Windows.
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Invalid characters for filenames on various platforms.
const INVALID_CHARS_WINDOWS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const INVALID_CHARS_UNIX: &[char] = &['/', '\0'];

/// Maximum path length (conservative default).
const MAX_PATH_LENGTH: usize = 4096;
const MAX_FILENAME_LENGTH: usize = 255;

/// Validator for rename operations.
pub struct RenameValidator {
    /// Maximum allowed path length.
    pub max_path_length: usize,
    /// Maximum allowed filename length.
    pub max_filename_length: usize,
    /// Whether to check for platform-specific restrictions.
    pub check_platform_restrictions: bool,
    /// Whether the destination filesystem ignores case when comparing paths.
    pub case_insensitive_fs: bool,
    /// Whether to allow hidden files (starting with .).
    pub allow_hidden: bool,
}

impl Default for RenameValidator {
    fn default() -> Self {
        Self {
            max_path_length: MAX_PATH_LENGTH,
            max_filename_length: MAX_FILENAME_LENGTH,
            // ':', '?', '*' and trailing periods are ordinary characters on Unix, and
            // rejecting them blocks the whole batch.
            check_platform_restrictions: cfg!(windows),
            case_insensitive_fs: cfg!(windows),
            allow_hidden: true,
        }
    }
}

impl RenameValidator {
    /// Create a new validator with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate a single filename.
    pub fn validate_filename(&self, name: &str) -> RenamerResult<()> {
        // Check for empty name
        if name.is_empty() {
            return Err(RenamerError::InvalidFilename {
                reason: "Filename cannot be empty".to_string(),
            });
        }

        // Check for whitespace-only name
        if name.trim().is_empty() {
            return Err(RenamerError::InvalidFilename {
                reason: "Filename cannot be only whitespace".to_string(),
            });
        }

        // Check filename length
        if name.len() > self.max_filename_length {
            return Err(RenamerError::InvalidFilename {
                reason: format!(
                    "Filename too long ({} chars, max {})",
                    name.len(),
                    self.max_filename_length
                ),
            });
        }

        // Check for invalid characters
        let invalid_chars = if self.check_platform_restrictions {
            INVALID_CHARS_WINDOWS
        } else {
            INVALID_CHARS_UNIX
        };

        for ch in invalid_chars {
            if name.contains(*ch) {
                return Err(RenamerError::InvalidFilename {
                    reason: format!("Filename contains invalid character: '{}'", ch),
                });
            }
        }

        // Check for control characters
        if name.chars().any(|c| c.is_control()) {
            return Err(RenamerError::InvalidFilename {
                reason: "Filename contains control characters".to_string(),
            });
        }

        // Check for names ending with space or period (Windows restriction)
        if self.check_platform_restrictions {
            if name.ends_with(' ') || name.ends_with('.') {
                return Err(RenamerError::InvalidFilename {
                    reason: "Filename cannot end with space or period".to_string(),
                });
            }
        }

        // Check for reserved names (Windows)
        if self.check_platform_restrictions {
            let name_upper = name.to_uppercase();
            let base_name = name_upper.split('.').next().unwrap_or(&name_upper);
            if WINDOWS_RESERVED.contains(&base_name) {
                return Err(RenamerError::InvalidFilename {
                    reason: format!("'{}' is a reserved filename", name),
                });
            }
        }

        Ok(())
    }

    /// Validate a complete path.
    pub fn validate_path(&self, path: &Path) -> RenamerResult<()> {
        let path_str = path.to_string_lossy();

        // Check path length
        if path_str.len() > self.max_path_length {
            return Err(RenamerError::PathTooLong {
                path: path_str.to_string(),
                length: path_str.len(),
                max: self.max_path_length,
            });
        }

        // Validate the filename component
        if let Some(name) = path.file_name() {
            self.validate_filename(&name.to_string_lossy())?;
        }

        Ok(())
    }

    /// Validate a batch of rename previews for conflicts.
    pub fn validate_batch(&self, previews: &[RenamePreview]) -> Vec<ValidationError> {
        self.validate_batch_internal(previews, None)
    }

    /// Validate a batch of rename previews with source file access checks.
    pub fn validate_batch_with_files(
        &self,
        previews: &[RenamePreview],
        files: &HashMap<Uuid, FileEntry>,
    ) -> Vec<ValidationError> {
        self.validate_batch_internal(previews, Some(files))
    }

    /// Key a path for comparison against other paths. Case folding is only correct
    /// where the filesystem itself ignores case: on ext4 'A.txt' and 'a.txt' are two
    /// different files, and treating them as one hides a real overwrite.
    ///
    /// The directory is resolved first, because one directory reached two ways (a
    /// symlinked parent, a '..' component) otherwise keys one file under two names and
    /// the collision checks below never see it. `dirs` memoises that lookup, since a
    /// batch normally shares a parent.
    fn path_key(&self, path: &Path, dirs: &mut HashMap<PathBuf, PathBuf>) -> String {
        let resolved = match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => dirs
                .entry(parent.to_path_buf())
                // A parent that cannot be resolved fails the rename on its own, and
                // keying it literally is what this did before.
                .or_insert_with(|| {
                    std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf())
                })
                .join(name),
            _ => path.to_path_buf(),
        };

        let key = resolved.to_string_lossy();
        if self.case_insensitive_fs {
            key.to_lowercase()
        } else {
            key.into_owned()
        }
    }

    fn validate_batch_internal(
        &self,
        previews: &[RenamePreview],
        files: Option<&HashMap<Uuid, FileEntry>>,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let mut target_names: HashMap<String, Vec<usize>> = HashMap::new();
        let mut dirs: HashMap<PathBuf, PathBuf> = HashMap::new();
        // Paths the two-phase executor will vacate before it moves anything into place.
        let mut source_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Some(files) = files {
            for preview in previews
                .iter()
                .filter(|preview| matches!(preview.status, RenameStatus::WillRename))
            {
                if let Some(entry) = files.get(&preview.file_id) {
                    source_paths.insert(self.path_key(&entry.path, &mut dirs));
                }
            }
        }

        // First pass: collect all target names and check for individual errors
        for (idx, preview) in previews.iter().enumerate() {
            // Skip already errored or unchanged entries. Conflicts are already reported
            // and are never executed, so re-reporting them here would fail the batch on
            // the second validation pass.
            if matches!(
                preview.status,
                RenameStatus::Error | RenameStatus::Unchanged | RenameStatus::InternalConflict
            ) {
                continue;
            }

            // Validate the generated NAME before the path. validate_path inspects
            // path.file_name() after the join, so a separator inside the name is
            // absorbed into path structure and never seen: "ev/il.txt" silently
            // relocates into a subdirectory and "../x" escapes the parent entirely.
            // CSV import feeds new_name as free text, so this is reachable.
            if let Err(err) = self.validate_filename(&preview.new_name) {
                errors.push(ValidationError {
                    file_index: idx,
                    original_name: preview.original_name.clone(),
                    error_type: if preview.new_name.trim().is_empty() {
                        ValidationErrorType::NameEmpty
                    } else {
                        ValidationErrorType::InvalidCharacters
                    },
                    message: err.to_string(),
                });
                continue;
            }

            if let Err(err) = self.validate_path(&preview.new_path) {
                errors.push(ValidationError {
                    file_index: idx,
                    original_name: preview.original_name.clone(),
                    error_type: match err {
                        RenamerError::PathTooLong { .. } => ValidationErrorType::PathTooLong,
                        RenamerError::InvalidFilename { .. } => {
                            if preview.new_name.trim().is_empty() {
                                ValidationErrorType::NameEmpty
                            } else {
                                ValidationErrorType::InvalidCharacters
                            }
                        }
                        _ => ValidationErrorType::InvalidCharacters,
                    },
                    message: err.to_string(),
                });
                continue;
            }

            if let Some(files) = files {
                match files.get(&preview.file_id) {
                    Some(entry) => {
                        if let Err(err) = self.check_file_access(entry) {
                            errors.push(ValidationError {
                                file_index: idx,
                                original_name: preview.original_name.clone(),
                                error_type: match err {
                                    RenamerError::FileNotFound { .. } => {
                                        ValidationErrorType::FileNotFound
                                    }
                                    RenamerError::PermissionDenied { .. } => {
                                        ValidationErrorType::PermissionDenied
                                    }
                                    _ => ValidationErrorType::PermissionDenied,
                                },
                                message: err.to_string(),
                            });
                            continue;
                        }
                    }
                    None => {
                        errors.push(ValidationError {
                            file_index: idx,
                            original_name: preview.original_name.clone(),
                            error_type: ValidationErrorType::FileNotFound,
                            message: "Source file is no longer in the rename queue".to_string(),
                        });
                        continue;
                    }
                }
            }

            // Check if target path already exists on disk. It is only safe to ignore
            // when the file sitting there is one this batch moves out of the way: this
            // preview's own source (a case-only rename on a case-insensitive
            // filesystem), or another source that is itself being renamed.
            let new_path_key = self.path_key(&preview.new_path, &mut dirs);
            let own_source = files
                .and_then(|files| files.get(&preview.file_id))
                .map(|entry| {
                    self.path_key(&entry.path, &mut dirs) == new_path_key
                        // Device/inode comparison catches case-only renames on
                        // case-insensitive mounts reachable from a
                        // case-sensitive OS, where path keys alone differ.
                        || super::engine::paths_are_same_file(&entry.path, &preview.new_path)
                })
                .unwrap_or(false);

            if preview.new_path.exists()
                && preview.new_name != preview.original_name
                && !own_source
                && !source_paths.contains(&new_path_key)
            {
                errors.push(ValidationError {
                    file_index: idx,
                    original_name: preview.original_name.clone(),
                    error_type: ValidationErrorType::Conflict,
                    message: format!("'{}' already exists", preview.new_name),
                });
                continue;
            }

            // Track target names for internal conflict detection
            target_names
                .entry(new_path_key)
                .or_insert_with(Vec::new)
                .push(idx);
        }

        // Second pass: detect internal conflicts (multiple files -> same name)
        for (_target, indices) in &target_names {
            if indices.len() > 1 {
                for &idx in indices {
                    errors.push(ValidationError {
                        file_index: idx,
                        original_name: previews[idx].original_name.clone(),
                        error_type: ValidationErrorType::Conflict,
                        message: format!(
                            "Multiple files would be renamed to '{}'",
                            previews[idx].new_name
                        ),
                    });
                }
            }
        }

        errors
    }

    /// Check if a file can be renamed (permissions, existence, etc.).
    pub fn check_file_access(&self, entry: &FileEntry) -> RenamerResult<()> {
        // Check if file exists
        if !entry.path.exists() {
            return Err(RenamerError::FileNotFound {
                path: entry.path.clone(),
            });
        }

        // Check if parent directory is writable
        if let Some(parent) = entry.path.parent() {
            let metadata = std::fs::metadata(parent).map_err(|_| RenamerError::PermissionDenied {
                path: parent.to_path_buf(),
            })?;

            // On Unix, check write permission
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let permissions = metadata.permissions();
                let mode = permissions.mode();

                // Check if we can write to the directory
                // This is a simplified check; real check would need to consider user/group
                if mode & 0o200 == 0 && mode & 0o020 == 0 && mode & 0o002 == 0 {
                    return Err(RenamerError::PermissionDenied {
                        path: parent.to_path_buf(),
                    });
                }
            }
        }

        Ok(())
    }
}

/// Sanitize a filename by removing or replacing invalid characters, using the default
/// validator settings for this platform.
pub fn sanitize_filename(name: &str, replacement: char) -> String {
    RenameValidator::new().sanitize_filename(name, replacement)
}

impl RenameValidator {
    /// Sanitize a filename by removing or replacing invalid characters. Which
    /// characters are invalid follows the same setting `validate_filename` uses, so a
    /// name this produces is a name that validates.
    pub fn sanitize_filename(&self, name: &str, replacement: char) -> String {
        let invalid_chars: &[char] = if self.check_platform_restrictions {
            INVALID_CHARS_WINDOWS
        } else {
            INVALID_CHARS_UNIX
        };

        let mut result: String = name
            .chars()
            .map(|c| {
                if invalid_chars.contains(&c) || c.is_control() {
                    replacement
                } else {
                    c
                }
            })
            .collect();

        if self.check_platform_restrictions {
            // Remove trailing spaces and periods (Windows restriction)
            while result.ends_with(' ') || result.ends_with('.') {
                result.pop();
            }

            // Check for reserved names
            let upper = result.to_uppercase();
            let base_name = upper.split('.').next().unwrap_or(&upper);
            if WINDOWS_RESERVED.contains(&base_name) {
                result = format!("_{}", result);
            }
        }

        // Ensure non-empty
        if result.is_empty() {
            result = String::from("unnamed");
        }

        // Truncate if too long
        if result.len() > self.max_filename_length {
            // Try to preserve extension
            if let Some(dot_pos) = result.rfind('.') {
                let ext_len = result.len() - dot_pos;
                if ext_len < 20 {
                    let stem_max = self.max_filename_length - ext_len;
                    let stem: String = result.chars().take(stem_max).collect();
                    let ext: String = result.chars().skip(dot_pos).collect();
                    result = format!("{}{}", stem, ext);
                } else {
                    result = result.chars().take(self.max_filename_length).collect();
                }
            } else {
                result = result.chars().take(self.max_filename_length).collect();
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_empty_filename() {
        let validator = RenameValidator::new();
        assert!(validator.validate_filename("").is_err());
        assert!(validator.validate_filename("   ").is_err());
    }

    #[test]
    fn test_validate_valid_filename() {
        let validator = RenameValidator::new();
        assert!(validator.validate_filename("hello.txt").is_ok());
        assert!(validator.validate_filename("my document.pdf").is_ok());
        assert!(validator.validate_filename("file_name-123").is_ok());
    }

    /// Validator configured the way Windows configures itself, so the platform rules
    /// stay under test on every platform.
    fn windows_validator() -> RenameValidator {
        RenameValidator {
            check_platform_restrictions: true,
            case_insensitive_fs: true,
            ..RenameValidator::new()
        }
    }

    #[test]
    fn test_validate_reserved_names() {
        let validator = windows_validator();
        assert!(validator.validate_filename("CON").is_err());
        assert!(validator.validate_filename("con.txt").is_err());
        assert!(validator.validate_filename("NUL").is_err());
    }

    #[test]
    fn test_sanitize_filename() {
        let validator = windows_validator();
        assert_eq!(
            validator.sanitize_filename("hello<world>.txt", '_'),
            "hello_world_.txt"
        );
        assert_eq!(validator.sanitize_filename("file:name", '_'), "file_name");
        assert_eq!(validator.sanitize_filename("", '_'), "unnamed");
    }

    #[test]
    #[cfg(unix)]
    fn unix_names_windows_rejects_are_valid_by_default() {
        let validator = RenameValidator::new();

        // Legal on ext4, and rejecting them greys out the rename for the whole batch.
        assert!(validator.validate_filename("12:30 meeting.txt").is_ok());
        assert!(validator.validate_filename("why?.txt").is_ok());
        assert!(validator.validate_filename("wild*card.txt").is_ok());
        assert!(validator.validate_filename("trailing.").is_ok());
        assert!(validator.validate_filename("CON").is_ok());

        // What is actually illegal on Unix still is.
        assert!(validator.validate_filename("a/b.txt").is_err());
        assert!(validator.validate_filename("nul\0byte").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn sanitize_keeps_characters_that_are_legal_on_unix() {
        assert_eq!(sanitize_filename("12:30 meeting?.txt", '_'), "12:30 meeting?.txt");
        assert_eq!(sanitize_filename("a/b.txt", '_'), "a_b.txt");
        assert_eq!(sanitize_filename("", '_'), "unnamed");
    }

    /// A generated name containing a separator must be rejected, not absorbed
    /// into path structure. validate_path alone cannot catch this because it
    /// inspects file_name() after the join.
    #[test]
    fn generated_names_cannot_relocate_files() {
        let validator = RenameValidator::new();
        let dir = std::path::Path::new("/tmp/br_traversal_probe");

        for name in ["ev/il.txt", "../escaped.txt", "a/../../b.txt"] {
            let preview = RenamePreview {
                file_id: Uuid::new_v4(),
                original_name: "orig.txt".to_string(),
                new_name: name.to_string(),
                new_path: dir.join(name),
                status: RenameStatus::WillRename,
                message: None,
            };
            let errors = validator.validate_batch(&[preview]);
            assert!(
                !errors.is_empty(),
                "name {name:?} was accepted; it would move the file out of its directory"
            );
        }
    }
}
