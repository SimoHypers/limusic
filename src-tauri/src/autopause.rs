//! Notices when another app is playing media, for the "Pause when other apps play media" setting.
//!
//! It reads the apps' own media status, not the sound card, so a notification ding or a game's
//! audio never counts: only apps that publish a playing/paused state to the system media controls
//! do (browsers, video players, Spotify). Windows reads SMTC. The result goes to
//! `AppState::on_external_audio`, which owns the delays and the pause/resume decisions.

use std::sync::Arc;

use crate::state::AppState;

/// Start the platform watcher. A no-op where none exists yet.
pub fn spawn(state: Arc<AppState>) {
    #[cfg(windows)]
    windows_watcher::spawn(state);
    #[cfg(not(windows))]
    let _ = state;
}

#[cfg(windows)]
mod windows_watcher {
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::time::Duration;

    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    use crate::state::AppState;

    /// Polled, not event-driven: one cheap call a second, and the delays in AppState already
    /// absorb the lag.
    const POLL: Duration = Duration::from_secs(1);

    pub fn spawn(state: Arc<AppState>) {
        let spawned =
            std::thread::Builder::new().name("autopause-smtc".into()).spawn(move || run(state));
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "auto-pause watcher thread spawn failed");
        }
    }

    fn run(state: Arc<AppState>) {
        // Touch the system media controls only once the setting has been turned on: until then this
        // thread does nothing but wait.
        while !state.auto_pause_enabled() {
            std::thread::sleep(POLL);
        }
        // WinRT wants an initialized thread; the multithreaded apartment lets `get()` block here.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let manager = match Manager::RequestAsync().and_then(|op| op.get()) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "system media controls unavailable, auto-pause is inactive");
                return;
            }
        };
        let mut last: BTreeSet<String> = BTreeSet::new();
        loop {
            std::thread::sleep(POLL);
            if !state.auto_pause_enabled() {
                last.clear();
                continue;
            }
            let playing = match playing_apps(&manager) {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!(target: "app_lib::autopause", error = %e, "listing media sessions failed");
                    continue;
                }
            };
            if playing != last {
                tracing::info!(target: "app_lib::autopause", apps = ?playing, "other apps playing media changed");
                last = playing.clone();
            }
            state.on_external_apps(playing);
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

    /// Limusic's own media session (souvlaki) is in the same list and must never pause us.
    fn is_own(id: &str) -> bool {
        id.to_ascii_lowercase().contains("limusic")
    }
}
