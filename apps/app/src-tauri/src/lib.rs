#[cfg(windows)]
mod video_surface;

use base64::Engine as _;
use go_media::{MediaBackend, PlaybackEngine, PlaybackState};
use go_media_mpv::MpvPlaybackEngine;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, Manager, State, Window};

#[derive(Default)]
struct AppState {
    engine: Mutex<Option<MpvPlaybackEngine>>,
    #[cfg(windows)]
    video_hwnd: Mutex<Option<video_surface::VideoSurfaceHwnd>>,
    /// Whether mpv's window should be visible (video vs audio).
    #[cfg(windows)]
    video_shown: Mutex<bool>,
    /// Forced hide independent of media kind — set during the EOF rewind
    /// so mpv's cleared (grey) frame never shows.
    #[cfg(windows)]
    surface_forced_hidden: Mutex<bool>,
}

#[derive(Serialize)]
struct MediaBackendInfo {
    backend: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
struct MediaMetadataDto {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    year: Option<u32>,
    genre: Option<String>,
    comment: Option<String>,
}

#[derive(Serialize)]
struct MediaInfoDto {
    path: String,
    duration_ms: Option<u64>,
    metadata: MediaMetadataDto,
    /// Ready-to-use `data:` URL of embedded or sidecar cover art (audio).
    cover_art: Option<String>,
}

#[derive(Clone, Serialize)]
struct PlaybackStateDto {
    status: &'static str,
    position_ms: u64,
    duration_ms: Option<u64>,
}

impl From<PlaybackState> for PlaybackStateDto {
    fn from(s: PlaybackState) -> Self {
        Self {
            status: match s.status {
                go_media::PlaybackStatus::Stopped => "stopped",
                go_media::PlaybackStatus::Playing => "playing",
                go_media::PlaybackStatus::Paused => "paused",
                go_media::PlaybackStatus::Ended => "ended",
                go_media::PlaybackStatus::Error => "error",
            },
            position_ms: s.position.as_millis() as u64,
            duration_ms: s.duration.map(|d| d.as_millis() as u64),
        }
    }
}

#[derive(Serialize)]
struct SubtitleSummary {
    cue_count: u32,
    language: Option<String>,
    duration_ms: u64,
}

#[tauri::command]
fn media_backend() -> MediaBackendInfo {
    MediaBackendInfo {
        backend: "mpv",
        name: MediaBackend::Mpv.name(),
    }
}

#[tauri::command]
fn load_subtitles(path: String) -> Result<SubtitleSummary, String> {
    let path = PathBuf::from(path);
    let format = go_subtitles::detect_format(&path)
        .ok_or_else(|| "unsupported subtitle format".to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read failed: {e}"))?;
    let doc = go_subtitles::parse(&text, format).map_err(|e| format!("parse failed: {e}"))?;
    doc.validate()
        .map_err(|e| format!("invalid subtitle document: {e}"))?;
    let duration_ms = doc
        .cues
        .last()
        .map(|c| c.end.as_millis() as u64)
        .unwrap_or(0);
    Ok(SubtitleSummary {
        cue_count: doc.cues.len() as u32,
        language: doc.language,
        duration_ms,
    })
}

/// Visibility the surface should have right now: shown for video unless
/// an EOF rewind has force-hidden it.
#[cfg(windows)]
fn effective_surface_visible(state: &AppState) -> bool {
    let shown = state.video_shown.lock().map(|g| *g).unwrap_or(false);
    let forced = state
        .surface_forced_hidden
        .lock()
        .map(|g| *g)
        .unwrap_or(false);
    shown && !forced
}

/// mpv creates its own child window under the `wid` parent (the main
/// window). A WinEvent hook — not a poll — detects it the instant it
/// appears, then keeps re-asserting our z-order / visibility every time
/// mpv touches that window afterwards (mpv re-shows itself when tracks
/// start, which is what let the audio cover flash fullscreen).
#[cfg(windows)]
fn ensure_mpv_surface(app: &tauri::AppHandle) {
    {
        let state = app.state::<AppState>();
        if state
            .video_hwnd
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false)
        {
            return;
        }
    }
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(hwnd) = window.hwnd() else { return };
    let parent = hwnd.0 as isize; // HWND is !Send; rebuild inside the thread
    let app = app.clone();
    std::thread::spawn(move || {
        let parent = windows::Win32::Foundation::HWND(parent as *mut _);
        let parent_isize = parent.0 as isize;
        unsafe {
            video_surface::manage_mpv_child(parent, Duration::from_secs(5), move |child, full| {
                let parent = windows::Win32::Foundation::HWND(parent_isize as *mut _);
                let surface = video_surface::VideoSurfaceHwnd(child.0 as isize);
                let state = app.state::<AppState>();
                // Z-order churn is only needed when mpv re-asserts itself
                // (dirty events); the heartbeat skips it — SetWindowPos on
                // the webview every tick was itself a flicker source.
                if full {
                    video_surface::set_bottom(surface);
                    video_surface::bring_webview_to_top(parent);
                }
                video_surface::resize_to_parent(parent, surface);
                video_surface::set_visible(surface, effective_surface_visible(&state));
                video_surface::set_black_background(surface);
                if state
                    .video_hwnd
                    .lock()
                    .map(|mut g| g.replace(surface).is_none())
                    .unwrap_or(false)
                {
                    eprintln!("[go-player] mpv child window: {}", child.0 as isize);
                }
            });
        }
    });
}

/// Wire the engine's push callbacks to Tauri events. Called once, right
/// after engine creation: `media://state` carries throttled playback
/// snapshots (replacing frontend polling), `media://eof` fires the
/// instant mpv hits end-of-file so the host can hide the surface while
/// the engine rewinds to frame 0.
fn wire_engine_callbacks(engine: &MpvPlaybackEngine, app: &tauri::AppHandle) {
    let app_state = app.clone();
    engine.set_state_callback(move |s| {
        let _ = app_state.emit("media://state", PlaybackStateDto::from(s));
    });

    let app_eof = app.clone();
    engine.set_eof_callback(move || {
        #[cfg(windows)]
        {
            let state = app_eof.state::<AppState>();
            if let Ok(mut g) = state.surface_forced_hidden.lock() {
                *g = true;
            }
            // Apply effective visibility directly — no IPC roundtrip.
            if let Ok(surface) = state.video_hwnd.lock()
                && let Some(s) = *surface
            {
                unsafe {
                    video_surface::set_visible(s, effective_surface_visible(&state));
                }
            }
        }
        let _ = app_eof.emit("media://eof", ());
    });
}

#[tauri::command]
fn open_media(
    window: Window,
    state: State<AppState>,
    path: String,
    has_video: bool,
) -> Result<MediaInfoDto, String> {
    let src = PathBuf::from(&path);
    let mut guard = state.engine.lock().map_err(|e| e.to_string())?;

    if guard.is_none() {
        #[cfg(windows)]
        {
            // Pass the main window HWND: mpv embeds by creating its own
            // child window under it, which the guardian pins below the
            // transparent webview (see ensure_mpv_surface).
            let hwnd = window.hwnd().map_err(|e| format!("hwnd failed: {e}"))?;
            let parent = windows::Win32::Foundation::HWND(hwnd.0);
            let engine = MpvPlaybackEngine::with_video_surface(parent.0 as isize)
                .map_err(|e| format!("mpv init failed: {e}"))?;
            eprintln!(
                "[go-player] mpv engine initialized (wid={})",
                parent.0 as isize
            );
            wire_engine_callbacks(&engine, window.app_handle());
            *guard = Some(engine);
        }
        #[cfg(not(windows))]
        {
            let _ = &window;
            let _ = has_video;
            let engine =
                MpvPlaybackEngine::new_headless().map_err(|e| format!("mpv init failed: {e}"))?;
            wire_engine_callbacks(&engine, window.app_handle());
            *guard = Some(engine);
        }
    }

    // Configure the surface before opening: mpv's window appears as soon
    // as the VO initializes during loadfile, so visibility must already
    // be correct — hidden for audio so cover art never flashes
    // fullscreen, shown for video. The WinEvent hook is armed here so it
    // catches the window the moment mpv creates it.
    #[cfg(windows)]
    {
        if let Ok(mut g) = state.video_shown.lock() {
            *g = has_video;
        }
        if let Ok(mut g) = state.surface_forced_hidden.lock() {
            *g = false;
        }
        ensure_mpv_surface(window.app_handle());
        if let Ok(surface) = state.video_hwnd.lock()
            && let Some(s) = *surface
        {
            unsafe {
                video_surface::set_visible(s, effective_surface_visible(&state));
            }
        }
    }
    #[cfg(not(windows))]
    let _ = has_video;

    let engine = guard.as_mut().unwrap();
    let info = engine
        .open(&src)
        .map_err(|e| format!("open failed for {}: {e}", src.display()))?;
    eprintln!("[go-player] opened {}", src.display());

    // Cover art comes from the file's tags / sidecar image — not from
    // mpv's rendered window, so it works regardless of surface state.
    // Returned as a ready-to-use data URL (mime included).
    let cover_art = if has_video {
        None
    } else {
        go_tags::cover_art(&src).map(|art| {
            format!(
                "data:{};base64,{}",
                art.mime,
                base64::engine::general_purpose::STANDARD.encode(art.data)
            )
        })
    };

    Ok(MediaInfoDto {
        path: info.path.display().to_string(),
        duration_ms: info.duration.map(|d| d.as_millis() as u64),
        cover_art,
        metadata: MediaMetadataDto {
            title: info.metadata.title,
            artist: info.metadata.artist,
            album: info.metadata.album,
            year: info.metadata.year,
            genre: info.metadata.genre,
            comment: info.metadata.comment,
        },
    })
}

fn with_engine<T>(
    state: &State<AppState>,
    f: impl FnOnce(&mut MpvPlaybackEngine) -> go_media::MediaResult<T>,
) -> Result<T, String> {
    let mut guard = state.engine.lock().map_err(|e| e.to_string())?;
    let engine = guard.as_mut().ok_or("no media loaded")?;
    f(engine).map_err(|e| e.to_string())
}

#[tauri::command]
fn media_state(state: State<AppState>) -> Result<PlaybackStateDto, String> {
    let s = with_engine(&state, |e| e.state())?;
    Ok(PlaybackStateDto::from(s))
}

#[tauri::command]
fn media_play(state: State<AppState>) -> Result<(), String> {
    with_engine(&state, |e| e.play())
}

#[tauri::command]
fn media_pause(state: State<AppState>) -> Result<(), String> {
    with_engine(&state, |e| e.pause())
}

#[tauri::command]
fn media_stop(state: State<AppState>) -> Result<(), String> {
    with_engine(&state, |e| e.stop())?;
    #[cfg(windows)]
    {
        if let Ok(mut g) = state.video_shown.lock() {
            *g = false;
        }
        if let Ok(mut g) = state.surface_forced_hidden.lock() {
            *g = false;
        }
        if let Some(surface) = *state.video_hwnd.lock().map_err(|e| e.to_string())? {
            unsafe { video_surface::set_visible(surface, false) };
        }
    }
    Ok(())
}

#[tauri::command]
fn media_seek(state: State<AppState>, position_ms: u64) -> Result<(), String> {
    with_engine(&state, |e| e.seek(Duration::from_millis(position_ms)))
}

#[tauri::command]
fn media_set_volume(state: State<AppState>, volume: f64) -> Result<(), String> {
    let guard = state.engine.lock().map_err(|e| e.to_string())?;
    guard
        .as_ref()
        .ok_or("no media loaded")?
        .set_volume(volume)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn media_set_muted(state: State<AppState>, muted: bool) -> Result<(), String> {
    let guard = state.engine.lock().map_err(|e| e.to_string())?;
    guard
        .as_ref()
        .ok_or("no media loaded")?
        .set_muted(muted)
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn surface_set_visible(state: &State<AppState>, visible: bool) -> Result<(), String> {
    if let Some(surface) = *state.video_hwnd.lock().map_err(|e| e.to_string())? {
        unsafe { video_surface::set_visible(surface, visible) };
    }
    Ok(())
}

#[cfg(not(windows))]
fn surface_set_visible(_state: &State<AppState>, _visible: bool) -> Result<(), String> {
    Ok(())
}

/// Hide mpv's video surface (used at EOF while rewinding to frame 0).
#[tauri::command]
fn media_surface_hide(state: State<AppState>) -> Result<(), String> {
    #[cfg(windows)]
    if let Ok(mut g) = state.surface_forced_hidden.lock() {
        *g = true;
    }
    surface_set_visible(&state, false)
}

/// Restore mpv's surface visibility per the current media kind.
#[tauri::command]
fn media_surface_show(state: State<AppState>) -> Result<(), String> {
    #[cfg(not(windows))]
    let _ = &state;
    #[cfg(windows)]
    {
        if let Ok(mut g) = state.surface_forced_hidden.lock() {
            *g = false;
        }
        if let Some(surface) = *state.video_hwnd.lock().map_err(|e| e.to_string())? {
            unsafe {
                video_surface::set_visible(surface, effective_surface_visible(&state));
            }
        }
    }
    Ok(())
}

fn reposition_video_surface(state: &AppState, window: &Window) {
    #[cfg(windows)]
    {
        let Some(surface) = *state.video_hwnd.lock().unwrap() else {
            return;
        };
        let Ok(hwnd) = window.hwnd() else { return };
        let parent = windows::Win32::Foundation::HWND(hwnd.0);
        // Re-assert z-order (mpv below, webview on top) and match the
        // new client size.
        unsafe {
            video_surface::set_bottom(surface);
            video_surface::resize_to_parent(parent, surface);
            video_surface::bring_webview_to_top(parent);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (state, window);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            media_backend,
            load_subtitles,
            open_media,
            media_state,
            media_play,
            media_pause,
            media_stop,
            media_seek,
            media_set_volume,
            media_set_muted,
            media_surface_hide,
            media_surface_show,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Resized(_) = event {
                reposition_video_surface(&window.state::<AppState>(), window);
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    app.run(|_window, _event| {});
}
