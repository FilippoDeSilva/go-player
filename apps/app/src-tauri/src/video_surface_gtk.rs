//! `GtkGLArea`-backed video surface for libmpv's render API (Linux).
//!
//! Why this exists: on Linux the webview draws directly into the
//! toplevel X11/Wayland window — there is no separate webview child
//! window, so mpv's `wid` child window would cover the entire UI and
//! eat all input. Instead we insert a `GtkGLArea` *below* the webview
//! in GTK's own widget tree: the vbox that holds the webview is
//! reparented into a `GtkOverlay` with the GL area as the bottom
//! child, so DOM content composites on top exactly like the Windows
//! child-window model. mpv renders through its render API
//! (`vo=libmpv`) into the GL area's framebuffer on the GTK main
//! thread — no frames cross IPC.
//!
//! This works identically on X11 and Wayland because it never touches
//! windowing-system child windows.
//!
//! GTK widgets are `!Send`, so cross-thread steering stores the
//! widget's pointer as a `usize` and reconstructs a non-owning
//! wrapper on the main thread. The widget outlives all uses because
//! it is parented to the toplevel window for the app's lifetime.

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_void;
use std::rc::Rc;
use std::sync::OnceLock;

use go_media_mpv::GlRenderContext;
use gtk::glib;
use gtk::glib::translate::{FromGlibPtrNone, ToGlibPtr};
use gtk::prelude::*;
use tauri::{AppHandle, Manager, Window};

use crate::AppState;

/// Steers the GL surface widget; all GTK calls are marshalled onto the
/// main thread.
#[derive(Clone)]
pub struct SurfaceHandle {
    app: AppHandle,
    /// Borrowed `GtkGLArea*` — valid for the app's lifetime.
    area_ptr: usize,
}

/// Query the currently bound GL draw framebuffer (0 = default). Must
/// be called with the GL context current (inside the render signal).
fn current_draw_fbo() -> i32 {
    // GL_DRAW_FRAMEBUFFER_BINDING
    const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
    type GetIntegerv = unsafe extern "C" fn(u32, *mut i32);
    let p = gl_get_proc_address(&(), "glGetIntegerv");
    if p.is_null() {
        return 0;
    }
    let f: GetIntegerv = unsafe { std::mem::transmute(p) };
    let mut v = 0i32;
    // Safety: valid GL function, called with a live context.
    unsafe { f(GL_DRAW_FRAMEBUFFER_BINDING, &mut v) };
    v
}

/// Rebuild a non-owning `GLArea` wrapper on the GTK main thread.
///
/// # Safety
/// `ptr` must be a live `GtkGLArea` owned by the widget tree; callers
/// guarantee this because the widget is parented to the toplevel.
unsafe fn gl_area_from_ptr(ptr: usize) -> gtk::GLArea {
    unsafe {
        <gtk::GLArea as FromGlibPtrNone<*mut gtk::ffi::GtkGLArea>>::from_glib_none(
            ptr as *mut gtk::ffi::GtkGLArea,
        )
    }
}

impl SurfaceHandle {
    /// Nudge GTK to redraw the GL area (used to (re)create the mpv
    /// render context for a newly installed engine before opening a
    /// file — vo=libmpv refuses to play without one).
    pub fn request_draw(&self) {
        let app = self.app.clone();
        let ptr = self.area_ptr;
        let _ = app.run_on_main_thread(move || unsafe {
            gl_area_from_ptr(ptr).queue_render();
        });
    }

    pub fn set_visible(&self, visible: bool) {
        let app = self.app.clone();
        let ptr = self.area_ptr;
        let _ = app.run_on_main_thread(move || unsafe {
            gl_area_from_ptr(ptr).set_visible(visible);
        });
    }
}

/// GL function lookup for mpv's render context: epoxy first (it
/// handles GLX/EGL/deferral correctly), then raw GLX/EGL loaders.
/// The resolved libraries are intentionally leaked — they must outlive
/// every GL function pointer mpv caches.
fn gl_get_proc_address(_ctx: &(), name: &str) -> *mut c_void {
    type GpaFn = unsafe extern "C" fn(*const std::ffi::c_char) -> *mut c_void;
    static GPA: OnceLock<Option<GpaFn>> = OnceLock::new();
    let f = GPA.get_or_init(|| {
        for (lib_name, sym_name) in [
            ("libepoxy.so.0", "epoxy_get_proc_address"),
            ("libGL.so.1", "glXGetProcAddressARB"),
            ("libEGL.so.1", "eglGetProcAddress"),
        ] {
            // Safety: loading a system GL/EGL library by soname is safe;
            // we only fetch a well-known symbol.
            let Ok(lib) = (unsafe { libloading::Library::new(lib_name) }) else {
                continue;
            };
            // Safety: `sym_name` is a valid NUL-free symbol name in this
            // library or the get fails.
            if let Ok(sym) = unsafe { lib.get::<GpaFn>(sym_name.as_bytes()) } {
                let f = *sym;
                std::mem::forget(lib);
                return Some(f);
            }
        }
        None
    });
    let Some(f) = *f else {
        eprintln!("[go-player] no GL proc-address resolver found");
        return std::ptr::null_mut();
    };
    let Ok(cname) = CString::new(name) else {
        return std::ptr::null_mut();
    };
    // Safety: `f` is a valid GL proc-address function for the current
    // GL context; `name` is NUL-terminated.
    unsafe { f(cname.as_ptr()) }
}

/// Schedule the widget surgery on the GTK main thread. Idempotent —
/// the handle is stored in `AppState::gtk_surface`; repeated calls are
/// no-ops.
pub fn install(app: &AppHandle, window: &Window) {
    let app2 = app.clone();
    let window = window.clone();
    let _ = app.run_on_main_thread(move || install_on_main(&app2, &window));
}

fn install_on_main(app: &AppHandle, window: &Window) {
    let state = app.state::<AppState>();
    if state
        .gtk_surface
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false)
    {
        return;
    }

    let Ok(gtk_window) = window.gtk_window() else {
        eprintln!("[go-player] gtk surface: no gtk window");
        return;
    };
    let Ok(vbox) = window.default_vbox() else {
        eprintln!("[go-player] gtk surface: no default vbox");
        return;
    };

    let gl_area = gtk::GLArea::new();
    gl_area.set_auto_render(false);
    gl_area.set_hexpand(true);
    gl_area.set_vexpand(true);
    gl_area.set_has_depth_buffer(true);
    gl_area.set_visible(false);
    let area_ptr: *mut gtk::ffi::GtkGLArea = gl_area.to_glib_none().0;
    let area_ptr = area_ptr as usize;

    // Render state lives on the main thread only; the render context is
    // recreated whenever the GL-capable engine instance changes.
    let render_ctx: Rc<RefCell<Option<(u64, GlRenderContext)>>> = Rc::new(RefCell::new(None));
    // Set by mpv's update callback (mpv thread) when a new frame is
    // ready; consumed by the render signal on the main thread. mpv's
    // render() clears to black when called without a fresh frame, so
    // we must NOT call it on unrelated GTK draws.
    let frame_pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let last_size = Rc::new(std::cell::Cell::new((0i32, 0i32)));
    let app_render = app.clone();
    {
        let render_ctx = Rc::clone(&render_ctx);
        let frame_pending = std::sync::Arc::clone(&frame_pending);
        let last_size = Rc::clone(&last_size);
        gl_area.connect_render(move |area, _gdk_ctx| {
            let w = area.allocated_width();
            let h = area.allocated_height();
            if w <= 0 || h <= 0 {
                return glib::Propagation::Proceed;
            }

            // The GL-capable engine's mpv handle lives in `gl_engine`
            // (a dedicated slot) so this callback never contends with
            // `open_media`, which holds the engine mutex across the
            // render-context wait.
            let app_state = app_render.state::<AppState>();
            let gl_engine = app_state
                .gl_engine
                .lock()
                .map(|g| g.clone())
                .unwrap_or(None);
            let Some((id, mpv)) = gl_engine else {
                return glib::Propagation::Proceed;
            };
            let mut slot = render_ctx.borrow_mut();
            if slot.as_ref().map(|(i, _)| *i) != Some(id) {
                match GlRenderContext::create(
                    std::sync::Arc::clone(&mpv),
                    go_media_mpv::GlRenderParams {
                        get_proc_address: gl_get_proc_address,
                        user: (),
                    },
                ) {
                    Ok(mut rc) => {
                        // The update callback runs on an mpv thread and
                        // must reach the main thread; the widget pointer
                        // is Send-safe as a usize.
                        let wake_ptr: *mut gtk::ffi::GtkGLArea = area.to_glib_none().0;
                        let wake_ptr = wake_ptr as usize;
                        let pending = std::sync::Arc::clone(&frame_pending);
                        rc.set_update_callback(move || {
                            pending.store(true, std::sync::atomic::Ordering::Relaxed);
                            glib::idle_add(move || {
                                // Safety: `wake_ptr` is the live GLArea
                                // owned by the widget tree.
                                unsafe { gl_area_from_ptr(wake_ptr) }.queue_render();
                                glib::ControlFlow::Continue
                            });
                        });
                        *slot = Some((id, rc));
                        app_render
                            .state::<AppState>()
                            .render_ctx_id
                            .store(id, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!("[go-player] gl render context failed: {e}");
                        return glib::Propagation::Proceed;
                    }
                }
            }
            if let Some((_, rc)) = slot.as_mut() {
                let size_changed = last_size.get() != (w, h);
                last_size.set((w, h));
                // Only render when mpv signalled a new frame (or the
                // surface resized — mpv must repaint at the new size).
                // Calling render() otherwise makes mpv clear the
                // surface to black.
                let fresh = frame_pending.swap(false, std::sync::atomic::Ordering::Relaxed);
                if fresh || size_changed {
                    // GtkGLArea may render into its own FBO rather
                    // than the default framebuffer — query whatever is
                    // bound right now and hand it to mpv.
                    let fbo = current_draw_fbo();
                    if let Err(e) = rc.render(fbo, w, h, true) {
                        eprintln!("[go-player] gl render failed: {e}");
                    }
                    rc.report_swap();
                }
            }
            glib::Propagation::Stop
        });
    }

    // Reparent the webview's vbox into an overlay with the GL area as
    // the bottom child, so the webview paints on top.
    let overlay = gtk::Overlay::new();
    if let Some(parent) = vbox.parent()
        && let Ok(container) = parent.downcast::<gtk::Container>()
    {
        container.remove(&vbox);
    }
    if let Some(old) = gtk_window.child() {
        gtk_window.remove(&old);
    }
    overlay.add(&gl_area);
    overlay.add_overlay(&vbox);
    gtk_window.add(&overlay);
    overlay.show_all();

    // Apply the current desired visibility immediately — commands that
    // ran before this install completed saw an empty handle and did
    // nothing. Then request a draw so the render callback can create
    // the mpv render context promptly (vo=libmpv needs it before
    // loadfile).
    let visible = crate::effective_surface_visible(&state);
    gl_area.set_visible(visible);
    gl_area.queue_render();

    let handle = SurfaceHandle {
        app: app.clone(),
        area_ptr,
    };
    if let Ok(mut g) = state.gtk_surface.lock() {
        *g = Some(handle);
    }
}
