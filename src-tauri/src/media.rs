//! OS media integration (MPRIS / SMTC / NowPlaying) via `souvlaki`. context/16, D11.
//!
//! `souvlaki`'s `MediaControls` isn't `Send`, and on Windows/macOS its events arrive on the
//! platform's own loop — so we give it a dedicated owner thread. The app talks to that thread over
//! a channel ([`MediaHandle`]); OS control presses route back into [`AppState`] through the
//! captured `AppHandle`. The two share the same commands the UI uses, so they never drift.

use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, OnceLock};
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
    Album(String), // learned after the track's metadata went out; keeps the duration
    Duration(f64),
    Playback { playing: bool, pos: f64 },
    Volume(i64), // slider percent, 0-100
}

/// App-side handle to the media-controls thread. Cheap to clone-send into. `None` when the OS
/// integration failed to initialize (e.g. no session bus) — every push is then a no-op.
pub struct MediaHandle {
    tx: Sender<MediaUpdate>,
}

impl MediaHandle {
    pub fn set_metadata(
        &self,
        title: &str,
        artist: &str,
        album: Option<&str>,
        cover: Option<&str>,
    ) {
        let _ = self.tx.send(MediaUpdate::Metadata {
            title: title.to_owned(),
            artist: artist.to_owned(),
            album: album.map(str::to_owned),
            cover: cover.map(str::to_owned),
        });
    }

    /// A search or home card starts playing with no album name; the radio fetched behind it
    /// carries one (#377, the MPRIS side of #309).
    pub fn set_album(&self, album: &str) {
        let _ = self.tx.send(MediaUpdate::Album(album.to_owned()));
    }

    // souvlaki exposes MPRIS Rate as 1.0, so publish listening seconds, like the UI.
    pub fn set_duration(&self, secs: f64, speed: f64) {
        let _ = self.tx.send(MediaUpdate::Duration(secs / speed));
    }

    pub fn set_playback(&self, playing: bool, pos: f64, speed: f64) {
        let _ = self.tx.send(MediaUpdate::Playback { playing, pos: pos / speed });
    }

    pub fn set_volume(&self, volume: i64) {
        let _ = self.tx.send(MediaUpdate::Volume(volume));
    }
}

/// Spawn the media-controls owner thread. Returns `None` if the platform controls can't be
/// created (integration simply absent then — MPRIS-only fallback is blessed, context/16).
pub fn spawn(app: AppHandle) -> Option<MediaHandle> {
    let (tx, rx) = channel::<MediaUpdate>();
    let spawned =
        std::thread::Builder::new().name("media-controls".into()).spawn(move || run(app, rx));
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
    let hwnd = app.get_webview_window("main").and_then(|w| w.hwnd().ok()).map(|h| {
        tracing::info!("Found main window HWND: {:?}", h.0);
        h.0 as *mut std::ffi::c_void
    });
    #[cfg(target_os = "windows")]
    if hwnd.is_none() {
        tracing::warn!(
            "No main window HWND found, skipping media controls on Windows to prevent panic"
        );
        return;
    }
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
    #[cfg(target_os = "linux")]
    let mut volume: Option<i64> = None;

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
            MediaUpdate::Album(al) => {
                album = Some(al);
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
            // MPRIS only: SMTC and NowPlaying have no per-app volume. Skipped when unchanged, as
            // every push is a D-Bus broadcast and a slider release repeats the drag's last value.
            #[cfg(target_os = "linux")]
            MediaUpdate::Volume(v) => {
                if volume != Some(v) {
                    volume = Some(v);
                    let _ = controls.set_volume(v as f64 / 100.0);
                }
            }
            #[cfg(not(target_os = "linux"))]
            MediaUpdate::Volume(_) => {}
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

/// Request a cover big enough for a media widget or Discord card: stored thumbs are usually
/// row-sized (120px), which looks pixelated anywhere larger (#360). Mirrors `ui/src/lib/thumb.ts`:
/// only googleusercontent-style URLs carry their size in the URL; i.ytimg path-variant thumbs and
/// local `file://` covers pass through unchanged (other sizes can 404). 512 is the size Discord has
/// always requested, so it is known to be served.
pub(crate) fn cover_url(url: &str) -> String {
    static WH: OnceLock<regex::Regex> = OnceLock::new();
    static S: OnceLock<regex::Regex> = OnceLock::new();
    let wh = WH.get_or_init(|| regex::Regex::new(r"=w\d+-h\d+").expect("static regex"));
    let s = S.get_or_init(|| regex::Regex::new(r"=s\d+").expect("static regex"));
    if wh.is_match(url) {
        wh.replace(url, "=w512-h512").into_owned()
    } else if s.is_match(url) {
        s.replace(url, "=s512").into_owned()
    } else {
        url.to_owned()
    }
}

/// Route an OS control press into the same [`AppState`] methods the UI commands use. Runs the
/// async work on the Tauri runtime (the callback fires on souvlaki's own thread, and on Windows
/// also on the UI thread via the taskbar toolbar).
pub(crate) fn handle_event(app: &AppHandle, event: MediaControlEvent) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>() else { return };
        let state = state.inner().clone();
        match event {
            MediaControlEvent::Play | MediaControlEvent::Toggle => state.resume_or_toggle().await,
            MediaControlEvent::Pause | MediaControlEvent::Stop => {
                let _ = state.player.pause();
            }
            // GNOME's panel media widget calls MPRIS Raise when the entry itself is clicked.
            MediaControlEvent::Raise => crate::tray::show_main(&app),
            MediaControlEvent::Next => state.next_in_queue().await,
            MediaControlEvent::Previous => state.prev_in_queue().await,
            MediaControlEvent::SetPosition(MediaPosition(pos)) => {
                let _ = state.player.seek(pos.as_secs_f64() * state.player.speed());
            }
            MediaControlEvent::SeekBy(dir, by) => {
                let delta = if matches!(dir, SeekDirection::Forward) {
                    by.as_secs_f64()
                } else {
                    -by.as_secs_f64()
                };
                let delta = delta * state.player.speed();
                let _ = state.player.seek((state.current_position() + delta).max(0.0));
            }
            MediaControlEvent::Seek(dir) => {
                let delta = if matches!(dir, SeekDirection::Forward) { 10.0 } else { -10.0 };
                let delta = delta * state.player.speed();
                let _ = state.player.seek((state.current_position() + delta).max(0.0));
            }
            // An MPRIS widget or KDE Connect (#220). Same path as a volume hotkey, which also
            // pushes the new level back: without that the widget snaps to the old one.
            MediaControlEvent::SetVolume(v) => {
                crate::hotkeys::change_volume(&state, &app, |_| (v * 100.0).round() as i64)
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_timeline_uses_listening_seconds() {
        let (tx, rx) = channel();
        let media = MediaHandle { tx };
        for (speed, expected_duration, expected_pos) in
            [(0.5, 480.0, 120.0), (1.0, 240.0, 60.0), (1.25, 192.0, 48.0), (2.0, 120.0, 30.0)]
        {
            media.set_duration(240.0, speed);
            let MediaUpdate::Duration(duration) = rx.recv().unwrap() else { panic!() };
            assert_eq!(duration, expected_duration);
            for playing in [true, false] {
                media.set_playback(playing, 60.0, speed);
                let MediaUpdate::Playback { playing: sent_playing, pos } = rx.recv().unwrap()
                else {
                    panic!()
                };
                assert_eq!(sent_playing, playing);
                assert_eq!(pos, expected_pos);
            }
        }
    }
}
