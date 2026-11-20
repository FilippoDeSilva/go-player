//! Prevents additional console window on Windows, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebKitGTK's accelerated compositing path fails to present inside
    // this process on some Linux GL stacks (page loads fine, widget is
    // mapped, but no pixels reach the screen — the window shows only
    // its background color). Forcing the non-composited path renders
    // the UI correctly. Scoped to Linux; other platforms unaffected.
    #[cfg(target_os = "linux")]
    {
        // SAFETY: called before any GTK/WebKit initialization; setting
        // env vars this early is the documented WebKitGTK workaround.
        unsafe {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
    go_player_lib::run()
}
