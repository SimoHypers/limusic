//! Custom app icon (#173): the icon the user picked, pushed at every surface that can take one
//! while the app is running.
//!
//! Three of the four Windows icon surfaces are reachable from here. The fourth is not: the icon
//! Explorer, the Start menu and a pinned shortcut show comes from a resource compiled into the
//! .exe, and a running process cannot rewrite its own mapped binary (an update would replace it
//! anyway). That one stays whatever was bundled.
//!
//! There is no settings row behind this. The file's existence *is* the setting, which is also why
//! the picked image is copied here rather than referenced in place: deleting or moving the
//! original would otherwise leave the app iconless at the next launch.

use std::path::PathBuf;

use tauri::image::Image;
use tauri::{AppHandle, Manager};

/// Where the copy lives. `None` only if the platform has no app data dir, in which case the
/// feature is simply off.
pub fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("app-icon.png"))
}

/// The picked icon, if there is one. Its existence on disk is the whole setting.
pub fn custom_path(app: &AppHandle) -> Option<PathBuf> {
    path(app).filter(|p| p.is_file())
}

/// The icon the app should be wearing: the user's, or the bundled one.
pub fn current(app: &AppHandle) -> Option<Image<'static>> {
    let custom = custom_path(app).and_then(|p| match Image::from_path(&p) {
        Ok(img) => Some(img),
        Err(e) => {
            tracing::warn!(error = %e, path = %p.display(), "custom app icon unreadable");
            None
        }
    });
    custom.or_else(|| app.default_window_icon().map(|i| i.clone().to_owned()))
}

/// Repaint every runtime surface. Called at startup and whenever the icon changes.
pub fn apply(app: &AppHandle) {
    let Some(icon) = current(app) else { return };
    if let Some(w) = app.get_webview_window("main") {
        // Alt-Tab and the small titlebar icon. Tauri routes this to tao's `set_window_icon`,
        // which sends WM_SETICON with ICON_SMALL and nothing else. The taskbar button reads
        // ICON_BIG, so on Windows this call alone changes nothing the user is looking at.
        let _ = w.set_icon(icon.clone());
        #[cfg(target_os = "windows")]
        crate::taskbar::set_big_icon(&w, &icon);
    }
    // The mini player is `skip_taskbar(true)` and undecorated, so its icon is never drawn.
    crate::tray::set_icon(app, &icon);
}

/// Straight-alpha RGBA to premultiplied BGRA, one `u32` per pixel (`0xAARRGGBB`, little-endian, so
/// B,G,R,A in memory). That is the layout `CreateIconIndirect`'s colour bitmap is blended as.
///
/// It lives here rather than in `taskbar.rs` so it can be tested on the machine this is written on:
/// getting the channel order wrong shows up as a blue icon on Windows and nowhere else.
#[cfg(any(target_os = "windows", test))]
pub fn premultiplied_bgra(rgba: &[u8]) -> Vec<u32> {
    rgba.chunks_exact(4)
        .map(|p| {
            let a = p[3] as u32;
            let pm = |c: u8| c as u32 * a / 255;
            a << 24 | pm(p[0]) << 16 | pm(p[1]) << 8 | pm(p[2])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn rgba_to_premultiplied_bgra() {
        // Opaque pure red stays red in the R byte, not the B byte.
        assert_eq!(super::premultiplied_bgra(&[255, 0, 0, 255]), vec![0xffff_0000]);
        assert_eq!(super::premultiplied_bgra(&[0, 0, 255, 255]), vec![0xff00_00ff]);
        // Half-transparent white: every channel scales with the alpha.
        assert_eq!(super::premultiplied_bgra(&[255, 255, 255, 128]), vec![0x8080_8080]);
        // Fully transparent pixels carry no colour at all.
        assert_eq!(super::premultiplied_bgra(&[255, 255, 255, 0]), vec![0]);
    }
}
