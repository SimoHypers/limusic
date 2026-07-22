//! Limusic Tauri app. Wires transport + player + db + orchestrator behind the command boundary.

mod cipher;
mod commands;
mod db;
mod discord;
mod lastfm;
mod listentogether;
mod lyrics;
mod media;
mod orchestrator;
mod potoken;
mod session;
mod state;
mod tray;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // NVIDIA + Wayland: WebKitGTK's DMABUF renderer trips over NVIDIA's explicit
    // sync (GBM buffer failures / blank window / Gdk Error 71). Disabling explicit
    // sync keeps hardware-accelerated rendering, unlike the old
    // WEBKIT_DISABLE_DMABUF_RENDERER=1 workaround which forced CPU software
    // rendering on WebKitGTK 2.46+ and made the whole UI laggy. Harmless no-op on
    // non-NVIDIA drivers. ponytail: blanket-set on Linux; probe driver/session if
    // an X11/NVIDIA blank-window report ever comes in.
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none() {
            std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
        }
        // AppImage + NVIDIA: the AppImage ships its own WebKitGTK/GTK stack, and inside
        // that environment the DMABUF renderer fails GBM buffer allocation ("Failed to
        // create GBM buffer … Invalid argument") → solid white window. The explicit-sync
        // fix above does NOT cover this case — verified 2026-07-15: the raw binary renders
        // on the system webkit (same 2.52.4) while the AppImage white-screens, and only
        // disabling DMABUF makes the AppImage paint. Cost: CPU software rendering, so gate
        // it tightly — rpm/dev builds and non-NVIDIA AppImages keep full GPU compositing.
        if std::env::var_os("APPIMAGE").is_some()
            && std::path::Path::new("/dev/nvidiactl").exists()
            && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,limusic_app=debug".into()),
        )
        .init();

    tauri::Builder::default()
        // Must be the first plugin registered (its documented requirement). A second launch —
        // e.g. clicking the app icon while we're hidden in the tray — re-shows this instance
        // instead of spawning a second one (which would fight over SQLite and mpv).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let handle = app.handle().clone();

            // App data dir for the SQLite file and mpv's on-disk audio cache.
            let data_dir = app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir());
            std::fs::create_dir_all(&data_dir).ok();
            let cache_dir = data_dir.join("audio-cache");
            std::fs::create_dir_all(&cache_dir).ok();

            let db = Db::open(&data_dir.join("limusic.sqlite")).expect("open sqlite");

            // Session bootstrap (context/15 startup ordering): load the persisted login session
            // (cookie/dataSyncId/visitorData) from settings; fetch visitorData anonymously
            // (context/04 §A) only if we've never stored one.
            let proxy = db.get_setting("proxy");
            let cookie = db.get_setting("session_cookie").filter(|s| !s.is_empty());
            let data_sync_id = db.get_setting("data_sync_id").filter(|s| !s.is_empty());
            let visitor_data = db.get_setting("visitor_data").filter(|s| !s.is_empty());
            // First run (no stored visitorData): bootstrap it in the background after the window is
            // up, rather than blocking setup on a network GET (up to 60s on a bad connection). See
            // the spawned task after AppState is created.
            let needs_visitor_bootstrap = visitor_data.is_none();
            if cookie.is_some() {
                tracing::info!("loaded persisted login session");
            }

            let visitor_for_prewarm = visitor_data.clone();
            let session = Session {
                locale: Locale::default(),
                visitor_data,
                data_sync_id,
                cookie,
            };
            let it = InnerTube::new(session, proxy.as_deref()).expect("build InnerTube");
            let clients = Clients::bundled();

            let mut player = Player::new(cache_dir.to_str().unwrap()).expect("init libmpv");
            let events = player.take_events().expect("player events");

            // Phase 2 extraction stack: cipher + PoToken hidden webviews behind the orchestrator.
            let config = Arc::new(PlayerConfigStore::new(&data_dir));
            let cipher = Arc::new(CipherDeobfuscator::new(handle.clone(), &data_dir, config));
            let potoken = Arc::new(PoTokenGenerator::new(handle.clone()));
            let orchestrator = Arc::new(Orchestrator::new(
                it.clone(),
                clients.clone(),
                cipher.clone(),
                potoken.clone(),
            ));

            // OS media controls (MPRIS/SMTC/NowPlaying). Its callback resolves AppState lazily, so
            // it's fine to spawn before AppState is managed. context/16, D11.
            let media = media::spawn(handle.clone());

            // Discord rich presence — off unless the user opted in; parks on its channel until then.
            let discord = discord::spawn(db.get_setting("discord_rpc").as_deref() == Some("true"));

            // Last.fm scrobbler — parks until a session key exists (titlebar connect flow).
            let lastfm = lastfm::spawn(db.get_setting("lastfm_session_key").filter(|s| !s.is_empty()));

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
                        Err(e) => tracing::warn!(error = %e, "visitorData bootstrap failed (continuing)"),
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
            // Mint-and-destroy policy (Phase-0 decision): drop the PoToken webview when idle.
            {
                let potoken = potoken.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        potoken.teardown_if_idle(Duration::from_secs(60)).await;
                    }
                });
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
            commands::add_to_queue,
            commands::next_track,
            commands::prev_track,
            commands::toggle_shuffle,
            commands::set_repeat,
            commands::toggle_pause,
            commands::seek,
            commands::set_volume,
            commands::get_queue,
            commands::get_settings,
            commands::set_setting,
            commands::get_stream_clients,
            commands::clear_caches,
            commands::set_cookie,
            commands::get_account,
            commands::sign_out,
            commands::login_webview,
            commands::get_home,
            commands::get_library,
            commands::get_playlist,
            commands::get_playlist_more,
            commands::get_album,
            commands::get_artist,
            commands::get_browse_grid,
            commands::play_playlist,
            commands::like,
            commands::add_to_playlist,
            commands::remove_from_playlist,
            commands::create_playlist,
            commands::rename_playlist,
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
        ])
        .on_window_event(|window, event| {
            // Close-to-tray: ✕ hides the main window and playback keeps running; real quit is
            // the tray's Quit item (or the "close_to_tray=false" setting). Label-gated: the
            // hidden cipher/PoToken webviews are windows too and must close normally.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let hide = window
                        .app_handle()
                        .try_state::<Arc<AppState>>()
                        .map(|s| close_hides(s.db.get_setting("close_to_tray").as_deref()))
                        .unwrap_or(true);
                    if hide {
                        api.prevent_close();
                        let _ = window.hide();
                    }
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
        let jumped = self.last_pos.is_nan() || (pos - self.last_pos).abs() > dt.as_secs_f64() + 0.75;
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
                        let _ = app
                            .emit("position", serde_json::json!({ "position": state.current_position() }));
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
                    tracing::warn!(error = %msg, "track failed");
                    if !state.on_track_failed().await {
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
