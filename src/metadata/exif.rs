//! EXIF metadata handling for images.

use crate::core::{ExifData, RenamerError, RenamerResult};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Read EXIF data from an image file.
pub fn read_exif(path: &Path) -> RenamerResult<ExifData> {
    let file = File::open(path).map_err(|e| RenamerError::Io(e))?;
    let mut bufreader = BufReader::new(&file);

    let exif_reader = exif::Reader::new();
    let exif = exif_reader
        .read_from_container(&mut bufreader)
        .map_err(|e| RenamerError::ExifError(e.to_string()))?;

    let mut data = ExifData {
        date_taken: None,
        camera_make: None,
        camera_model: None,
        focal_length: None,
        aperture: None,
        iso: None,
        exposure_time: None,
        gps_latitude: None,
        gps_longitude: None,
        orientation: None,
        width: None,
        height: None,
    };

    // Date taken
    if let Some(field) = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY) {
        if let exif::Value::Ascii(ref vec) = field.value {
            if let Some(bytes) = vec.first() {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    data.date_taken = parse_exif_datetime(s);
                }
            }
        }
    }

    // Camera make
    if let Some(field) = exif.get_field(exif::Tag::Make, exif::In::PRIMARY) {
        data.camera_make = Some(field.display_value().to_string().trim_matches('"').to_string());
    }

    // Camera model
    if let Some(field) = exif.get_field(exif::Tag::Model, exif::In::PRIMARY) {
        data.camera_model = Some(field.display_value().to_string().trim_matches('"').to_string());
    }

    // Focal length
    if let Some(field) = exif.get_field(exif::Tag::FocalLength, exif::In::PRIMARY) {
        if let exif::Value::Rational(ref vec) = field.value {
            if let Some(r) = vec.first() {
                data.focal_length = Some(r.num as f64 / r.denom as f64);
            }
        }
    }

    // Aperture (F-number)
    if let Some(field) = exif.get_field(exif::Tag::FNumber, exif::In::PRIMARY) {
        if let exif::Value::Rational(ref vec) = field.value {
            if let Some(r) = vec.first() {
                data.aperture = Some(r.num as f64 / r.denom as f64);
            }
        }
    }

    // ISO
    if let Some(field) = exif.get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY) {
        if let exif::Value::Short(ref vec) = field.value {
            if let Some(&iso) = vec.first() {
                data.iso = Some(iso as u32);
            }
        }
    }

    // Exposure time
    if let Some(field) = exif.get_field(exif::Tag::ExposureTime, exif::In::PRIMARY) {
        data.exposure_time = Some(field.display_value().to_string());
    }

    // GPS coordinates
    if let (Some(lat_ref), Some(lat), Some(lon_ref), Some(lon)) = (
        exif.get_field(exif::Tag::GPSLatitudeRef, exif::In::PRIMARY),
        exif.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY),
        exif.get_field(exif::Tag::GPSLongitudeRef, exif::In::PRIMARY),
        exif.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY),
    ) {
        data.gps_latitude = parse_gps_coordinate(&lat.value, &lat_ref.display_value().to_string());
        data.gps_longitude = parse_gps_coordinate(&lon.value, &lon_ref.display_value().to_string());
    }

    // Orientation
    if let Some(field) = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY) {
        if let exif::Value::Short(ref vec) = field.value {
            if let Some(&orientation) = vec.first() {
                data.orientation = Some(orientation);
            }
        }
    }

    // Dimensions
    if let Some(field) = exif.get_field(exif::Tag::PixelXDimension, exif::In::PRIMARY) {
        data.width = get_dimension_value(&field.value);
    } else if let Some(field) = exif.get_field(exif::Tag::ImageWidth, exif::In::PRIMARY) {
        data.width = get_dimension_value(&field.value);
    }

    if let Some(field) = exif.get_field(exif::Tag::PixelYDimension, exif::In::PRIMARY) {
        data.height = get_dimension_value(&field.value);
    } else if let Some(field) = exif.get_field(exif::Tag::ImageLength, exif::In::PRIMARY) {
        data.height = get_dimension_value(&field.value);
    }

    Ok(data)
}

/// Parse EXIF datetime format (YYYY:MM:DD HH:MM:SS).
fn parse_exif_datetime(s: &str) -> Option<DateTime<Local>> {
    // EXIF format: "2024:01:15 14:30:00"
    let dt = NaiveDateTime::parse_from_str(s.trim(), "%Y:%m:%d %H:%M:%S").ok()?;
    Some(Local.from_local_datetime(&dt).single()?)
}

/// Parse GPS coordinate from EXIF rational values.
fn parse_gps_coordinate(value: &exif::Value, reference: &str) -> Option<f64> {
    if let exif::Value::Rational(vec) = value {
        if vec.len() >= 3 {
            let degrees = vec[0].num as f64 / vec[0].denom as f64;
            let minutes = vec[1].num as f64 / vec[1].denom as f64;
            let seconds = vec[2].num as f64 / vec[2].denom as f64;

            let mut coord = degrees + minutes / 60.0 + seconds / 3600.0;

            // Apply sign based on reference
            let ref_lower = reference.to_lowercase();
            if ref_lower.contains('s') || ref_lower.contains('w') {
                coord = -coord;
            }

            return Some(coord);
        }
    }
    None
}

/// Get dimension value from EXIF value.
fn get_dimension_value(value: &exif::Value) -> Option<u32> {
    match value {
        exif::Value::Short(vec) => vec.first().map(|&v| v as u32),
        exif::Value::Long(vec) => vec.first().copied(),
        _ => None,
    }
}

/// Update EXIF date taken field.
/// Note: This is a complex operation that requires rewriting the EXIF data.
/// For now, we provide a placeholder that could be implemented with a library
/// that supports EXIF writing (like little-exif or img-parts).
pub fn update_exif_date(
    _path: &Path,
    _new_date: DateTime<Local>,
) -> RenamerResult<()> {
    // TODO: Implement EXIF date modification
    // This requires a library that can modify EXIF data in-place
    // Options include: little-exif, img-parts, or calling exiftool via command
    Err(RenamerError::ExifError(
        "EXIF date modification not yet implemented".to_string(),
    ))
}

/// Get a summary of EXIF data for display.
pub fn format_exif_summary(data: &ExifData) -> String {
    let mut parts = Vec::new();

    if let Some(ref make) = data.camera_make {
        parts.push(format!("Camera: {}", make));
    }
    if let Some(ref model) = data.camera_model {
        parts.push(format!("Model: {}", model));
    }
    if let Some(date) = data.date_taken {
        parts.push(format!("Date: {}", date.format("%Y-%m-%d %H:%M")));
    }
    if let (Some(w), Some(h)) = (data.width, data.height) {
        parts.push(format!("Size: {}x{}", w, h));
    }
    if let Some(focal) = data.focal_length {
        parts.push(format!("Focal: {:.0}mm", focal));
    }
    if let Some(aperture) = data.aperture {
        parts.push(format!("f/{:.1}", aperture));
    }
    if let Some(iso) = data.iso {
        parts.push(format!("ISO {}", iso));
    }
    if let Some(ref exposure) = data.exposure_time {
        parts.push(format!("Exp: {}", exposure));
    }

    parts.join(" | ")
}
