//! System-wide global hotkeys management for Limusic.
//!
//! Uses `tauri-plugin-global-shortcut` to listen for keyboard events at the OS level
//! (e.g. Windows `RegisterHotKey`), allowing users to control playback even when
//! Limusic is minimized to the system tray or running in the background.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use innertube::{Rating, SongItem};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::commands::{
    db_err, editable_playlist, local_key, playlist_row, require_login, GONE, LIKED_MUSIC_ID,
    PLAYLIST_INDEX_MAX_PAGES,
};
use crate::db::Db;
use crate::state::{is_local_playlist, AppState};

pub const SETTINGS_KEY: &str = "global_hotkeys";

/// The level mute-toggle returns to. The UI's `set_volume` writes it too, so muting in the app and
/// unmuting with the hotkey (or the reverse) lands on the same level.
pub(crate) static LAST_NONZERO_VOLUME: AtomicI64 = AtomicI64::new(100);

/// Supported playback and application actions for global hotkeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    PlayPause,
    NextTrack,
    PrevTrack,
    VolumeUp,
    VolumeDown,
    MuteToggle,
    SeekForward,
    SeekBackward,
    ToggleShuffle,
    ToggleRepeat,
    TogglePlaylist,
    ShowApp,
}

fn default_true() -> bool {
    true
}

/// Global hotkeys configuration saved in local settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeysConfig {
    pub enabled: bool,
    pub bindings: HashMap<HotkeyAction, String>,
    #[serde(default)]
    pub target_playlist_id: Option<String>,
    #[serde(default)]
    pub target_playlist_name: Option<String>,
    #[serde(default = "default_true")]
    pub notify_playlist_toggle: bool,
}

impl Default for HotkeysConfig {
    // Mute, shuffle, repeat and playlist toggle start unbound: Windows sends AltGr as Ctrl+Alt,
    // so a Ctrl+Alt+letter default would take a character (Polish ś, German µ) from every app once
    // hotkeys are on.
    fn default() -> Self {
        let mut bindings = HashMap::new();
        bindings.insert(HotkeyAction::PlayPause, "Ctrl+Alt+Space".into());
        bindings.insert(HotkeyAction::NextTrack, "Ctrl+Alt+Right".into());
        bindings.insert(HotkeyAction::PrevTrack, "Ctrl+Alt+Left".into());
        bindings.insert(HotkeyAction::VolumeUp, "Ctrl+Alt+Up".into());
        bindings.insert(HotkeyAction::VolumeDown, "Ctrl+Alt+Down".into());
        bindings.insert(HotkeyAction::SeekForward, "Ctrl+Alt+PageUp".into());
        bindings.insert(HotkeyAction::SeekBackward, "Ctrl+Alt+PageDown".into());
        bindings.insert(HotkeyAction::ShowApp, "Ctrl+Alt+Home".into());
        Self {
            enabled: false,
            bindings,
            target_playlist_id: None,
            target_playlist_name: None,
            notify_playlist_toggle: true,
        }
    }
}

/// Result returned to frontend after registering hotkeys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyRegisterResult {
    pub success: bool,
    pub config: HotkeysConfig,
    pub errors: HashMap<HotkeyAction, String>,
}

/// Parse a stored binding ("Ctrl+Alt+Space") with the plugin's own parser. A bare key is refused:
/// a global grab on Space or a letter would take that key from every app on the desktop. F-keys
/// are the exception, nothing types with them.
pub fn parse_shortcut(input: &str) -> Result<Shortcut, String> {
    let shortcut: Shortcut = input.trim().parse().map_err(|e| format!("{e}"))?;
    let modified = shortcut.mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER);
    use Code::*;
    #[rustfmt::skip]
    let fkey = matches!(shortcut.key, F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12
        | F13 | F14 | F15 | F16 | F17 | F18 | F19 | F20 | F21 | F22 | F23 | F24);
    if !modified && !fkey {
        return Err(format!("'{input}' needs Ctrl, Alt or Super"));
    }
    Ok(shortcut)
}

/// Manages hotkey registration state and event dispatching.
pub struct HotkeysManager {
    pub config: Mutex<HotkeysConfig>,
    pub registered: Mutex<HashMap<Shortcut, HotkeyAction>>,
}

impl HotkeysManager {
    pub fn new(config: HotkeysConfig) -> Self {
        Self { config: Mutex::new(config), registered: Mutex::new(HashMap::new()) }
    }

    /// Register all shortcuts from `new_config`, unregistering previous ones.
    pub fn apply_config(&self, app: &AppHandle, new_config: HotkeysConfig) -> HotkeyRegisterResult {
        let mut errors = HashMap::new();
        let mut new_registered = HashMap::new();

        // Clear existing system registrations
        if let Err(e) = app.global_shortcut().unregister_all() {
            tracing::error!(error = ?e, "failed to unregister existing global hotkeys");
            let config = self.config.lock().unwrap().clone();
            return HotkeyRegisterResult { success: false, config, errors: HashMap::new() };
        }

        if new_config.enabled {
            for (&action, shortcut_str) in &new_config.bindings {
                let s = shortcut_str.trim();
                if s.is_empty() {
                    continue;
                }

                match parse_shortcut(s) {
                    Ok(shortcut) => match app.global_shortcut().register(shortcut) {
                        Ok(_) => {
                            new_registered.insert(shortcut, action);
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = ?e,
                                action = ?action,
                                shortcut = %s,
                                "failed to register global hotkey"
                            );
                            errors.insert(action, format!("{e}"));
                        }
                    },
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            action = ?action,
                            shortcut = %s,
                            "invalid shortcut string"
                        );
                        errors.insert(action, e);
                    }
                }
            }
        }

        // Swap in the new map at the end without holding the lock across register()
        {
            let mut registered_map = self.registered.lock().unwrap();
            *registered_map = new_registered;
        }

        let success = errors.is_empty();
        *self.config.lock().unwrap() = new_config.clone();

        HotkeyRegisterResult { success, config: new_config, errors }
    }

    /// Dispatch triggered shortcut to its bound action.
    pub fn handle_event(&self, app: &AppHandle, shortcut: &Shortcut, state: ShortcutState) {
        if state != ShortcutState::Pressed {
            return;
        }

        let registered_map = self.registered.lock().unwrap();
        if let Some(action) = registered_map.get(shortcut) {
            execute_action(app, *action);
        }
    }

    pub fn get_config(&self) -> HotkeysConfig {
        self.config.lock().unwrap().clone()
    }
}

/// Execute a hotkey action on `AppState`.
pub fn execute_action(app: &AppHandle, action: HotkeyAction) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>() else { return };
        let state = state.inner().clone();
        match action {
            HotkeyAction::PlayPause => {
                state.resume_or_toggle().await;
            }
            HotkeyAction::NextTrack => {
                state.next_in_queue().await;
            }
            HotkeyAction::PrevTrack => {
                state.prev_in_queue().await;
            }
            HotkeyAction::VolumeUp => change_volume(&state, &app, |v| v + 5),
            HotkeyAction::VolumeDown => change_volume(&state, &app, |v| v - 5),
            HotkeyAction::MuteToggle => change_volume(&state, &app, |v| {
                if v > 0 {
                    LAST_NONZERO_VOLUME.store(v, Ordering::Relaxed);
                    0
                } else {
                    LAST_NONZERO_VOLUME.load(Ordering::Relaxed)
                }
            }),
            HotkeyAction::SeekForward => {
                let pos = state.current_position();
                let _ = state.user_seek((pos + 10.0).max(0.0)).await;
            }
            HotkeyAction::SeekBackward => {
                let pos = state.current_position();
                let _ = state.user_seek((pos - 10.0).max(0.0)).await;
            }
            HotkeyAction::ToggleShuffle => {
                state.toggle_shuffle().await;
            }
            HotkeyAction::ToggleRepeat => {
                state.cycle_repeat().await;
            }
            HotkeyAction::TogglePlaylist => {
                toggle_current_in_playlist(&app, &state).await;
            }
            HotkeyAction::ShowApp => {
                crate::tray::show_main(&app);
            }
        }
    });
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistTogglePayload<'a> {
    playlist_id: &'a str,
    song: &'a SongItem,
    added: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    set_video_id: Option<&'a str>,
}

fn emit_playlist_toggle(
    app: &AppHandle,
    playlist_id: &str,
    song: &SongItem,
    added: bool,
    set_video_id: Option<&str>,
) {
    let payload = PlaylistTogglePayload { playlist_id, song, added, set_video_id };
    let _ = app.emit("quick-playlist-toggled", payload);
}

/// Recent `(playlist_id, video_id) -> setVideoId` handles returned by YouTube adds, so an immediate
/// second press to undo an accidental add removes the track in one request instead of paging.
fn recent_handles() -> &'static Mutex<HashMap<(String, String), String>> {
    static HANDLES: OnceLock<Mutex<HashMap<(String, String), String>>> = OnceLock::new();
    HANDLES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember_handle(playlist_id: &str, video_id: &str, set_video_id: String) {
    if set_video_id.is_empty() {
        return;
    }
    let mut map = recent_handles().lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= 64 {
        map.clear();
    }
    map.insert((playlist_id.to_owned(), video_id.to_owned()), set_video_id);
}

fn take_handle(playlist_id: &str, video_id: &str) -> Option<String> {
    let mut map = recent_handles().lock().unwrap_or_else(|e| e.into_inner());
    map.remove(&(playlist_id.to_owned(), video_id.to_owned()))
}

fn track_label(song: &SongItem) -> String {
    if song.artists.is_empty() {
        song.title.clone()
    } else {
        format!("{} \u{2014} {}", song.title, song.artists)
    }
}

fn find_row_handle(items: Vec<SongItem>, video_id: &str) -> Option<String> {
    items.into_iter().find(|s| s.video_id == video_id).and_then(|s| s.set_video_id)
}

async fn toggle_current_in_playlist(app: &AppHandle, state: &Arc<AppState>) {
    // Serialize presses so a quick double-press adds then cleanly removes instead of racing.
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = LOCK.lock().await;

    let Some(mgr) = app.try_state::<Arc<HotkeysManager>>() else { return };
    let cfg = mgr.get_config();
    let notify = cfg.notify_playlist_toggle;
    let playlist_id = cfg.target_playlist_id.as_deref().unwrap_or("").trim().to_owned();
    let playlist_name = cfg.target_playlist_name.as_deref().unwrap_or("").trim().to_owned();

    if playlist_id.is_empty() {
        if notify {
            crate::notify::action(
                app,
                "No target playlist selected",
                "Pick a playlist in Settings \u{203a} Global Hotkeys.",
            );
        }
        return;
    }

    let Some(song) = state.current_item().await else {
        if notify {
            crate::notify::action(
                app,
                "Nothing is playing",
                "Start a song to add it to a playlist.",
            );
        }
        return;
    };

    let res = if is_local_playlist(&playlist_id) {
        toggle_local_playlist(app, state, &playlist_id, &song)
    } else if playlist_id == LIKED_MUSIC_ID {
        toggle_liked_music(app, state, &song).await
    } else {
        toggle_youtube_playlist(app, state, &playlist_id, &playlist_name, &song).await
    };

    match res {
        Ok((added, title)) if notify => {
            let summary =
                if added { format!("Added to {title}") } else { format!("Removed from {title}") };
            crate::notify::action(app, &summary, &track_label(&song));
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(error = %err, %playlist_id, "quick playlist toggle failed");
            if notify {
                crate::notify::action(app, "Playlist update failed", &err);
            }
        }
    }
}

fn toggle_local_playlist(
    app: &AppHandle,
    state: &Arc<AppState>,
    playlist_id: &str,
    song: &SongItem,
) -> Result<(bool, String), String> {
    let key = local_key(playlist_id)?;
    let playlist = state.db.local_playlist(key).ok_or_else(|| GONE.to_owned())?;
    let now = crate::db::now_secs();
    if let Some(row) = state.db.local_playlist_row_for_video(key, &song.video_id) {
        state.db.remove_local_playlist_tracks(key, &[row], now).map_err(db_err)?;
        let row_str = row.to_string();
        emit_playlist_toggle(app, playlist_id, song, false, Some(&row_str));
        Ok((false, playlist.title))
    } else {
        let row_song = playlist_row(song.clone());
        let json = serde_json::to_string(&row_song).map_err(|e| e.to_string())?;
        state
            .db
            .add_local_playlist_tracks(key, &[(row_song.video_id, json)], now)
            .map_err(db_err)?;
        emit_playlist_toggle(app, playlist_id, song, true, None);
        Ok((true, playlist.title))
    }
}

async fn toggle_liked_music(
    app: &AppHandle,
    state: &Arc<AppState>,
    song: &SongItem,
) -> Result<(bool, String), String> {
    if crate::local::is_local_song(&song.video_id) {
        return Err("Local files can only be added to local playlists.".into());
    }
    let client = require_login(state)?;
    let liked = match song.rating {
        Some(Rating::Like) => true,
        Some(_) => false,
        None => state.db.has_playlist_track(LIKED_MUSIC_ID, &song.video_id),
    };
    let next = if liked { Rating::Indifferent } else { Rating::Like };
    state.rate_epoch.fetch_add(1, Ordering::SeqCst);
    state.it.rate(client, &song.video_id, next).await.map_err(|e| e.to_string())?;
    let added = next == Rating::Like;
    if added {
        state.db.add_playlist_track(LIKED_MUSIC_ID, &song.video_id);
    } else {
        state.db.remove_playlist_track(LIKED_MUSIC_ID, &song.video_id);
    }
    state.set_queue_rating(&song.video_id, next).await;
    let _ = app.emit("rating", serde_json::json!({ "videoId": song.video_id, "rating": next }));
    emit_playlist_toggle(app, LIKED_MUSIC_ID, song, added, None);
    Ok((added, "Liked Music".into()))
}

async fn toggle_youtube_playlist(
    app: &AppHandle,
    state: &Arc<AppState>,
    playlist_id: &str,
    playlist_name: &str,
    song: &SongItem,
) -> Result<(bool, String), String> {
    let vid = &song.video_id;
    if crate::local::is_local_song(vid) {
        return Err("Local files can only be added to local playlists.".into());
    }
    let client = editable_playlist(state, playlist_id)?;
    let title = if playlist_name.is_empty() { "Playlist" } else { playlist_name };
    let cached_handle = take_handle(playlist_id, vid);
    let in_playlist = cached_handle.is_some() || state.db.has_playlist_track(playlist_id, vid);

    if in_playlist {
        let removed = remove_youtube_track(state, client, playlist_id, vid, cached_handle).await?;
        if let Some(svi) = removed {
            state.db.remove_playlist_track(playlist_id, vid);
            emit_playlist_toggle(app, playlist_id, song, false, Some(&svi));
            return Ok((false, title.to_owned()));
        }
        // Stale index: the track is no longer on YouTube, so fall through and add it.
        state.db.remove_playlist_track(playlist_id, vid);
    }

    match state.it.playlist_add_handle(client, playlist_id, vid).await {
        Ok(Some(handle)) => {
            remember_handle(playlist_id, vid, handle);
            state.db.add_playlist_track(playlist_id, vid);
            emit_playlist_toggle(app, playlist_id, song, true, None);
            Ok((true, title.to_owned()))
        }
        Ok(None) => {
            // Stale index: YouTube refused a duplicate because the track is already there, so
            // toggle it off instead of silently doing nothing.
            let removed = remove_youtube_track(state, client, playlist_id, vid, None).await?;
            let Some(svi) = removed else {
                state.db.add_playlist_track(playlist_id, vid);
                return Err(format!("Could not locate track in {title} to remove it."));
            };
            state.db.remove_playlist_track(playlist_id, vid);
            emit_playlist_toggle(app, playlist_id, song, false, Some(&svi));
            Ok((false, title.to_owned()))
        }
        Err(innertube::Error::PlaylistNotEditable) => Err(format!("{title} can't be edited.")),
        Err(e) => Err(e.to_string()),
    }
}

async fn remove_youtube_track(
    state: &Arc<AppState>,
    client: &innertube::YouTubeClient,
    playlist_id: &str,
    video_id: &str,
    cached_handle: Option<String>,
) -> Result<Option<String>, String> {
    if let Some(svi) = cached_handle {
        if state.it.playlist_remove(client, playlist_id, video_id, &svi).await.is_ok() {
            return Ok(Some(svi));
        }
    }
    let Some(svi) = find_set_video_id(state, client, playlist_id, video_id).await? else {
        return Ok(None);
    };
    state
        .it
        .playlist_remove(client, playlist_id, video_id, &svi)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(svi))
}

async fn find_set_video_id(
    state: &Arc<AppState>,
    client: &innertube::YouTubeClient,
    playlist_id: &str,
    video_id: &str,
) -> Result<Option<String>, String> {
    let page = state.it.playlist(client, playlist_id, None).await.map_err(|e| e.to_string())?;
    if let Some(svi) = find_row_handle(page.items, video_id) {
        return Ok(Some(svi));
    }
    let mut token = page.continuation;
    for _ in 0..PLAYLIST_INDEX_MAX_PAGES {
        let Some(next) = token.take() else { break };
        let more =
            state.it.playlist_continuation(client, &next).await.map_err(|e| e.to_string())?;
        if let Some(svi) = find_row_handle(more.items, video_id) {
            return Ok(Some(svi));
        }
        token = more.continuation;
    }
    Ok(None)
}

/// Read, change, set, persist and echo the volume, the same as a slider commit. Serialized: each
/// press is its own task, so a held key would otherwise lose steps and two mutes could both mute.
pub(crate) fn change_volume(state: &AppState, app: &AppHandle, f: impl FnOnce(i64) -> i64) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    let volume = f(state.player.volume()).clamp(0, 100);
    if state.player.set_volume(volume).is_ok() {
        if volume > 0 {
            LAST_NONZERO_VOLUME.store(volume, Ordering::Relaxed);
        }
        state.db.set_setting("volume", &volume.to_string());
        let _ = app.emit("volume", volume);
        state.media_set_volume(volume);
    }
}

pub fn load_config(db: &Db) -> HotkeysConfig {
    if let Some(json) = db.get_setting(SETTINGS_KEY) {
        if let Ok(cfg) = serde_json::from_str::<HotkeysConfig>(&json) {
            return cfg;
        }
    }
    HotkeysConfig::default()
}

pub fn save_config(db: &Db, config: &HotkeysConfig) {
    if let Ok(json) = serde_json::to_string(config) {
        db.set_setting(SETTINGS_KEY, &json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_through_settings_json() {
        let json =
            r#"{"enabled":true,"bindings":{"play_pause":"Ctrl+Alt+Space","show_app":"F12"}}"#;
        let cfg: HotkeysConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.bindings[&HotkeyAction::ShowApp], "F12");
        assert_eq!(cfg.target_playlist_id, None);
        assert_eq!(cfg.target_playlist_name, None);
        assert!(cfg.notify_playlist_toggle);
        let back: HotkeysConfig =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.bindings, cfg.bindings);
    }

    #[test]
    fn toggle_playlist_starts_unbound_and_round_trips_target() {
        let mut cfg = HotkeysConfig::default();
        assert!(!cfg.bindings.contains_key(&HotkeyAction::TogglePlaylist));
        assert!(cfg.notify_playlist_toggle);
        cfg.bindings.insert(HotkeyAction::TogglePlaylist, "Ctrl+Shift+P".into());
        cfg.target_playlist_id = Some("VLPL123".into());
        cfg.target_playlist_name = Some("Favorites".into());
        cfg.notify_playlist_toggle = false;
        let back: HotkeysConfig =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.bindings[&HotkeyAction::TogglePlaylist], "Ctrl+Shift+P");
        assert_eq!(back.target_playlist_id.as_deref(), Some("VLPL123"));
        assert_eq!(back.target_playlist_name.as_deref(), Some("Favorites"));
        assert!(!back.notify_playlist_toggle);
    }

    #[test]
    fn track_label_and_row_handle_helpers() {
        let song = SongItem {
            video_id: "vid1".into(),
            title: "Song".into(),
            artists: "Artist".into(),
            set_video_id: Some("svi1".into()),
            ..Default::default()
        };
        assert_eq!(track_label(&song), "Song \u{2014} Artist");
        assert_eq!(track_label(&SongItem { title: "Solo".into(), ..Default::default() }), "Solo");
        assert_eq!(find_row_handle(vec![song], "vid1").as_deref(), Some("svi1"));
    }

    #[test]
    fn test_parse_shortcut_valid() {
        assert!(parse_shortcut("Ctrl+Alt+Space").is_ok());
        assert!(parse_shortcut("Ctrl+Shift+F1").is_ok());
        assert!(parse_shortcut("Alt+PageUp").is_ok());
        assert!(parse_shortcut("Ctrl+Alt+M").is_ok());
        assert!(parse_shortcut("Ctrl+Shift+=").is_ok());
        assert!(parse_shortcut("Super+Numpad5").is_ok());
        assert!(parse_shortcut("F12").is_ok());
        assert!(parse_shortcut("F13").is_ok());
        for combo in HotkeysConfig::default().bindings.values() {
            assert!(parse_shortcut(combo).is_ok(), "{combo}");
        }
    }

    #[test]
    fn test_parse_shortcut_invalid() {
        assert!(parse_shortcut("").is_err());
        assert!(parse_shortcut("Ctrl+Alt").is_err());
        assert!(parse_shortcut("NonexistentKey").is_err());
        // Bare keys would be grabbed from every app on the desktop.
        assert!(parse_shortcut("Space").is_err());
        assert!(parse_shortcut("A").is_err());
        assert!(parse_shortcut("Shift+Enter").is_err());
        assert!(parse_shortcut("MediaPlayPause").is_err());
    }
}
