#[cfg(windows)]
mod video_surface;

use go_media::{MediaBackend, PlaybackEngine};
use go_media_mpv::MpvPlaybackEngine;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Manager, State, Window};

#[derive(Default)]
struct AppState {
    engine: Mutex<Option<MpvPlaybackEngine>>,
    #[cfg(windows)]
    video_hwnd: Mutex<Option<video_surface::VideoSurfaceHwnd>>,
    /// Whether mpv's window should be visible (video vs audio).
    #[cfg(windows)]
    video_shown: Mutex<bool>,
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
}

#[derive(Serialize)]
struct PlaybackStateDto {
    status: &'static str,
    position_ms: u64,
    duration_ms: Option<u64>,
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

/// mpv creates its own child window under the `wid` parent (the main
/// window). Poll for it on a background thread — it only appears once
/// the VO initializes after `loadfile` — then pin it to the bottom of
/// the z-order (below the transparent webview), size it to the full
/// client area, and apply the current visibility.
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
        for _ in 0..100 {
            if let Some(child) = unsafe { video_surface::find_mpv_child(parent) } {
                let surface = video_surface::VideoSurfaceHwnd(child.0 as isize);
                let state = app.state::<AppState>();
                let show = state.video_shown.lock().map(|g| *g).unwrap_or(false);
                unsafe {
                    video_surface::set_bottom(surface);
                    video_surface::resize_to_parent(parent, surface);
                    video_surface::bring_webview_to_top(parent);
                    video_surface::set_visible(surface, show);
                    video_surface::dump_children(parent);
                }
                eprintln!("[go-player] mpv child window: {}", child.0 as isize);
                if let Ok(mut g) = state.video_hwnd.lock() {
                    *g = Some(surface);
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!("[go-player] warning: mpv child window not found after 5s");
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
            // child window under it, which we later pin below the
            // transparent webview (see ensure_mpv_surface).
            let hwnd = window.hwnd().map_err(|e| format!("hwnd failed: {e}"))?;
            let parent = windows::Win32::Foundation::HWND(hwnd.0);
            let engine = MpvPlaybackEngine::with_video_surface(parent.0 as isize)
                .map_err(|e| format!("mpv init failed: {e}"))?;
            eprintln!(
                "[go-player] mpv engine initialized (wid={})",
                parent.0 as isize
            );
            *guard = Some(engine);
        }
        #[cfg(not(windows))]
        {
            let _ = &window;
            let _ = has_video;
            let engine =
                MpvPlaybackEngine::new_headless().map_err(|e| format!("mpv init failed: {e}"))?;
            *guard = Some(engine);
        }
    }

    let engine = guard.as_mut().unwrap();
    let info = engine
        .open(&src)
        .map_err(|e| format!("open failed for {}: {e}", src.display()))?;
    eprintln!("[go-player] opened {}", src.display());

    #[cfg(windows)]
    {
        if let Ok(mut g) = state.video_shown.lock() {
            *g = has_video;
        }
        ensure_mpv_surface(window.app_handle());
        if let Some(surface) = *state.video_hwnd.lock().map_err(|e| e.to_string())? {
            unsafe { video_surface::set_visible(surface, has_video) };
        }
    }
    #[cfg(not(windows))]
    let _ = has_video;

    Ok(MediaInfoDto {
        path: info.path.display().to_string(),
        duration_ms: info.duration.map(|d| d.as_millis() as u64),
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
    Ok(PlaybackStateDto {
        status: match s.status {
            go_media::PlaybackStatus::Stopped => "stopped",
            go_media::PlaybackStatus::Playing => "playing",
            go_media::PlaybackStatus::Paused => "paused",
            go_media::PlaybackStatus::Ended => "ended",
            go_media::PlaybackStatus::Error => "error",
        },
        position_ms: s.position.as_millis() as u64,
        duration_ms: s.duration.map(|d| d.as_millis() as u64),
    })
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
