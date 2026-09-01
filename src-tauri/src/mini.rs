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

use std::sync::Arc;

use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::state::AppState;

pub const LABEL: &str = "mini";

/// Logical size of the widget. Fixed: it's a pill, not a window you arrange.
const W: f64 = 560.0;
const H: f64 = 180.0;
/// Inset from the screen edge the first time it opens.
const MARGIN: f64 = 24.0;
/// Where the user last dragged it, as physical `"x,y"`. Physical because monitor geometry is, and
/// two displays can disagree on scale factor.
const POS_KEY: &str = "mini_position";

/// Build (or re-show) the widget, and hide the main window behind it.
///
/// **Main thread only** — GTK wants window creation there, same rule as the login and cipher
/// webviews. `commands::open_mini` does the hop.
pub fn open(app: &AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.show();
        let _ = w.set_focus();
    } else {
        let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
            .title("Limusic")
            .inner_size(W, H)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            // A square WM shadow around a rounded transparent window looks broken.
            .shadow(false)
            // Positioned before it is shown, so it never flashes wherever the WM guessed first.
            .visible(false)
            .build()
            .map_err(|e| format!("couldn't open the mini player: {e}"))?;

        if let Some(p) = placement(app, &win) {
            let _ = win.set_position(p);
        }
        // Same treatment as the main window: this is a second web process, and it needs the media
        // and 3D stacks even less than the app does.
        #[cfg(target_os = "linux")]
        crate::tune_webview_labelled(app, LABEL, false);
        let _ = win.show();
        let _ = win.set_focus();
    }
    // Last, and only once the widget is actually up: a failed build must not leave the app with
    // no window at all, reachable only from the tray.
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.hide();
        crate::tray::set_main_visible(app, false);
    }
    Ok(())
}

/// Remember where the widget currently sits. No-op when it isn't up. Its own function because
/// quitting from the tray is a way down that never reaches [`close`].
pub fn save_position(app: &AppHandle) {
    let Some(w) = app.get_webview_window(LABEL) else { return };
    if let (Ok(p), Some(state)) = (w.outer_position(), app.try_state::<Arc<AppState>>()) {
        state.db.set_setting(POS_KEY, &format!("{},{}", p.x, p.y));
    }
}

/// Take the widget down, remembering where it ended up. Callable from any thread; bringing the
/// main window back is [`crate::tray::show_main`]'s job, which is what calls this.
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

/// Where to put it: the last position if that spot still exists (a display can be unplugged
/// between sessions), otherwise the bottom-right of whichever display the app is on.
fn placement(app: &AppHandle, win: &WebviewWindow) -> Option<PhysicalPosition<i32>> {
    app.try_state::<Arc<AppState>>()
        .and_then(|s| s.db.get_setting(POS_KEY))
        .and_then(|v| parse_pos(&v))
        .filter(|p| on_a_display(win, *p))
        .or_else(|| bottom_right(app, win))
}

/// `"x,y"` in physical pixels, as [`close`] wrote it.
fn parse_pos(s: &str) -> Option<PhysicalPosition<i32>> {
    let (x, y) = s.split_once(',')?;
    Some(PhysicalPosition::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Is that point on a display that is currently connected? Checked on the top-left corner, which
/// is what `set_position` sets and what the WM keeps reachable.
fn on_a_display(win: &WebviewWindow, p: PhysicalPosition<i32>) -> bool {
    win.available_monitors()
        .is_ok_and(|monitors| monitors.iter().any(|m| contains(*m.position(), *m.size(), p)))
}

/// Bottom-right of the display the main window is on, inside its *work area* so a taskbar or dock
/// doesn't end up sitting on top of the widget.
fn bottom_right(app: &AppHandle, win: &WebviewWindow) -> Option<PhysicalPosition<i32>> {
    let anchor = app.get_webview_window("main").unwrap_or_else(|| win.clone());
    let m = anchor
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| anchor.primary_monitor().ok().flatten())?;
    let area = m.work_area();
    let px = |logical: f64| (logical * m.scale_factor()).round() as i32;
    Some(PhysicalPosition::new(
        area.position.x + area.size.width as i32 - px(W + MARGIN),
        area.position.y + area.size.height as i32 - px(H + MARGIN),
    ))
}

/// Point-in-rect, physical pixels. Its own function because a second monitor placed left of or
/// above the primary has a negative origin, which is exactly the case that goes wrong.
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
    use super::{contains, parse_pos};
    use tauri::{PhysicalPosition, PhysicalSize};

    #[test]
    fn parses_what_close_wrote() {
        assert_eq!(parse_pos("120,64"), Some(PhysicalPosition::new(120, 64)));
        // A monitor left of the primary gives negative coordinates.
        assert_eq!(parse_pos("-1800,-200"), Some(PhysicalPosition::new(-1800, -200)));
        assert_eq!(parse_pos("garbage"), None);
        assert_eq!(parse_pos("12,"), None);
    }

    #[test]
    fn point_lands_on_the_right_display() {
        let primary = (PhysicalPosition::new(0, 0), PhysicalSize::new(1920u32, 1080));
        // Second monitor to the *left* of the primary: origin is negative.
        let left = (PhysicalPosition::new(-1920, 0), PhysicalSize::new(1920u32, 1080));

        assert!(contains(primary.0, primary.1, PhysicalPosition::new(1300, 800)));
        assert!(!contains(primary.0, primary.1, PhysicalPosition::new(-500, 800)));
        assert!(contains(left.0, left.1, PhysicalPosition::new(-500, 800)));
        // Right/bottom edges are exclusive — that pixel belongs to the next display.
        assert!(!contains(primary.0, primary.1, PhysicalPosition::new(1920, 500)));
        assert!(!contains(primary.0, primary.1, PhysicalPosition::new(500, 1080)));
    }
}
