//! System tray: app icon + menu (Show / Play-Pause / Next / Previous / Restart / Quit). Menu actions
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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

pub use imp::{init, set_icon, set_playing, set_visible};

/// Whether a tray icon actually exists for the user to click.
///
/// On Linux it is a StatusNotifierItem, and a bar that only speaks the older XEmbed systray spec
/// (i3bar, dwm, xfce4-panel without its SNI plugin) runs no `StatusNotifierWatcher` at all, so
/// registration fails and no icon ever appears (#232). Closing to a tray that isn't there leaves
/// the app running with no window and no way back except a second launch, so ✕ checks this before
/// hiding. Optimistic: cleared by the Linux backend once it knows there is no watcher, and by
/// [`set_available`] when `init` fails.
static AVAILABLE: AtomicBool = AtomicBool::new(true);

/// Can the window be brought back from the tray? See [`AVAILABLE`].
pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

pub fn set_available(val: bool) {
    AVAILABLE.store(val, Ordering::Relaxed);
}

/// Bring the main window back from close-to-tray, minimize, or the mini player. Every "come back"
/// path — tray menu, tray click, second launch, the widget's restore button — goes through here so
/// they can't drift apart.
pub fn show_main(app: &AppHandle) {
    crate::mini::close(app);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        set_main_visible(app, true);
    }
}

/// Tell the main window's SPA whether anyone can see it.
///
/// WebKitGTK does not pass a GTK hide down to the page: `document.visibilityState` stays
/// `"visible"` for a window that is closed to the tray, so the SPA cannot work this out on its own
/// and keeps restyling for nobody. That is worse than wasted work. While the window is unmapped
/// the web process never gives any of it back: measured over 20 track changes with the window
/// hidden, it grew 137 MB and held it, then dropped the whole lot within two seconds of the window
/// being shown again. A night in the tray is thousands of those. See `theme.svelte.ts`.
pub fn set_main_visible(app: &AppHandle, visible: bool) {
    let _ = app.emit_to("main", "ui-visible", visible);
}

/// Shared by both backends: menu ids are the contract between them.
fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main(app),
        "quit" | "restart" => {
            // Users now quit mid-song from the tray; persist the exact resume position first.
            if let Some(state) = app.try_state::<Arc<AppState>>() {
                state.flush_position();
            }
            // Same for the widget's own position, if that's what they were quitting from.
            crate::mini::save_position(app);
            if id == "restart" {
                // `request_restart`, not `restart`: it goes through RunEvent::Exit, which is
                // where the single-instance plugin releases the D-Bus name. Skip that and the
                // relaunched process hands off to the still-dying one and exits, leaving no app.
                //
                // ponytail: under `cargo tauri dev` this leaves you with no window. The CLI
                // exits when its child does, taking the vite server with it, so the relaunched
                // binary navigates to a dead devUrl and the transparent frameless window paints
                // nothing (taskbar + tray entry, no visible window). Release builds embed the
                // frontend, so restart is only usable there.
                app.request_restart();
            } else {
                app.exit(0);
            }
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
    use std::sync::atomic::Ordering;
    use std::sync::OnceLock;

    use ksni::menu::{MenuItem, StandardItem};
    use ksni::{Handle, Icon, OfflineReason, Tray, TrayMethods};
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

        fn watcher_online(&self) {
            super::AVAILABLE.store(true, Ordering::Relaxed);
        }

        /// No watcher on the bus: the icon is not showing anywhere, whatever the user's setting
        /// says. Returning `true` keeps the service running, so a watcher that appears later
        /// (`snixembed`, a restarted shell) still gets the icon and flips this back.
        fn watcher_offline(&self, reason: OfflineReason) -> bool {
            tracing::warn!(
                "tray: no StatusNotifierWatcher ({reason:?}); ✕ will quit instead of hiding"
            );
            super::AVAILABLE.store(false, Ordering::Relaxed);
            true
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
                item("Restart", "restart"),
                item("Quit", "quit"),
            ]
        }
    }

    /// Tauri hands us RGBA; StatusNotifierItem wants ARGB32 in network byte order.
    fn icon_pixmap(img: &tauri::image::Image<'_>) -> Vec<Icon> {
        let mut data = img.rgba().to_vec();
        for px in data.chunks_exact_mut(4) {
            px.rotate_right(1); // [R,G,B,A] -> [A,R,G,B]
        }
        vec![Icon { width: img.width() as i32, height: img.height() as i32, data }]
    }

    pub fn init(app: &AppHandle) -> tauri::Result<()> {
        let icon = crate::appicon::current(app).map(|i| icon_pixmap(&i)).unwrap_or_default();
        let tray = LimusicTray { app: app.clone(), playing: false, icon };
        // Registering with the StatusNotifierWatcher is async and can outlive setup(); a failure
        // here costs the tray, not the app, so it's logged rather than propagated.
        //
        // `assume_sni_available`: a missing watcher becomes a `watcher_offline` call instead of a
        // hard error, which both keeps the service alive for a watcher that starts later and is
        // how `AVAILABLE` learns the truth (spawn calls it before returning `Ok`).
        tauri::async_runtime::spawn(async move {
            match tray.assume_sni_available(true).spawn().await {
                Ok(handle) => {
                    let _ = HANDLE.set(handle);
                }
                Err(e) => {
                    super::AVAILABLE.store(false, Ordering::Relaxed);
                    tracing::error!("tray: StatusNotifierItem registration failed: {e}");
                }
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

    pub fn set_icon(_app: &AppHandle, icon: &tauri::image::Image<'_>) {
        let Some(handle) = HANDLE.get() else { return };
        let pixmap = icon_pixmap(icon);
        tauri::async_runtime::spawn(async move {
            handle.update(|t| t.icon = pixmap).await;
        });
    }

    /// Not offered on Linux, so this is never reached from the UI: the setting is macOS-only. The
    /// StatusNotifierItem has no cheap hide, and hiding it there would strand close-to-tray with
    /// nothing to come back from. Always `false` — there is no icon here to have reached.
    pub fn set_visible(_app: &AppHandle, _visible: bool) -> bool {
        false
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
        let restart = MenuItem::with_id(app, "restart", "Restart", true, None::<&str>)?;
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
                &restart,
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
        if let Some(icon) = crate::appicon::current(app) {
            builder = builder.icon(sized(&icon));
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

    pub fn set_icon(app: &AppHandle, icon: &tauri::image::Image<'_>) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_icon(Some(sized(icon)));
        }
    }

    /// Show or hide the icon, reporting whether the call reached a live tray item and succeeded
    /// (`init` can have failed, leaving none). macOS-only in practice (Settings > General): the
    /// status item has no reason to exist next to the Dock, and the setting is not offered where
    /// there is no Dock to fall back on. Hidden, the window is gone too, so callers pair this with
    /// [`super::AVAILABLE`].
    pub fn set_visible(app: &AppHandle, visible: bool) -> bool {
        match app.tray_by_id("main") {
            Some(tray) => tray.set_visible(visible).is_ok(),
            None => false,
        }
    }

    /// Windows draws the notification area at `SM_CXSMICON` and stretches anything else without
    /// filtering (`tray-icon` builds the HICON at whatever size it is handed), so resample first.
    /// That is half of #225: the bundled fallback was a 32px `.ico` entry, a custom icon up to
    /// 1024px, and neither is what the tray draws at.
    #[cfg(target_os = "windows")]
    fn sized(icon: &tauri::image::Image<'_>) -> tauri::image::Image<'static> {
        let s = crate::taskbar::tray_icon_size();
        crate::appicon::scaled(icon, s, s)
    }

    /// macOS sets the NSImage's size itself, so there this is a copy and nothing more.
    #[cfg(not(target_os = "windows"))]
    fn sized(icon: &tauri::image::Image<'_>) -> tauri::image::Image<'static> {
        icon.clone().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{available, AVAILABLE};

    /// The gate lib.rs reads before hiding the window on ✕.
    #[test]
    fn no_watcher_means_no_close_to_tray() {
        assert!(available(), "a desktop with a working tray is the default");
        AVAILABLE.store(false, Ordering::Relaxed); // what watcher_offline does on i3bar (#232)
        assert!(!available());
        AVAILABLE.store(true, Ordering::Relaxed);
    }
}
