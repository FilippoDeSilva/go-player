//! Native child window hosting the mpv video surface.
//!
//! The Tauri webview is a child window covering the whole client area,
//! so mpv cannot draw on the parent HWND (it would sit behind the
//! webview). We create a `WS_CHILD` window over the video region, above
//! the webview in z-order, and hand its HWND to libmpv via `wid`.
//!
//! Safety: callers must pass a valid parent HWND and run on the thread
//! that owns the window (Tauri UI thread). The class registers once and
//! uses `DefWindowProc`; the child dies with its parent.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetWindowLongPtrW, HWND_TOP, RegisterClassW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos, WINDOW_LONG_PTR_INDEX,
    WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
};
use windows::core::PCWSTR;

static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
const CLASS_NAME: &str = "GoPlayerVideoSurface";
const GW_HINSTANCE: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(-6);

/// Win32 `hwnd` as `isize` so it can cross into libmpv's `wid` option.
#[derive(Debug, Clone, Copy)]
pub struct VideoSurfaceHwnd(pub isize);

extern "system" fn video_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn hinstance_of(hwnd: HWND) -> HINSTANCE {
    unsafe { HINSTANCE(GetWindowLongPtrW(hwnd, GW_HINSTANCE) as *mut _) }
}

/// Create the video child window filling `rect` (client coords) of
/// `parent`. Returns its HWND for use as mpv's `wid`.
///
/// # Safety
/// `parent` must be a valid window handle owned by the calling thread.
pub unsafe fn create_video_surface(parent: HWND, rect: RECT) -> Option<VideoSurfaceHwnd> {
    if !CLASS_REGISTERED.swap(true, Ordering::SeqCst) {
        let class_name = wide(CLASS_NAME);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(video_wndproc),
            hInstance: hinstance_of(parent),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&wc) } == 0 {
            CLASS_REGISTERED.store(false, Ordering::SeqCst);
            return None;
        }
    }

    let class_name = wide(CLASS_NAME);
    let hwnd = unsafe {
        CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            Some(parent),
            None,
            Some(hinstance_of(parent)),
            None,
        )
    }
    .ok()?;

    // Above the webview child in z-order.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    };
    Some(VideoSurfaceHwnd(hwnd.0 as isize))
}

/// Move/resize the video surface to fill `rect` (client coords).
///
/// # Safety
/// `hwnd` must be a valid window handle.
pub unsafe fn resize_video_surface(hwnd: VideoSurfaceHwnd, rect: RECT) {
    let hwnd = HWND(hwnd.0 as *mut _);
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    };
}

/// Client rect of `hwnd`.
///
/// # Safety
/// `hwnd` must be a valid window handle.
pub unsafe fn client_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) }.ok()?;
    Some(rect)
}
