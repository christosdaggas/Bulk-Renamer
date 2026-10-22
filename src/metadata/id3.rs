//! ID3 tag handling for audio files.

use crate::core::{Id3Data, RenamerError, RenamerResult};
use id3::{Tag, TagLike};
use std::path::Path;

/// Read ID3 tags from an audio file.
pub fn read_id3_tags(path: &Path) -> RenamerResult<Id3Data> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "mp3" => read_mp3_tags(path),
        "m4a" | "m4b" | "m4p" | "m4v" | "mp4" => read_m4a_tags(path),
        "flac" | "ogg" | "wav" => {
            // For these formats, try ID3 first, then fall back to empty
            read_mp3_tags(path).or_else(|_| Ok(Id3Data::default()))
        }
        _ => Err(RenamerError::Id3Error(format!(
            "Unsupported audio format: {}",
            ext
        ))),
    }
}

/// Read ID3 tags from MP3 files.
fn read_mp3_tags(path: &Path) -> RenamerResult<Id3Data> {
    let tag = Tag::read_from_path(path)
        .map_err(|e| RenamerError::Id3Error(e.to_string()))?;

    Ok(Id3Data {
        title: tag.title().map(String::from),
        artist: tag.artist().map(String::from),
        album: tag.album().map(String::from),
        year: tag.year().map(|y| y as i32),
        track: tag.track(),
        genre: tag.genre_parsed().map(|g| g.to_string()),
        duration: tag.duration(),
        bitrate: None, // ID3 doesn't store bitrate directly
    })
}

/// Read tags from M4A/MP4 files using mp4ameta.
fn read_m4a_tags(path: &Path) -> RenamerResult<Id3Data> {
    let tag = mp4ameta::Tag::read_from_path(path)
        .map_err(|e| RenamerError::Id3Error(e.to_string()))?;

    Ok(Id3Data {
        title: tag.title().map(String::from),
        artist: tag.artist().map(String::from),
        album: tag.album().map(String::from),
        year: tag.year().and_then(|y| y.parse().ok()),
        track: tag.track().0.map(|t| t as u32),
        genre: tag.genre().map(String::from),
        duration: tag.duration().map(|d| d.as_secs() as u32),
        bitrate: None,
    })
}

/// Write ID3 tags to an audio file.
pub fn write_id3_tags(path: &Path, data: &Id3Data) -> RenamerResult<()> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "mp3" => write_mp3_tags(path, data),
        "m4a" | "m4b" | "m4p" | "m4v" | "mp4" => write_m4a_tags(path, data),
        _ => Err(RenamerError::Id3Error(format!(
            "Writing tags not supported for format: {}",
            ext
        ))),
    }
}

/// Write ID3 tags to MP3 file.
fn write_mp3_tags(path: &Path, data: &Id3Data) -> RenamerResult<()> {
    let mut tag = Tag::read_from_path(path).unwrap_or_else(|_| Tag::new());

    if let Some(ref title) = data.title {
        tag.set_title(title);
    }
    if let Some(ref artist) = data.artist {
        tag.set_artist(artist);
    }
    if let Some(ref album) = data.album {
        tag.set_album(album);
    }
    if let Some(year) = data.year {
        tag.set_year(year);
    }
    if let Some(track) = data.track {
        tag.set_track(track);
    }
    if let Some(ref genre) = data.genre {
        tag.set_genre(genre);
    }

    tag.write_to_path(path, id3::Version::Id3v24)
        .map_err(|e| RenamerError::Id3Error(e.to_string()))?;

    Ok(())
}

/// Write tags to M4A file.
fn write_m4a_tags(path: &Path, data: &Id3Data) -> RenamerResult<()> {
    let mut tag = mp4ameta::Tag::read_from_path(path)
        .unwrap_or_else(|_| mp4ameta::Tag::default());

    if let Some(ref title) = data.title {
        tag.set_title(title);
    }
    if let Some(ref artist) = data.artist {
        tag.set_artist(artist);
    }
    if let Some(ref album) = data.album {
        tag.set_album(album);
    }
    if let Some(year) = data.year {
        tag.set_year(format!("{}", year));
    }
    if let Some(track) = data.track {
        tag.set_track_number(track as u16);
    }
    if let Some(ref genre) = data.genre {
        tag.set_genre(genre);
    }

    tag.write_to_path(path)
        .map_err(|e| RenamerError::Id3Error(e.to_string()))?;

    Ok(())
}

/// Format ID3 data as a summary string.
pub fn format_id3_summary(data: &Id3Data) -> String {
    let mut parts = Vec::new();

    if let Some(ref artist) = data.artist {
        parts.push(format!("Artist: {}", artist));
    }
    if let Some(ref album) = data.album {
        parts.push(format!("Album: {}", album));
    }
    if let Some(ref title) = data.title {
        parts.push(format!("Title: {}", title));
    }
    if let Some(track) = data.track {
        parts.push(format!("Track: {}", track));
    }
    if let Some(year) = data.year {
        parts.push(format!("Year: {}", year));
    }
    if let Some(ref genre) = data.genre {
        parts.push(format!("Genre: {}", genre));
    }

    parts.join(", ")
}
