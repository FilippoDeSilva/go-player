use go_media::MediaBackend;
use serde::Serialize;

#[derive(Serialize)]
struct MediaBackendInfo {
    backend: &'static str,
    name: &'static str,
}

#[tauri::command]
fn media_backend() -> MediaBackendInfo {
    MediaBackendInfo {
        backend: "mpv",
        name: MediaBackend::Mpv.name(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![media_backend])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
