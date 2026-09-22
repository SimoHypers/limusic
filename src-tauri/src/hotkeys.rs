//! System-wide global hotkeys management for Limusic.
//!
//! Uses `tauri-plugin-global-shortcut` to listen for keyboard events at the OS level
//! (e.g. Windows `RegisterHotKey`), allowing users to control playback even when
//! Limusic is minimized to the system tray or running in the background.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::db::Db;
use crate::state::AppState;

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
    ShowApp,
}

/// Global hotkeys configuration saved in local settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeysConfig {
    pub enabled: bool,
    pub bindings: HashMap<HotkeyAction, String>,
}

impl Default for HotkeysConfig {
    // Mute, shuffle and repeat start unbound: Windows sends AltGr as Ctrl+Alt, so a Ctrl+Alt+letter
    // default would take a character (Polish ś, German µ) from every app once hotkeys are on.
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
        Self { enabled: false, bindings }
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
            HotkeyAction::VolumeUp => set_volume(&state, &app, state.player.volume() + 5),
            HotkeyAction::VolumeDown => set_volume(&state, &app, state.player.volume() - 5),
            HotkeyAction::MuteToggle => {
                let cur = state.player.volume();
                if cur > 0 {
                    LAST_NONZERO_VOLUME.store(cur, Ordering::Relaxed);
                    set_volume(&state, &app, 0);
                } else {
                    set_volume(&state, &app, LAST_NONZERO_VOLUME.load(Ordering::Relaxed));
                }
            }
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
            HotkeyAction::ShowApp => {
                crate::tray::show_main(&app);
            }
        }
    });
}

/// Set, persist and echo a volume, the same as a slider commit.
fn set_volume(state: &AppState, app: &AppHandle, volume: i64) {
    let volume = volume.clamp(0, 100);
    if state.player.set_volume(volume).is_ok() {
        if volume > 0 {
            LAST_NONZERO_VOLUME.store(volume, Ordering::Relaxed);
        }
        state.db.set_setting("volume", &volume.to_string());
        let _ = app.emit("volume", volume);
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
        let back: HotkeysConfig =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.bindings, cfg.bindings);
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
