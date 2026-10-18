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
    custom.or_else(|| bundled(app))
}

/// The bundled icon, at a resolution worth scaling down from.
///
/// `default_window_icon` is fine everywhere but Windows, where tauri's codegen builds it from the
/// *first* entry of `icons/icon.ico` rather than the biggest, and that entry is 32x32. Every
/// Windows install was therefore feeding a 32px source to a notification area that draws at 16 to
/// 32px depending on DPI, which is the ragged tray icon in #225. The 256px PNG is the same artwork.
fn bundled(app: &AppHandle) -> Option<Image<'static>> {
    #[cfg(target_os = "windows")]
    {
        match Image::from_bytes(include_bytes!("../icons/128x128@2x.png")) {
            Ok(img) => return Some(img),
            Err(e) => tracing::warn!(error = %e, "bundled 256px icon unreadable"),
        }
    }
    app.default_window_icon().map(|i| i.clone().to_owned())
}

/// Resample to `w` x `h` by averaging each destination pixel's source rectangle.
///
/// Windows never does this for us: `tray-icon` and tao both build the HICON at whatever size they
/// are handed and leave the shell to stretch it, so a 1024x1024 custom icon reached a 16px tray
/// slot as an unfiltered point sample. Every caller here scales down from a larger source, which
/// is exactly what a box filter is for.
///
/// It lives here rather than beside its Windows callers so it can be tested on Linux, which is
/// also the only reason it is compiled there.
#[cfg(any(target_os = "windows", test))]
pub fn scaled(img: &Image<'_>, w: u32, h: u32) -> Image<'static> {
    let (sw, sh) = (img.width(), img.height());
    if (sw, sh) == (w, h) || sw == 0 || sh == 0 || w == 0 || h == 0 {
        return Image::new_owned(img.rgba().to_vec(), sw, sh);
    }
    let src = img.rgba();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let y0 = y * sh / h;
        let y1 = ((y + 1) * sh / h).max(y0 + 1);
        for x in 0..w {
            let x0 = x * sw / w;
            let x1 = ((x + 1) * sw / w).max(x0 + 1);
            // Colour is weighted by alpha, so the transparent pixels around a glyph cannot drag
            // its edge towards whatever colour they happen to carry.
            let (mut rgb, mut alpha) = ([0u32; 3], 0u32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = ((sy * sw + sx) * 4) as usize;
                    let a = src[p + 3] as u32;
                    for (acc, c) in rgb.iter_mut().zip(&src[p..p + 3]) {
                        *acc += *c as u32 * a;
                    }
                    alpha += a;
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            out.extend(rgb.map(|c| if alpha == 0 { 0 } else { (c / alpha) as u8 }));
            out.push((alpha / n) as u8);
        }
    }
    Image::new_owned(out, w, h)
}

/// Repaint every runtime surface. Called at startup and whenever the icon changes.
pub fn apply(app: &AppHandle) {
    let Some(icon) = current(app) else { return };
    if let Some(w) = app.get_webview_window("main") {
        // Alt-Tab and the small titlebar icon. Tauri routes this to tao's `set_window_icon`,
        // which sends WM_SETICON with ICON_SMALL and nothing else, at the source's own size.
        //
        // Windows takes the other path instead of both. `apply` runs off the main thread, so this
        // call is a *queued* window message while `set_icons` is a synchronous `SendMessageW`:
        // whichever lands last wins, and leaving both in would put the unscaled icon back at
        // random. `set_icons` covers ICON_SMALL as well, at the size the shell draws.
        #[cfg(not(any(target_os = "windows", target_os = "android")))]
        let _ = w.set_icon(icon.clone());
        #[cfg(target_os = "windows")]
        crate::taskbar::set_icons(&w, &icon);
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
    use tauri::image::Image;

    /// The tray and taskbar hand the result straight to `CreateIconIndirect`, which takes the
    /// buffer's length on trust, so a short one is a crash on Windows and nowhere else.
    #[test]
    fn scaled_halves_a_solid_square() {
        let red = Image::new_owned([255u8, 0, 0, 255].repeat(16), 4, 4);
        let out = super::scaled(&red, 2, 2);
        assert_eq!((out.width(), out.height()), (2, 2));
        assert_eq!(out.rgba(), [255u8, 0, 0, 255].repeat(4));
    }

    /// Averaging straight RGBA would pull the opaque pixel towards the transparent one's colour.
    #[test]
    fn scaled_ignores_the_colour_of_transparent_pixels() {
        // One opaque red pixel, three fully transparent green ones.
        let mut rgba = vec![255u8, 0, 0, 255];
        rgba.extend([0u8, 255, 0, 0].repeat(3));
        let out = super::scaled(&Image::new_owned(rgba, 2, 2), 1, 1);
        assert_eq!(out.rgba(), &[255, 0, 0, 63]);
    }

    /// A source smaller than the target still has to produce exactly w*h*4 bytes.
    #[test]
    fn scaled_upwards_is_still_the_right_length() {
        let out = super::scaled(&Image::new_owned(vec![1, 2, 3, 4], 1, 1), 5, 3);
        assert_eq!(out.rgba().len(), 5 * 3 * 4);
        assert_eq!(out.rgba()[..4], [1, 2, 3, 4]);
    }

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
