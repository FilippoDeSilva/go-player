//! libmpv implementation of [`go_media::PlaybackEngine`].
//!
//! ## Status
//!
//! Playback works: open / play / pause / stop / seek / track selection /
//! state queries. `with_video_surface` embeds video into a native window
//! via mpv's `wid` option; `new_headless` uses `vo=null` for tests and
//! audio-only probing.
//!
//! ## Event model
//!
//! A dedicated watcher thread is the single consumer of mpv's event
//! queue. It observes `time-pos`, `pause`, `eof-reached`, `duration`,
//! and `path` via `mpv_observe_property`, so playback state reaches the
//! host as push notifications (`set_state_callback`) instead of the host
//! polling `state()`. End-of-file is handled inside the watcher: on
//! `END_FILE`/EOF it invokes `set_eof_callback` (so the host can hide the
//! video surface for the no-frame moment), then rewinds to 0 and pauses.
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
pub mod render_gl;
use libmpv2::{Format, events};
pub use render_gl::{GlRenderContext, GlRenderParams};
// Re-exported so hosts can hold the mpv handle (e.g. to create GL
// render contexts without borrowing the whole engine).
pub use libmpv2::Mpv;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

/// Shared slot for a host callback of signature `Fn(T) + Send + Sync`.
type CallbackSlot<T> = Arc<Mutex<Option<Arc<T>>>>;

/// Playback-state push channel: `Fn(PlaybackState) + Send + Sync`.
type StateCallback = dyn Fn(PlaybackState) + Send + Sync;
/// End-of-file notification: `Fn() + Send + Sync`.
type EofCallback = dyn Fn() + Send + Sync;

/// How often the watcher emits a state snapshot while only position
/// ticks are arriving. Status changes (play/pause/eof/load) emit
/// immediately.
const STATE_EMIT_INTERVAL: Duration = Duration::from_millis(200);
/// How long `open()` waits for mpv's FILE_LOADED event.
const LOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// libmpv-backed playback engine.
///
/// `Mpv` is `Send + Sync`; all calls marshal through libmpv's own
/// thread-safe client API, so this type is safe to move between threads.
/// `mpv` is shared with a background event watcher that owns mpv's event
/// queue and pushes [`PlaybackState`] snapshots to the registered
/// callback.
pub struct MpvPlaybackEngine {
    mpv: Arc<Mpv>,
    /// Monotonic id distinguishing engine instances for render-context
    /// invalidation on the host side.
    instance_id: u64,
    current: Option<PathBuf>,
    /// Set by the event watcher when mpv reports FILE_LOADED.
    file_loaded: Arc<AtomicBool>,
    state_cb: CallbackSlot<StateCallback>,
    eof_cb: CallbackSlot<EofCallback>,
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
            // GPU-first: hardware decode where available, software
            // fallback otherwise (auto-safe never fails on missing HW).
            // `hwdec` is an mpv option, not a runtime property — it must
            // go through set_option (set_property would fail init).
            init.set_option("hwdec", "auto-safe")?;
            // Keep mpv's child window alive at end-of-file so the last
            // frame stays on screen instead of the surface going blank.
            init.set_property("keep-open", "yes")?;
            init.set_property("force-window", "yes")?;
            Ok(())
        })
    }

    /// Create an engine with video output disabled but normal audio
    /// routing. Used where no native video surface is available — e.g.
    /// Linux Wayland sessions, where mpv cannot embed into the window,
    /// or platforms still on the headless path.
    pub fn with_audio_only() -> MediaResult<Self> {
        Self::build(|init| {
            init.set_property("vo", "null")?;
            // NOTE: `ao` is deliberately left at its default (auto) so
            // audio actually plays — unlike `new_headless`, which is
            // for tests/probing only.
            init.set_property("idle", "yes")?;
            init.set_property("terminal", "no")?;
            init.set_property("input-default-bindings", "no")?;
            init.set_option("hwdec", "auto-safe")?;
            Ok(())
        })
    }

    /// Create an engine whose video is rendered through libmpv's
    /// render API into a client-supplied GL context (see
    /// [`GlRenderContext`]). Used where mpv cannot own a window —
    /// e.g. GTK on Linux, where the webview draws into the toplevel
    /// and a native child window would cover the whole UI.
    pub fn with_gl_surface() -> MediaResult<Self> {
        Self::build(|init| {
            init.set_property("vo", "libmpv")?;
            init.set_property("idle", "yes")?;
            init.set_property("terminal", "no")?;
            init.set_property("input-default-bindings", "no")?;
            // GPU-first decode; the render API presents via the host GL.
            init.set_option("hwdec", "auto-safe")?;
            init.set_property("keep-open", "yes")?;
            // mpv's render() blocks for frame pacing by default — that
            // would stall the UI thread that owns the GL context.
            init.set_option("video-timing-offset", "0")?;
            Ok(())
        })
    }

    /// Create the OpenGL render context for this engine. Must be called
    /// with the target GL context current, on the thread that will do
    /// all rendering (e.g. the GTK main thread inside a `GtkGLArea`
    /// render callback).
    pub fn create_gl_render_context<C: 'static>(
        &self,
        get_proc_address: fn(&C, &str) -> *mut std::ffi::c_void,
        user: C,
    ) -> MediaResult<GlRenderContext> {
        GlRenderContext::create(
            Arc::clone(&self.mpv),
            GlRenderParams {
                get_proc_address,
                user,
            },
        )
    }

    /// Unique identity of this engine instance — used by the host to
    /// detect engine replacement and recreate render contexts. A
    /// counter (not a pointer) so a freed engine's address can never
    /// alias a new one.
    pub fn instance_id(&self) -> u64 {
        self.instance_id
    }

    /// Shared handle to the underlying mpv instance, for hosts that
    /// must create render contexts without holding the engine (the
    /// engine mutex may be locked during `open`).
    pub fn shared_mpv(&self) -> Arc<Mpv> {
        Arc::clone(&self.mpv)
    }

    /// Register a callback that receives [`PlaybackState`] snapshots.
    /// Position updates arrive at ~5 Hz; status changes arrive
    /// immediately. Replaces host-side polling of
    /// [`PlaybackEngine::state`].
    pub fn set_state_callback(&self, cb: impl Fn(PlaybackState) + Send + Sync + 'static) {
        if let Ok(mut g) = self.state_cb.lock() {
            *g = Some(Arc::new(cb));
        }
    }

    /// Register a callback fired the instant mpv reports end-of-file,
    /// before the automatic rewind-to-start. Used by the host to hide the
    /// video surface during the brief no-frame moment at EOF.
    pub fn set_eof_callback(&self, cb: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut g) = self.eof_cb.lock() {
            *g = Some(Arc::new(cb));
        }
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
        // libplacebo (inside libmpv) refuses to initialize unless
        // LC_NUMERIC is "C" — it prints "Non-C locale detected" and
        // mpv init fails. GTK/toolkits call setlocale(LC_ALL, "")
        // during init, so pin the numeric category here, immediately
        // before mpv_create, regardless of the ambient locale.
        // Safety: setlocale is async-signal-safe to call this early,
        // before mpv's threads exist.
        #[cfg(unix)]
        unsafe {
            libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr());
        }

        let mpv = Mpv::with_initializer(configure).map_err(engine_err)?;
        let mpv = Arc::new(mpv);

        let file_loaded = Arc::new(AtomicBool::new(false));
        let state_cb: CallbackSlot<StateCallback> = Arc::new(Mutex::new(None));
        let eof_cb: CallbackSlot<EofCallback> = Arc::new(Mutex::new(None));

        // Property observations feed the watcher; ids must be unique.
        observe(&mpv, "time-pos", Format::Double, 1);
        observe(&mpv, "pause", Format::Flag, 2);
        observe(&mpv, "eof-reached", Format::Flag, 3);
        observe(&mpv, "duration", Format::Double, 4);
        observe(&mpv, "path", Format::String, 5);
        let _ = mpv.enable_event(events::mpv_event_id::FileLoaded);
        let _ = mpv.enable_event(events::mpv_event_id::EndFile);
        let _ = mpv.enable_event(events::mpv_event_id::Shutdown);

        spawn_event_watcher(
            Arc::clone(&mpv),
            Arc::clone(&file_loaded),
            Arc::clone(&state_cb),
            Arc::clone(&eof_cb),
        );

        static NEXT_INSTANCE_ID: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            mpv,
            instance_id: NEXT_INSTANCE_ID.fetch_add(1, Ordering::Relaxed),
            current: None,
            file_loaded,
            state_cb,
            eof_cb,
        })
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

    /// Block until the event watcher reports the newly loaded file (or
    /// times out), so duration and metadata reads see populated values.
    /// mpv loads asynchronously — reading properties right after
    /// `loadfile` returns almost always yields empty results. The watcher
    /// owns mpv's event queue, so this polls a flag rather than events.
    fn wait_until_loaded(&self) {
        let deadline = Instant::now() + LOAD_TIMEOUT;
        while !self.file_loaded.load(Ordering::Relaxed) {
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Register an mpv property observation; failures are non-fatal (the
/// property simply won't push updates).
fn observe(mpv: &Mpv, name: &str, format: Format, id: u64) {
    let _ = mpv.observe_property(name, format, id);
}

/// mpv's `MPV_END_FILE_REASON_EOF` (0). Other end reasons — stop, quit,
/// error, redirect — must not trigger an automatic rewind.
const END_FILE_REASON_EOF: u32 = 0;

/// Latest observed mpv property values, maintained by the watcher.
#[derive(Default)]
struct Observed {
    position: f64,
    paused: bool,
    eof: bool,
    duration: Option<f64>,
    /// Whether mpv reports a file loaded (`path` property non-empty).
    has_file: bool,
}

impl Observed {
    fn snapshot(&self) -> PlaybackState {
        let duration = self
            .duration
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(Duration::from_secs_f64);
        let position = if self.position.is_finite() && self.position > 0.0 {
            Duration::from_secs_f64(self.position)
        } else {
            Duration::ZERO
        };
        let status = if !self.has_file {
            PlaybackStatus::Stopped
        } else if self.eof {
            PlaybackStatus::Ended
        } else if self.paused {
            PlaybackStatus::Paused
        } else {
            PlaybackStatus::Playing
        };
        PlaybackState {
            status,
            position,
            duration,
        }
    }
}

/// Consume mpv's event queue on a dedicated thread — the only
/// `wait_event` caller, since the queue cannot be drained twice.
///
/// Pushes throttled [`PlaybackState`] snapshots to `state_cb` (immediate
/// on status changes, ~5 Hz for position ticks), flags `file_loaded` for
/// `open()`, and on END_FILE/EOF notifies `eof_cb`, then rewinds to the
/// start and pauses.
fn spawn_event_watcher(
    mpv: Arc<Mpv>,
    file_loaded: Arc<AtomicBool>,
    state_cb: CallbackSlot<StateCallback>,
    eof_cb: CallbackSlot<EofCallback>,
) {
    std::thread::spawn(move || {
        let mut obs = Observed::default();
        let mut last_emit = Instant::now()
            .checked_sub(STATE_EMIT_INTERVAL)
            .unwrap_or_else(Instant::now);
        // Set when something other than a position tick changed, or a
        // file-level event arrived — forces an immediate emit.
        let mut urgent = true;

        loop {
            // Compute the wait so the next emit happens on schedule even
            // if no events arrive.
            let since_emit = last_emit.elapsed();
            let timeout = if urgent {
                0.0
            } else {
                STATE_EMIT_INTERVAL
                    .saturating_sub(since_emit)
                    .as_secs_f64()
                    .max(0.001)
            };

            let shutdown = match mpv.wait_event(timeout) {
                Some(Ok(events::Event::Shutdown)) => true,
                Some(Ok(event)) => {
                    handle_event(&mpv, event, &mut obs, &file_loaded, &eof_cb, &mut urgent);
                    false
                }
                _ => false,
            };
            if shutdown {
                break;
            }

            if urgent || last_emit.elapsed() >= STATE_EMIT_INTERVAL {
                let cb = state_cb.lock().ok().and_then(|g| g.clone());
                if let Some(cb) = cb {
                    cb(obs.snapshot());
                }
                last_emit = Instant::now();
                urgent = false;
            }
        }
    });
}

fn handle_event(
    mpv: &Mpv,
    event: events::Event<'_>,
    obs: &mut Observed,
    file_loaded: &AtomicBool,
    eof_cb: &CallbackSlot<EofCallback>,
    urgent: &mut bool,
) {
    match event {
        events::Event::FileLoaded => {
            file_loaded.store(true, Ordering::Relaxed);
            obs.has_file = true;
            *urgent = true;
        }
        events::Event::EndFile(reason) => {
            file_loaded.store(false, Ordering::Relaxed);
            if reason == END_FILE_REASON_EOF {
                // Notify the host first so it can hide the surface before
                // mpv clears its last frame.
                let cb = eof_cb.lock().ok().and_then(|g| g.clone());
                if let Some(cb) = cb {
                    cb();
                }
                let _ = mpv.command("seek", &["0", "absolute"]);
                let _ = mpv.set_property("pause", true);
            }
            *urgent = true;
        }
        events::Event::StartFile => {
            *urgent = true;
        }
        events::Event::PropertyChange { name, change, .. } => {
            apply_property(obs, name, &change);
            // Position ticks are throttled; everything else is urgent.
            if name != "time-pos" {
                *urgent = true;
            }
        }
        _ => {}
    }
}

fn apply_property(obs: &mut Observed, name: &str, change: &events::PropertyData<'_>) {
    match (name, change) {
        ("time-pos", events::PropertyData::Double(v)) => obs.position = *v,
        ("pause", events::PropertyData::Flag(v)) => obs.paused = *v,
        ("eof-reached", events::PropertyData::Flag(v)) => obs.eof = *v,
        ("duration", events::PropertyData::Double(v)) => {
            obs.duration = if v.is_finite() && *v > 0.0 {
                Some(*v)
            } else {
                None
            };
        }
        ("path", events::PropertyData::Str(s)) => obs.has_file = !s.is_empty(),
        ("path", events::PropertyData::None) => obs.has_file = false,
        // Properties go None when mpv tears down track state (e.g. after
        // EndFile); treat observed flags as cleared.
        ("eof-reached", events::PropertyData::None) => obs.eof = false,
        ("pause", events::PropertyData::None) => obs.paused = false,
        ("time-pos", events::PropertyData::None) => obs.position = 0.0,
        _ => {}
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
        self.file_loaded.store(false, Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_snapshot_maps_status() {
        let mut obs = Observed {
            has_file: true,
            paused: true,
            ..Observed::default()
        };
        assert_eq!(obs.snapshot().status, PlaybackStatus::Paused);
        obs.eof = true;
        assert_eq!(obs.snapshot().status, PlaybackStatus::Ended);
        obs.has_file = false;
        assert_eq!(obs.snapshot().status, PlaybackStatus::Stopped);
    }
}
