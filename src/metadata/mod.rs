//! Metadata handling module.
//!
//! This module provides handlers for reading and writing file metadata,
//! including EXIF data for images and ID3 tags for audio files.

mod exif;
mod id3;
mod attributes;

pub use exif::*;
pub use id3::*;
pub use attributes::*;

use crate::core::{FileEntry, MetadataCache, RenamerResult};
use std::path::Path;

/// Load metadata for a file entry.
pub fn load_metadata(entry: &mut FileEntry) -> RenamerResult<()> {
    let path = &entry.path;
    
    // Determine file type by extension
    let ext = entry
        .extension
        .as_ref()
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let mut cache = MetadataCache::default();

    // Load EXIF for images
    if matches!(ext.as_str(), "jpg" | "jpeg" | "tiff" | "tif" | "webp" | "heic" | "heif") {
        cache.exif = read_exif(path).ok();
        
        // Also get dimensions
        if let Some(exif) = &cache.exif {
            if let (Some(w), Some(h)) = (exif.width, exif.height) {
                cache.dimensions = Some((w, h));
            }
        }
        
        // Try to get dimensions from image if not in EXIF
        if cache.dimensions.is_none() {
            cache.dimensions = get_image_dimensions(path).ok();
        }
    }

    // Load ID3 for audio
    if matches!(ext.as_str(), "mp3" | "flac" | "ogg" | "m4a" | "aac" | "wav") {
        cache.id3 = read_id3_tags(path).ok();
        
        if let Some(id3) = &cache.id3 {
            if let Some(bitrate) = id3.bitrate {
                cache.bitrate = Some(bitrate);
            }
            if let Some(duration) = id3.duration {
                cache.duration = Some(duration as f64);
            }
        }
    }

    // Load image dimensions for other image formats
    if matches!(ext.as_str(), "png" | "gif" | "bmp" | "ico" | "svg") {
        cache.dimensions = get_image_dimensions(path).ok();
    }

    entry.metadata_cache = Some(cache);
    Ok(())
}

/// Get image dimensions using the image crate.
fn get_image_dimensions(path: &Path) -> RenamerResult<(u32, u32)> {
    let dimensions = image::image_dimensions(path)
        .map_err(|e| crate::core::RenamerError::MetadataError(e.to_string()))?;
    Ok(dimensions)
}

/// Batch load metadata for multiple files.
pub async fn load_metadata_batch(entries: &mut [FileEntry]) {
    for entry in entries.iter_mut() {
        let _ = load_metadata(entry);
    }
}
