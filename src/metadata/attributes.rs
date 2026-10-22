//! File attribute handling.

use crate::core::{RenamerError, RenamerResult};
use chrono::{DateTime, Local};
use filetime::{set_file_atime, set_file_mtime, FileTime};
use std::path::Path;

/// File attributes that can be modified.
#[derive(Debug, Clone)]
pub struct FileAttributes {
    /// Hidden attribute (Unix: starts with dot, Windows: hidden flag).
    pub hidden: Option<bool>,
    /// Read-only attribute.
    pub read_only: Option<bool>,
    /// Archive attribute (Windows only).
    pub archive: Option<bool>,
}

impl Default for FileAttributes {
    fn default() -> Self {
        Self {
            hidden: None,
            read_only: None,
            archive: None,
        }
    }
}

/// Read file attributes.
pub fn get_file_attributes(path: &Path) -> RenamerResult<FileAttributes> {
    let mut attrs = FileAttributes::default();

    // Check hidden (Unix: starts with dot)
    #[cfg(unix)]
    {
        if let Some(name) = path.file_name() {
            attrs.hidden = Some(name.to_string_lossy().starts_with('.'));
        }
    }

    // Read-only check
    let metadata = std::fs::metadata(path).map_err(|e| RenamerError::Io(e))?;
    attrs.read_only = Some(metadata.permissions().readonly());

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let file_attrs = metadata.file_attributes();
        attrs.hidden = Some((file_attrs & 0x2) != 0); // FILE_ATTRIBUTE_HIDDEN
        attrs.archive = Some((file_attrs & 0x20) != 0); // FILE_ATTRIBUTE_ARCHIVE
    }

    Ok(attrs)
}

/// Set file attributes.
pub fn set_file_attributes(path: &Path, attrs: &FileAttributes) -> RenamerResult<()> {
    // Handle read-only
    if let Some(read_only) = attrs.read_only {
        let metadata = std::fs::metadata(path).map_err(|e| RenamerError::Io(e))?;
        let mut permissions = metadata.permissions();
        permissions.set_readonly(read_only);
        std::fs::set_permissions(path, permissions).map_err(|e| RenamerError::Io(e))?;
    }

    // Platform-specific attributes
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Windows-specific attribute handling would go here
        // This requires using the SetFileAttributes Windows API
    }

    #[cfg(unix)]
    {
        // On Unix, "hidden" is just a convention (dot prefix)
        // which is handled during rename, not as an attribute
    }

    Ok(())
}

/// Set file modification time.
pub fn set_modified_time(path: &Path, time: DateTime<Local>) -> RenamerResult<()> {
    let file_time = FileTime::from_unix_time(time.timestamp(), time.timestamp_subsec_nanos());
    set_file_mtime(path, file_time).map_err(|e| RenamerError::Io(e))?;
    Ok(())
}

/// Set file access time.
pub fn set_accessed_time(path: &Path, time: DateTime<Local>) -> RenamerResult<()> {
    let file_time = FileTime::from_unix_time(time.timestamp(), time.timestamp_subsec_nanos());
    set_file_atime(path, file_time).map_err(|e| RenamerError::Io(e))?;
    Ok(())
}

/// Set both modified and accessed time.
pub fn set_file_times(
    path: &Path,
    modified: Option<DateTime<Local>>,
    accessed: Option<DateTime<Local>>,
) -> RenamerResult<()> {
    if let Some(mtime) = modified {
        set_modified_time(path, mtime)?;
    }
    if let Some(atime) = accessed {
        set_accessed_time(path, atime)?;
    }
    Ok(())
}

/// Copy timestamps from one file to another.
pub fn copy_timestamps(source: &Path, dest: &Path) -> RenamerResult<()> {
    let metadata = std::fs::metadata(source).map_err(|e| RenamerError::Io(e))?;

    if let Ok(mtime) = metadata.modified() {
        let file_time = FileTime::from_system_time(mtime);
        set_file_mtime(dest, file_time).map_err(|e| RenamerError::Io(e))?;
    }

    if let Ok(atime) = metadata.accessed() {
        let file_time = FileTime::from_system_time(atime);
        set_file_atime(dest, file_time).map_err(|e| RenamerError::Io(e))?;
    }

    Ok(())
}

#[cfg(unix)]
/// Get extended attributes (Unix only).
pub fn get_xattr(path: &Path, name: &str) -> RenamerResult<Option<Vec<u8>>> {
    use xattr::FileExt;
    
    let file = std::fs::File::open(path).map_err(|e| RenamerError::Io(e))?;
    match file.get_xattr(name) {
        Ok(value) => Ok(value),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(RenamerError::Io(e)),
    }
}

#[cfg(unix)]
/// Set extended attribute (Unix only).
pub fn set_xattr(path: &Path, name: &str, value: &[u8]) -> RenamerResult<()> {
    use xattr::FileExt;
    
    let file = std::fs::File::open(path).map_err(|e| RenamerError::Io(e))?;
    file.set_xattr(name, value).map_err(|e| RenamerError::Io(e))?;
    Ok(())
}

#[cfg(unix)]
/// List extended attributes (Unix only).
pub fn list_xattrs(path: &Path) -> RenamerResult<Vec<String>> {
    use xattr::FileExt;
    
    let file = std::fs::File::open(path).map_err(|e| RenamerError::Io(e))?;
    let attrs: Vec<String> = file
        .list_xattr()
        .map_err(|e| RenamerError::Io(e))?
        .filter_map(|a| a.to_str().map(|s| s.to_string()))
        .collect();
    Ok(attrs)
}
