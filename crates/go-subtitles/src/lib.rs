//! Subtitle domain: canonical document model plus format adapters.
//!
//! File formats (SRT, WebVTT, ASS/SSA) are import/export concerns —
//! parse into [`SubtitleDocument`], serialize back out. The canonical
//! model never depends on a format.

mod model;
mod srt;
mod vtt;

pub use model::{SubtitleCue, SubtitleDocument, SubtitleValidationError};
pub use srt::{format_srt_timestamp, parse_srt, parse_srt_timing, to_srt};
pub use vtt::{format_vtt_timestamp, parse_vtt, parse_vtt_timing, to_vtt};

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SubtitleError {
    #[error("empty subtitle block")]
    EmptyBlock,
    #[error("missing cue timing line")]
    MissingTiming,
    #[error("cue has no text")]
    EmptyCueText,
    #[error("malformed timing line: {0}")]
    BadTiming(String),
    #[error("malformed timestamp: {0}")]
    BadTimestamp(String),
    #[error("missing WEBVTT header")]
    MissingVttHeader,
    #[error("unsupported subtitle format: {0}")]
    UnsupportedFormat(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("document validation failed: {0}")]
    Validation(#[from] SubtitleValidationError),
}

/// Detected subtitle format for a file path, based on extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleFormat {
    Srt,
    Vtt,
    Ass,
}

pub fn detect_format(path: &Path) -> Option<SubtitleFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "srt" => Some(SubtitleFormat::Srt),
        "vtt" => Some(SubtitleFormat::Vtt),
        "ass" | "ssa" => Some(SubtitleFormat::Ass),
        _ => None,
    }
}

/// Parse subtitle text in the given format.
pub fn parse(text: &str, format: SubtitleFormat) -> Result<SubtitleDocument, SubtitleError> {
    match format {
        SubtitleFormat::Srt => parse_srt(text),
        SubtitleFormat::Vtt => parse_vtt(text),
        SubtitleFormat::Ass => Err(SubtitleError::UnsupportedFormat("ass".into())),
    }
}

/// Serialize a document to the given format.
pub fn serialize(doc: &SubtitleDocument, format: SubtitleFormat) -> Result<String, SubtitleError> {
    match format {
        SubtitleFormat::Srt => Ok(to_srt(doc)),
        SubtitleFormat::Vtt => Ok(to_vtt(doc)),
        SubtitleFormat::Ass => Err(SubtitleError::UnsupportedFormat("ass".into())),
    }
}
