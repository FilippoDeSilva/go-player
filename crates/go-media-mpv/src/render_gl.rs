//! OpenGL render-API wrapper around libmpv's `mpv_render_context`.
//!
//! Used on platforms where mpv cannot own a child window — Linux GTK
//! (and later macOS): the host supplies a GL context (e.g. a
//! `GtkGLArea`) and we render into its bound framebuffer each frame.
//!
//! ## Threading contract
//!
//! All `mpv_render_*` calls on a given context must happen on ONE
//! thread — the thread whose GL context we render into (the GTK main
//! thread for `GtkGLArea`). [`GlRenderContext`] is `Send` only so it
//! can live inside shared application state; moving it between threads
//! is a usage error.
//!
//! `render()` must not block the caller for frame pacing: engines
//! created via [`crate::MpvPlaybackEngine::with_gl_surface`] set
//! `video-timing-offset=0`, which disables mpv's render-side blocking.

use crate::{MediaError, MediaResult};
use libmpv2::Mpv;
use libmpv2_sys as sys;
use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::Arc;

/// Parameters for [`GlRenderContext::create`].
///
/// `get_proc_address` resolves OpenGL entry points; it is invoked by
/// mpv while a GL context is current.
pub struct GlRenderParams<C> {
    /// `ctx` passed through to `get_proc_address`.
    pub get_proc_address: fn(&C, &str) -> *mut c_void,
    /// User data forwarded to `get_proc_address`.
    pub user: C,
}

/// Owned `mpv_render_context` for the OpenGL backend.
pub struct GlRenderContext {
    ctx: *mut sys::mpv_render_context,
    /// Keeps the mpv handle alive: a render context must not outlive
    /// its `mpv_handle`, and the owning engine may be dropped while a
    /// host still holds this context (e.g. during engine replacement).
    /// Never read directly — held for its `Drop` side effect.
    #[allow(dead_code)]
    mpv: Arc<Mpv>,
    /// Drops the type-erased `InitParams` box; the pointed-to params
    /// must stay alive for the render context's lifetime because mpv
    /// stores `get_proc_address_ctx` and calls it lazily.
    init_ctx_cleanup: Option<Box<dyn FnOnce()>>,
    update_cleanup: Option<Box<dyn FnOnce()>>,
}

// The raw pointer is !Send by default; we opt in so the context can
// live in shared state. Single-threaded *use* is a documented contract.
unsafe impl Send for GlRenderContext {}

/// Type-erased init params handed to mpv's `get_proc_address`.
struct InitParams<C> {
    get_proc_address: fn(&C, &str) -> *mut c_void,
    user: C,
}

unsafe extern "C" fn gpa_wrapper<C>(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    if ctx.is_null() {
        return ptr::null_mut();
    }
    let params = unsafe { &*(ctx as *const InitParams<C>) };
    let name = unsafe { CStr::from_ptr(name) };
    match name.to_str() {
        Ok(name) => (params.get_proc_address)(&params.user, name),
        Err(_) => ptr::null_mut(),
    }
}

unsafe extern "C" fn update_wrapper(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let cb = unsafe { &*(ctx as *const Box<dyn Fn() + Send>) };
    cb();
}

impl GlRenderContext {
    /// Create an OpenGL render context for `mpv`. Must be called with
    /// the target GL context current (mpv queries GL state/functions
    /// during creation).
    pub fn create<C: 'static>(mpv: Arc<Mpv>, params: GlRenderParams<C>) -> MediaResult<Self> {
        let init_ctx = Box::into_raw(Box::new(InitParams {
            get_proc_address: params.get_proc_address,
            user: params.user,
        })) as *mut c_void;

        // Safety: mpv copies the init params during create; we keep
        // `init_ctx` alive ourselves for the context's lifetime.
        let init_params = Box::into_raw(Box::new(sys::mpv_opengl_init_params {
            get_proc_address: Some(gpa_wrapper::<C>),
            get_proc_address_ctx: init_ctx,
        }));

        let raw_params = [
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                data: sys::MPV_RENDER_API_TYPE_OPENGL.as_ptr() as *mut c_void,
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: init_params as *mut c_void,
            },
            sys::mpv_render_param {
                type_: 0,
                data: ptr::null_mut(),
            },
        ];

        let mut ctx: *mut sys::mpv_render_context = ptr::null_mut();
        let err = unsafe {
            sys::mpv_render_context_create(
                &mut ctx,
                mpv.ctx.as_ptr(),
                raw_params.as_ptr() as *mut sys::mpv_render_param,
            )
        };
        unsafe {
            drop(Box::from_raw(init_params));
        }
        if err != sys::mpv_error_MPV_ERROR_SUCCESS || ctx.is_null() {
            // Free the user params box we allocated.
            unsafe {
                drop(Box::from_raw(init_ctx as *mut InitParams<C>));
            }
            return Err(MediaError::Internal(format!(
                "mpv render context create failed: {}",
                err
            )));
        }

        Ok(Self {
            ctx,
            mpv,
            init_ctx_cleanup: Some(Box::new(move || unsafe {
                drop(Box::from_raw(init_ctx as *mut InitParams<C>));
            })),
            update_cleanup: None,
        })
    }

    /// Register a callback invoked when a new video frame is available
    /// or a redraw is needed. Runs on an mpv-internal thread — marshal
    /// to the UI thread inside the callback. Must not call mpv APIs.
    pub fn set_update_callback(&mut self, cb: impl Fn() + Send + 'static) {
        let boxed: Box<dyn Fn() + Send> = Box::new(cb);
        let raw = Box::into_raw(Box::new(boxed));
        unsafe {
            sys::mpv_render_context_set_update_callback(
                self.ctx,
                Some(update_wrapper),
                raw as *mut c_void,
            );
        }
        self.update_cleanup = Some(Box::new(move || unsafe {
            drop(Box::from_raw(raw));
        }));
    }

    /// Render one frame into the currently bound framebuffer.
    ///
    /// `fbo` is the GL framebuffer object (0 = default/backbuffer),
    /// `width`/`height` the target size in pixels, `flip` whether the
    /// image must be drawn upside down for the GL coordinate system.
    pub fn render(&self, fbo: i32, width: i32, height: i32, flip: bool) -> MediaResult<()> {
        let fbo_param = Box::new(sys::mpv_opengl_fbo {
            fbo,
            w: width,
            h: height,
            internal_format: 0,
        });
        let flip_param = Box::new(flip as i32);
        let raw_params = [
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
                data: Box::into_raw(fbo_param) as *mut c_void,
            },
            sys::mpv_render_param {
                type_: sys::mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
                data: Box::into_raw(flip_param) as *mut c_void,
            },
            sys::mpv_render_param {
                type_: 0,
                data: ptr::null_mut(),
            },
        ];
        let err = unsafe {
            sys::mpv_render_context_render(
                self.ctx,
                raw_params.as_ptr() as *mut sys::mpv_render_param,
            )
        };
        // Render params are consumed by the call; free our boxes.
        unsafe {
            let _ = Box::from_raw(raw_params[0].data as *mut sys::mpv_opengl_fbo);
            let _ = Box::from_raw(raw_params[1].data as *mut i32);
        }
        if err == sys::mpv_error_MPV_ERROR_SUCCESS {
            Ok(())
        } else {
            Err(MediaError::Playback(format!("mpv render failed: {}", err)))
        }
    }

    /// Report that a frame was presented (swap happened). Improves
    /// frame timing; safe to call after each GL swap.
    pub fn report_swap(&self) {
        unsafe { sys::mpv_render_context_report_swap(self.ctx) }
    }
}

impl Drop for GlRenderContext {
    fn drop(&mut self) {
        if let Some(cleanup) = self.update_cleanup.take() {
            cleanup();
        }
        unsafe {
            sys::mpv_render_context_free(self.ctx);
        }
        if let Some(cleanup) = self.init_ctx_cleanup.take() {
            cleanup();
        }
    }
}
