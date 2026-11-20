//! Prevents additional console window on Windows, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebKitGTK's accelerated compositing fails on some drivers (VMware
    // svga / missing DRI3), producing a blank webview. Force software
    // compositing + disable dmabuf renderer so the UI actually paints.
    // Must run before GTK/WebKit init. Opt-out via GO_PLAYER_WEBKIT_GPU=1
    // for machines where GPU compositing works and is preferred.
    #[cfg(target_os = "linux")]
    // SAFETY: called at process start before any threads exist, so no
    // concurrent env access is possible.
    unsafe {
        if std::env::var("GO_PLAYER_WEBKIT_GPU").is_err() {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            eprintln!(
                "[go-player] WebKit: software compositing (set GO_PLAYER_WEBKIT_GPU=1 to try GPU)"
            );
        } else {
            eprintln!("[go-player] WebKit: GPU compositing (GO_PLAYER_WEBKIT_GPU=1)");
        }
    }
    go_player_lib::run()
}
