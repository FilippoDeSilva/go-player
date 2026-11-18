//! Locates and manages the child window that libmpv creates under the
//! main window HWND (which we pass to mpv as its `wid` option).
//!
//! The Tauri window is transparent and the WebView2 child stays stacked
//! above mpv's window, so DOM UI — including translucent overlays —
//! composites directly over the video and keeps normal mouse input.
//! This module only repositions mpv's window: bottom of the sibling
//! z-order, full client size, shown for video / hidden for audio.
//!
//! Discovery is event-driven: a `SetWinEventHook` (filtered to this
//! process) fires the instant mpv creates its window — no polling. The
//! hook stays installed afterwards as a "guardian": mpv re-asserts its
//! own visibility and z-order when tracks start or frames change, so
//! every subsequent window event on that hwnd re-runs our configuration.
//!
//! Safety: callers must pass valid window handles. mpv's window is
//! created on mpv's own thread, so every call here must be short and
//! must not hold locks across it.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{ClientToScreen, CreateSolidBrush};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_OBJECT_CREATE, EVENT_OBJECT_SHOW, EnumChildWindows, GCLP_HBRBACKGROUND,
    GetClassNameW, GetClientRect, GetParent, GetWindowRect, HWND_BOTTOM, HWND_TOP, IsWindowVisible,
    MSG, OBJID_WINDOW, PM_REMOVE, PeekMessageW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetClassLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    WINEVENT_OUTOFCONTEXT,
};
use windows::core::BOOL;

/// Handle to mpv's child window, stored as `isize`.
#[derive(Debug, Clone, Copy)]
pub struct VideoSurfaceHwnd(pub isize);

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

/// Resize mpv's window to the parent's full client rect. Compares the
/// current window rect first so the guardian's periodic re-assert is a
/// true no-op in steady state — an unconditional `SetWindowPos` sends
/// WM_WINDOWPOSCHANGED, which makes mpv re-render a frame and flickers.
pub unsafe fn resize_to_parent(parent: HWND, hwnd: VideoSurfaceHwnd) {
    let mut rc = RECT::default();
    if unsafe { GetClientRect(parent, &mut rc) }.is_err() {
        return;
    }
    let mut tl = POINT {
        x: rc.left,
        y: rc.top,
    };
    let mut br = POINT {
        x: rc.right,
        y: rc.bottom,
    };
    unsafe {
        let _ = ClientToScreen(parent, &mut tl);
        let _ = ClientToScreen(parent, &mut br);
    }
    let mut cur = RECT::default();
    if unsafe { GetWindowRect(HWND(hwnd.0 as *mut _), &mut cur) }.is_err() {
        return;
    }
    if cur.left == tl.x && cur.top == tl.y && cur.right == br.x && cur.bottom == br.y {
        return;
    }
    let _ = unsafe {
        SetWindowPos(
            HWND(hwnd.0 as *mut _),
            None,
            tl.x,
            tl.y,
            br.x - tl.x,
            br.y - tl.y,
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

/// Show or hide mpv's window without changing its position. No-op when
/// the window is already in the desired state — calling `ShowWindow`
/// unconditionally every guardian tick generates window messages and
/// repaints that show up as video flicker.
pub unsafe fn set_visible(hwnd: VideoSurfaceHwnd, visible: bool) {
    let h = HWND(hwnd.0 as *mut _);
    if unsafe { IsWindowVisible(h) }.as_bool() == visible {
        return;
    }
    let _ = unsafe { ShowWindow(h, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
}

/// Paint mpv's window background pure black. Between EOF and the
/// auto-rewind there is a moment with no frame to draw; without this the
/// window's default class brush can show as a grey rectangle. Runs once:
/// `CreateSolidBrush` allocates a GDI object, so repeating it leaks a
/// brush every call.
pub unsafe fn set_black_background(hwnd: VideoSurfaceHwnd) {
    // Keyed by hwnd so a recreated mpv window still gets the brush,
    // while repeat calls for the same window don't leak GDI brushes.
    static DONE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
    if DONE.swap(hwnd.0, std::sync::atomic::Ordering::Relaxed) == hwnd.0 {
        return;
    }
    let brush = unsafe { CreateSolidBrush(COLORREF(0)) };
    let _ =
        unsafe { SetClassLongPtrW(HWND(hwnd.0 as *mut _), GCLP_HBRBACKGROUND, brush.0 as isize) };
}

/// Context shared with the WinEvent callback. Callbacks for
/// `WINEVENT_OUTOFCONTEXT` hooks are delivered through the message queue
/// of the thread that installed the hook, so the context lives in a
/// `thread_local` on that thread.
struct WinEventCtx {
    parent: HWND,
    found: Option<HWND>,
    /// Once discovered, the mpv surface hwnd — later events on it mark
    /// `dirty` so the guardian re-applies our window configuration.
    surface: Option<HWND>,
    dirty: bool,
}

thread_local! {
    static WIN_EVENT_CTX: RefCell<Option<WinEventCtx>> = const { RefCell::new(None) };
}

unsafe extern "system" fn win_event_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _id_thread: u32,
    _time: u32,
) {
    // OBJID_WINDOW marks events about the window itself (not children).
    if id_object != OBJID_WINDOW.0 {
        return;
    }
    if !class_name(hwnd).eq_ignore_ascii_case("mpv") {
        return;
    }
    WIN_EVENT_CTX.with(|c| {
        let mut b = c.borrow_mut();
        if let Some(ctx) = b.as_mut() {
            if ctx.surface.map(|h| h.0 == hwnd.0).unwrap_or(false) {
                ctx.dirty = true;
                return;
            }
            let is_child = unsafe { GetParent(hwnd) }
                .map(|p| p.0 == ctx.parent.0)
                .unwrap_or(false);
            if is_child {
                ctx.found = Some(hwnd);
            }
        }
    });
}

/// Find mpv's `mpv`-class child window under `parent` (event-driven via
/// `SetWinEventHook`, filtered to this process), invoke `configure` once
/// with it, then keep guarding: mpv re-asserts its own visibility and
/// z-order when tracks start or frames change, so every subsequent window
/// event on that hwnd — plus a slow periodic re-assert — re-runs
/// `configure`. This thread never returns; it lives for the app's
/// lifetime and costs one `PeekMessage` pump per ~5ms while idle.
///
/// `WINEVENT_OUTOFCONTEXT` callbacks are delivered through this thread's
/// message queue, which is why the pump lives here.
pub unsafe fn manage_mpv_child(
    parent: HWND,
    timeout: Duration,
    mut configure: impl FnMut(HWND, bool) + Send + 'static,
) {
    WIN_EVENT_CTX.with(|c| {
        *c.borrow_mut() = Some(WinEventCtx {
            parent,
            found: None,
            surface: None,
            dirty: false,
        });
    });

    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_CREATE,
            EVENT_OBJECT_SHOW,
            None,
            Some(win_event_proc),
            GetCurrentProcessId(),
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };

    let mut msg = MSG::default();
    let deadline = Instant::now() + timeout;

    // Phase 1: wait for mpv's child window to be created.
    let child = loop {
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let hit = WIN_EVENT_CTX.with(|c| c.borrow().as_ref().and_then(|x| x.found));
        if hit.is_some() || Instant::now() >= deadline {
            break hit;
        }
        std::thread::sleep(Duration::from_millis(2));
    };

    let Some(child) = child else {
        unsafe {
            let _ = UnhookWinEvent(hook);
        }
        WIN_EVENT_CTX.with(|c| *c.borrow_mut() = None);
        eprintln!("[go-player] warning: mpv child window not found");
        return;
    };

    // Phase 2: configure once, then guard forever. `full` is true only
    // for the initial pass and for mpv's own window events — the
    // heartbeat passes false so it performs just the cheap, self-skipping
    // geometry/visibility checks instead of churning z-order every tick.
    WIN_EVENT_CTX.with(|c| {
        if let Some(ctx) = c.borrow_mut().as_mut() {
            ctx.surface = Some(child);
        }
    });
    configure(child, true);

    let mut last_enforce = Instant::now();
    loop {
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let dirty = WIN_EVENT_CTX.with(|c| {
            let mut b = c.borrow_mut();
            let d = b.as_ref().map(|x| x.dirty).unwrap_or(false);
            if let Some(ctx) = b.as_mut() {
                ctx.dirty = false;
            }
            d
        });
        // Re-assert on mpv's own window events, plus a slow heartbeat in
        // case an event is ever missed (e.g. during heavy decode load).
        if dirty {
            configure(child, true);
        } else if last_enforce.elapsed() > Duration::from_millis(500) {
            last_enforce = Instant::now();
            configure(child, false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
