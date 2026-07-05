//! Limusic Tauri app. Wires transport + player + db + orchestrator behind the command boundary.

mod cipher;
mod commands;
mod db;
mod orchestrator;
mod potoken;
mod session;
mod state;
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,limusic_app=debug".into()),
        )
        .init();

    tauri::Builder::default()
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
            let mut visitor_data = db.get_setting("visitor_data").filter(|s| !s.is_empty());
            if visitor_data.is_none() {
                visitor_data = tauri::async_runtime::block_on(async {
                    let boot = InnerTube::new(Session::default(), proxy.as_deref()).ok()?;
                    match boot.fetch_visitor_data().await {
                        Ok(vd) => {
                            tracing::info!("visitorData bootstrapped");
                            Some(vd)
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "visitorData bootstrap failed (continuing)");
                            None
                        }
                    }
                });
                if let Some(vd) = &visitor_data {
                    db.set_setting("visitor_data", vd);
                }
            }
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

            let app_state =
                Arc::new(AppState::new(it, clients, player, db, handle.clone(), orchestrator));
            app.manage(app_state.clone());

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
            commands::play,
            commands::play_index,
            commands::next_track,
            commands::prev_track,
            commands::toggle_pause,
            commands::seek,
            commands::set_volume,
            commands::get_queue,
            commands::get_settings,
            commands::set_setting,
            commands::set_cookie,
            commands::get_account,
            commands::sign_out,
            commands::login_webview,
            commands::get_home,
            commands::get_library,
            commands::get_playlist,
            commands::get_playlist_more,
            commands::play_playlist,
            commands::like,
            commands::add_to_playlist,
            commands::remove_from_playlist,
            commands::create_playlist,
            commands::delete_playlist,
            commands::subscribe,
        ])
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

fn spawn_event_pump(
    state: Arc<AppState>,
    app: tauri::AppHandle,
    mut events: tokio::sync::mpsc::UnboundedReceiver<PlayerEvent>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                PlayerEvent::Position(p) => {
                    let _ = app.emit("position", serde_json::json!({ "position": p }));
                }
                PlayerEvent::Duration(d) => {
                    let _ = app.emit("duration", serde_json::json!({ "duration": d }));
                }
                PlayerEvent::Playing => {
                    let _ = app.emit("playback-state", "playing");
                }
                PlayerEvent::Paused => {
                    let _ = app.emit("playback-state", "paused");
                }
                PlayerEvent::TrackEnded => {
                    state.on_track_ended().await;
                }
                PlayerEvent::TrackFailed(msg) => {
                    // The track died (dead/403 URL etc). Surface the error, then advance — via
                    // on_track_failed, which records a WEB_REMIX 403 (context/06 §2) and evicts the
                    // poisoned cache before on_track_ended reads mpv's actual state.
                    tracing::warn!(error = %msg, "track failed — skipping ahead");
                    let _ = app.emit("playback-error", serde_json::json!({ "message": msg }));
                    state.on_track_failed().await;
                }
                PlayerEvent::Error(msg) => {
                    tracing::error!(error = %msg, "player error");
                    let _ = app.emit("playback-error", serde_json::json!({ "message": msg }));
                }
            }
        }
    });
}
