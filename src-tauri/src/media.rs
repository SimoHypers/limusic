//! OS media integration (MPRIS / SMTC / NowPlaying) via `souvlaki`. context/16, D11.
//!
//! `souvlaki`'s `MediaControls` isn't `Send`, and on Windows/macOS its events arrive on the
//! platform's own loop — so we give it a dedicated owner thread. The app talks to that thread over
//! a channel ([`MediaHandle`]); OS control presses route back into [`AppState`] through the
//! captured `AppHandle`. The two share the same commands the UI uses, so they never drift.

use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// Update messages: app → media-controls owner thread.
enum MediaUpdate {
    Metadata { title: String, artist: String, album: Option<String>, cover: Option<String> },
    Duration(f64),
    Playback { playing: bool, pos: f64 },
}

/// App-side handle to the media-controls thread. Cheap to clone-send into. `None` when the OS
/// integration failed to initialize (e.g. no session bus) — every push is then a no-op.
pub struct MediaHandle {
    tx: Sender<MediaUpdate>,
}

impl MediaHandle {
    pub fn set_metadata(&self, title: &str, artist: &str, album: Option<&str>, cover: Option<&str>) {
        let _ = self.tx.send(MediaUpdate::Metadata {
            title: title.to_owned(),
            artist: artist.to_owned(),
            album: album.map(str::to_owned),
            cover: cover.map(str::to_owned),
        });
    }

    pub fn set_duration(&self, secs: f64) {
        let _ = self.tx.send(MediaUpdate::Duration(secs));
    }

    pub fn set_playback(&self, playing: bool, pos: f64) {
        let _ = self.tx.send(MediaUpdate::Playback { playing, pos });
    }
}

/// Spawn the media-controls owner thread. Returns `None` if the platform controls can't be
/// created (integration simply absent then — MPRIS-only fallback is blessed, context/16).
pub fn spawn(app: AppHandle) -> Option<MediaHandle> {
    let (tx, rx) = channel::<MediaUpdate>();
    let spawned = std::thread::Builder::new()
        .name("media-controls".into())
        .spawn(move || run(app, rx));
    match spawned {
        Ok(_) => Some(MediaHandle { tx }),
        Err(e) => {
            tracing::warn!(error = %e, "media-controls thread spawn failed");
            None
        }
    }
}

// `duration` is reset per track in the Metadata arm; the lint can't see the loop's later reads.
#[allow(unused_assignments)]
fn run(app: AppHandle, rx: std::sync::mpsc::Receiver<MediaUpdate>) {
    // On Windows SMTC needs the main window handle; Linux/macOS ignore it.
    #[cfg(target_os = "windows")]
    let hwnd = app
        .get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as *mut std::ffi::c_void);
    #[cfg(not(target_os = "windows"))]
    let hwnd = None;

    let config = PlatformConfig { dbus_name: "limusic", display_name: "Limusic", hwnd };
    let mut controls = match MediaControls::new(config) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = ?e, "OS media controls unavailable — skipping (context/16)");
            return;
        }
    };
    let cb_app = app.clone();
    if let Err(e) = controls.attach(move |event| handle_event(&cb_app, event)) {
        tracing::warn!(error = ?e, "media controls attach failed");
        return;
    }
    tracing::info!("OS media controls attached");

    // Owner-thread-local mirror of what the OS should show; rebuilt on each change.
    let mut title = String::new();
    let mut artist = String::new();
    let mut album: Option<String> = None;
    let mut cover: Option<String> = None;
    let mut duration: Option<f64> = None;

    // `recv` blocks until the sender drops (app shutdown), keeping `controls` alive.
    while let Ok(update) = rx.recv() {
        match update {
            MediaUpdate::Metadata { title: t, artist: a, album: al, cover: c } => {
                title = t;
                artist = a;
                album = al;
                cover = c;
                duration = None; // new track — length not known until mpv reports it
                apply_metadata(&mut controls, &title, &artist, &album, &cover, duration);
            }
            MediaUpdate::Duration(secs) => {
                duration = Some(secs);
                apply_metadata(&mut controls, &title, &artist, &album, &cover, duration);
            }
            MediaUpdate::Playback { playing, pos } => {
                let progress = Some(MediaPosition(Duration::from_secs_f64(pos.max(0.0))));
                let state = if playing {
                    MediaPlayback::Playing { progress }
                } else {
                    MediaPlayback::Paused { progress }
                };
                let _ = controls.set_playback(state);
            }
        }
    }
}

fn apply_metadata(
    controls: &mut MediaControls,
    title: &str,
    artist: &str,
    album: &Option<String>,
    cover: &Option<String>,
    duration: Option<f64>,
) {
    let _ = controls.set_metadata(MediaMetadata {
        title: Some(title),
        artist: Some(artist),
        album: album.as_deref(),
        cover_url: cover.as_deref(),
        duration: duration.map(Duration::from_secs_f64),
    });
}

/// Route an OS control press into the same [`AppState`] methods the UI commands use. Runs the
/// async work on the Tauri runtime (the callback fires on souvlaki's own thread).
fn handle_event(app: &AppHandle, event: MediaControlEvent) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>() else { return };
        let state = state.inner().clone();
        match event {
            MediaControlEvent::Play | MediaControlEvent::Toggle => state.resume_or_toggle().await,
            MediaControlEvent::Pause | MediaControlEvent::Stop => {
                let _ = state.player.pause();
            }
            MediaControlEvent::Next => state.next_in_queue().await,
            MediaControlEvent::Previous => state.prev_in_queue().await,
            MediaControlEvent::SetPosition(MediaPosition(pos)) => {
                let _ = state.player.seek(pos.as_secs_f64());
            }
            MediaControlEvent::SeekBy(dir, by) => {
                let delta = if matches!(dir, SeekDirection::Forward) { by.as_secs_f64() } else { -by.as_secs_f64() };
                let _ = state.player.seek((state.current_position() + delta).max(0.0));
            }
            MediaControlEvent::Seek(dir) => {
                let delta = if matches!(dir, SeekDirection::Forward) { 10.0 } else { -10.0 };
                let _ = state.player.seek((state.current_position() + delta).max(0.0));
            }
            _ => {}
        }
    });
}
