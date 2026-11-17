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
    parent_hwnd: Mutex<Option<isize>>,
}

#[derive(Serialize)]
struct MediaBackendInfo {
    backend: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
struct MediaInfoDto {
    path: String,
    duration_ms: Option<u64>,
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

/// Video region in client coords: below the 40px titlebar, above the
/// 56px control bar, full width.
#[cfg(windows)]
fn video_rect(client: windows::Win32::Foundation::RECT) -> windows::Win32::Foundation::RECT {
    use windows::Win32::Foundation::RECT;
    RECT {
        left: 0,
        top: 40,
        right: client.right,
        bottom: (client.bottom - 56).max(40),
    }
}

#[tauri::command]
fn open_media(
    window: Window,
    state: State<AppState>,
    path: String,
) -> Result<MediaInfoDto, String> {
    let src = PathBuf::from(&path);
    let mut guard = state.engine.lock().map_err(|e| e.to_string())?;

    if guard.is_none() {
        #[cfg(windows)]
        {
            let hwnd = window.hwnd().map_err(|e| format!("hwnd failed: {e}"))?;
            let parent = windows::Win32::Foundation::HWND(hwnd.0);
            let client =
                unsafe { video_surface::client_rect(parent) }.ok_or("failed to get client rect")?;
            eprintln!("[go-player] client rect: {client:?}");
            let surface =
                unsafe { video_surface::create_video_surface(parent, video_rect(client)) }
                    .ok_or("failed to create video surface")?;
            eprintln!("[go-player] video surface hwnd: {}", surface.0);
            *state.video_hwnd.lock().map_err(|e| e.to_string())? = Some(surface);
            *state.parent_hwnd.lock().map_err(|e| e.to_string())? = Some(parent.0 as isize);
            let engine = MpvPlaybackEngine::with_video_surface(surface.0)
                .map_err(|e| format!("mpv init failed: {e}"))?;
            eprintln!("[go-player] mpv engine initialized");
            *guard = Some(engine);
        }
        #[cfg(not(windows))]
        {
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
    Ok(MediaInfoDto {
        path: info.path.display().to_string(),
        duration_ms: info.duration.map(|d| d.as_millis() as u64),
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
    with_engine(&state, |e| e.stop())
}

#[tauri::command]
fn media_seek(state: State<AppState>, position_ms: u64) -> Result<(), String> {
    with_engine(&state, |e| e.seek(Duration::from_millis(position_ms)))
}

fn reposition_video_surface(state: &AppState) {
    #[cfg(windows)]
    {
        let (Some(surface), Some(parent)) = (
            *state.video_hwnd.lock().unwrap(),
            *state.parent_hwnd.lock().unwrap(),
        ) else {
            return;
        };
        let parent = windows::Win32::Foundation::HWND(parent as *mut _);
        if let Some(client) = unsafe { video_surface::client_rect(parent) } {
            unsafe { video_surface::resize_video_surface(surface, video_rect(client)) };
        }
    }
    #[cfg(not(windows))]
    {
        let _ = state;
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Resized(_) = event {
                reposition_video_surface(&window.state::<AppState>());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
