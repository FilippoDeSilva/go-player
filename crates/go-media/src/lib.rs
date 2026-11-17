//! Media playback domain types and the engine abstraction.
//!
//! This crate defines the stable contract that all playback backends
//! (libmpv, and any future engine) must satisfy. Backend-specific types
//! must never appear in these interfaces.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("media engine is not available: {0}")]
    EngineUnavailable(String),
    #[error("failed to open media: {0}")]
    OpenFailed(String),
    #[error("invalid media source: {0}")]
    InvalidSource(String),
    #[error("unsupported media: {0}")]
    Unsupported(String),
    #[error("playback error: {0}")]
    Playback(String),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type MediaResult<T> = Result<T, MediaError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: u32,
    pub kind: TrackKind,
    /// Human-readable label, e.g. codec name, language, or track title.
    pub title: String,
    pub language: Option<String>,
    pub is_default: bool,
    pub is_selected: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub duration: Option<Duration>,
    pub tracks: Vec<TrackInfo>,
    pub metadata: MediaMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
    Ended,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackState {
    pub status: PlaybackStatus,
    pub position: Duration,
    pub duration: Option<Duration>,
}

/// Identifies which backend implementation is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaBackend {
    /// libmpv — selected in ADR-0001.
    Mpv,
    /// Reserved for a future engine; not implemented.
    Other,
}

impl MediaBackend {
    pub fn name(self) -> &'static str {
        match self {
            MediaBackend::Mpv => "libmpv",
            MediaBackend::Other => "other",
        }
    }
}

/// The stable playback contract. Implementations own all engine-specific
/// threading, rendering, and decoding concerns.
///
/// All methods are expected to be safe to call from the application core;
/// implementations must not block the caller on media I/O or decode work.
pub trait PlaybackEngine: Send {
    /// Identifier for diagnostics and capability reporting.
    fn backend(&self) -> MediaBackend;

    /// Open a media source for playback. Replaces any currently loaded media.
    fn open(&mut self, source: &std::path::Path) -> MediaResult<MediaInfo>;

    fn play(&mut self) -> MediaResult<()>;
    fn pause(&mut self) -> MediaResult<()>;
    fn stop(&mut self) -> MediaResult<()>;

    /// Seek to an absolute position.
    fn seek(&mut self, position: Duration) -> MediaResult<()>;

    fn state(&self) -> MediaResult<PlaybackState>;

    /// Select a track by id. `None` disables selection where supported.
    fn select_track(&mut self, kind: TrackKind, id: Option<u32>) -> MediaResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names_are_stable() {
        assert_eq!(MediaBackend::Mpv.name(), "libmpv");
    }

    #[test]
    fn playback_state_serializes() {
        let state = PlaybackState {
            status: PlaybackStatus::Playing,
            position: Duration::from_secs(12),
            duration: Some(Duration::from_secs(600)),
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"playing\""));
    }
}
