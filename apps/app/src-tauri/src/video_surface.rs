//! Locates and manages the child window that libmpv creates under the
//! main window HWND (which we pass to mpv as its `wid` option).
//!
//! The Tauri window is transparent and the WebView2 child stays stacked
//! above mpv's window, so DOM UI — including translucent overlays —
//! composites directly over the video and keeps normal mouse input.
//! This module only repositions mpv's window: bottom of the sibling
//! z-order, full client size, shown for video / hidden for audio.
//!
//! Safety: callers must pass valid window handles. mpv's window is
//! created on mpv's own thread, so every call here must be short and
//! must not hold locks across it.

#![allow(unsafe_code)]

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GW_CHILD, GW_HWNDNEXT, GetClassNameW, GetClientRect, GetWindow, HWND_BOTTOM,
    HWND_TOP, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow,
};
use windows::core::BOOL;

/// Handle to mpv's child window, stored as `isize`.
#[derive(Debug, Clone, Copy)]
pub struct VideoSurfaceHwnd(pub isize);

/// Find the `mpv` class child window under `parent`, if mpv has created
/// it yet. Returns `None` while playback has not initialized a VO.
pub unsafe fn find_mpv_child(parent: HWND) -> Option<HWND> {
    let mut child = unsafe { GetWindow(parent, GW_CHILD) }.ok()?;
    if child.0.is_null() {
        return None;
    }
    loop {
        let mut buf = [0u16; 64];
        let len = unsafe { GetClassNameW(child, &mut buf) } as usize;
        if String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case("mpv") {
            return Some(child);
        }
        match unsafe { GetWindow(child, GW_HWNDNEXT) } {
            Ok(next) if !next.0.is_null() => child = next,
            _ => return None,
        }
    }
}

/// Push mpv's window to the bottom of the sibling z-order so the
/// (transparent) webview composites above it.
pub unsafe fn set_bottom(hwnd: VideoSurfaceHwnd) {
    let _ = unsafe {
        SetWindowPos(
            HWND(hwnd.0 as *mut _),
            Some(HWND_BOTTOM),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
}

/// Resize mpv's window to the parent's full client rect.
pub unsafe fn resize_to_parent(parent: HWND, hwnd: VideoSurfaceHwnd) {
    let mut rc = RECT::default();
    if unsafe { GetClientRect(parent, &mut rc) }.is_err() {
        return;
    }
    let _ = unsafe {
        SetWindowPos(
            HWND(hwnd.0 as *mut _),
            None,
            rc.left,
            rc.top,
            rc.right - rc.left,
            rc.bottom - rc.top,
            SWP_NOACTIVATE,
        )
    };
}

unsafe extern "system" fn collect_children(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let v = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    v.push(hwnd);
    BOOL(1)
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

/// Log every descendant window's class for z-order diagnostics.
pub unsafe fn dump_children(parent: HWND) {
    let mut kids: Vec<HWND> = Vec::new();
    let _ = unsafe {
        EnumChildWindows(
            Some(parent),
            Some(collect_children),
            LPARAM(&mut kids as *mut Vec<HWND> as isize),
        )
    };
    for k in kids {
        eprintln!(
            "[go-player] child hwnd={} class={}",
            k.0 as isize,
            class_name(k)
        );
    }
}

/// Bring the webview hierarchy to the top of the sibling z-order, above
/// mpv's window. `WRY_WEBVIEW` is the direct child of the main window
/// that competes with mpv's window; the `Chrome_Widget*` descendants
/// inside it are topped as well for safety. Called after positioning
/// mpv so the DOM UI always composites over the video.
pub unsafe fn bring_webview_to_top(parent: HWND) {
    let mut kids: Vec<HWND> = Vec::new();
    let _ = unsafe {
        EnumChildWindows(
            Some(parent),
            Some(collect_children),
            LPARAM(&mut kids as *mut Vec<HWND> as isize),
        )
    };
    for child in kids {
        let cls = class_name(child);
        if cls == "WRY_WEBVIEW" || cls.starts_with("Chrome_Widget") {
            let _ = unsafe {
                SetWindowPos(
                    child,
                    Some(HWND_TOP),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
            };
        }
    }
}

/// Show or hide mpv's window without changing its position.
pub unsafe fn set_visible(hwnd: VideoSurfaceHwnd, visible: bool) {
    let _ = unsafe {
        ShowWindow(
            HWND(hwnd.0 as *mut _),
            if visible { SW_SHOW } else { SW_HIDE },
        )
    };
}
