//! Prevents additional console window on Windows, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Linux WebKit/GL compatibility for broken or limited GPU stacks.
///
/// WebKitGTK's accelerated compositing requires a working EGL/GBM
/// stack. On some drivers (notably VMware's `vmwgfx`, but also
/// headless/VM setups) the WebProcess dies inside EGL display
/// creation (`EGL_BAD_PARAMETER`) and the webview stays blank while
/// the process keeps running — the window maps but shows nothing.
///
/// Rather than guessing which EGL path fails, we remove WebKit's
/// dependence on EGL entirely on X11: `WEBKIT_DISABLE_COMPOSITING_MODE`
/// makes the WebProcess rasterize the UI without the accelerated
/// compositor, and `WEBKIT_DISABLE_DMABUF_RENDERER` covers the
/// buffer-sharing path. Both are WebKitGTK's official knobs; the UI
/// stays fully functional, just without GPU-composited animations —
/// irrelevant for a media player. Crucially this leaves the process
/// GL stack untouched, so our own GtkGLArea/libmpv rendering still
/// gets full hardware acceleration.
///
/// Scoped to X11 sessions: on native Wayland the dmabuf/compositing
/// path is the primary buffer mechanism and is left alone.
#[cfg(target_os = "linux")]
fn apply_linux_gl_workarounds() {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return;
    }

    // SAFETY: single-threaded here, before GTK/WebKit spawn threads
    // that read these variables; spawned processes inherit them.
    unsafe {
        if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    apply_linux_gl_workarounds();
    go_player_lib::run()
}
