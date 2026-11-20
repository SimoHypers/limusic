//! Music video on Linux, drawn by mpv underneath the webview instead of by a `<video>` inside it.
//!
//! The `<video>` element could only ever be *kept in step* with mpv, which plays the audio: two
//! clocks, reconciled from JavaScript, and on WebKitGTK every correction is expensive (a
//! `playbackRate` write is a flushing seek, a seek costs about a second of re-buffering), so a
//! scrub could leave the picture 10-15 s out (#321). Here mpv plays the picture as one more track
//! of the file it is already playing (crates/player/src/video.rs) and draws it into a GtkGLArea
//! placed *under* the webview. The webview is transparent already (the window is, so the page can
//! round its own corners); the page leaves a hole where the picture goes and tells us where that is
//! (`set_rect`).
//!
//! The widget tree, rebuilt once at startup from the one tao and wry made (GtkWindow > GtkBox >
//! WebKitWebView):
//!
//! ```text
//! GtkWindow
//! └── GtkOverlay
//!     ├── GtkBox          Tauri's, now empty: the main child, it only gives the overlay its size
//!     ├── GtkGLArea       overlay child, placed at the hole by `get-child-position`
//!     └── WebKitWebView   overlay child on top, filling the overlay
//! ```
//!
//! The overlay takes the box's place rather than going inside it because the webview's
//! *grandparent* has to stay the window: tauri-runtime-wry's resize handler for undecorated
//! windows runs on every click and touch in the webview, and unwraps `webview.parent().parent()`
//! as a `gtk::Window`. Anything else there aborts the app on the first click (a panic in a GTK
//! signal cannot unwind). Overlay children add nothing to the overlay's size request, so the
//! picture's box can never hold the window open at a minimum size the way a GtkFixed child would.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void, CString};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use player::{GlDisplay, VideoRenderer};
use webkit2gtk::WebViewExt;

use crate::state::AppState;

/// The widget tree is in place. Cleared again if the GL context cannot be made, which sends the
/// page back to the `<video>` element.
static AVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

/// `(x, y, w, h)` in GTK logical pixels.
type Place = (i32, i32, i32, i32);

struct Surface {
    overlay: gtk::Overlay,
    area: gtk::GLArea,
    webview: webkit2gtk::WebView,
    renderer: Rc<RefCell<Option<VideoRenderer>>>,
    /// Where the GLArea goes.
    at: Rc<Cell<Option<Place>>>,
    /// The pending [`IDLE_GRACE`] release, while the picture is down.
    release: Cell<Option<glib::SourceId>>,
}

/// How long the GL surface outlives the picture going down. The GL context and mpv's render
/// contexts are not free: measured on Fedora/NVIDIA, ~490 MiB of GPU memory in this process,
/// most of it the CUDA context NVDEC decodes in, and none of it goes when mpv merely stops
/// decoding. Past this the area is unrealized, which frees all of it; the next picture realizes it
/// again. Short absences (closing the view to pick a song, the mini player) keep it.
/// ponytail: the same flat minute as the `<video>` path's IDLE_GRACE (VideoSurface.svelte).
const IDLE_GRACE: Duration = Duration::from_secs(60);

thread_local! {
    /// GTK main thread only, like every widget in it.
    static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) };
}

/// Rebuild the main window's widget tree around its webview. Best-effort: on any failure the tree
/// is left as wry made it and music videos keep using the `<video>` element.
pub fn install(win: &tauri::WebviewWindow, state: Arc<AppState>) {
    let res = win.with_webview(move |wv| match build(wv.inner(), state) {
        Ok(()) => {
            AVAILABLE.store(true, Ordering::Relaxed);
            tracing::info!("native video: GL surface installed under the webview");
        }
        Err(e) => tracing::warn!(error = e, "native video: not installed, videos use <video>"),
    });
    if let Err(e) = res {
        tracing::warn!(error = %e, "native video: could not reach the webview");
    }
}

fn build(webview: webkit2gtk::WebView, state: Arc<AppState>) -> Result<(), &'static str> {
    let vbox = webview
        .parent()
        .and_then(|p| p.downcast::<gtk::Box>().ok())
        .ok_or("the webview is not in a GtkBox")?;
    let window = vbox
        .parent()
        .and_then(|p| p.downcast::<gtk::Window>().ok())
        .ok_or("the webview's box is not in a GtkWindow")?;
    // Which GL API GTK uses decides the loader: EGL on Wayland, GLX on X11 (GTK 3 has no EGL-on-X11).
    let display = gtk::gdk::Display::default().ok_or("no display")?;
    let wayland = display.type_().name() == "GdkWaylandDisplay";
    LOADER.get_or_init(|| load_gl_loader(wayland));

    vbox.remove(&webview);
    window.remove(&vbox);
    let overlay = gtk::Overlay::new();
    overlay.add(&vbox);
    let area = gtk::GLArea::new();
    // Drawn when mpv has a frame, not on every redraw of the window around it.
    area.set_auto_render(false);
    // `show_all` below must leave it hidden: it appears only while the page has a hole for it.
    area.set_no_show_all(true);
    overlay.add_overlay(&area);
    overlay.set_overlay_pass_through(&area, true);
    overlay.add_overlay(&webview);
    window.add(&overlay);
    overlay.show_all();
    webview.grab_focus();

    let at = Rc::new(Cell::new(None));
    {
        let (at, area) = (at.clone(), area.clone());
        overlay.connect_get_child_position(move |_, child| {
            if child != area.upcast_ref::<gtk::Widget>() {
                return None; // the webview: the default, which fills the overlay
            }
            at.get().map(|(x, y, w, h)| gtk::gdk::Rectangle::new(x, y, w, h))
        });
    }

    let renderer: Rc<RefCell<Option<VideoRenderer>>> = Rc::new(RefCell::new(None));
    let capture = Rc::new(RefCell::new(Capture::default()));
    {
        let renderer = renderer.clone();
        area.connect_realize(move |area| {
            area.make_current();
            if let Some(e) = area.error() {
                tracing::warn!(error = %e, "native video: no GL context");
                return;
            }
            let mut r = state.player.video_renderer(gl_proc, gl_display(&display, wayland), wake);
            match r.ensure_contexts() {
                Ok(()) => *renderer.borrow_mut() = Some(r),
                Err(e) => tracing::warn!(error = %e, "native video: mpv refused the GL context"),
            }
        });
    }
    {
        let (renderer, capture) = (renderer.clone(), capture.clone());
        // mpv's contexts go while the GL context they were made in is still there to free them in.
        area.connect_unrealize(move |area| {
            area.make_current();
            renderer.borrow_mut().take();
            capture.borrow_mut().free();
        });
    }
    {
        let renderer = renderer.clone();
        area.connect_render(move |area, _| {
            // `try_`: mpv can call `wake` from inside a render call, and on this thread that runs
            // straight away rather than being queued.
            if let Ok(mut r) = renderer.try_borrow_mut() {
                if let Some(r) = r.as_mut() {
                    let s = area.scale_factor();
                    let (w, h) = (area.allocated_width() * s, area.allocated_height() * s);
                    let fbo = current_fbo();
                    match r.render(fbo, w, h) {
                        Ok(()) if WAITING.load(Ordering::Relaxed) => {
                            grab(&capture, area, fbo, w, h)
                        }
                        Ok(()) => {}
                        Err(e) => tracing::debug!(error = %e, "native video: render failed"),
                    }
                }
            }
            glib::Propagation::Stop
        });
    }

    let release = Cell::new(None);
    SURFACE.with(|s| {
        *s.borrow_mut() = Some(Surface { overlay, area, webview, renderer, at, release })
    });
    Ok(())
}

/// The picture has been down for [`IDLE_GRACE`]: give back the GPU memory behind it. mpv is already
/// decoding no video (`set_video_visible(false)`), so its render contexts can go.
fn release_surface() {
    SURFACE.with(|s| {
        let Some(s) = &*s.borrow() else { return };
        s.release.set(None);
        if !s.area.is_visible() {
            s.area.unrealize(); // the unrealize handler frees mpv's contexts first
            tracing::debug!("native video: GL surface released");
        }
    });
}

/// mpv has a frame to show, or a new deck needs a render context. Any thread.
fn wake() {
    glib::MainContext::default().invoke(|| {
        SURFACE.with(|s| {
            let Some(s) = &*s.borrow() else { return };
            if s.area.is_visible() {
                s.area.queue_render(); // the render call makes any missing context itself
                return;
            }
            // Hidden, so no render call is coming: make the context now, or a crossfade deck
            // built while the view was shut would have no way to show its picture.
            if s.area.is_realized() {
                s.area.make_current();
                if let Ok(mut r) = s.renderer.try_borrow_mut() {
                    if let Some(r) = r.as_mut() {
                        let _ = r.ensure_contexts();
                    }
                }
            }
        })
    });
}

/// Put the picture at `rect` (`[x, y, w, h]`, CSS pixels relative to the viewport, as the page
/// measured its hole) or take it away, and tell mpv whether anyone can see it. `vw` is the page's
/// `innerWidth`. Returns whether the picture is up; `false` for a rect means there is no GL surface
/// and the page should fall back to the `<video>` element.
pub async fn set_rect(
    app: &tauri::AppHandle,
    state: Arc<AppState>,
    rect: Option<[f64; 4]>,
    vw: Option<f64>,
) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let res = app.run_on_main_thread(move || {
        let shown = SURFACE.with(|s| {
            let Some(s) = &*s.borrow() else { return false };
            let Some([x, y, w, h]) = rect.filter(|r| r[2] >= 1.0 && r[3] >= 1.0) else {
                s.at.set(None);
                s.area.hide();
                if s.area.is_realized() {
                    let pending = s.release.take();
                    s.release.set(Some(pending.unwrap_or_else(|| {
                        glib::timeout_add_local_once(IDLE_GRACE, release_surface)
                    })));
                }
                return false;
            };
            if let Some(id) = s.release.take() {
                id.remove();
            }
            // CSS pixels to GTK's, and a pixel of slack all round, because a picture a pixel short
            // of the hole would leave a hairline of desktop showing through the transparent window.
            // The page's surround covers the excess. The factor is the webview's width over the
            // page's: `zoom_level()` alone leaves out the zoom WebKitGTK applies for the desktop's
            // text scaling factor (fontDPI / 96, divided back out of what it returns), which drew
            // the picture 1.15x too small at a factor of 1.15 (#682). `devicePixelRatio` only
            // includes page zoom from WebKitGTK 2.52, and the AppImage ships an older one.
            let z = vw
                .filter(|&v| v >= 1.0)
                .map_or_else(|| s.webview.zoom_level(), |v| s.webview.allocated_width() as f64 / v);
            let (x0, y0) = ((x * z).floor() as i32 - 1, (y * z).floor() as i32 - 1);
            let (x1, y1) = (((x + w) * z).ceil() as i32 + 1, ((y + h) * z).ceil() as i32 + 1);
            s.at.set(Some((x0, y0, x1 - x0, y1 - y0)));
            s.area.realize(); // the first time, this is what makes the renderer
            if s.renderer.borrow().is_none() {
                AVAILABLE.store(false, Ordering::Relaxed);
                s.at.set(None);
                return false;
            }
            s.area.show();
            s.overlay.queue_resize();
            true
        });
        tracing::debug!(?rect, shown, "native video: rect");
        // After the surface, so mpv never starts a video output with no context to draw into.
        state.player.set_video_visible(shown);
        let _ = tx.send(shown);
    });
    res.is_ok() && rx.await.unwrap_or(false)
}

// --- Ambient light ------------------------------------------------------------------------------
// The glow around the picture (ui/src/lib/ambient.ts) is drawn by the page, which cannot see what
// mpv draws under it. So while the page asks for it, a frame is also shrunk on the GPU to about
// 100x56 and read back, ~22 KB a frame, for `ambient_frame` to hand over. Only while a request is
// waiting: the page asks about 15 times a second, slower than most videos play, and a frame nobody
// takes still costs the GTK thread a context switch (an X round trip on X11) to read it back.

/// The newest small frame: `seq`, `w`, `h` as little-endian u32s, then RGBA rows bottom-up (GL's
/// order), exactly what `ambient_frame` returns.
static FRAMES: LazyLock<tokio::sync::watch::Sender<(u32, Arc<[u8]>)>> =
    LazyLock::new(|| tokio::sync::watch::channel((0, Arc::from([]))).0);

/// When the page last asked, in ms since [`EPOCH`] (0: never).
static WANTED_AT: AtomicU64 = AtomicU64::new(0);
static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);
/// A request is waiting for the next frame. Cleared when one is read back.
static WAITING: AtomicBool = AtomicBool::new(false);

fn now_ms() -> u64 {
    EPOCH.elapsed().as_millis() as u64 + 1
}

fn ambient_wanted() -> bool {
    let at = WANTED_AT.load(Ordering::Relaxed);
    at != 0 && now_ms().saturating_sub(at) < 1000
}

/// The newest frame other than `after`, waiting up to a quarter second for one. `None` when no
/// new frame came (paused, or no picture up), so the page can check whether to keep asking.
pub async fn next_frame(after: u32) -> Option<Arc<[u8]>> {
    let was = ambient_wanted();
    WANTED_AT.store(now_ms(), Ordering::Relaxed);
    WAITING.store(true, Ordering::Relaxed);
    let mut rx = FRAMES.subscribe();
    // Frames were not being grabbed, so the newest is from whenever they last were, maybe another
    // video: wait past it, and redraw the current frame, or a paused picture would give none.
    let after = if was { after } else { rx.borrow().0 };
    if !was {
        wake();
    }
    let wait = tokio::time::timeout(Duration::from_millis(250), rx.wait_for(|f| f.0 != after));
    let frame = wait.await.ok()?.ok()?.1.clone();
    Some(frame)
}

/// The downscale chain: renderbuffers halving from the picture's size until one is at most
/// [`GRAB_MAX_W`] wide. A single blit that far down would skip most pixels and shimmer; halving
/// with linear filtering averages every one.
#[derive(Default)]
struct Capture {
    /// `(framebuffer, renderbuffer, w, h)`, largest first.
    levels: Vec<(u32, u32, i32, i32)>,
    /// The picture size the chain was built for.
    from: (i32, i32),
    /// A grab is waiting to be read back.
    pending: bool,
    seq: u32,
}

const GRAB_MAX_W: i32 = 128;

impl Capture {
    fn free(&mut self) {
        if let Some(gl) = gl_fns() {
            for &(fbo, rb, ..) in &self.levels {
                // SAFETY: names made in this context, which is current.
                unsafe {
                    (gl.DeleteFramebuffers)(1, &fbo);
                    (gl.DeleteRenderbuffers)(1, &rb);
                }
            }
        }
        self.levels.clear();
        self.from = (0, 0);
        self.pending = false;
    }

    /// Build the chain for a `w` x `h` picture.
    fn alloc(&mut self, gl: &GlFns, w: i32, h: i32) {
        self.free();
        self.from = (w, h);
        let (mut lw, mut lh) = (w, h);
        while lw > GRAB_MAX_W {
            (lw, lh) = ((lw + 1) / 2, ((lh + 1) / 2).max(1));
            let (mut fbo, mut rb) = (0, 0);
            // SAFETY: plain object creation in the current context.
            unsafe {
                (gl.GenFramebuffers)(1, &mut fbo);
                (gl.GenRenderbuffers)(1, &mut rb);
                (gl.BindRenderbuffer)(GL_RENDERBUFFER, rb);
                (gl.RenderbufferStorage)(GL_RENDERBUFFER, GL_RGBA8, lw, lh);
                (gl.BindFramebuffer)(GL_FRAMEBUFFER, fbo);
                (gl.FramebufferRenderbuffer)(
                    GL_FRAMEBUFFER,
                    GL_COLOR_ATTACHMENT0,
                    GL_RENDERBUFFER,
                    rb,
                );
            }
            self.levels.push((fbo, rb, lw, lh));
        }
    }
}

/// Shrink the frame mpv just drew into `fbo` down the chain, and read it back a frame later: by then
/// the GPU has long finished, so the read costs no stall on this thread.
fn grab(capture: &Rc<RefCell<Capture>>, area: &gtk::GLArea, fbo: i32, w: i32, h: i32) {
    let Some(gl) = gl_fns() else { return };
    let mut c = capture.borrow_mut();
    if c.from != (w, h) {
        c.alloc(gl, w, h);
    }
    if c.levels.is_empty() {
        return; // a picture already under GRAB_MAX_W wide: not worth a glow
    }
    let mut src = (fbo as u32, w, h);
    // SAFETY: the area's context is current (this runs from its render signal).
    unsafe {
        (gl.Disable)(GL_SCISSOR_TEST); // blits are scissored, and mpv may leave it on
        for &(dst, _, lw, lh) in &c.levels {
            (gl.BindFramebuffer)(GL_READ_FRAMEBUFFER, src.0);
            (gl.BindFramebuffer)(GL_DRAW_FRAMEBUFFER, dst);
            (gl.BlitFramebuffer)(0, 0, src.1, src.2, 0, 0, lw, lh, GL_COLOR_BUFFER_BIT, GL_LINEAR);
            src = (dst, lw, lh);
        }
        (gl.BindFramebuffer)(GL_FRAMEBUFFER, fbo as u32);
    }
    if std::mem::replace(&mut c.pending, true) {
        return; // the read already scheduled picks this one up instead
    }
    let (capture, area) = (capture.clone(), area.clone());
    glib::timeout_add_local_once(Duration::from_millis(16), move || {
        let mut c = capture.borrow_mut();
        let Some(&(fbo, _, lw, lh)) = c.levels.last() else { return };
        if !std::mem::replace(&mut c.pending, false) || !area.is_realized() {
            return;
        }
        area.make_current();
        let mut out = Vec::with_capacity(12 + (lw * lh * 4) as usize);
        c.seq = c.seq.wrapping_add(1).max(1);
        for v in [c.seq, lw as u32, lh as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.resize(12 + (lw * lh * 4) as usize, 0);
        // SAFETY: the context that owns `fbo` is current, and `out` has room for lw*lh RGBA pixels
        // (rows of lw*4 bytes need no padding at the default pack alignment of 4).
        unsafe {
            (gl.BindFramebuffer)(GL_READ_FRAMEBUFFER, fbo);
            (gl.ReadPixels)(0, 0, lw, lh, GL_RGBA, GL_UNSIGNED_BYTE, out[12..].as_mut_ptr().cast());
        }
        // Before the send: the request it answers is the only one out, and the next may follow it
        // at once.
        WAITING.store(false, Ordering::Relaxed);
        FRAMES.send_replace((c.seq, out.into()));
    });
}

const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const GL_RENDERBUFFER: u32 = 0x8D41;
const GL_RGBA8: u32 = 0x8058;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
const GL_LINEAR: u32 = 0x2601;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_SCISSOR_TEST: u32 = 0x0C11;

/// The GL 3.0 / GLES 3.0 calls the grab needs, resolved once through mpv's loader.
#[allow(non_snake_case)]
struct GlFns {
    GenFramebuffers: unsafe extern "C" fn(i32, *mut u32),
    DeleteFramebuffers: unsafe extern "C" fn(i32, *const u32),
    BindFramebuffer: unsafe extern "C" fn(u32, u32),
    GenRenderbuffers: unsafe extern "C" fn(i32, *mut u32),
    DeleteRenderbuffers: unsafe extern "C" fn(i32, *const u32),
    BindRenderbuffer: unsafe extern "C" fn(u32, u32),
    RenderbufferStorage: unsafe extern "C" fn(u32, u32, i32, i32),
    FramebufferRenderbuffer: unsafe extern "C" fn(u32, u32, u32, u32),
    BlitFramebuffer: unsafe extern "C" fn(i32, i32, i32, i32, i32, i32, i32, i32, u32, u32),
    ReadPixels: unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void),
    Disable: unsafe extern "C" fn(u32),
}

// The transmute's target is inferred from the field it fills, which is where the signature is
// written down, once.
#[allow(clippy::missing_transmute_annotations)]
fn gl_fns() -> Option<&'static GlFns> {
    static F: OnceLock<Option<GlFns>> = OnceLock::new();
    F.get_or_init(|| {
        macro_rules! f {
            ($name:literal) => {{
                let p = gl_proc(&(), $name);
                if p.is_null() {
                    tracing::warn!(function = $name, "ambient light: GL function missing");
                    return None;
                }
                // SAFETY: the loader's pointer for this name, cast to its signature in the field.
                unsafe { std::mem::transmute::<*mut c_void, _>(p) }
            }};
        }
        Some(GlFns {
            GenFramebuffers: f!("glGenFramebuffers"),
            DeleteFramebuffers: f!("glDeleteFramebuffers"),
            BindFramebuffer: f!("glBindFramebuffer"),
            GenRenderbuffers: f!("glGenRenderbuffers"),
            DeleteRenderbuffers: f!("glDeleteRenderbuffers"),
            BindRenderbuffer: f!("glBindRenderbuffer"),
            RenderbufferStorage: f!("glRenderbufferStorage"),
            FramebufferRenderbuffer: f!("glFramebufferRenderbuffer"),
            BlitFramebuffer: f!("glBlitFramebuffer"),
            ReadPixels: f!("glReadPixels"),
            Disable: f!("glDisable"),
        })
    })
    .as_ref()
}

// --- GL plumbing -------------------------------------------------------------------------------

type GetProc = unsafe extern "C" fn(*const c_char) -> *mut c_void;

/// `eglGetProcAddress` or `glXGetProcAddressARB`, whichever GTK's contexts come from.
static LOADER: OnceLock<Option<GetProc>> = OnceLock::new();

fn load_gl_loader(wayland: bool) -> Option<GetProc> {
    let (lib, sym) = if wayland {
        (c"libEGL.so.1", c"eglGetProcAddress")
    } else {
        (c"libGL.so.1", c"glXGetProcAddressARB")
    };
    // SAFETY: plain dlopen/dlsym; the symbol has exactly the `GetProc` signature in both APIs.
    unsafe {
        let h = libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY);
        if h.is_null() {
            return None;
        }
        let f = libc::dlsym(h, sym.as_ptr());
        (!f.is_null()).then(|| std::mem::transmute::<*mut c_void, GetProc>(f))
    }
}

/// mpv's GL function loader.
fn gl_proc(_: &(), name: &str) -> *mut c_void {
    let Ok(name) = CString::new(name) else { return std::ptr::null_mut() };
    // SAFETY: both are lookups by name that return null when there is no such function.
    unsafe {
        let p = match LOADER.get().copied().flatten() {
            Some(f) => f(name.as_ptr()),
            None => std::ptr::null_mut(),
        };
        // An EGL older than 1.5 only hands out extension functions; core ones are plain symbols.
        if p.is_null() {
            libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr())
        } else {
            p
        }
    }
}

/// The framebuffer GtkGLArea bound before emitting `render`, which is where mpv has to draw.
fn current_fbo() -> i32 {
    type GetIntegerv = unsafe extern "C" fn(u32, *mut i32);
    const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
    static F: OnceLock<Option<GetIntegerv>> = OnceLock::new();
    let f = F.get_or_init(|| {
        let p = gl_proc(&(), "glGetIntegerv");
        // SAFETY: glGetIntegerv's signature.
        (!p.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, GetIntegerv>(p) })
    });
    let mut fbo = 0;
    if let Some(f) = f {
        // SAFETY: called from `render`, with the area's context current.
        unsafe { f(GL_DRAW_FRAMEBUFFER_BINDING, &mut fbo) };
    }
    fbo
}

/// The native display behind GDK's, for mpv's hardware-decoding interop. Looked up by symbol so
/// this needs neither of gdk's backend features; `None` just costs zero-copy decoding.
fn gl_display(display: &gtk::gdk::Display, wayland: bool) -> Option<GlDisplay> {
    let sym = if wayland {
        c"gdk_wayland_display_get_wl_display"
    } else {
        c"gdk_x11_display_get_xdisplay"
    };
    // SAFETY: both take a GdkDisplay of their own backend, which `wayland` was read from.
    unsafe {
        let f = libc::dlsym(libc::RTLD_DEFAULT, sym.as_ptr());
        if f.is_null() {
            return None;
        }
        let f: unsafe extern "C" fn(*mut c_void) -> *mut c_void = std::mem::transmute(f);
        let p = f(display.as_ptr() as *mut c_void);
        (!p.is_null()).then_some(if wayland { GlDisplay::Wayland(p) } else { GlDisplay::X11(p) })
    }
}
