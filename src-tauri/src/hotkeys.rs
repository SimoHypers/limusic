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

static LAST_NONZERO_VOLUME: AtomicI64 = AtomicI64::new(100);

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
    fn default() -> Self {
        let mut bindings = HashMap::new();
        bindings.insert(HotkeyAction::PlayPause, "Ctrl+Alt+Space".into());
        bindings.insert(HotkeyAction::NextTrack, "Ctrl+Alt+Right".into());
        bindings.insert(HotkeyAction::PrevTrack, "Ctrl+Alt+Left".into());
        bindings.insert(HotkeyAction::VolumeUp, "Ctrl+Alt+Up".into());
        bindings.insert(HotkeyAction::VolumeDown, "Ctrl+Alt+Down".into());
        bindings.insert(HotkeyAction::MuteToggle, "Ctrl+Alt+M".into());
        bindings.insert(HotkeyAction::SeekForward, "Ctrl+Alt+PageUp".into());
        bindings.insert(HotkeyAction::SeekBackward, "Ctrl+Alt+PageDown".into());
        bindings.insert(HotkeyAction::ToggleShuffle, "Ctrl+Alt+S".into());
        bindings.insert(HotkeyAction::ToggleRepeat, "Ctrl+Alt+R".into());
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

/// Parse a user-provided shortcut string into a `Shortcut` struct.
/// Supports F-keys (F1-F12), 2-key and 3-key combinations with Ctrl, Alt, Shift, Super.
/// E.g. "Alt+PageUp", "Ctrl+Shift+F1", "Ctrl+Alt+Space".
pub fn parse_shortcut(input: &str) -> Result<Shortcut, String> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err("Shortcut string cannot be empty".into());
    }

    let mut mods = Modifiers::empty();
    let mut key_code: Option<Code> = None;

    for part in raw.split('+') {
        let token = part.trim();
        if token.is_empty() {
            continue;
        }

        match token.to_lowercase().as_str() {
            "ctrl" | "control" => mods.insert(Modifiers::CONTROL),
            "alt" | "option" => mods.insert(Modifiers::ALT),
            "shift" => mods.insert(Modifiers::SHIFT),
            "super" | "win" | "windows" | "meta" | "cmd" | "command" => {
                mods.insert(Modifiers::SUPER)
            }
            other => {
                if key_code.is_some() {
                    return Err(format!(
                        "Multiple primary keys specified in shortcut: '{}' and '{}'",
                        token, raw
                    ));
                }
                let code = parse_key_code(other)
                    .ok_or_else(|| format!("Unknown key: '{}' in shortcut '{}'", token, raw))?;
                key_code = Some(code);
            }
        }
    }

    let code = key_code
        .ok_or_else(|| format!("No primary key found in shortcut '{}' (modifiers only)", raw))?;

    let mod_opt = if mods.is_empty() { None } else { Some(mods) };
    Ok(Shortcut::new(mod_opt, code))
}

fn parse_key_code(token: &str) -> Option<Code> {
    match token {
        // Function keys
        "f1" => Some(Code::F1),
        "f2" => Some(Code::F2),
        "f3" => Some(Code::F3),
        "f4" => Some(Code::F4),
        "f5" => Some(Code::F5),
        "f6" => Some(Code::F6),
        "f7" => Some(Code::F7),
        "f8" => Some(Code::F8),
        "f9" => Some(Code::F9),
        "f10" => Some(Code::F10),
        "f11" => Some(Code::F11),
        "f12" => Some(Code::F12),

        // Navigation
        "pageup" | "page_up" | "pgup" => Some(Code::PageUp),
        "pagedown" | "page_down" | "pgdn" => Some(Code::PageDown),
        "home" => Some(Code::Home),
        "end" => Some(Code::End),
        "insert" | "ins" => Some(Code::Insert),
        "delete" | "del" => Some(Code::Delete),

        // Arrows
        "up" | "arrowup" | "arrow_up" => Some(Code::ArrowUp),
        "down" | "arrowdown" | "arrow_down" => Some(Code::ArrowDown),
        "left" | "arrowleft" | "arrow_left" => Some(Code::ArrowLeft),
        "right" | "arrowright" | "arrow_right" => Some(Code::ArrowRight),

        // Common keys
        "space" | "spacebar" => Some(Code::Space),
        "enter" | "return" => Some(Code::Enter),
        "tab" => Some(Code::Tab),
        "backspace" => Some(Code::Backspace),
        "escape" | "esc" => Some(Code::Escape),

        // Letters
        "a" => Some(Code::KeyA),
        "b" => Some(Code::KeyB),
        "c" => Some(Code::KeyC),
        "d" => Some(Code::KeyD),
        "e" => Some(Code::KeyE),
        "f" => Some(Code::KeyF),
        "g" => Some(Code::KeyG),
        "h" => Some(Code::KeyH),
        "i" => Some(Code::KeyI),
        "j" => Some(Code::KeyJ),
        "k" => Some(Code::KeyK),
        "l" => Some(Code::KeyL),
        "m" => Some(Code::KeyM),
        "n" => Some(Code::KeyN),
        "o" => Some(Code::KeyO),
        "p" => Some(Code::KeyP),
        "q" => Some(Code::KeyQ),
        "r" => Some(Code::KeyR),
        "s" => Some(Code::KeyS),
        "t" => Some(Code::KeyT),
        "u" => Some(Code::KeyU),
        "v" => Some(Code::KeyV),
        "w" => Some(Code::KeyW),
        "x" => Some(Code::KeyX),
        "y" => Some(Code::KeyY),
        "z" => Some(Code::KeyZ),

        // Digits
        "0" => Some(Code::Digit0),
        "1" => Some(Code::Digit1),
        "2" => Some(Code::Digit2),
        "3" => Some(Code::Digit3),
        "4" => Some(Code::Digit4),
        "5" => Some(Code::Digit5),
        "6" => Some(Code::Digit6),
        "7" => Some(Code::Digit7),
        "8" => Some(Code::Digit8),
        "9" => Some(Code::Digit9),

        // Numpad
        "numpad0" | "num0" => Some(Code::Numpad0),
        "numpad1" | "num1" => Some(Code::Numpad1),
        "numpad2" | "num2" => Some(Code::Numpad2),
        "numpad3" | "num3" => Some(Code::Numpad3),
        "numpad4" | "num4" => Some(Code::Numpad4),
        "numpad5" | "num5" => Some(Code::Numpad5),
        "numpad6" | "num6" => Some(Code::Numpad6),
        "numpad7" | "num7" => Some(Code::Numpad7),
        "numpad8" | "num8" => Some(Code::Numpad8),
        "numpad9" | "num9" => Some(Code::Numpad9),

        // Media keys
        "mediaplaypause" | "playpause" => Some(Code::MediaPlayPause),
        "mediatracknext" | "nexttrack" | "medianext" => Some(Code::MediaTrackNext),
        "mediatrackprevious" | "prevtrack" | "mediaprev" => Some(Code::MediaTrackPrevious),
        "mediastop" | "stop" => Some(Code::MediaStop),
        "audiovolumeup" | "volumeup" => Some(Code::AudioVolumeUp),
        "audiovolumedown" | "volumedown" => Some(Code::AudioVolumeDown),
        "audiovolumemute" | "volumemute" | "mute" => Some(Code::AudioVolumeMute),

        // Symbols
        "-" | "minus" => Some(Code::Minus),
        "=" | "equal" => Some(Code::Equal),
        "[" | "bracketleft" => Some(Code::BracketLeft),
        "]" | "bracketright" => Some(Code::BracketRight),
        "\\" | "backslash" => Some(Code::Backslash),
        ";" | "semicolon" => Some(Code::Semicolon),
        "'" | "quote" => Some(Code::Quote),
        "," | "comma" => Some(Code::Comma),
        "." | "period" => Some(Code::Period),
        "/" | "slash" => Some(Code::Slash),

        _ => None,
    }
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
            HotkeyAction::VolumeUp => {
                let cur = state.player.volume();
                let next = (cur + 5).clamp(0, 100);
                if state.player.set_volume(next).is_ok() {
                    if next > 0 {
                        LAST_NONZERO_VOLUME.store(next, Ordering::Relaxed);
                    }
                    state.db.set_setting("volume", &next.to_string());
                    let _ = app.emit("volume", next);
                }
            }
            HotkeyAction::VolumeDown => {
                let cur = state.player.volume();
                let next = (cur - 5).clamp(0, 100);
                if state.player.set_volume(next).is_ok() {
                    if next > 0 {
                        LAST_NONZERO_VOLUME.store(next, Ordering::Relaxed);
                    }
                    state.db.set_setting("volume", &next.to_string());
                    let _ = app.emit("volume", next);
                }
            }
            HotkeyAction::MuteToggle => {
                let cur = state.player.volume();
                let next = if cur > 0 {
                    LAST_NONZERO_VOLUME.store(cur, Ordering::Relaxed);
                    0
                } else {
                    let last = LAST_NONZERO_VOLUME.load(Ordering::Relaxed);
                    if last > 0 {
                        last
                    } else {
                        let saved = crate::state::saved_volume(&state.db);
                        if saved > 0 {
                            saved
                        } else {
                            100
                        }
                    }
                };
                if state.player.set_volume(next).is_ok() {
                    state.db.set_setting("volume", &next.to_string());
                    let _ = app.emit("volume", next);
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
    fn test_parse_shortcut_valid() {
        assert!(parse_shortcut("Ctrl+Alt+Space").is_ok());
        assert!(parse_shortcut("Ctrl+Shift+F1").is_ok());
        assert!(parse_shortcut("Alt+PageUp").is_ok());
        assert!(parse_shortcut("Ctrl+Alt+M").is_ok());
        assert!(parse_shortcut("Ctrl+Alt+Right").is_ok());
        assert!(parse_shortcut("Ctrl+Alt+Down").is_ok());
        assert!(parse_shortcut("Ctrl+Alt+Home").is_ok());
        assert!(parse_shortcut("Ctrl+Shift+=").is_ok());
        assert!(parse_shortcut("F12").is_ok());
    }

    #[test]
    fn test_parse_shortcut_invalid() {
        assert!(parse_shortcut("").is_err());
        assert!(parse_shortcut("Ctrl+Alt").is_err());
        assert!(parse_shortcut("Ctrl+F13").is_err());
        assert!(parse_shortcut("NonexistentKey").is_err());
    }
}
