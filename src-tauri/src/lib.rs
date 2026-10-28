//! Limusic Tauri app. Wires transport + player + db + orchestrator behind the command boundary.

mod appicon;
mod audioproxy;
mod blocked;
mod cipher;
mod commands;
mod db;
mod diagnostics;
mod discord;
mod hotkeys;
mod http;
mod import;
#[cfg(target_os = "linux")]
mod inhibit;
mod lastfm;
mod listenbrainz;
mod listentogether;
mod local;
mod lyrics;
mod matugen;
mod media;
mod mini;
#[cfg(target_os = "linux")]
mod nativevideo;
#[cfg(windows)]
#[path = "nativevideo_windows.rs"]
mod nativevideo;
mod notify;
mod orchestrator;
mod potoken;
mod romanize;
mod scrobble_outbox;
mod secrets;
mod session;
mod sleep_timer;
mod spotify;
mod state;
mod stats;
#[cfg(target_os = "windows")]
mod taskbar;
mod tray;
mod videoproxy;
mod webview;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use innertube::{Clients, InnerTube, Locale, Session};
use player::{Player, PlayerEvent};
use tauri::{Emitter, Manager};

use cipher::{CipherDeobfuscator, PlayerConfigStore};
use db::Db;
use orchestrator::Orchestrator;
use potoken::PoTokenGenerator;
use state::AppState;

/// Hand glibc's freed-but-retained heap back to the OS every few minutes.
///
/// glibc gives each thread its own arena and never returns those pages on `free`, so this process
/// (45 threads across tokio, GTK, mpv and souvlaki) accumulates empty heap it will never reuse.
/// Measured against a running 0.3.2 build: `malloc_trim(0)` dropped it from 211 MiB to 160 MiB PSS
/// and the slack came back at roughly 15 MiB per 15 minutes, so a periodic trim keeps it flat.
///
/// ponytail: trim only. `mallopt(M_ARENA_MAX, 2)` would cap the sprawl at the source, but it
/// serialises allocation across all those threads for a win the trim already gets. Reach for it
/// only if RSS starts climbing between trims.
#[cfg(target_os = "linux")]
fn spawn_heap_trimmer() {
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_secs(180)).await;
            // Safe: no arguments, no allocation, glibc walks its own arenas.
            unsafe { libc::malloc_trim(0) };
        }
    });
}

/// Pull WebKitGTK off its full-browser defaults, which wry never touches.
///
/// **Caches.** WebKitGTK defaults to `WEBKIT_CACHE_MODEL_WEB_BROWSER` ("cache a very large number of
/// resources and previously viewed content"), sized against total system RAM. A music client
/// browsing YTM shelves fills that with thumbnails: measured 627 MiB of on-disk WebKitCache and a
/// web process that would not give the memory back (`malloc_trim` there freed 1 MiB, so it is all
/// live cache). `DocumentBrowser` keeps a working cache but drops dead resources instead of
/// hoarding them. wry also hard-enables the back/forward page cache (`webkitgtk/mod.rs:438`), which
/// keeps whole previous documents alive; this is a SvelteKit SPA doing client-side routing, so it
/// never gets a back/forward navigation to restore and that memory is pure waste.
///
/// **Subsystems.** Audio is libmpv's job and the UI has no `<audio>`, `AudioContext`,
/// `getUserMedia` or WebGL anywhere in it (only 2D canvas, in `theme.svelte.ts`), yet every web
/// process boots the media and 3D stacks regardless: GStreamer, libLLVM and Mesa's gallium are all
/// mapped into it. Measured A/B in `cargo tauri dev`, same build otherwise, home feed loaded:
/// **259 MiB → 247 MiB** PSS at T+180s (236 → 223 at T+60s).
///
/// `media` is the one exception, and only the main window passes `true`: the player view draws a
/// `<video>` for music videos (plan 031). That is a plain `<video src>`, so `mediasource`,
/// `media_stream`, `media_capabilities`, `encrypted_media`, `webaudio`, `webrtc` and `webgl` all
/// stay off. The mini player has no video surface, so it keeps the whole media stack off. WebGL
/// comes back on in the main window only while the ambient light is on ([`set_webgl`]).
///
/// Applies to one webview, because WebKit settings are per-view: the main window and the mini
/// player each cost their own web process, so each has to be told. The hidden cipher/PoToken
/// webviews are deliberately left at the defaults, since the fingerprinting code they exist to run
/// is entitled to probe whatever it likes.
#[cfg(target_os = "linux")]
fn tune_webview(win: &tauri::WebviewWindow, media: bool) {
    use webkit2gtk::{
        CacheModel, NetworkProxyMode, NetworkProxySettings, SettingsExt, WebContextExt, WebViewExt,
        WebsiteDataManagerExt,
    };

    let label = win.label().to_owned();
    let res = win.with_webview(move |wv| {
        let webview = wv.inner();
        // Context-wide, so the second call is a no-op. Set here anyway: whichever window comes up
        // first should not depend on the other existing.
        if let Some(ctx) = WebViewExt::context(&webview) {
            ctx.set_cache_model(CacheModel::DocumentBrowser);
            // The proxy setting, for what the webviews load themselves (#465). Not tauri's
            // `proxy_url`: wry gives WebKit no bypass list, and WebKit then sends loopback through
            // the proxy too (unlike WebView2), so the dev server and the music-video proxy would
            // be out of reach of a remote one.
            if let (Some(url), Some(dm)) = (http::webview_proxy(), ctx.website_data_manager()) {
                let bypass = ["localhost", "127.0.0.0/8", "::1"];
                let mut proxy = NetworkProxySettings::new(Some(url.as_str()), &bypass);
                dm.set_network_proxy_settings(NetworkProxyMode::Custom, Some(&mut proxy));
            }
        }
        if let Some(settings) = WebViewExt::settings(&webview) {
            settings.set_enable_page_cache(false);
            settings.set_enable_media(media);
            settings.set_enable_mediasource(false);
            settings.set_enable_media_stream(false);
            settings.set_enable_media_capabilities(false);
            settings.set_enable_encrypted_media(false);
            settings.set_enable_webaudio(false);
            settings.set_enable_webrtc(false);
            settings.set_enable_webgl(false);
            settings.set_enable_html5_database(false); // WebSQL. localStorage is a separate switch.

            // Two-finger swipe to go back (#302). WebKit walks its own back/forward list, which
            // for this SPA is SvelteKit's pushState entries: the same ones the titlebar's back
            // button steps through. It only fires once a horizontal scroller has run out, so the
            // shelves keep their swipes. Windows has this on by default; macOS would need
            // WKWebView's allowsBackForwardNavigationGestures, which wry does not expose.
            settings.set_enable_back_forward_navigation_gestures(true);

            if compositor_features_off(&settings) {
                restart_web_process(&webview);
            }
        }
        heal_lost_release(&webview);
    });
    match res {
        Ok(()) => {
            tracing::info!(label, media, "webkit: DocumentBrowser cache, page cache + webgl off")
        }
        Err(e) => tracing::warn!(label, error = %e, "webkit tuning failed (continuing)"),
    }
}

/// WebKitGTK 2.54's new compositing, back to what every earlier release did (#433). Returns
/// whether anything changed.
///
/// 2.54 composites with Skia, and Skia draws a layer at its opacity from before an animation on the
/// frame the animation ends, so every fade blinks: switches, dialogs, the player view. Any GPU,
/// opaque windows too. `UseSkiaForComposition` off brings back TextureMapper. 2.54 also has the
/// compositor draw only the damaged rectangles, and on Intel/Mesa (GNOME, niri) a buffer that
/// holds nothing outside them leaves our transparent window see-through on alternate frames, so
/// both damage switches go off as well and every frame is drawn whole, as on 2.52.
/// `ui/perf/flashprobe.py` on 2.54.1: 7-8 one-frame flashes per 6.5 s of fades, 0 with these off,
/// 0 on 2.52.1.
///
/// WebKit's feature list (2.42+) has no binding in webkit2gtk 2.0, so its six functions come from
/// the already-loaded library by name. A WebKit without one of them, or without a feature, is left
/// alone.
///
/// ponytail: drop this once a WebKitGTK release fixes both; run flashprobe.py on it first.
#[cfg(target_os = "linux")]
fn compositor_features_off(settings: &webkit2gtk::Settings) -> bool {
    use gtk::glib::translate::ToGlibPtr;
    use std::ffi::{c_char, c_int, c_void, CStr};
    type AllFeatures = unsafe extern "C" fn() -> *mut c_void;
    type Length = unsafe extern "C" fn(*mut c_void) -> usize;
    type Get = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;
    type Identifier = unsafe extern "C" fn(*mut c_void) -> *const c_char;
    type Enabled = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
    type SetEnabled = unsafe extern "C" fn(*mut c_void, *mut c_void, c_int);
    type Unref = unsafe extern "C" fn(*mut c_void);
    const OFF: [&[u8]; 3] = [
        b"UseSkiaForComposition",
        b"UseDamagingInformationForCompositing",
        b"PropagateDamagingInformation",
    ];

    // SAFETY: the version calls take nothing. The rest are dlsym'd from the WebKitGTK this process
    // already links, each cast to its signature in webkit/WebKitSettings.h and WebKitFeature.h.
    // The list is ours to unref; the features in it are borrowed from the list.
    unsafe {
        if webkit2gtk::ffi::webkit_get_minor_version() < 54 {
            return false;
        }
        let sym = |name: &CStr| libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr());
        let syms = [
            sym(c"webkit_settings_get_all_features"),
            sym(c"webkit_feature_list_get_length"),
            sym(c"webkit_feature_list_get"),
            sym(c"webkit_feature_get_identifier"),
            sym(c"webkit_settings_get_feature_enabled"),
            sym(c"webkit_settings_set_feature_enabled"),
            sym(c"webkit_feature_list_unref"),
        ];
        if syms.iter().any(|s| s.is_null()) {
            return false;
        }
        let all: AllFeatures = std::mem::transmute(syms[0]);
        let length: Length = std::mem::transmute(syms[1]);
        let get: Get = std::mem::transmute(syms[2]);
        let identifier: Identifier = std::mem::transmute(syms[3]);
        let enabled: Enabled = std::mem::transmute(syms[4]);
        let set_enabled: SetEnabled = std::mem::transmute(syms[5]);
        let unref: Unref = std::mem::transmute(syms[6]);

        let raw: *mut webkit2gtk::ffi::WebKitSettings = settings.to_glib_none().0;
        let list = all();
        if list.is_null() {
            return false;
        }
        let mut changed = false;
        for i in 0..length(list) {
            let feature = get(list, i);
            let id = identifier(feature);
            if !id.is_null()
                && OFF.contains(&CStr::from_ptr(id).to_bytes())
                && enabled(raw.cast(), feature) != 0
            {
                set_enabled(raw.cast(), feature, 0);
                changed = true;
            }
        }
        unref(list);
        changed
    }
}

/// Builds the page again so [`compositor_features_off`] takes effect. The web process reads those
/// settings once, when it creates the page, and wry starts that load inside `build()`, before tauri
/// hands the webview over: flipped afterwards they are ignored (measured). So the process is
/// killed and the same URL loaded again. `reload()` would not do: the first load has usually not
/// committed yet, and a reload of nothing paints nothing. Costs one extra web-process start per
/// window, only on 2.54+. A later crash is not ours to answer, hence the reason check.
#[cfg(target_os = "linux")]
fn restart_web_process(webview: &webkit2gtk::WebView) {
    use webkit2gtk::{WebProcessTerminationReason, WebViewExt};

    let Some(uri) = webview.uri() else {
        tracing::warn!(
            "webkit 2.54+: no URI to rebuild the page with, compositing unchanged (#433)"
        );
        return;
    };
    webview.connect_web_process_terminated(move |view, reason| {
        if reason == WebProcessTerminationReason::TerminatedByApi {
            view.load_uri(&uri);
        }
    });
    webview.terminate_web_process();
    tracing::info!("webkit 2.54+: Skia compositing and damage tracking off, page rebuilt (#433)");
}

/// Hands WebKit a left-button release it never got. A titlebar drag starts a compositor move grab
/// (tauri's `start_dragging`, run later on the GTK loop), and the grab eats the release of whichever
/// press it caught. WebKit then still counts the button as down and turns the next press into a
/// `pointermove`, so nothing listening for `pointerdown` sees it: after a double-click on the bar,
/// the first click outside an open dialog didn't close it (#427). A second left press without a
/// release in between can only mean one was lost, so it is sent first. At (-1, -1), so its
/// `mouseup`/`click` land on `<html>` rather than on whatever is now under the old spot.
/// `ui/perf/dragpress.py` shows both the stuck press and this fix on the system WebKitGTK.
#[cfg(target_os = "linux")]
fn heal_lost_release(webview: &webkit2gtk::WebView) {
    use gtk::gdk::{self, glib::translate::ToGlibPtrMut};
    use gtk::prelude::*;
    use std::{cell::Cell, rc::Rc};

    let held = Rc::new(Cell::new(false));
    let on_press = held.clone();
    webview.connect_button_press_event(move |view, e| {
        if e.button() != 1 || e.event_type() != gdk::EventType::ButtonPress {
            return gtk::glib::Propagation::Proceed;
        }
        if on_press.get() {
            let mut release = e.clone();
            // SAFETY: `release` is our own copy of a GdkEventButton; only plain fields change.
            unsafe {
                let raw: *mut gdk::ffi::GdkEventButton = release.to_glib_none_mut().0;
                (*raw).type_ = gdk::ffi::GDK_BUTTON_RELEASE;
                (*raw).x = -1.0;
                (*raw).y = -1.0;
                (*raw).state |= gdk::ffi::GDK_BUTTON1_MASK;
            }
            view.event(&release); // runs the release handler below, which clears `held`
        }
        on_press.set(true);
        gtk::glib::Propagation::Proceed
    });
    webview.connect_button_release_event(move |_, e| {
        if e.button() == 1 {
            held.set(false);
        }
        gtk::glib::Propagation::Proceed
    });
}

/// WebGL in the main window, which [`tune_webview`] turns off: the ambient light draws its glow with
/// it (ui/src/lib/ambient.ts), so it is on exactly while that setting is. WebKit checks the setting
/// when the page asks for a context, so a change applies without a reload.
#[cfg(target_os = "linux")]
pub(crate) fn set_webgl(app: &tauri::AppHandle, on: bool) {
    use webkit2gtk::{SettingsExt, WebViewExt};

    if let Some(win) = app.get_webview_window("main") {
        let _ = win.with_webview(move |wv| {
            if let Some(settings) = WebViewExt::settings(&wv.inner()) {
                settings.set_enable_webgl(on);
            }
        });
    }
}

/// [`tune_webview`] for a window looked up by label. No-op if it isn't up.
#[cfg(target_os = "linux")]
pub(crate) fn tune_webview_labelled(app: &tauri::AppHandle, label: &str, media: bool) {
    if let Some(win) = app.get_webview_window(label) {
        tune_webview(&win, media);
    }
}

/// Pull the main window back onto a display if the window-state plugin restored it somewhere that
/// no longer exists (#459). The plugin keeps a saved position when any one corner lands on a
/// monitor, so after a layout change the window could come back with only its invisible resize
/// border on screen and no titlebar to drag. Called from `setup`: the plugin has restored by then
/// (it runs as the window is built, just before), and the window is still hidden.
///
/// Wayland gives the app no real position and ignores `center`, which is fine: the compositor
/// places the window there, so the bug cannot happen.
fn keep_on_screen(w: &tauri::WebviewWindow) {
    let (Ok(p), Ok(size)) = (w.outer_position(), w.outer_size()) else { return };
    if mini::on_a_display(w, p, size) {
        return;
    }
    tracing::info!(x = p.x, y = p.y, "main window restored off-screen, centering it");
    // A maximized window has to come back to its normal size to move: maximizing again then fills
    // the display it was centered on.
    let maximized = w.is_maximized().unwrap_or(false);
    if maximized {
        let _ = w.unmaximize();
    }
    let _ = w.center();
    if maximized {
        let _ = w.maximize();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Logs to stdout **and** to `<app data>/limusic.log`, truncated each launch (the previous run is
/// kept as `limusic.log.1`).
///
/// The filter names `app_lib`, the `[lib]` name, because that is what every tracing target in this
/// crate carries (`app_lib::orchestrator`). It said `limusic_app` until 2026-08-29, which only ever
/// matched `main.rs`, so every `debug!` in the app was dropped.
///
/// The file is the only way a Windows or macOS user can produce a log at all: `main.rs` sets
/// `windows_subsystem = "windows"` in release, so there is no console to print to, and a bug that
/// only reproduces on their machine (issue #71) otherwise has to be debugged by guessing.
/// `RUST_LOG` still overrides the default filter for both.
fn init_logging(dir: &std::path::Path) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;

    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info,app_lib=debug".into())
    };
    let path = dir.join("limusic.log");
    let _ = std::fs::rename(&path, dir.join("limusic.log.1"));
    // ponytail: one file per launch, no size cap. A run long enough to matter is a run whose log
    // someone wants anyway; add rotation if that stops being true.
    let file = std::fs::File::create(&path).ok().map(std::sync::Mutex::new);
    let file_layer = file.map(|f| {
        tracing_subscriber::fmt::layer().with_ansi(false).with_writer(f).with_filter(filter())
    });
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter()))
        .with(file_layer)
        .init();
}

/// Raise the open-file soft limit to the hard limit, capped.
///
/// WebKitGTK's DMABUF renderer spends a file descriptor per buffer, so a busy page can hold
/// hundreds. Against the 1024 soft limit a login shell often hands us, the web process runs out,
/// and a web process that cannot open an fd cannot allocate a buffer or even create a GWakeup
/// pipe: it stops painting and never recovers, while playback carries on in this process. That is
/// the "window frozen, music still playing" report, and it is a resource limit, not a driver bug.
///
/// Browsers do exactly this at startup for the same reason. The cap keeps us clear of code that
/// sizes arrays by the limit or loops over every possible fd; 64k is ~60x the headroom we need.
#[cfg(target_os = "linux")]
fn raise_fd_limit() {
    const WANT: libc::rlim_t = 65536;
    let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: both calls take a pointer to a live, fully initialised `rlimit`.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) != 0 {
            return;
        }
        let want = WANT.min(lim.rlim_max);
        if lim.rlim_cur >= want {
            return;
        }
        let old = lim.rlim_cur;
        lim.rlim_cur = want;
        if libc::setrlimit(libc::RLIMIT_NOFILE, &lim) == 0 {
            tracing::info!(from = old, to = want, "raised open-file limit");
        }
    }
}

/// The vendor of the EGL that serves the X11 display: "NVIDIA" when the proprietary driver draws
/// the screen, "Mesa Project" when Intel or AMD does, which is a hybrid laptop's default even with
/// the NVIDIA driver loaded (#458). WebKit's UI process opens its EGL display the same way, and
/// the GPU behind it is the one the DMABUF renderer allocates from, so this asks the question
/// WebKit is about to answer. `None` when there is no EGL or it will not initialize.
#[cfg(target_os = "linux")]
fn x11_egl_vendor() -> Option<String> {
    use std::ffi::{c_char, c_void, CStr};
    type GetPlatformDisplay = unsafe extern "C" fn(u32, *mut c_void, *const isize) -> *mut c_void;
    type Initialize = unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32) -> u32;
    type Terminate = unsafe extern "C" fn(*mut c_void) -> u32;
    type QueryString = unsafe extern "C" fn(*mut c_void, i32) -> *const c_char;
    const EGL_PLATFORM_X11_KHR: u32 = 0x31D5;
    const EGL_VENDOR: i32 = 0x3053;

    // SAFETY: dlopen/dlsym by name, each symbol cast to its EGL 1.5 signature. Never dlclosed:
    // WebKit loads the same library a moment later, and vendor drivers do not like being unloaded.
    unsafe {
        let lib = libc::dlopen(c"libEGL.so.1".as_ptr(), libc::RTLD_LAZY);
        if lib.is_null() {
            return None;
        }
        let sym = |name: &CStr| libc::dlsym(lib, name.as_ptr());
        let syms = [
            sym(c"eglGetPlatformDisplay"),
            sym(c"eglInitialize"),
            sym(c"eglTerminate"),
            sym(c"eglQueryString"),
        ];
        if syms.iter().any(|s| s.is_null()) {
            return None;
        }
        let get_display: GetPlatformDisplay = std::mem::transmute(syms[0]);
        let initialize: Initialize = std::mem::transmute(syms[1]);
        let terminate: Terminate = std::mem::transmute(syms[2]);
        let query: QueryString = std::mem::transmute(syms[3]);

        // A null native display is $DISPLAY (EGL_KHR_platform_x11).
        let dpy = get_display(EGL_PLATFORM_X11_KHR, std::ptr::null_mut(), std::ptr::null());
        if dpy.is_null() || initialize(dpy, std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
            return None;
        }
        let vendor = query(dpy, EGL_VENDOR);
        let vendor = (!vendor.is_null()).then(|| CStr::from_ptr(vendor).to_string_lossy().into());
        terminate(dpy);
        vendor
    }
}

/// Log a startup failure and stop, instead of panicking.
///
/// Everything this is called for happens inside Tauri's `setup`, before any window exists.
/// `main.rs` sets `windows_subsystem = "windows"` in release, so a panic here writes to a stderr
/// that is not connected to anything: the user sees a process start and disappear, with no window,
/// no console and no log line. `init_logging` has already run by the time any caller gets here, so
/// this reaches limusic.log (the writer is unbuffered, so the line lands before the exit).
///
/// There is deliberately no dialog, although the dialog plugin is registered. `setup` runs on the
/// main thread inside the event loop's `Ready` handler, and the plugin shows a message box by
/// posting a task to that same thread. Blocking on it from here (directly, or by joining a thread
/// that does) waits on a task that can never run, so the process hangs instead of exiting, and a
/// hung instance keeps the single-instance name: every later launch hands off to it and silently
/// does nothing. A dialog needs a path that does not go through the event loop.
fn fatal(what: &str, detail: &str) -> ! {
    tracing::error!("fatal at startup: {what}: {detail}");
    std::process::exit(1)
}

/// Whether the OS launched us at login, through the autostart entry's `--autostart`.
///
/// Not the argument alone: Tauri restarts (the update banner's relaunch, the tray's Restart) hand
/// the old process's arguments on, so a copy started at login would come back hidden after every
/// update. A restarted process inherits the environment instead, and finds [`RESTARTED_ENV`],
/// which `run` sets once it has read this. Windows keeps it too: the updater starts the per-user
/// NSIS installer with a plain ShellExecute, and the installer starts the new version the same way.
static AUTOSTARTED: AtomicBool = AtomicBool::new(false);
const RESTARTED_ENV: &str = "LIMUSIC_RESTARTED";

/// What a cold launch was given, so `limusic-app 'https://music.youtube.com/watch?v=…'` opens the
/// link once the SPA has mounted (#348). Taken once by `take_launch_args`. A launch while we are
/// already running reaches the single-instance callback instead, which emits `open-link`. Raw
/// strings either way: `parseYtLink` in the UI decides what is a link, so there is one parser.
pub(crate) static LAUNCH_ARGS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Raised by `show_main` once the SPA has mounted, whether it shows the window or stays in the
/// tray. `play_on_start` waits for it: started off the setup timeline, the music began while the
/// window was still coming up.
pub(crate) static UI_MOUNTED: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Stay in the tray instead of showing the window: launched at login with "Start minimized to
/// tray" on, and a tray icon to come back from. Asked by the startup safety net and by
/// `show_main` once the SPA has mounted.
fn should_start_minimized(db: &Db) -> bool {
    AUTOSTARTED.load(Ordering::Relaxed)
        && db.get_setting("start_minimized").as_deref() == Some("true")
        && tray::available()
}

/// Tauri entry point. Applies the platform boot fixes (open-fd limit, NVIDIA/WebKit env), restores
/// the persisted session, wires every command and plugin, and runs the event loop. context/01
/// §startup.
pub fn run() {
    // Must happen before any webview exists: the limit is inherited by the web processes WebKit
    // forks, and cannot be raised for them afterwards.
    #[cfg(target_os = "linux")]
    raise_fd_limit();

    // See `AUTOSTARTED`. Up here with the other env writes, before any thread exists to read it.
    AUTOSTARTED.store(
        std::env::args_os().any(|a| a == "--autostart")
            && std::env::var_os(RESTARTED_ENV).is_none(),
        Ordering::Relaxed,
    );
    // Not on a restart either: the relaunch replays argv, so the linked song would start over after
    // every update. `args_os`, because `args` panics on an argument that is not UTF-8.
    if std::env::var_os(RESTARTED_ENV).is_none() {
        *LAUNCH_ARGS.lock().unwrap() =
            std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    }
    std::env::set_var(RESTARTED_ENV, "1");

    // Two separate NVIDIA/WebKitGTK failures, two separate variables. Neither substitutes for
    // the other, which is the mistake ee48c55 made.
    #[cfg(target_os = "linux")]
    {
        // Without this the window dies at startup with "Gdk-Message: Error 71 (Protocol error)
        // dispatching to Wayland display". Harmless no-op on non-NVIDIA drivers.
        if std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none() {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        }
        // On XWayland the proprietary driver cannot back the DMABUF renderer at all: "Failed to
        // create GBM buffer of size WxH: Invalid argument", zero frames, the window never paints.
        // The AppImage always lands there, because linuxdeploy-plugin-gtk's AppRun hook exports
        // GDK_BACKEND=x11 and Tauri's bundler ships that hook; GDK_BACKEND is set nowhere in this
        // repo. `WEBKIT_DMABUF_RENDERER_FORCE_SHM=1` keeps the renderer, and the GPU, while
        // bypassing GBM. It costs about half the CPU of software rendering: 10% vs 18% on one
        // composited animation in `ui/perf/renderprobe.py`. A/B under `GDK_BACKEND=x11` on
        // 2026-08-31 (GTX 1060, driver 580.173.02, WebKitGTK 2.52.5): without it, a black window
        // and two GBM failures; with it, the app paints and keeps repainting across a track change.
        //
        // Native Wayland needs none of this and does not get it. The full DMABUF path is the
        // cheapest of the three (5% on that animation, 17% vs 23% scrolling 400 layered cards) and
        // it is stable: the "window frozen, music still playing" freeze this gate used to work
        // around was fd exhaustion, fixed by `raise_fd_limit` above. GPU compositing costs about
        // 90 MiB of web-process RSS, which is the whole price.
        //
        // /dev/nvidiactl is the proprietary driver's control node, present whenever it is loaded
        // and absent under nouveau, which does not have this bug. Loaded is not drawing, though:
        // a hybrid laptop has it and draws on Intel by default, where FORCE_SHM made album covers
        // flicker black on hover and plain DMABUF was fine (#458, UHD 630 + GTX 1050, X11). So the
        // last check asks EGL which vendor serves the display, and only a clear non-NVIDIA answer
        // stands the gate down: no answer keeps the old behaviour, since the failure it guards
        // against is a window that never paints. Optimus-manager's "nvidia" mode and PRIME
        // offload should both answer NVIDIA, which is right: NVIDIA draws there.
        //
        // The variable is only defaulted, and skipped if either WEBKIT_ knob is already set by
        // hand, so retesting stays possible.
        //
        // ponytail: delete this the day the AppImage stops forcing X11, which would put those
        // users on native Wayland and the full path, or the day the driver learns GBM on XWayland.
        //
        // GDK picks its backend from GDK_BACKEND when set, else Wayland when WAYLAND_DISPLAY is,
        // else X11.
        let on_x11 = match std::env::var("GDK_BACKEND") {
            Ok(b) => b.split(',').next() == Some("x11"),
            Err(_) => std::env::var_os("WAYLAND_DISPLAY").is_none(),
        };
        if on_x11
            && std::path::Path::new("/dev/nvidiactl").exists()
            && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
            && std::env::var_os("WEBKIT_DMABUF_RENDERER_FORCE_SHM").is_none()
            && x11_egl_vendor().map_or(true, |v| v.contains("NVIDIA"))
        {
            std::env::set_var("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1");
        }
    }

    let mut builder = tauri::Builder::default();

    // Must be the first plugin registered (its documented requirement). A second launch —
    // e.g. clicking the app icon while we're hidden in the tray — re-shows this instance
    // instead of spawning a second one (which would fight over SQLite and mpv).
    //
    // `LIMUSIC_MULTI=1` lifts the guard, because the guard is exactly what makes Listen Together
    // impossible to test on one machine. Pair it with `XDG_DATA_HOME` (Linux) or `APPDATA`
    // (Windows) pointing somewhere else, or the second copy opens the first one's SQLite file and
    // the two fight over it, which is what the guard exists to prevent:
    //
    //     LIMUSIC_MULTI=1 XDG_DATA_HOME=/tmp/limusic-b ./target/debug/limusic-app
    if std::env::var_os("LIMUSIC_MULTI").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            tray::show_main(app);
            // `limusic-app <link>` against this instance (#348). argv[0] leads on every platform.
            if args.len() > 1 {
                let _ = app.emit_to("main", "open-link", &args[1..]);
            }
        }));
    }

    builder
        // The hidden cipher webview's document (webview.rs). A registered scheme, because a
        // `data:` URL is not a document WebView2 will navigate to.
        .register_uri_scheme_protocol(webview::SCHEME, |_ctx, _req| {
            tauri::http::Response::builder()
                .header(tauri::http::header::CONTENT_TYPE, "text/html")
                .body(webview::HARNESS_HTML.as_bytes())
                .expect("static harness response")
        })
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // Folder picker for the local-music library (local.rs).
        .plugin(tauri_plugin_dialog::init())
        // Copying goes through Rust, not the webview. WebKitGTK gates JavaScript clipboard writes
        // (both `execCommand('copy')` and `navigator.clipboard`) behind its own policy, and on
        // Fedora every copy button in the app silently did nothing. The OS clipboard from the app
        // process has no such gate. See ui/src/lib/clipboard.ts.
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        // Reopen at the size/position the window was left at. Only "main": the mini widget is
        // fixed-size and the login/cipher/PoToken webviews are windows too. Size, position and
        // maximized only — VISIBLE would restore a window hidden to the tray as invisible, and
        // DECORATIONS would fight the custom titlebar.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .with_filter(|label| label == "main")
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if let Some(mgr) = app.try_state::<Arc<hotkeys::HotkeysManager>>() {
                        mgr.handle_event(app, shortcut, event.state());
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();

            // GtkSettings belongs to the main thread, which is where setup runs (#481).
            #[cfg(target_os = "linux")]
            if let Some(layout) = gtk::Settings::default()
                .and_then(|s| gtk::prelude::GtkSettingsExt::gtk_decoration_layout(&s))
            {
                let _ = commands::BUTTON_LAYOUT.set(layout.into());
            }

            // App data dir for the SQLite file and mpv's on-disk audio cache.
            let data_dir = app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir());
            std::fs::create_dir_all(&data_dir).ok();
            init_logging(&data_dir);
            let cache_dir = data_dir.join("audio-cache");
            std::fs::create_dir_all(&cache_dir).ok();

            // Shared: the PoToken generator persists its session token through the same file,
            // and it is built before AppState takes ownership of everything else.
            let (db, quarantined) = match Db::open_or_quarantine(&data_dir.join("limusic.sqlite")) {
                Ok(v) => v,
                Err(e) => fatal(
                    "Limusic could not open or create its database",
                    &format!("{}: {e}", data_dir.display()),
                ),
            };
            // Log only: nothing tells the UI yet, so to the user this is an empty library. A
            // notice needs a startup-event path to the SPA, and there is none to reuse.
            if let Some(aside) = &quarantined {
                tracing::warn!(
                    "started with a fresh database; the previous one is at {}",
                    aside.display()
                );
            }
            let db = Arc::new(db);

            // Session bootstrap (context/15 startup ordering): load the persisted login session
            // (cookie/dataSyncId/visitorData) from settings; fetch visitorData anonymously
            // (context/04 §A) only if we've never stored one.
            // `LIMUSIC_PROXY` overrides the stored setting, so a region-locked surface can be
            // tested for one run without a system-wide VPN (CLAUDE.md).
            let proxy = std::env::var("LIMUSIC_PROXY")
                .ok()
                .filter(|p| !p.trim().is_empty())
                .or_else(|| db.get_setting("proxy").filter(|p| !p.trim().is_empty()));
            // Before the first fetch: the shared client builds itself on first use.
            if let Err(error) = http::set_proxy(proxy.as_deref()) { fatal("Limusic cannot safely use the configured proxy", error); }
            // Built here instead of from the config (`create: false`), because WebView2 only takes
            // a proxy at creation and it isn't known until the database is open (#465).
            if let Some(config) = app.config().app.windows.first() {
                let w =
                    http::proxied(tauri::WebviewWindowBuilder::from_config(&handle, config)?)
                        .build()?;
                keep_on_screen(&w);
            }
            let read_secret = |key: &str| db.get_setting_checked(key)
                .unwrap_or_else(|e| fatal("Limusic could not unlock saved credentials", &e.to_string()))
                .filter(|s| !s.is_empty());
            let cookie = read_secret("session_cookie");
            let data_sync_id = state::persisted_data_sync_id(&db);
            let visitor_data = db.get_setting("visitor_data").filter(|s| !s.is_empty());
            // First run (no stored visitorData): bootstrap it in the background after the window is
            // up, rather than blocking setup on a network GET (up to 60s on a bad connection). See
            // the spawned task after AppState is created.
            let needs_visitor_bootstrap = visitor_data.is_none();
            if cookie.is_some() {
                tracing::info!("loaded persisted login session");
            }

            let visitor_for_prewarm = visitor_data.clone();
            let session = Session { locale: Locale::default(), visitor_data, data_sync_id, cookie };
            let it = match InnerTube::new(session, proxy.as_deref()) {
                Ok(it) => it,
                Err(e) => fatal("Limusic could not start its network client", &e.to_string()),
            };
            // Shelf titles, mood chips and playlist subtitles are YouTube's text, so the UI's
            // language has to go out with the request (#274). Persisted rather than pushed from the
            // SPA at startup, because the first home fetch is already in flight by the time the
            // webview could tell us; the SPA writes it whenever it changes (`set_setting`).
            if let Some(hl) = db.get_setting("locale") {
                it.set_locale(&hl);
            }
            it.set_hide_videos(db.get_setting("hide_videos").as_deref() == Some("true"));
            // Read while `db` is still ours; the window is decorated further down, once the rest of
            // the setup that could fail is out of the way.
            #[cfg_attr(target_os = "macos", allow(unused_variables))]
            let system_titlebar = db.get_setting("system_titlebar").as_deref() == Some("true");
            it.set_blocked(blocked::block_list(&db));
            let clients = Clients::bundled();

            let mut player = match Player::new(&cache_dir.to_string_lossy()) {
                Ok(p) => p,
                Err(e) => fatal(
                    "Limusic could not load libmpv, which it uses to play audio",
                    &format!(
                        "{e}. On Linux, install your distribution's mpv library \
                         (Fedora: mpv-libs, Debian/Ubuntu: libmpv2)."
                    ),
                ),
            };
            // The audio bytes are the one request that never went through the proxy setting (#241).
            if let Err(e) = player.set_http_proxy(proxy.as_deref()) {
                fatal("Limusic could not apply the audio proxy", &e.to_string());
            }
            // Before anything can play: the first track of a restored queue has to come out at the
            // level the user left, not at 100.
            let volume = state::saved_volume(&db);
            let _ = player.set_volume(volume);
            if volume > 0 {
                // The level a mute hotkey returns to, when the app muted before any change.
                hotkeys::LAST_NONZERO_VOLUME.store(volume, std::sync::atomic::Ordering::Relaxed);
            }
            player.set_crossfade(state::saved_crossfade(&db));
            let events = match player.take_events() {
                Some(ev) => ev,
                None => fatal(
                    "Limusic could not start its audio event loop",
                    "the player's event channel was already taken, which is a bug",
                ),
            };

            // Phase 2 extraction stack: cipher + PoToken hidden webviews behind the orchestrator.
            let config = Arc::new(PlayerConfigStore::new(&data_dir));
            let cipher = Arc::new(CipherDeobfuscator::new(handle.clone(), &data_dir, config));
            let potoken = Arc::new(PoTokenGenerator::new(db.clone()));
            let orchestrator = Arc::new(Orchestrator::new(
                it.clone(),
                clients.clone(),
                cipher.clone(),
                potoken.clone(),
            ));

            // OS media controls (MPRIS/SMTC/NowPlaying). Its callback resolves AppState lazily, so
            // it's fine to spawn before AppState is managed. context/16, D11.
            let media = media::spawn(handle.clone());
            // MPRIS starts at 1.0 and would show 100% until the first change.
            if let Some(m) = &media {
                m.set_volume(volume);
            }
            // Taskbar preview buttons (#47). Windows-only, and a different API from the SMTC
            // session above.
            #[cfg(target_os = "windows")]
            taskbar::init(&handle);

            // Discord rich presence — off unless the user opted in; parks on its channel until then.
            let discord = discord::spawn(
                db.get_setting("discord_rpc").as_deref() == Some("true"),
                discord::RpcConfig::parse(db.get_setting("discord_rpc_config").as_deref()),
            );

            // Scrobbler — one task, one clock for Last.fm + ListenBrainz. Each half parks until
            // its own credential exists (session key via the titlebar flow, user token via settings).
            let lastfm = lastfm::spawn(
                handle.clone(),
                db.clone(),
                read_secret("lastfm_session_key"),
                read_secret("listenbrainz_token"),
                lastfm::ScrobbleConfig::load(&db),
            );

            // Listen Together session (context/19). Server URL is a DB setting so "home PC → VPS" is
            // config, not a rebuild. The sync channel feeds the guest-playback bridge below.
            // Empty means the built-in default, which `LtSession` resolves when it connects. The
            // URL does not live here so that nothing ever has to hand it to the UI.
            let lt_url = db.get_setting("lt_server_url").unwrap_or_default();
            let (lt, lt_sync_rx) = listentogether::LtSession::new(handle.clone(), lt_url);

            let app_state = Arc::new(AppState::new(
                it,
                clients,
                player,
                db.clone(),
                handle.clone(),
                orchestrator,
                lt,
                cache_dir.clone(),
                media,
                discord,
                lastfm,
            ));
            app.manage(app_state.clone());
            let timer_state = app_state.clone();
            tauri::async_runtime::spawn(async move {
                let mut ticks = tokio::time::interval(std::time::Duration::from_secs(1));
                loop {
                    ticks.tick().await;
                    timer_state.finish_sleep_timer(false).await;
                    let snapshot = timer_state.sleep_timer.snapshot();
                    if snapshot != sleep_timer::Snapshot::Off {
                        let _ = timer_state.app.emit("sleep-timer", snapshot);
                    }
                }
            });

            // The player view's <video> pulls its bytes from Rust over loopback, so the webview
            // never sees a googlevideo URL (context/11). videoproxy.rs explains why a socket and
            // not a custom scheme.
            videoproxy::start(app_state.clone());

            // mpv's audio goes through a second loopback socket so the open-ended range ffmpeg
            // sends becomes bounded ranges upstream, which is the difference between 32 KB/s and
            // several MB/s on the same URL. audioproxy.rs has the measurements.
            audioproxy::start();

            // Local music artwork reaches the webview over the asset protocol, whose configured
            // scope is empty — the folders it may read are the ones the user picked (local.rs).
            local::allow_music_paths(&handle, &app_state.db);

            // The login entry runs whichever binary last wrote it. A dev build that wrote it leaves
            // the login launch loading the vite server, which isn't running then: tray icon, no
            // window, "Could not connect to localhost". An AppImage moved after enabling leaves it
            // pointing at nothing. So an installed build repoints an existing entry at itself.
            // Only an existing one: `is_enabled` is false after a Task Manager disable on Windows,
            // and that choice is the user's.
            if !tauri::is_dev() {
                use tauri_plugin_autostart::ManagerExt;
                let al = app.autolaunch();
                if al.is_enabled().unwrap_or(false) {
                    if let Err(e) = al.enable() {
                        tracing::warn!(error = %e, "could not repoint the autostart entry");
                    }
                }
            }

            // System tray: playback controls + show/quit while running in the background.
            if let Err(e) = tray::init(&handle) {
                tracing::warn!(error = %e, "tray init failed (continuing without tray)");
                tray::set_available(false);
            }

            // System-wide global hotkeys for playback control
            let hotkeys_cfg = hotkeys::load_config(&app_state.db);
            let hotkeys_mgr = Arc::new(hotkeys::HotkeysManager::new(hotkeys_cfg.clone()));
            app.manage(hotkeys_mgr.clone());
            let reg_res = hotkeys_mgr.apply_config(&handle, hotkeys_cfg);
            if !reg_res.success {
                tracing::warn!(errors = ?reg_res.errors, "some global hotkeys could not be registered on startup");
            }

            // A custom app icon (#173) has to be pushed at each surface every launch, since only
            // the .exe/.desktop icon is baked in and that one we can't touch. Guarded, rather than
            // unconditional: with no custom icon there is nothing to restore, and on Windows
            // `apply` would take over ICON_BIG from the .exe's own icon for no reason.
            if appicon::custom_path(&handle).is_some() {
                appicon::apply(&handle);
            }

            // Bridge: apply Listen Together sync commands (guest playback / host seed) to AppState.
            {
                let st = app_state.clone();
                let mut rx = lt_sync_rx;
                tauri::async_runtime::spawn(async move {
                    while let Some(cmd) = rx.recv().await {
                        st.apply_sync(cmd).await;
                    }
                });
            }

            // Restore the last session's queue, then optionally start playback.
            {
                let st = app_state.clone();
                tauri::async_runtime::spawn(async move {
                    st.restore_queue().await;
                    st.play_on_start().await;
                });
            }

            // First-run visitorData bootstrap, off the startup path. `set_visitor_data` writes
            // through the shared session (Arc<RwLock>), so the orchestrator's InnerTube clone sees
            // it; resolves degrade gracefully (no PoToken) until it lands. context/04 §A.
            if needs_visitor_bootstrap {
                let st = app_state.clone();
                let potoken = potoken.clone();
                tauri::async_runtime::spawn(async move {
                    match st.it.fetch_visitor_data().await {
                        Ok(vd) => {
                            st.it.set_visitor_data(Some(vd.clone()));
                            st.db.set_setting("visitor_data", &vd);
                            tracing::info!("visitorData bootstrapped (background)");
                            potoken.prewarm(&vd).await;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "visitorData bootstrap failed (continuing)")
                        }
                    }
                });
            }

            // Keep the login alive by itself (#165 / KI-2). Two signals from the transport:
            // `cookie_changed` when a response rotated `__Secure-*SIDTS` (persist it, or the next
            // launch starts from a dead snapshot again), and `session_rejected` when YouTube
            // turned the session down (re-mint it from the login webview's own Google session,
            // instead of leaving the user with a "sign in again" they have to act on).
            {
                let st = app_state.clone();
                let app_handle = handle.clone();
                let rejected = st.it.session_rejected();
                let rotated = st.it.cookie_changed();
                tauri::async_runtime::spawn(async move {
                    // Ask once, up front, rather than waiting for whatever the UI happens to
                    // request first: the app can start straight to the tray, with nothing on
                    // screen to run into the dead session and report it. One request, and the
                    // 401 it may come back with goes down the same healing path as any other.
                    if st.it.is_logged_in() {
                        if let Some(client) = st.clients.get(innertube::METADATA_CLIENT) {
                            // No heal-waiting for it: this runs *before* the loop below, so
                            // there is nobody to answer a wait yet, and a dead session would
                            // stall the healer's own startup for the whole timeout. Its 401
                            // raises the flag instead, and the loop picks that up on entry.
                            let _ = innertube::without_healing(st.it.account_menu(client)).await;
                        }
                    }
                    // Google rolls its short-lived tokens on the requests the app makes, so an
                    // idle night leaves the jar to expire on its own. Half-hourly is well inside
                    // the window and costs one request.
                    let mut keepalive = tokio::time::interval(Duration::from_secs(30 * 60));
                    keepalive.tick().await; // the first tick is immediate; the ping above covered it
                    loop {
                        tokio::select! {
                            _ = rejected.notified() => {
                                // The guard is what parked requests are watching: while it lives
                                // they keep waiting however long this takes, and dropping it
                                // releases them whether or not anything was re-minted. They retry
                                // either way, so the ones that really are dead can say so instead
                                // of holding a spinner until the timeout.
                                let _heal = st.it.begin_heal();
                                innertube::without_healing(
                                    session::refresh_session(app_handle.clone(), st.clone()),
                                )
                                .await;
                            }
                            _ = rotated.notified() => st.persist_rotated_cookie().await,
                            _ = keepalive.tick() => st.keep_session_alive().await,
                        }
                    }
                });
            }

            // Pump mpv events → UI events + queue advance. context/11 events, context/14 §TrackEnded.
            #[cfg(any(target_os = "linux", windows))]
            let video_state = app_state.clone();
            spawn_event_pump(app_state, handle, events);

            // Prewarm the webviews off the first-play path (context/04 §startup). The delays let
            // the event loop come up first (run_on_main_thread needs it pumping).
            {
                let cipher = cipher.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    cipher.prewarm().await;
                });
            }
            if let Some(vd) = visitor_for_prewarm {
                let potoken = potoken.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(2500)).await;
                    potoken.prewarm(&vd).await;
                });
            }
            // Mint-and-destroy policy (Phase-0 decision), now applied to the BotGuard V8 isolate
            // rather than a webview. Measured: the live isolate costs ~92 MB RSS, of which a
            // teardown returns ~39 MB (the rest is V8 platform + arenas, retained for the life of
            // the process once BotGuard has run at all).
            //
            // The idle window has to outlast a track, not a track gap. The gapless lookahead mints
            // for the next track seconds after the current one starts, so the runtime then sits
            // idle for the whole song: at 60s it was torn down and rebuilt once per track, and a
            // cold bootstrap (~0.8-2.3s) landed on the critical path whenever a track started from
            // a stop. 10 minutes covers any normal song, so continuous playback keeps one isolate,
            // and the memory still comes back ten minutes after the user stops listening.
            //
            // The cipher webview rides the same tick, for the same reason: it is a whole
            // `WebKitWebProcess` (91 MiB PSS / 234 MiB RSS measured on Fedora) held for two
            // functions that run once per track resolve. Not on Windows, where rebuilding it
            // freezes the app: see `CipherDeobfuscator::teardown_if_idle` (issue #288).
            {
                let potoken = potoken.clone();
                let cipher = cipher.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        potoken.teardown_if_idle(Duration::from_secs(600)).await;
                        cipher.teardown_if_idle(Duration::from_secs(600)).await;
                    }
                });
            }

            // Hand the frame to the compositor when the user asked for it (issue #65). The window
            // is created undecorated, so this is the one place that reverses it; macOS never gets
            // here, its traffic lights come from `tauri.macos.conf.json`. Done before the SPA shows
            // the window, so the frame is already there rather than popping in.
            #[cfg(not(target_os = "macos"))]
            if system_titlebar {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_decorations(true);
                }
            }

            // The window starts hidden and the SPA shows it once it has mounted, so the saved size
            // is already applied by then (#45). Safety net: if the frontend never gets that far,
            // show it anyway rather than leaving the app with no window at all. Not when starting
            // in the tray, where the hidden window is the point.
            if let Some(w) = app.get_webview_window("main") {
                if !should_start_minimized(&db) {
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        if !w.is_visible().unwrap_or(true) {
                            let _ = w.show();
                        }
                    });
                }
            }

            #[cfg(target_os = "linux")]
            {
                tune_webview_labelled(app.handle(), "main", true);
                if db.get_setting("ambient_light").as_deref() == Some("true") {
                    set_webgl(app.handle(), true);
                }
                if let Some(w) = app.get_webview_window("main") {
                    nativevideo::install(&w, video_state);
                }
                spawn_heap_trimmer();
            }
            #[cfg(windows)]
            {
                if let Some(w) = app.get_webview_window("main") {
                    nativevideo::install(&w, video_state);
                }
            }
            Ok(())
        })
        .invoke_handler(only_ui(tauri::generate_handler![
            commands::search,
            commands::search_all,
            commands::search_suggestions,
            commands::search_cards,
            commands::search_videos,
            commands::song,
            commands::play,
            commands::play_index,
            commands::switch_version,
            commands::remove_from_queue,
            commands::clear_queued,
            commands::add_to_queue,
            commands::move_in_queue,
            commands::play_next,
            commands::next_track,
            commands::prev_track,
            commands::back_to_previous,
            commands::toggle_shuffle,
            commands::set_repeat,
            commands::toggle_pause,
            commands::get_sleep_timer,
            commands::set_sleep_timer,
            commands::seek,
            commands::set_volume,
            commands::set_playback_params,
            commands::get_queue,
            commands::get_playback,
            commands::video_stream,
            commands::forget_video_stream,
            commands::native_video_rect,
            commands::ambient_frame,
            commands::get_matugen_theme,
            commands::start_matugen_watcher,
            commands::stop_matugen_watcher,
            commands::get_settings,
            commands::set_setting,
            commands::get_global_hotkeys,
            commands::global_hotkeys_on_wayland,
            commands::set_global_hotkeys,
            commands::reset_global_hotkeys,
            commands::get_stream_clients,
            commands::clear_caches,
            commands::set_app_icon,
            commands::app_icon_path,
            commands::get_account,
            commands::get_account_identities,
            commands::switch_account,
            commands::sign_out,
            commands::login_webview,
            commands::get_google_accounts,
            commands::switch_google_account,
            commands::remove_google_account,
            commands::open_mini,
            commands::close_mini,
            commands::set_mini_compact,
            commands::show_main,
            commands::take_launch_args,
            commands::get_home,
            commands::network_reachable,
            commands::get_home_more,
            commands::get_comments,
            commands::get_comments_more,
            commands::get_comment_replies,
            commands::comment_action,
            commands::comment_create,
            commands::comment_reply,
            commands::comment_edit,
            commands::comment_delete,
            commands::get_library,
            commands::get_library_albums,
            commands::get_library_artists,
            commands::get_library_subscriptions,
            commands::get_upload_albums,
            commands::get_history,
            commands::get_playlist,
            commands::get_playlist_more,
            commands::get_playlist_suggestions,
            commands::playlist_index,
            commands::sync_playlist_index,
            commands::play_counts,
            commands::listening_stats,
            commands::top_artists,
            commands::clear_listening_history,
            commands::get_album,
            commands::get_blocked_artists,
            commands::block_artist,
            commands::unblock_artist,
            commands::get_local_library,
            commands::add_local_folder,
            commands::remove_local_folder,
            commands::allow_font_file,
            commands::get_artist,
            commands::get_artist_appears_on,
            commands::get_browse_grid,
            commands::get_moods,
            commands::get_mood_art,
            commands::play_playlist,
            commands::start_radio,
            commands::rate,
            commands::set_song_saved,
            commands::set_album_saved,
            commands::add_to_playlist,
            commands::add_to_local_playlist,
            commands::local_playlists,
            commands::remove_from_playlist,
            commands::remove_many_from_playlist,
            commands::move_in_playlist,
            commands::create_playlist,
            commands::edit_playlist_details,
            commands::set_playlist_cover,
            commands::set_playlist_sort,
            commands::delete_playlist,
            commands::subscribe,
            commands::import_read,
            commands::import_read_file,
            commands::import_start,
            commands::import_status,
            commands::import_rows,
            commands::import_pick,
            commands::import_create,
            commands::import_cancel,
            commands::import_source,
            commands::import_update,
            commands::import_resolve,
            commands::lt_get_state,
            commands::lt_set_server_url,
            commands::lt_create_room,
            commands::lt_join_room,
            commands::lt_leave,
            commands::lt_approve_join,
            commands::lt_reject_join,
            commands::lt_kick,
            commands::lt_transfer_host,
            commands::lt_suggest,
            commands::lt_approve_suggestion,
            commands::lt_reject_suggestion,
            commands::lt_request_sync,
            commands::get_lyrics,
            commands::choose_lyrics_source,
            commands::set_lyrics_offset,
            commands::lyrics_providers,
            commands::lastfm_connect,
            commands::lastfm_disconnect,
            commands::lastfm_status,
            commands::lastfm_preview,
            commands::lastfm_profile,
            commands::listenbrainz_connect,
            commands::listenbrainz_disconnect,
            commands::listenbrainz_status,
            commands::listenbrainz_profile,
            commands::theater_fullscreen,
            commands::release_notes,
            commands::can_self_update,
            commands::check_beta_update,
            commands::check_stable_update,
            commands::open_external,
            commands::quit_app,
            commands::diagnostics,
            commands::diagnostics_summary,
            commands::save_diagnostics,
            commands::log_ui,
        ]))
        .on_window_event(|window, event| {
            // Close-to-tray: ✕ hides the main window and playback keeps running; real quit is
            // the tray's Quit item (or the "close_to_tray=false" setting). Label-gated: the
            // hidden cipher/PoToken webviews are windows too and must close normally.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "main" => {
                        let hide = tray::available()
                            && window
                                .app_handle()
                                .try_state::<Arc<AppState>>()
                                .map(|s| close_hides(s.db.get_setting("close_to_tray").as_deref()))
                                .unwrap_or(true);
                        if hide {
                            api.prevent_close();
                            let _ = window.hide();
                            tray::set_main_visible(window.app_handle(), false);
                        }
                    }
                    // Nothing in the widget closes it, but a WM shortcut still can. Turn that into
                    // the ordinary "back to the app" path — closing it on its own would leave the
                    // app running with no window at all.
                    mini::LABEL => {
                        api.prevent_close();
                        tray::show_main(window.app_handle());
                    }
                    _ => {}
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|handle, event| {
            // The hidden cipher/PoToken webviews are windows too, so closing the main window no
            // longer auto-exits the app. Quit when the main window is destroyed.
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } = &event
            {
                if label == "main" {
                    handle.exit(0);
                }
            }

            // The single place that saves positions on exit. It covers `app.exit`,
            // `request_restart`, the main window closing, and macOS Cmd+Q from the native menu.
            if let tauri::RunEvent::Exit = &event {
                if let Some(state) = handle.try_state::<Arc<AppState>>() {
                    state.flush_position();
                }
                mini::save_position(handle);
            }
        });
}

/// Let app commands answer only the UI's own windows. Tauri checks capabilities for plugin
/// commands, but without an app ACL manifest an app command is open to any webview on a local
/// origin, and the hidden cipher webview is one: its `limusicharness://` document runs YouTube's
/// player.js. It needs no IPC, the bridge drives it through eval callbacks (`webview.rs`). The
/// login windows load remote pages, which Tauri refuses on its own.
fn only_ui<R: tauri::Runtime>(
    handler: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        if matches!(invoke.message.webview_ref().label(), "main" | mini::LABEL) {
            return handler(invoke);
        }
        invoke.resolver.reject("not allowed from this webview");
        true
    }
}

/// ✕ hides to tray unless the user explicitly set close_to_tray=false (unset → default on).
///
/// Gated by [`tray::available`] at the call site: with no tray to click, hiding would strand the
/// app with no window (#232).
fn close_hides(setting: Option<&str>) -> bool {
    setting != Some("false")
}

/// Decide whether a position tick is worth forwarding to the UI. Passes ~4 Hz of steady
/// playback through, plus any discontinuity (seek/track change) immediately so the slider
/// never lags a jump. Pure so it's testable; the pump owns the state.
// ponytail: fixed 250ms cadence; make it adaptive only if someone ever wants sub-second UI time.
struct PositionThrottle {
    last_emit: std::time::Instant,
    last_pos: f64,
}

impl PositionThrottle {
    fn new() -> Self {
        Self {
            last_emit: std::time::Instant::now() - std::time::Duration::from_secs(1),
            last_pos: f64::NAN,
        }
    }
    fn should_emit(&mut self, pos: f64, now: std::time::Instant) -> bool {
        let dt = now.duration_since(self.last_emit);
        // A jump is any move that couldn't be normal playback since the last emit (+0.75s slack).
        let jumped =
            self.last_pos.is_nan() || (pos - self.last_pos).abs() > dt.as_secs_f64() + 0.75;
        if jumped || dt >= std::time::Duration::from_millis(250) {
            self.last_emit = now;
            self.last_pos = pos;
            return true;
        }
        false
    }
}

fn spawn_event_pump(
    state: Arc<AppState>,
    app: tauri::AppHandle,
    mut events: tokio::sync::mpsc::UnboundedReceiver<PlayerEvent>,
) {
    tauri::async_runtime::spawn(async move {
        let mut throttle = PositionThrottle::new();
        while let Some(ev) = events.recv().await {
            match ev {
                PlayerEvent::Position(p) => {
                    if throttle.should_emit(p, std::time::Instant::now()) {
                        let _ = app.emit("position", serde_json::json!({ "position": p }));
                    }
                    state.on_position(p).await;
                }
                PlayerEvent::Duration(d) => {
                    let _ = app.emit("duration", serde_json::json!({ "duration": d }));
                    state.on_duration(d).await;
                }
                PlayerEvent::Playing(playing) => {
                    let _ = app.emit("playback-state", if playing { "playing" } else { "paused" });
                    if !playing {
                        state.flush_position(); // persist exact resume position on pause
                        let _ = app.emit(
                            "position",
                            serde_json::json!({ "position": state.current_position() }),
                        );
                    }
                    state.media_set_playing(playing);
                    // Keep the tray's toggle label honest — this arm is the same chokepoint
                    // MPRIS uses, so tray state can't drift from media-key state.
                    tray::set_playing(&app, playing);
                }
                // Listen Together host: broadcast a pause/resume. Not from `Playing`, which also
                // flips when a track runs out, and that arrives here after the next track was
                // announced (`on_track_ended` loads it on this pump), with the old track's end as
                // the position. Guests then seek the new track to it, past the end of a shorter one.
                PlayerEvent::Paused(paused) => state.lt_on_play_state(!paused).await,
                PlayerEvent::TrackEnded => {
                    state.on_track_ended().await;
                }
                PlayerEvent::Looped => state.on_looped().await,
                PlayerEvent::TrackFailed(msg) => {
                    // The track died (dead/403 URL etc). on_track_failed records a WEB_REMIX 403
                    // (context/06 §2), evicts the poisoned cache, and retries the track once via
                    // the fallback clients — only toast the error if it gave up and advanced.
                    //
                    // Read the client before the retry: it advances the queue when it gives up,
                    // and which client served the dead URL is the one fact a bug report has to
                    // carry. Nobody can read the log on Windows (no console, no log file), so it
                    // goes in the message the user can see and copy. Issue #71.
                    let client = state.current_stream_client().await;
                    tracing::warn!(error = %msg, client = ?client, "track failed");
                    if !state.on_track_failed().await {
                        let msg = match client {
                            Some(c) => format!("{msg} ({c})"),
                            None => msg,
                        };
                        let _ = app.emit("playback-error", serde_json::json!({ "message": msg }));
                    }
                }
                PlayerEvent::AudioDeviceLost => {
                    tracing::warn!("audio device unavailable, holding the queue where it is");
                    state.on_audio_device_lost().await;
                    let _ = app.emit(
                        "playback-error",
                        serde_json::json!({
                            "message": "Your audio device is unavailable. Press play once it's back."
                        }),
                    );
                }
                PlayerEvent::LookaheadFailed(msg) => {
                    // No toast: the user is still hearing the current track and nothing they can
                    // see has gone wrong. The only thing owed is the eviction.
                    tracing::warn!(error = %msg, "lookahead preload failed");
                    state.on_lookahead_failed().await;
                }
                PlayerEvent::VideoFailed(audio) => state.on_video_failed(&audio).await,
                PlayerEvent::Error(msg) => {
                    tracing::error!(error = %msg, "player error");
                    let _ = app.emit("playback-error", serde_json::json!({ "message": msg }));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{close_hides, PositionThrottle};
    use std::time::{Duration, Instant};

    #[tauri::command]
    fn ping() {}

    /// The hidden cipher webview runs YouTube's player.js on a local origin, so Tauri lets it
    /// reach app commands unless `only_ui` turns it away.
    #[test]
    fn app_commands_answer_only_the_ui() {
        let app = tauri::test::mock_builder()
            .invoke_handler(super::only_ui(tauri::generate_handler![ping]))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        for (label, allowed) in [("main", true), ("mini", true), ("cipher", false)] {
            let wv =
                tauri::WebviewWindowBuilder::new(&app, label, Default::default()).build().unwrap();
            let res = tauri::test::get_ipc_response(
                &wv,
                tauri::webview::InvokeRequest {
                    cmd: "ping".into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" }
                        .parse()
                        .unwrap(),
                    body: Default::default(),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.into(),
                },
            );
            assert_eq!(res.is_ok(), allowed, "{label}");
        }
    }

    /// `tauri.macos.conf.json` overrides the main window so macOS gets real traffic lights over the
    /// app's own titlebar (issue #65). Tauri merges platform config with RFC 7386 JSON Merge Patch,
    /// which replaces arrays wholesale, so that file has to repeat every window key from
    /// `tauri.conf.json` rather than patch the few it changes. Nothing on this machine builds for
    /// macOS, so a key dropped from the copy would only surface as a wrongly sized window shipped
    /// by CI. This fails instead.
    #[test]
    fn macos_window_config_mirrors_the_base_window() {
        let base: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let mac: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.macos.conf.json")).unwrap();
        let base_win = base["app"]["windows"][0].as_object().unwrap();
        let mac_win = mac["app"]["windows"][0].as_object().unwrap();

        // Only these two may differ; the frame is the compositor's on macOS.
        let overridden = ["decorations", "transparent"];
        for (k, v) in base_win {
            let got = mac_win.get(k).unwrap_or_else(|| panic!("tauri.macos.conf.json drops `{k}`"));
            if !overridden.contains(&k.as_str()) {
                assert_eq!(got, v, "tauri.macos.conf.json disagrees on `{k}`");
            }
        }
        assert_eq!(mac_win["decorations"], serde_json::json!(true));
        assert_eq!(mac_win["titleBarStyle"], serde_json::json!("Overlay"));
        assert_eq!(mac_win["hiddenTitle"], serde_json::json!(true));

        // Catches a typo or a key Tauri would reject: WindowConfig is `deny_unknown_fields`.
        serde_json::from_value::<tauri::utils::config::WindowConfig>(
            mac["app"]["windows"][0].clone(),
        )
        .expect("macOS window config is not a valid WindowConfig");
    }

    #[test]
    fn close_hides_unless_explicitly_disabled() {
        assert!(close_hides(None)); // fresh install → tray on
        assert!(close_hides(Some("true")));
        assert!(close_hides(Some("garbage")));
        assert!(!close_hides(Some("false")));
    }

    #[test]
    fn steady_playback_throttles_to_250ms() {
        let mut t = PositionThrottle::new();
        let base = Instant::now();
        // First tick ever → emitted regardless of cadence.
        assert!(t.should_emit(0.0, base));
        // 100ms later, small forward move → still within the 250ms window, suppressed.
        assert!(!t.should_emit(0.1, base + Duration::from_millis(100)));
        assert!(!t.should_emit(0.2, base + Duration::from_millis(200)));
        // 250ms accumulated since last emit → emitted again.
        assert!(t.should_emit(0.25, base + Duration::from_millis(250)));
    }

    #[test]
    fn forward_jump_emits_immediately() {
        let mut t = PositionThrottle::new();
        let base = Instant::now();
        assert!(t.should_emit(10.0, base));
        // 50ms later but position jumped +30s (e.g. media-key seek) → emit despite short dt.
        assert!(t.should_emit(40.0, base + Duration::from_millis(50)));
    }

    #[test]
    fn backward_jump_emits_immediately() {
        let mut t = PositionThrottle::new();
        let base = Instant::now();
        assert!(t.should_emit(60.0, base));
        // 50ms later but position jumped -30s → emit despite short dt.
        assert!(t.should_emit(30.0, base + Duration::from_millis(50)));
    }

    #[test]
    fn first_tick_ever_emits() {
        let mut t = PositionThrottle::new();
        // NaN last_pos (fresh throttle) → always emits on the very first tick, even at t=now.
        assert!(t.should_emit(5.0, Instant::now()));
    }
}
