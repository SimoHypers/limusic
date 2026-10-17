//! Mini player: a small always-on-top widget that stands in for the main window.
//!
//! It loads the same SPA as the main window — the root layout branches on the window label — so
//! there is no second bundle and no second copy of the playback state: every event this app emits
//! is global (`app.emit`, never `emit_to`), so both webviews are driven by the same stream.
//!
//! Opening it hides the main window; the app keeps running in the tray. Coming back is always
//! [`crate::tray::show_main`] — the widget's restore button, a tray click, the tray menu and a
//! second launch all land there, so they cannot drift apart.
//!
//! ponytail: a second webview costs a second WebKit web process. Resizing the main window in place
//! would be cheaper, but it throws away everything the app has rendered on every toggle. Revisit
//! only if idle memory in mini mode ever matters more than an instant restore.

#[cfg(not(target_os = "android"))]
mod desktop_mini {
    use std::sync::Arc;

    use tauri::{
        AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
        WebviewWindowBuilder,
    };

    use crate::state::AppState;

    pub const LABEL: &str = "mini";

    const FULL: (f64, f64) = (560.0, 180.0);
    const COMPACT: (f64, f64) = (320.0, 88.0);
    const COMPACT_KEY: &str = "mini_compact";
    const MARGIN: f64 = 24.0;
    const POS_KEY: &str = "mini_position";

    pub fn open(app: &AppHandle) -> Result<(), String> {
        if let Some(w) = app.get_webview_window(LABEL) {
            let _ = w.show();
            let _ = w.set_focus();
        } else {
            let (w, h) = size(app);
            let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
                .title("Limusic Mini Player")
                .inner_size(w, h)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .shadow(false)
                .visible(false);
            let win = crate::http::proxied(win)
                .build()
                .map_err(|e| format!("couldn't open the mini player: {e}"))?;

            #[cfg(target_os = "linux")]
            let _ = win.with_webview(|wv| {
                use gtk::prelude::WidgetExt;
                wv.inner().set_size_request(1, 1);
            });
            if let Some(p) = placement(app, &win, (w, h)) {
                let _ = win.set_position(p);
            }
            #[cfg(target_os = "linux")]
            crate::tune_webview_labelled(app, LABEL, false);
            let _ = win.show();
            let _ = win.set_focus();
        }
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.hide();
            crate::tray::set_main_visible(app, false);
        }
        Ok(())
    }

    pub fn set_compact(app: &AppHandle, compact: bool) -> Result<(), String> {
        let win = app.get_webview_window(LABEL).ok_or("the mini player isn't open")?;
        let (w, h) = if compact { COMPACT } else { FULL };
        let scale = win.scale_factor().map_err(|e| e.to_string())?;
        let new = PhysicalSize::new((w * scale).round() as u32, (h * scale).round() as u32);
        let to = match (win.outer_position(), win.outer_size(), win.current_monitor()) {
            (Ok(pos), Ok(old), Ok(Some(m))) => Some(anchored(pos, old, new, *m.position(), *m.size())),
            _ => None,
        };
        win.set_size(new).map_err(|e| e.to_string())?;
        if let Some(p) = to {
            let _ = win.set_position(p);
        }
        if let Some(state) = app.try_state::<Arc<AppState>>() {
            state.db.set_setting(COMPACT_KEY, if compact { "true" } else { "false" });
        }
        Ok(())
    }

    fn size(app: &AppHandle) -> (f64, f64) {
        let compact = app
            .try_state::<Arc<AppState>>()
            .and_then(|s| s.db.get_setting(COMPACT_KEY))
            .is_some_and(|v| v == "true");
        if compact {
            COMPACT
        } else {
            FULL
        }
    }

    fn anchored(
        pos: PhysicalPosition<i32>,
        old: PhysicalSize<u32>,
        new: PhysicalSize<u32>,
        origin: PhysicalPosition<i32>,
        display: PhysicalSize<u32>,
    ) -> PhysicalPosition<i32> {
        let axis = |p: i32, old: u32, new: u32, start: i32, len: u32| {
            let far_half = 2 * p + old as i32 > 2 * start + len as i32;
            if far_half {
                p + old as i32 - new as i32
            } else {
                p
            }
        };
        PhysicalPosition::new(
            axis(pos.x, old.width, new.width, origin.x, display.width),
            axis(pos.y, old.height, new.height, origin.y, display.height),
        )
    }

    pub fn save_position(app: &AppHandle) {
        let Some(w) = app.get_webview_window(LABEL) else { return };
        if let (Ok(p), Some(state)) = (w.outer_position(), app.try_state::<Arc<AppState>>()) {
            state.db.set_setting(POS_KEY, &format!("{},{}", p.x, p.y));
        }
    }

    pub fn close(app: &AppHandle) {
        if app.get_webview_window(LABEL).is_none() {
            return;
        }
        save_position(app);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(w) = handle.get_webview_window(LABEL) {
                let _ = w.destroy();
            }
        });
    }

    fn placement(
        app: &AppHandle,
        win: &WebviewWindow,
        size: (f64, f64),
    ) -> Option<PhysicalPosition<i32>> {
        app.try_state::<Arc<AppState>>()
            .and_then(|s| s.db.get_setting(POS_KEY))
            .and_then(|v| parse_pos(&v))
            .filter(|p| on_a_display(win, *p, win.outer_size().unwrap_or_default()))
            .or_else(|| bottom_right(app, win, size))
    }

    fn parse_pos(s: &str) -> Option<PhysicalPosition<i32>> {
        let (x, y) = s.split_once(',')?;
        Some(PhysicalPosition::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
    }

    pub(crate) fn on_a_display(
        win: &WebviewWindow,
        p: PhysicalPosition<i32>,
        size: PhysicalSize<u32>,
    ) -> bool {
        let mid = middle(p, size);
        win.available_monitors()
            .is_ok_and(|monitors| monitors.iter().any(|m| contains(*m.position(), *m.size(), mid)))
    }

    fn middle(p: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> PhysicalPosition<i32> {
        PhysicalPosition::new(p.x + size.width as i32 / 2, p.y + size.height as i32 / 2)
    }

    fn bottom_right(
        app: &AppHandle,
        win: &WebviewWindow,
        (w, h): (f64, f64),
    ) -> Option<PhysicalPosition<i32>> {
        let anchor = app.get_webview_window("main").unwrap_or_else(|| win.clone());
        let m = anchor
            .current_monitor()
            .ok()
            .flatten()
            .or_else(|| anchor.primary_monitor().ok().flatten())?;
        let area = m.work_area();
        let px = |logical: f64| (logical * m.scale_factor()).round() as i32;
        Some(PhysicalPosition::new(
            area.position.x + area.size.width as i32 - px(w + MARGIN),
            area.position.y + area.size.height as i32 - px(h + MARGIN),
        ))
    }

    fn contains(
        origin: PhysicalPosition<i32>,
        size: PhysicalSize<u32>,
        p: PhysicalPosition<i32>,
    ) -> bool {
        (origin.x..origin.x + size.width as i32).contains(&p.x)
            && (origin.y..origin.y + size.height as i32).contains(&p.y)
    }

    #[cfg(test)]
    mod tests {
        use super::{anchored, contains, middle, parse_pos};
        use tauri::{PhysicalPosition, PhysicalSize};

        #[test]
        fn parses_what_close_wrote() {
            assert_eq!(parse_pos("120,64"), Some(PhysicalPosition::new(120, 64)));
            assert_eq!(parse_pos("-1800,-200"), Some(PhysicalPosition::new(-1800, -200)));
            assert_eq!(parse_pos("garbage"), None);
            assert_eq!(parse_pos("12,"), None);
        }

        #[test]
        fn point_lands_on_the_right_display() {
            let primary = (PhysicalPosition::new(0, 0), PhysicalSize::new(1920u32, 1080));
            let left = (PhysicalPosition::new(-1920, 0), PhysicalSize::new(1920u32, 1080));

            assert!(contains(primary.0, primary.1, PhysicalPosition::new(1300, 800)));
            assert!(!contains(primary.0, primary.1, PhysicalPosition::new(-500, 800)));
            assert!(contains(left.0, left.1, PhysicalPosition::new(-500, 800)));
            assert!(!contains(primary.0, primary.1, PhysicalPosition::new(1920, 500)));
            assert!(!contains(primary.0, primary.1, PhysicalPosition::new(500, 1080)));
        }

        #[test]
        fn a_window_counts_by_its_middle() {
            let a = (PhysicalPosition::new(0, 0), PhysicalSize::new(1920u32, 1080));
            let (at, size) = (PhysicalPosition::new(1911, -9), PhysicalSize::new(1900u32, 1008));
            assert!(contains(a.0, a.1, PhysicalPosition::new(at.x, at.y + size.height as i32)));
            assert!(!contains(a.0, a.1, middle(at, size)));
            let maxed = middle(PhysicalPosition::new(-9, -9), PhysicalSize::new(1938, 1098));
            assert!(contains(a.0, a.1, maxed));
        }

        #[test]
        fn resizing_keeps_the_nearest_screen_edges() {
            let full = PhysicalSize::new(560u32, 180);
            let small = PhysicalSize::new(320u32, 88);
            let at = |x, y| PhysicalPosition::new(x, y);
            let shrink = |p, origin, display| anchored(p, full, small, origin, display);
            let grow = |p, origin, display| anchored(p, small, full, origin, display);
            let fhd = PhysicalSize::new(1920u32, 1080);

            assert_eq!(shrink(at(1336, 876), at(0, 0), fhd), at(1576, 968));
            assert_eq!(shrink(at(24, 24), at(0, 0), fhd), at(24, 24));
            assert_eq!(shrink(at(1336, 24), at(0, 0), fhd), at(1576, 24));
            assert_eq!(shrink(at(-584, 876), at(-1920, 0), fhd), at(-344, 968));

            for (p, origin) in
                [(at(1336, 876), at(0, 0)), (at(24, 24), at(0, 0)), (at(900, 300), at(0, 0))]
            {
                assert_eq!(grow(shrink(p, origin, fhd), origin, fhd), p);
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
pub use desktop_mini::*;

#[cfg(target_os = "android")]
mod android_mini {
    use tauri::{AppHandle, PhysicalPosition, PhysicalSize, WebviewWindow};

    pub const LABEL: &str = "mini";

    pub fn open(_app: &AppHandle) -> Result<(), String> {
        Err("mini player is not supported on Android".into())
    }

    pub fn set_compact(_app: &AppHandle, _compact: bool) -> Result<(), String> {
        Ok(())
    }

    pub fn save_position(_app: &AppHandle) {}

    pub fn close(_app: &AppHandle) {}

    pub(crate) fn on_a_display(
        _win: &WebviewWindow,
        _p: PhysicalPosition<i32>,
        _size: PhysicalSize<u32>,
    ) -> bool {
        true
    }
}

#[cfg(target_os = "android")]
pub use android_mini::*;
