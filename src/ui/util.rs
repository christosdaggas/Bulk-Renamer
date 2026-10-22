//! Small display helpers shared by the panels.

pub(crate) fn get_icon_for_extension(ext: Option<&str>) -> &'static str {
    match ext.map(str::to_ascii_lowercase).as_deref() {
        Some("jpg") | Some("jpeg") | Some("png") | Some("gif") | Some("webp") | Some("svg") => {
            "image-x-generic-symbolic"
        }
        Some("mp3") | Some("flac") | Some("ogg") | Some("wav") | Some("m4a") | Some("aac") => {
            "audio-x-generic-symbolic"
        }
        Some("mp4") | Some("mkv") | Some("avi") | Some("mov") | Some("webm") => {
            "video-x-generic-symbolic"
        }
        Some("pdf") => "x-office-document-symbolic",
        Some("doc") | Some("docx") | Some("odt") => "x-office-document-symbolic",
        Some("xls") | Some("xlsx") | Some("ods") => "x-office-spreadsheet-symbolic",
        Some("ppt") | Some("pptx") | Some("odp") => "x-office-presentation-symbolic",
        Some("txt") | Some("md") | Some("rst") | Some("html") | Some("htm") | Some("xml")
        | Some("json") | Some("yaml") | Some("yml") | Some("toml") => "text-x-generic-symbolic",
        Some("rs") | Some("py") | Some("js") | Some("ts") | Some("c") | Some("cpp")
        | Some("h") | Some("hpp") | Some("css") | Some("scss") => "text-x-generic-symbolic",
        Some("zip") | Some("tar") | Some("gz") | Some("7z") | Some("rar") => {
            "package-x-generic-symbolic"
        }
        Some("appimage") | Some("exe") | Some("deb") | Some("rpm") => {
            "application-x-executable-symbolic"
        }
        _ => "text-x-generic-symbolic",
    }
}

pub(crate) fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.1} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.1} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}
