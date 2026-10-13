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

/// Logical sizes of the widget, full and compact (#301). Not user-resizable: it's a pill, not a
/// window you arrange, and its own shrink/expand button is the only way between the two.
const FULL: (f64, f64) = (560.0, 180.0);
const COMPACT: (f64, f64) = (320.0, 88.0);
/// `"true"` while the user wants it compact. Sticky: the widget reopens at the size it was left.
const COMPACT_KEY: &str = "mini_compact";
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
        let (w, h) = size(app);
        let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
            // Distinct from the main window's "Limusic" so compositor window rules (niri, KDE) can
            // match it, and set here because they only read the title the window is created with
            // (#362). Deliberately untranslated: a localised title breaks the rule on a language
            // switch. The app_id can't differ instead, GTK sets that once per process.
            .title("Limusic Mini Player")
            .inner_size(w, h)
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

        // WebKitGTK's view keeps a 200px minimum height and GTK sizes the window to fit it, so on
        // Linux the 180px widget came out 200 and the compact one couldn't shrink at all. Without
        // the view's own size request, the window's size is the only one left. Synchronous here:
        // this runs on the main thread, so it lands before the window is first shown.
        #[cfg(target_os = "linux")]
        let _ = win.with_webview(|wv| {
            use gtk::prelude::WidgetExt;
            wv.inner().set_size_request(1, 1);
        });
        if let Some(p) = placement(app, &win, (w, h)) {
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

/// Switch the widget between its full and compact size, and remember the choice. The widget's
/// layout follows its window's width, so the resize is the whole switch: there is no second copy
/// of the mode in the webview to disagree with this one.
pub fn set_compact(app: &AppHandle, compact: bool) -> Result<(), String> {
    let win = app.get_webview_window(LABEL).ok_or("the mini player isn't open")?;
    let (w, h) = if compact { COMPACT } else { FULL };
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    let new = PhysicalSize::new((w * scale).round() as u32, (h * scale).round() as u32);
    // Read before the resize: afterwards the old size may already be gone.
    let to = match (win.outer_position(), win.outer_size(), win.current_monitor()) {
        (Ok(pos), Ok(old), Ok(Some(m))) => Some(anchored(pos, old, new, *m.position(), *m.size())),
        _ => None,
    };
    win.set_size(new).map_err(|e| e.to_string())?;
    // A no-op on Wayland, where the compositor keeps the top-left corner where it was.
    if let Some(p) = to {
        let _ = win.set_position(p);
    }
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        state.db.set_setting(COMPACT_KEY, if compact { "true" } else { "false" });
    }
    Ok(())
}

/// The size to open at: whichever the user left it at.
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

/// Top-left for a widget going from `old` to `new` size, so it stays against the screen edges it
/// is parked near: shrunk in the bottom-right corner, it stays in that corner instead of leaving a
/// gap below and to the right. Decided per axis by which half of the display its centre is in, so
/// shrinking and growing back always returns it to the same spot.
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

/// `"x,y"` in physical pixels, as [`close`] wrote it.
fn parse_pos(s: &str) -> Option<PhysicalPosition<i32>> {
    let (x, y) = s.split_once(',')?;
    Some(PhysicalPosition::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Is a window at `p`, this size, on a display that is currently connected? Its middle has to be.
/// A corner is not enough: one can sit on a display with only the invisible resize border showing,
/// which is how the main window came back off-screen after a monitor layout change (#459).
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

/// Bottom-right of the display the main window is on, inside its *work area* so a taskbar or dock
/// doesn't end up sitting on top of the widget.
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
    use super::{anchored, contains, middle, parse_pos};
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

    #[test]
    fn a_window_counts_by_its_middle() {
        // #459: the window was saved maximized on a laptop display that sat right of monitor A,
        // then restored with A as the only display at that height.
        let a = (PhysicalPosition::new(0, 0), PhysicalSize::new(1920u32, 1080));
        let (at, size) = (PhysicalPosition::new(1911, -9), PhysicalSize::new(1900u32, 1008));
        // Its bottom-left corner is inside A, which is all the window-state plugin checks...
        assert!(contains(a.0, a.1, PhysicalPosition::new(at.x, at.y + size.height as i32)));
        // ...but its middle is on nothing.
        assert!(!contains(a.0, a.1, middle(at, size)));
        // Maximized on A, the frame overhangs every edge by its border and still counts.
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

        // Bottom-right (the default spot): right and bottom edges stay put.
        assert_eq!(shrink(at(1336, 876), at(0, 0), fhd), at(1576, 968));
        // Top-left: nothing moves.
        assert_eq!(shrink(at(24, 24), at(0, 0), fhd), at(24, 24));
        // Top-right: only the right edge is held.
        assert_eq!(shrink(at(1336, 24), at(0, 0), fhd), at(1576, 24));
        // Same corner on a display left of the primary, where the origin is negative.
        assert_eq!(shrink(at(-584, 876), at(-1920, 0), fhd), at(-344, 968));

        // Shrinking and growing back is a round trip, wherever it sits.
        for (p, origin) in
            [(at(1336, 876), at(0, 0)), (at(24, 24), at(0, 0)), (at(900, 300), at(0, 0))]
        {
            assert_eq!(grow(shrink(p, origin, fhd), origin, fhd), p);
        }
    }
}
