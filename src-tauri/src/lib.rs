//! Limusic Tauri app. Wires transport + player + db + orchestrator behind the command boundary.

mod cipher;
mod commands;
mod db;
mod diagnostics;
mod discord;
mod http;
mod lastfm;
mod listentogether;
mod local;
mod lyrics;
mod media;
mod mini;
mod orchestrator;
mod potoken;
mod session;
mod state;
#[cfg(target_os = "windows")]
mod taskbar;
mod tray;
mod videoproxy;
mod webview;

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
/// stay off. The mini player has no video surface, so it keeps the whole media stack off.
///
/// Applies to one webview, because WebKit settings are per-view: the main window and the mini
/// player each cost their own web process, so each has to be told. The hidden cipher/PoToken
/// webviews are deliberately left at the defaults, since the fingerprinting code they exist to run
/// is entitled to probe whatever it likes.
#[cfg(target_os = "linux")]
fn tune_webview(win: &tauri::WebviewWindow, media: bool) {
    use webkit2gtk::{CacheModel, SettingsExt, WebContextExt, WebViewExt};

    let label = win.label().to_owned();
    let res = win.with_webview(move |wv| {
        let webview = wv.inner();
        // Context-wide, so the second call is a no-op. Set here anyway: whichever window comes up
        // first should not depend on the other existing.
        if let Some(ctx) = WebViewExt::context(&webview) {
            ctx.set_cache_model(CacheModel::DocumentBrowser);
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
        }
    });
    match res {
        Ok(()) => {
            tracing::info!(label, media, "webkit: DocumentBrowser cache, page cache + webgl off")
        }
        Err(e) => tracing::warn!(label, error = %e, "webkit tuning failed (continuing)"),
    }
}

/// [`tune_webview`] for a window looked up by label. No-op if it isn't up.
#[cfg(target_os = "linux")]
pub(crate) fn tune_webview_labelled(app: &tauri::AppHandle, label: &str, media: bool) {
    if let Some(win) = app.get_webview_window(label) {
        tune_webview(&win, media);
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

/// Tauri entry point. Applies the platform boot fixes (open-fd limit, NVIDIA/WebKit env), restores
/// the persisted session, wires every command and plugin, and runs the event loop. context/01
/// §startup.
pub fn run() {
    // Must happen before any webview exists: the limit is inherited by the web processes WebKit
    // forks, and cannot be raised for them afterwards.
    #[cfg(target_os = "linux")]
    raise_fd_limit();

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
        // and absent under nouveau, which does not have this bug. The variable is only defaulted,
        // and skipped if either WEBKIT_ knob is already set by hand, so retesting stays possible.
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
        {
            std::env::set_var("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1");
        }
    }

    tauri::Builder::default()
        // Must be the first plugin registered (its documented requirement). A second launch —
        // e.g. clicking the app icon while we're hidden in the tray — re-shows this instance
        // instead of spawning a second one (which would fight over SQLite and mpv).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
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
            None,
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
        .setup(|app| {
            let handle = app.handle().clone();

            // App data dir for the SQLite file and mpv's on-disk audio cache.
            let data_dir = app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir());
            std::fs::create_dir_all(&data_dir).ok();
            init_logging(&data_dir);
            let cache_dir = data_dir.join("audio-cache");
            std::fs::create_dir_all(&cache_dir).ok();

            // Shared: the PoToken generator persists its session token through the same file,
            // and it is built before AppState takes ownership of everything else.
            let db = Arc::new(Db::open(&data_dir.join("limusic.sqlite")).expect("open sqlite"));

            // Session bootstrap (context/15 startup ordering): load the persisted login session
            // (cookie/dataSyncId/visitorData) from settings; fetch visitorData anonymously
            // (context/04 §A) only if we've never stored one.
            let proxy = db.get_setting("proxy");
            let cookie = db.get_setting("session_cookie").filter(|s| !s.is_empty());
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
            let it = InnerTube::new(session, proxy.as_deref()).expect("build InnerTube");
            it.set_hide_videos(db.get_setting("hide_videos").as_deref() == Some("true"));
            let clients = Clients::bundled();

            let mut player = Player::new(cache_dir.to_str().unwrap()).expect("init libmpv");
            // Before anything can play: the first track of a restored queue has to come out at the
            // level the user left, not at 100.
            let _ = player.set_volume(state::saved_volume(&db));
            let events = player.take_events().expect("player events");

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
            // Taskbar preview buttons (#47). Windows-only, and a different API from the SMTC
            // session above.
            #[cfg(target_os = "windows")]
            taskbar::init(&handle);

            // Discord rich presence — off unless the user opted in; parks on its channel until then.
            let discord = discord::spawn(db.get_setting("discord_rpc").as_deref() == Some("true"));

            // Last.fm scrobbler — parks until a session key exists (titlebar connect flow).
            let lastfm =
                lastfm::spawn(db.get_setting("lastfm_session_key").filter(|s| !s.is_empty()));

            // Listen Together session (context/19). Server URL is a DB setting so "home PC → VPS" is
            // config, not a rebuild. The sync channel feeds the guest-playback bridge below.
            let lt_url = db
                .get_setting("lt_server_url")
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| "wss://fedora-1.tail9c4985.ts.net/ws".into());
            let (lt, lt_sync_rx) = listentogether::LtSession::new(handle.clone(), lt_url);

            let app_state = Arc::new(AppState::new(
                it,
                clients,
                player,
                db,
                handle.clone(),
                orchestrator,
                lt,
                cache_dir.clone(),
                media,
                discord,
                lastfm,
            ));
            app.manage(app_state.clone());

            // The player view's <video> pulls its bytes from Rust over loopback, so the webview
            // never sees a googlevideo URL (context/11). videoproxy.rs explains why a socket and
            // not a custom scheme.
            videoproxy::start(app_state.clone());

            // Local music artwork reaches the webview over the asset protocol, whose configured
            // scope is empty — the folders it may read are the ones the user picked (local.rs).
            local::allow_music_paths(&handle, &app_state.db);

            // System tray: playback controls + show/quit while running in the background.
            if let Err(e) = tray::init(&handle) {
                tracing::warn!(error = %e, "tray init failed (continuing without tray)");
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

            // Restore the last session's queue (paused, not autoplaying). context/11 §state.
            {
                let st = app_state.clone();
                tauri::async_runtime::spawn(async move {
                    st.restore_queue().await;
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
                            let _ = st.it.account_menu(client).await;
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
                                session::refresh_session(app_handle.clone(), st.clone()).await;
                            }
                            _ = rotated.notified() => st.persist_rotated_cookie(),
                            _ = keepalive.tick() => st.keep_session_alive().await,
                        }
                    }
                });
            }

            // Pump mpv events → UI events + queue advance. context/11 events, context/14 §TrackEnded.
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
            // functions that run once per track resolve.
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

            // The window starts hidden and the SPA shows it once it has mounted, so the saved size
            // is already applied by then (#45). Safety net: if the frontend never gets that far,
            // show it anyway rather than leaving the app with no window at all.
            if let Some(w) = app.get_webview_window("main") {
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    if !w.is_visible().unwrap_or(true) {
                        let _ = w.show();
                    }
                });
            }

            #[cfg(target_os = "linux")]
            {
                tune_webview_labelled(app.handle(), "main", true);
                spawn_heap_trimmer();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            commands::search_all,
            commands::search_cards,
            commands::play,
            commands::play_index,
            commands::remove_from_queue,
            commands::clear_queued,
            commands::add_to_queue,
            commands::move_in_queue,
            commands::play_next,
            commands::next_track,
            commands::prev_track,
            commands::toggle_shuffle,
            commands::set_repeat,
            commands::toggle_pause,
            commands::seek,
            commands::set_volume,
            commands::set_playback_params,
            commands::get_queue,
            commands::get_playback,
            commands::video_stream,
            commands::forget_video_stream,
            commands::get_settings,
            commands::set_setting,
            commands::get_stream_clients,
            commands::clear_caches,
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
            commands::get_home,
            commands::get_home_more,
            commands::get_library,
            commands::get_library_albums,
            commands::get_library_artists,
            commands::get_upload_albums,
            commands::get_history,
            commands::get_playlist,
            commands::get_playlist_more,
            commands::playlist_index,
            commands::sync_playlist_index,
            commands::play_counts,
            commands::get_album,
            commands::get_local_library,
            commands::add_local_folder,
            commands::remove_local_folder,
            commands::allow_font_file,
            commands::get_artist,
            commands::get_browse_grid,
            commands::play_playlist,
            commands::start_radio,
            commands::rate,
            commands::set_song_saved,
            commands::set_album_saved,
            commands::add_to_playlist,
            commands::remove_from_playlist,
            commands::create_playlist,
            commands::edit_playlist_details,
            commands::set_playlist_cover,
            commands::set_playlist_sort,
            commands::delete_playlist,
            commands::subscribe,
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
            commands::lastfm_connect,
            commands::lastfm_disconnect,
            commands::lastfm_status,
            commands::theater_fullscreen,
            commands::release_notes,
            commands::can_self_update,
            commands::open_external,
            commands::diagnostics,
            commands::diagnostics_summary,
            commands::save_diagnostics,
            commands::log_ui,
        ])
        .on_window_event(|window, event| {
            // Close-to-tray: ✕ hides the main window and playback keeps running; real quit is
            // the tray's Quit item (or the "close_to_tray=false" setting). Label-gated: the
            // hidden cipher/PoToken webviews are windows too and must close normally.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "main" => {
                        let hide = window
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
        });
}

/// ✕ hides to tray unless the user explicitly set close_to_tray=false (unset → default on).
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
                    state.lt_on_play_state(playing).await; // Listen Together host → broadcast
                }
                PlayerEvent::TrackEnded => {
                    state.on_track_ended().await;
                }
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
