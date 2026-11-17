//! Cover-art and tag extraction for audio files.
//!
//! Reads embedded pictures via `lofty` and falls back to common external
//! cover files (`cover.jpg`, `folder.png`, …) sitting next to the media.
//! Kept separate from the playback engine: cover art must not depend on
//! mpv rendering a video track.

use lofty::file::TaggedFileExt;
use lofty::picture::Picture;
use std::path::{Path, PathBuf};

/// An image extracted from an audio file or its sidecar cover.
#[derive(Debug, Clone)]
pub struct CoverArt {
    pub data: Vec<u8>,
    pub mime: &'static str,
}

const EXTERNAL_NAMES: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "cover.webp",
    "folder.jpg",
    "folder.png",
    "album.jpg",
    "album.png",
    "front.jpg",
    "front.png",
];

fn mime_for(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("png") => Some("image/png"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

/// Extract cover art for `path`: embedded pictures first (largest wins),
/// then a sidecar image in the same directory. Returns `None` when the
/// file has no art or cannot be parsed.
pub fn cover_art(path: &Path) -> Option<CoverArt> {
    embedded_art(path).or_else(|| sidecar_art(path))
}

fn embedded_art(path: &Path) -> Option<CoverArt> {
    let file = lofty::read_from_path(path).ok()?;
    let mut best: Option<&Picture> = None;
    for tag in file.tags() {
        for pic in tag.pictures() {
            let better = match best {
                None => true,
                Some(b) => pic.data().len() > b.data().len(),
            };
            if better {
                best = Some(pic);
            }
        }
    }
    let pic = best?;
    let data = pic.data().to_vec();
    let mime = pic
        .mime_type()
        .map(|m| m.to_string())
        .unwrap_or_else(|| sniff_mime(&data).unwrap_or("image/jpeg").to_string());
    let mime: &'static str = match mime.as_str() {
        "image/png" => "image/png",
        "image/webp" => "image/webp",
        "image/bmp" => "image/bmp",
        "image/gif" => "image/gif",
        _ => "image/jpeg",
    };
    Some(CoverArt { data, mime })
}

/// Guess the image format from magic bytes when the tag has no MIME.
fn sniff_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if data.starts_with(b"BM") {
        Some("image/bmp")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
    }
}

fn sidecar_art(path: &Path) -> Option<CoverArt> {
    let dir = path.parent()?;
    for name in EXTERNAL_NAMES {
        let candidate: PathBuf = dir.join(name);
        let Ok(data) = std::fs::read(&candidate) else {
            continue;
        };
        if let Some(mime) = mime_for(&candidate) {
            return Some(CoverArt { data, mime });
        }
    }
    None
}
