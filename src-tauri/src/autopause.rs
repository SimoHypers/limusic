//! Notices when another app is playing media, for the "Pause when other apps play media" setting.
//!
//! It reads the apps' own media status, not the sound card, so a notification ding or a game's
//! audio never counts: only apps that publish a playing/paused state do (browsers, video players,
//! Spotify). Windows reads the system media controls (SMTC), Linux reads MPRIS over the session
//! bus. The result goes to `AppState::on_external_audio`, which owns the delays and the
//! pause/resume decisions.

#[cfg(any(windows, target_os = "linux"))]
use std::collections::BTreeSet;
use std::sync::Arc;
#[cfg(any(windows, target_os = "linux"))]
use std::time::Duration;

use crate::state::AppState;

/// Polled, not event-driven: one cheap call a second, and the delays in AppState already absorb
/// the lag.
#[cfg(any(windows, target_os = "linux"))]
const POLL: Duration = Duration::from_secs(1);

/// Start the platform watcher. A no-op where none exists yet.
pub fn spawn(state: Arc<AppState>) {
    #[cfg(windows)]
    windows_watcher::spawn(state);
    #[cfg(target_os = "linux")]
    linux_watcher::spawn(state);
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = state;
}

/// Until the setting is turned on a watcher does nothing at all, so it never touches the OS.
#[cfg(any(windows, target_os = "linux"))]
fn wait_until_enabled(state: &AppState) {
    while !state.auto_pause_enabled() {
        std::thread::sleep(POLL);
    }
}

/// One watcher step: skip while the setting is off, otherwise read who is playing and report it.
/// `last` only drives the log line, which fires when the set of playing apps changes.
#[cfg(any(windows, target_os = "linux"))]
fn poll_once(
    state: &Arc<AppState>,
    last: &mut BTreeSet<String>,
    read: impl FnOnce() -> Result<BTreeSet<String>, String>,
) {
    if !state.auto_pause_enabled() {
        last.clear();
        return;
    }
    let playing = match read() {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(target: "app_lib::autopause", error = %e, "listing media players failed");
            return;
        }
    };
    if playing != *last {
        tracing::info!(target: "app_lib::autopause", apps = ?playing, "other apps playing media changed");
        last.clone_from(&playing);
    }
    state.on_external_audio(!playing.is_empty());
}

/// Limusic's own media session (souvlaki) is in the same list and must never pause us.
#[cfg(any(windows, target_os = "linux"))]
fn is_own(id: &str) -> bool {
    id.to_ascii_lowercase().contains("limusic")
}

#[cfg(windows)]
mod windows_watcher {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    use super::{is_own, poll_once, wait_until_enabled, POLL};
    use crate::state::AppState;

    pub fn spawn(state: Arc<AppState>) {
        let spawned =
            std::thread::Builder::new().name("autopause-smtc".into()).spawn(move || run(state));
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "auto-pause watcher thread spawn failed");
        }
    }

    fn run(state: Arc<AppState>) {
        wait_until_enabled(&state);
        // WinRT wants an initialized thread; the multithreaded apartment lets `get()` block here.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let manager = match Manager::RequestAsync().and_then(|op| op.get()) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "system media controls unavailable, auto-pause is inactive");
                return;
            }
        };
        let mut last = BTreeSet::new();
        loop {
            std::thread::sleep(POLL);
            poll_once(&state, &mut last, || playing_apps(&manager).map_err(|e| e.to_string()));
        }
    }

    /// The ids of the other apps whose media session says Playing right now.
    fn playing_apps(manager: &Manager) -> windows::core::Result<BTreeSet<String>> {
        let mut apps = BTreeSet::new();
        for session in manager.GetSessions()? {
            let Ok(id) = session.SourceAppUserModelId() else { continue };
            let id = id.to_string();
            if is_own(&id) {
                continue;
            }
            let playing = session
                .GetPlaybackInfo()
                .and_then(|info| info.PlaybackStatus())
                .map(|status| status == Status::Playing)
                .unwrap_or(false);
            if playing {
                apps.insert(id);
            }
        }
        Ok(apps)
    }
}

#[cfg(target_os = "linux")]
mod linux_watcher {
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::time::Duration;

    use zbus::blocking::connection::Builder;
    use zbus::blocking::Connection;
    use zbus::zvariant::OwnedValue;

    use super::{is_own, poll_once, wait_until_enabled, POLL};
    use crate::state::AppState;

    const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
    const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
    const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

    pub fn spawn(state: Arc<AppState>) {
        let spawned =
            std::thread::Builder::new().name("autopause-mpris".into()).spawn(move || run(state));
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "auto-pause watcher thread spawn failed");
        }
    }

    fn run(state: Arc<AppState>) {
        wait_until_enabled(&state);
        // A player that stops answering must not stall the watcher for the bus's default timeout.
        let conn = match Builder::session()
            .and_then(|b| b.method_timeout(Duration::from_secs(2)).build())
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "session bus unavailable, auto-pause is inactive");
                return;
            }
        };
        let mut last = BTreeSet::new();
        loop {
            std::thread::sleep(POLL);
            poll_once(&state, &mut last, || playing_apps(&conn));
        }
    }

    /// The ids of the other MPRIS players whose `PlaybackStatus` says Playing right now.
    fn playing_apps(conn: &Connection) -> Result<BTreeSet<String>, String> {
        let names: Vec<String> = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "ListNames",
                &(),
            )
            .and_then(|reply| reply.body().deserialize())
            .map_err(|e| e.to_string())?;
        let mut apps = BTreeSet::new();
        for name in &names {
            let Some(id) = player_id(name) else { continue };
            // playerctld mirrors whichever player was active last, and that can be Limusic itself,
            // so counting it would let the music pause itself.
            if id == "playerctld" || is_own(&id) {
                continue;
            }
            if is_playing(conn, name) {
                apps.insert(id);
            }
        }
        Ok(apps)
    }

    /// The stable part of an MPRIS bus name, without the per-window suffix browsers add:
    /// "org.mpris.MediaPlayer2.firefox.instance_1_52" becomes "firefox".
    fn player_id(bus_name: &str) -> Option<String> {
        let rest = bus_name.strip_prefix(MPRIS_PREFIX)?;
        Some(rest.split(".instance").next().unwrap_or(rest).to_string())
    }

    fn is_playing(conn: &Connection, bus_name: &str) -> bool {
        conn.call_method(
            Some(bus_name),
            MPRIS_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(PLAYER_IFACE, "PlaybackStatus"),
        )
        .and_then(|reply| reply.body().deserialize::<OwnedValue>())
        .ok()
        .and_then(|status| String::try_from(status).ok())
        .is_some_and(|status| status == "Playing")
    }

    #[cfg(test)]
    mod tests {
        use super::player_id;

        #[test]
        fn the_instance_suffix_a_browser_adds_is_dropped() {
            assert_eq!(
                player_id("org.mpris.MediaPlayer2.firefox.instance_1_52").as_deref(),
                Some("firefox")
            );
            assert_eq!(
                player_id("org.mpris.MediaPlayer2.chromium.instance4242").as_deref(),
                Some("chromium")
            );
        }

        #[test]
        fn a_name_without_a_suffix_is_kept_whole() {
            assert_eq!(player_id("org.mpris.MediaPlayer2.vlc").as_deref(), Some("vlc"));
            assert_eq!(player_id("org.mpris.MediaPlayer2.spotify").as_deref(), Some("spotify"));
        }

        #[test]
        fn other_bus_names_are_not_players() {
            assert_eq!(player_id("org.freedesktop.Notifications"), None);
            assert_eq!(player_id(":1.42"), None);
        }
    }
}
