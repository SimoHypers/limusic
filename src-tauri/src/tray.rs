//! System tray: app icon + menu (Show / Play-Pause / Next / Previous / Quit). Menu actions
//! route into the same [`AppState`] methods the OS media keys use (see media.rs), so the tray
//! can never behave differently from MPRIS.

use std::sync::Arc;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;

/// Managed handle to the live-label item so the mpv event pump can flip "Play"/"Pause".
pub struct TrayState {
    pub play_pause: MenuItem<Wry>,
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Limusic", true, None::<&str>)?;
    let play_pause = MenuItem::with_id(app, "play_pause", "Play", true, None::<&str>)?;
    let next = MenuItem::with_id(app, "next", "Next", true, None::<&str>)?;
    let prev = MenuItem::with_id(app, "prev", "Previous", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &play_pause,
            &next,
            &prev,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    // ponytail: menu-only interaction. Linux appindicator never delivers click events, so the
    // menu is the one behavior every platform shares; add Win/mac left-click-to-show if asked.
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Limusic")
        .on_menu_event(|app, event| handle_menu(app, event.id.as_ref()));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    app.manage(TrayState { play_pause });
    Ok(())
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }
        "quit" => {
            // Users now quit mid-song from the tray; persist the exact resume position first.
            if let Some(state) = app.try_state::<Arc<AppState>>() {
                state.flush_position();
            }
            app.exit(0);
        }
        other => {
            let Some(state) = app.try_state::<Arc<AppState>>() else { return };
            let state = state.inner().clone();
            let id = other.to_string();
            tauri::async_runtime::spawn(async move {
                match id.as_str() {
                    "play_pause" => state.resume_or_toggle().await,
                    "next" => state.next_in_queue().await,
                    "prev" => state.prev_in_queue().await,
                    _ => {}
                }
            });
        }
    }
}
