//! Limusic Tauri app. Wires transport + player + db + orchestrator behind the command boundary.

mod commands;
mod db;
mod orchestrator;
mod state;

use std::sync::Arc;

use innertube::{Clients, InnerTube, Locale, Session};
use player::{Player, PlayerEvent};
use tauri::{Emitter, Manager};

use db::Db;
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

            // Session bootstrap: fetch visitorData anonymously (context/04 §A) before playback.
            let proxy = db.get_setting("proxy");
            let visitor_data = tauri::async_runtime::block_on(async {
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

            let session = Session {
                locale: Locale::default(),
                visitor_data,
                data_sync_id: None,
                cookie: None,
            };
            let it = InnerTube::new(session, proxy.as_deref()).expect("build InnerTube");
            let clients = Clients::bundled();

            let mut player = Player::new(cache_dir.to_str().unwrap()).expect("init libmpv");
            let events = player.take_events().expect("player events");

            let app_state = Arc::new(AppState::new(it, clients, player, db, handle.clone()));
            app.manage(app_state.clone());

            // Pump mpv events → UI events + queue advance. context/11 events, context/14 §TrackEnded.
            spawn_event_pump(app_state, handle, events);
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
                PlayerEvent::Error(msg) => {
                    tracing::error!(error = %msg, "player error");
                    let _ = app.emit("playback-error", serde_json::json!({ "message": msg }));
                }
            }
        }
    });
}
