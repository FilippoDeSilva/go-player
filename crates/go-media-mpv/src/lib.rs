//! libmpv implementation of [`go_media::PlaybackEngine`].
//!
//! ## Status
//!
//! Headless playback works: open / play / pause / stop / seek / track
//! selection / state queries. `with_video_surface` embeds video into a
//! native window via mpv's `wid` option; `new_headless` uses `vo=null`
//! for tests and audio-only probing.
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
use libmpv2::{Mpv, events};
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
            init.set_property("input-vo-keyboard", "no")?;
            // Keep mpv's child window alive at end-of-file so the last
            // frame stays on screen instead of the surface going blank.
            init.set_property("keep-open", "yes")?;
            init.set_property("force-window", "yes")?;
            Ok(())
        })
    }

    /// Set volume as a percentage (0–100).
    pub fn set_volume(&self, percent: f64) -> MediaResult<()> {
        self.mpv
            .set_property("volume", percent.clamp(0.0, 100.0))
            .map_err(mpv_err)
    }

    /// Mute or unmute playback.
    pub fn set_muted(&self, muted: bool) -> MediaResult<()> {
        self.mpv.set_property("mute", muted).map_err(mpv_err)
    }

    fn build<F>(configure: F) -> MediaResult<Self>
    where
        F: FnOnce(libmpv2::MpvInitializer) -> libmpv2::Result<()>,
    {
        let mpv = Mpv::with_initializer(configure).map_err(engine_err)?;
        // FileLoaded lets open() wait until mpv has actually loaded the
        // file; EndFile surfaces load failures as Err from wait_event.
        let _ = mpv.enable_event(events::mpv_event_id::FileLoaded);
        let _ = mpv.enable_event(events::mpv_event_id::EndFile);
        Ok(Self { mpv, current: None })
    }

    /// Read a numeric mpv property as a `Duration` (seconds), ignoring
    /// missing, non-finite, or negative values.
    fn prop_secs(&self, name: &str) -> Option<Duration> {
        self.mpv
            .get_property::<f64>(name)
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(Duration::from_secs_f64)
    }

    /// Read a `metadata/by-key/<key>` string property.
    fn meta_str(&self, key: &str) -> Option<String> {
        self.mpv
            .get_property::<String>(&format!("metadata/by-key/{key}"))
            .ok()
    }

    /// Block until mpv reports the newly loaded file (or fails / times
    /// out), so duration and metadata reads see populated values. mpv
    /// loads asynchronously — reading properties right after `loadfile`
    /// returns almost always yields empty results.
    fn wait_until_loaded(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline
                .saturating_duration_since(std::time::Instant::now())
                .as_secs_f64();
            if remaining <= 0.0 {
                return;
            }
            match self.mpv.wait_event(remaining) {
                Some(Ok(events::Event::FileLoaded)) | Some(Err(_)) | None => return,
                // Stale EndFile events belong to the previous file;
                // keep waiting for the new file's FileLoaded.
                _ => continue,
            }
        }
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
        self.wait_until_loaded();

        let duration = self.prop_secs("duration").filter(|d| !d.is_zero());

        // Common metadata keys: title, artist, album, date, genre, comment.
        let metadata = go_media::MediaMetadata {
            title: self.meta_str("title"),
            artist: self.meta_str("artist"),
            album: self.meta_str("album"),
            year: self.meta_str("date").and_then(|s| s.parse().ok()),
            genre: self.meta_str("genre"),
            comment: self.meta_str("comment"),
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
        // At EOF mpv has already stopped advancing; `pause=no` alone is a
        // no-op, so rewind to the start first to make Play act as replay.
        if self
            .mpv
            .get_property::<bool>("eof-reached")
            .unwrap_or(false)
        {
            let _ = self.seek(Duration::ZERO);
        }
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
        let position = self.prop_secs("time-pos").unwrap_or_default();
        let duration = self.prop_secs("duration").filter(|d| !d.is_zero());
        let eof = self
            .mpv
            .get_property::<bool>("eof-reached")
            .unwrap_or(false)
            || matches!((position, duration), (p, Some(d)) if p >= d);

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
