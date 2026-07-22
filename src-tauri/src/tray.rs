//! System tray: app icon + menu (Show / Play-Pause / Next / Previous / Quit). Menu actions
//! route into the same [`AppState`] methods the OS media keys use (see media.rs), so the tray
//! can never behave differently from MPRIS.
//!
//! Two backends, because clicking the icon to restore the window is unreachable on Linux
//! otherwise: Tauri's `tray-icon` talks to **libappindicator**, whose D-Bus item exposes no
//! `Activate` method at all (only `SecondaryActivate`/`Scroll`), so a left-click has nothing to
//! call and `TrayIconEvent` never fires — its GTK backend wires up zero signals. Electron apps
//! get click-to-restore by implementing StatusNotifierItem directly and advertising
//! `ItemIsMenu=false`; [`ksni`] does the same for us on Linux. Windows/macOS keep `tray-icon`.
//!
//! Both backends expose the same two entry points — [`init`] and [`set_playing`] — so lib.rs
//! never learns which one is live.

use std::sync::Arc;

use tauri::{AppHandle, Manager};

use crate::state::AppState;

pub use imp::{init, set_playing};

/// Bring the main window back from close-to-tray or minimize. Every "come back" path — tray
/// menu, tray click, second launch — goes through here so they can't drift apart.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Shared by both backends: menu ids are the contract between them.
fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main(app),
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

#[cfg(target_os = "linux")]
mod imp {
    use std::sync::OnceLock;

    use ksni::menu::{MenuItem, StandardItem};
    use ksni::{Handle, Icon, Tray, TrayMethods};
    use tauri::AppHandle;

    use super::{handle_menu, show_main};

    /// `Handle` isn't `Clone`, so it lives here rather than in Tauri's managed state — that also
    /// keeps [`set_playing`] callable without borrowing across an await.
    static HANDLE: OnceLock<Handle<LimusicTray>> = OnceLock::new();

    struct LimusicTray {
        app: AppHandle,
        playing: bool,
        icon: Vec<Icon>,
    }

    impl Tray for LimusicTray {
        fn id(&self) -> String {
            "limusic".into()
        }

        fn title(&self) -> String {
            "Limusic".into()
        }

        fn icon_pixmap(&self) -> Vec<Icon> {
            self.icon.clone()
        }

        /// The entire reason this backend exists: Plasma dispatches a left-click here.
        fn activate(&mut self, _x: i32, _y: i32) {
            show_main(&self.app);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let item = |label: &str, id: &'static str| {
                MenuItem::from(StandardItem {
                    label: label.into(),
                    activate: Box::new(move |t: &mut Self| handle_menu(&t.app, id)),
                    ..Default::default()
                })
            };
            vec![
                item("Show Limusic", "show"),
                MenuItem::Separator,
                item(if self.playing { "Pause" } else { "Play" }, "play_pause"),
                item("Next", "next"),
                item("Previous", "prev"),
                MenuItem::Separator,
                item("Quit", "quit"),
            ]
        }
    }

    /// Tauri hands us RGBA; StatusNotifierItem wants ARGB32 in network byte order.
    fn icon_pixmap(app: &AppHandle) -> Vec<Icon> {
        let Some(img) = app.default_window_icon() else { return Vec::new() };
        let mut data = img.rgba().to_vec();
        for px in data.chunks_exact_mut(4) {
            px.rotate_right(1); // [R,G,B,A] -> [A,R,G,B]
        }
        vec![Icon { width: img.width() as i32, height: img.height() as i32, data }]
    }

    pub fn init(app: &AppHandle) -> tauri::Result<()> {
        let tray =
            LimusicTray { app: app.clone(), playing: false, icon: icon_pixmap(app) };
        // Registering with the StatusNotifierWatcher is async and can outlive setup(); a failure
        // here costs the tray, not the app, so it's logged rather than propagated.
        tauri::async_runtime::spawn(async move {
            match tray.spawn().await {
                Ok(handle) => {
                    let _ = HANDLE.set(handle);
                }
                Err(e) => tracing::error!("tray: StatusNotifierItem registration failed: {e}"),
            }
        });
        Ok(())
    }

    pub fn set_playing(_app: &AppHandle, playing: bool) {
        let Some(handle) = HANDLE.get() else { return };
        tauri::async_runtime::spawn(async move {
            handle.update(|t| t.playing = playing).await;
        });
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
    use tauri::{AppHandle, Manager, Wry};

    use super::{handle_menu, show_main};

    /// Managed handle to the live-label item so the mpv event pump can flip "Play"/"Pause".
    struct TrayState {
        play_pause: MenuItem<Wry>,
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

        // Menu on right-click only, so a left double-click can't also pop it open behind the
        // window it just restored.
        let mut builder = TrayIconBuilder::with_id("main")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .tooltip("Limusic")
            .on_menu_event(|app, event| handle_menu(app, event.id.as_ref()))
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } = event {
                    show_main(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        builder.build(app)?;

        app.manage(TrayState { play_pause });
        Ok(())
    }

    pub fn set_playing(app: &AppHandle, playing: bool) {
        if let Some(t) = app.try_state::<TrayState>() {
            let _ = t.play_pause.set_text(if playing { "Pause" } else { "Play" });
        }
    }
}
