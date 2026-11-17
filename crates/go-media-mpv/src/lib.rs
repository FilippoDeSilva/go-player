//! libmpv implementation of [`go_media::PlaybackEngine`].
//!
//! ## Status
//!
//! Headless playback works: open / play / pause / stop / seek / track
//! selection / state queries. Video output is currently disabled
//! (`vo=null`) — the native surface/texture integration is the next
//! milestone (see ADR-0001 consequences).
//!
//! ## Native dependency
//!
//! This crate links against `libmpv-2`. At runtime the DLL must be
//! locatable (next to the executable or on `PATH`). At build time the
//! MSVC linker needs an import library — see `docs/` notes or the
//! mpv-player-windows dev package.

use go_media::{
    MediaBackend, MediaError, MediaInfo, MediaResult, PlaybackEngine, PlaybackState,
    PlaybackStatus, TrackKind,
};
use libmpv2::Mpv;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// libmpv-backed playback engine.
///
/// `Mpv` is `Send + Sync`; all calls marshal through libmpv's own
/// thread-safe client API, so this type is safe to move between threads.
pub struct MpvPlaybackEngine {
    mpv: Mpv,
    current: Option<PathBuf>,
}

impl MpvPlaybackEngine {
    /// Create a headless engine (`vo=null`, `ao=null`). Used for tests and
    /// audio-only probing.
    pub fn new_headless() -> MediaResult<Self> {
        Self::build(|init| {
            init.set_property("vo", "null")?;
            init.set_property("ao", "null")?;
            init.set_property("idle", "yes")?;
            init.set_property("input-default-bindings", "no")?;
            init.set_property("terminal", "no")?;
            Ok(())
        })
    }

    /// Create an engine rendering into an existing window via mpv's `wid`
    /// option (a Win32 HWND as `isize` on Windows).
    pub fn with_video_surface(wid: isize) -> MediaResult<Self> {
        Self::build(|init| {
            init.set_property("wid", wid as i64)?;
            init.set_property("idle", "yes")?;
            init.set_property("terminal", "no")?;
            Ok(())
        })
    }

    fn build<F>(configure: F) -> MediaResult<Self>
    where
        F: FnOnce(libmpv2::MpvInitializer) -> libmpv2::Result<()>,
    {
        let mpv = Mpv::with_initializer(configure).map_err(engine_err)?;
        Ok(Self { mpv, current: None })
    }
}

fn engine_err(e: impl std::fmt::Display) -> MediaError {
    MediaError::Internal(format!("libmpv: {e}"))
}

fn mpv_err(e: libmpv2::Error) -> MediaError {
    MediaError::Playback(format!("libmpv: {e}"))
}

impl PlaybackEngine for MpvPlaybackEngine {
    fn backend(&self) -> MediaBackend {
        MediaBackend::Mpv
    }

    fn open(&mut self, source: &Path) -> MediaResult<MediaInfo> {
        let path_str = source
            .to_str()
            .ok_or_else(|| MediaError::InvalidSource("non-UTF-8 path".into()))?;

        // Replace playlist with the single file and start playback.
        self.mpv
            .command("loadfile", &[path_str, "replace"])
            .map_err(mpv_err)?;

        self.current = Some(source.to_path_buf());

        let duration = self
            .mpv
            .get_property::<f64>("duration")
            .ok()
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(Duration::from_secs_f64);

        // Extract metadata from mpv's metadata property (a map of key/value pairs).
        // Common keys: title, artist, album, date, genre, comment.
        let metadata = go_media::MediaMetadata {
            title: self
                .mpv
                .get_property::<String>("metadata/by-key/title")
                .ok(),
            artist: self
                .mpv
                .get_property::<String>("metadata/by-key/artist")
                .ok(),
            album: self
                .mpv
                .get_property::<String>("metadata/by-key/album")
                .ok(),
            year: self
                .mpv
                .get_property::<String>("metadata/by-key/date")
                .ok()
                .and_then(|s| s.parse::<u32>().ok()),
            genre: self
                .mpv
                .get_property::<String>("metadata/by-key/genre")
                .ok(),
            comment: self
                .mpv
                .get_property::<String>("metadata/by-key/comment")
                .ok(),
        };

        // Track enumeration requires parsing mpv's track-list node; that
        // lands with the demuxer-facing work. For now report an empty list.
        Ok(MediaInfo {
            path: source.to_path_buf(),
            duration,
            tracks: Vec::new(),
            metadata,
        })
    }

    fn play(&mut self) -> MediaResult<()> {
        self.mpv.set_property("pause", false).map_err(mpv_err)
    }

    fn pause(&mut self) -> MediaResult<()> {
        self.mpv.set_property("pause", true).map_err(mpv_err)
    }

    fn stop(&mut self) -> MediaResult<()> {
        self.mpv.command("stop", &[]).map_err(mpv_err)?;
        self.current = None;
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> MediaResult<()> {
        let pos = position.as_secs_f64().to_string();
        self.mpv
            .command("seek", &[pos.as_str(), "absolute"])
            .map_err(mpv_err)
    }

    fn state(&self) -> MediaResult<PlaybackState> {
        let paused = self.mpv.get_property::<bool>("pause").map_err(mpv_err)?;
        let position = self
            .mpv
            .get_property::<f64>("time-pos")
            .ok()
            .filter(|t| t.is_finite() && *t >= 0.0)
            .map(Duration::from_secs_f64)
            .unwrap_or_default();
        let duration = self
            .mpv
            .get_property::<f64>("duration")
            .ok()
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(Duration::from_secs_f64);
        let eof = self
            .mpv
            .get_property::<bool>("eof-reached")
            .unwrap_or(false);

        let status = if self.current.is_none() {
            PlaybackStatus::Stopped
        } else if eof {
            PlaybackStatus::Ended
        } else if paused {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        };

        Ok(PlaybackState {
            status,
            position,
            duration,
        })
    }

    fn select_track(&mut self, kind: TrackKind, id: Option<u32>) -> MediaResult<()> {
        let (prop, value) = match (kind, id) {
            (TrackKind::Audio, Some(id)) => ("aid", id.to_string()),
            (TrackKind::Audio, None) => ("aid", "no".to_string()),
            (TrackKind::Subtitle, Some(id)) => ("sid", id.to_string()),
            (TrackKind::Subtitle, None) => ("sid", "no".to_string()),
            // Video track selection is a no-op for single-video files.
            (TrackKind::Video, _) => return Ok(()),
        };
        self.mpv.set_property(prop, value.as_str()).map_err(mpv_err)
    }
}
