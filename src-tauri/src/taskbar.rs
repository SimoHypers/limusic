//! Windows taskbar thumbnail toolbar: prev / play-pause / next under the taskbar preview (#47).
//!
//! Separate from the SMTC session `media.rs` drives. SMTC feeds the media OSD, the volume flyout
//! and the lock screen; the buttons on the taskbar hover preview come from `ITaskbarList3`
//! (`ThumbBarAddButtons`) and nothing else. Clicks route back through [`crate::media::handle_event`]
//! so the OS controls and this toolbar share one code path.
//!
//! Everything here runs on the main thread: `ITaskbarList3` is apartment-bound and the toolbar's
//! clicks arrive as `WM_COMMAND` on the window we subclass.

#![cfg(target_os = "windows")]

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use souvlaki::MediaControlEvent;
use tauri::{AppHandle, Manager};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THBF_ENABLED, THBN_CLICKED,
    THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, IsWindowVisible, RegisterWindowMessageW,
    SendMessageW, HICON, ICONINFO, ICON_BIG, SM_CXSMICON, SM_CYSMICON, WM_COMMAND, WM_SETICON,
};

const ID_PREV: u32 = 1;
const ID_PLAY_PAUSE: u32 = 2;
const ID_NEXT: u32 = 3;
const SUBCLASS_ID: usize = 0x11_4d_05_1c;

/// The `ICON_BIG` handle we last handed the window, so the next one can free it.
static PREV_BIG: AtomicIsize = AtomicIsize::new(0);

/// `RegisterWindowMessageW("TaskbarButtonCreated")`. The shell sends it once the taskbar button
/// exists; `ThumbBarAddButtons` before that silently does nothing, and it comes again after an
/// Explorer restart or whenever the window is hidden and shown, which is why the attach also lives
/// in the subclass proc.
static BUTTON_CREATED: AtomicU32 = AtomicU32::new(0);

struct Bar {
    app: AppHandle,
    hwnd: HWND,
    /// prev, play, pause, next.
    icons: [HICON; 4],
    list: Option<ITaskbarList3>,
    playing: bool,
}

thread_local! {
    static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
}

/// Subclass the main window and get ready to add the buttons. Call from `setup`, on the main
/// thread. A failure here just means no toolbar.
pub fn init(app: &AppHandle) {
    let Some(hwnd) = app.get_webview_window("main").and_then(|w| w.hwnd().ok()) else {
        tracing::warn!("taskbar toolbar: no main window HWND");
        return;
    };
    let icons = match make_icons() {
        Some(i) => i,
        None => {
            tracing::warn!("taskbar toolbar: could not build the button icons");
            return;
        }
    };
    BUTTON_CREATED
        .store(unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) }, Ordering::Relaxed);
    BAR.with(|b| {
        *b.borrow_mut() = Some(Bar { app: app.clone(), hwnd, icons, list: None, playing: false })
    });
    unsafe {
        // The window outlives us, so the subclass is never removed.
        if !SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0).as_bool() {
            tracing::warn!("taskbar toolbar: SetWindowSubclass failed");
        }
    }
    // If the window is already on screen, its taskbar button exists and the shell has sent the
    // one `TaskbarButtonCreated` it will ever send for it, most likely while WebView2's init was
    // pumping messages during window creation, long before the subclass above existed. That is
    // what left the toolbar missing until the window was hidden to the tray and shown again
    // (#47), so add to the button now rather than waiting for a message that has been and gone.
    // A window still hidden here gets its button (and the message, which now lands) on first show,
    // and must not be touched: `ThumbBarAddButtons` works once per window and there is nothing
    // yet to add to.
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        attach();
    }
}

/// Swap the middle button between play and pause. Safe to call from any thread.
pub fn set_playing(app: &AppHandle, playing: bool) {
    let _ = app.run_on_main_thread(move || {
        // Nothing to redraw until the shell has given us the taskbar button.
        let Some((list, hwnd, icons)) = BAR.with(|b| {
            let mut bar = b.borrow_mut();
            let bar = bar.as_mut()?;
            if bar.playing == playing {
                return None;
            }
            bar.playing = playing;
            Some((bar.list.clone()?, bar.hwnd, bar.icons))
        }) else {
            return;
        };
        let _ = unsafe { list.ThumbBarUpdateButtons(hwnd, &buttons(&icons, playing)) };
    });
}

/// Create the toolbar. Runs once per `TaskbarButtonCreated`, so an Explorer restart re-adds it.
fn attach() {
    let Some((hwnd, icons, playing)) =
        BAR.with(|b| b.borrow().as_ref().map(|x| (x.hwnd, x.icons, x.playing)))
    else {
        return;
    };
    // COM is already up on this thread (wry needs it for WebView2); this only covers the case
    // where it isn't, and a duplicate init on the same apartment is a no-op.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok().ok();
    let list: ITaskbarList3 = match unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_ALL) } {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "taskbar toolbar: CoCreateInstance(TaskbarList) failed");
            return;
        }
    };
    unsafe {
        if let Err(e) = list.HrInit() {
            tracing::warn!(error = %e, "taskbar toolbar: HrInit failed");
            return;
        }
        // One add per window per Explorer session: whichever of `init` and `TaskbarButtonCreated`
        // gets there second is left with the update call. After an Explorer restart the shell has
        // forgotten the window and the add is the one that takes.
        let b = buttons(&icons, playing);
        if let Err(add) = list.ThumbBarAddButtons(hwnd, &b) {
            if let Err(update) = list.ThumbBarUpdateButtons(hwnd, &b) {
                tracing::warn!(%add, %update, "taskbar toolbar: no buttons added");
                return;
            }
        }
    }
    // Held so ThumbBarUpdateButtons can reuse it; the borrow is taken only after the COM calls,
    // which can pump messages back into the subclass proc.
    BAR.with(|b| {
        if let Some(bar) = b.borrow_mut().as_mut() {
            bar.list = Some(list);
        }
    });
    tracing::info!("taskbar thumbnail toolbar attached");
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    umsg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let created = BUTTON_CREATED.load(Ordering::Relaxed);
    if created != 0 && umsg == created {
        attach();
    } else if umsg == WM_COMMAND && (wparam.0 >> 16) as u32 & 0xffff == THBN_CLICKED {
        let event = match (wparam.0 & 0xffff) as u32 {
            ID_PREV => Some(MediaControlEvent::Previous),
            ID_PLAY_PAUSE => Some(MediaControlEvent::Toggle),
            ID_NEXT => Some(MediaControlEvent::Next),
            _ => None,
        };
        if let Some(event) = event {
            // `handle_event` only spawns onto the Tauri runtime, so this can't re-enter the borrow.
            BAR.with(|b| {
                if let Some(bar) = b.borrow().as_ref() {
                    crate::media::handle_event(&bar.app, event);
                }
            });
            return LRESULT(0);
        }
    }
    unsafe { DefSubclassProc(hwnd, umsg, wparam, lparam) }
}

fn buttons(icons: &[HICON; 4], playing: bool) -> [THUMBBUTTON; 3] {
    let (mid, tip) = if playing { (icons[2], "Pause") } else { (icons[1], "Play") };
    [
        button(ID_PREV, icons[0], "Previous"),
        button(ID_PLAY_PAUSE, mid, tip),
        button(ID_NEXT, icons[3], "Next"),
    ]
}

fn button(id: u32, icon: HICON, tip: &str) -> THUMBBUTTON {
    let mut b = THUMBBUTTON {
        dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
        iId: id,
        hIcon: icon,
        dwFlags: THBF_ENABLED,
        ..Default::default()
    };
    for (slot, c) in b.szTip.iter_mut().zip(tip.encode_utf16()) {
        *slot = c;
    }
    b
}

// --- icons -------------------------------------------------------------------------------------
//
// ponytail: drawn at startup instead of shipped as .ico resources. Four flat glyphs at ~16px is
// less code than a bundled asset plus the loader, and it scales with SM_CXSMICON for free. Swap in
// real assets if the design ever wants more than a white shape.

/// Is the normalized point inside the glyph? `x`/`y` run 0..1 over the icon box.
type Glyph = fn(f32, f32) -> bool;

fn play_glyph(x: f32, y: f32) -> bool {
    (0.28..=0.76).contains(&x) && (y - 0.5).abs() <= 0.32 * (0.76 - x) / 0.48
}

fn pause_glyph(x: f32, y: f32) -> bool {
    (0.20..=0.80).contains(&y) && ((0.28..=0.43).contains(&x) || (0.57..=0.72).contains(&x))
}

fn next_glyph(x: f32, y: f32) -> bool {
    ((0.20..=0.62).contains(&x) && (y - 0.5).abs() <= 0.32 * (0.62 - x) / 0.42)
        || ((0.68..=0.80).contains(&x) && (0.20..=0.80).contains(&y))
}

fn prev_glyph(x: f32, y: f32) -> bool {
    next_glyph(1.0 - x, y)
}

fn make_icons() -> Option<[HICON; 4]> {
    let w = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let h = unsafe { GetSystemMetrics(SM_CYSMICON) }.max(16);
    let g: [Glyph; 4] = [prev_glyph, play_glyph, pause_glyph, next_glyph];
    let mut out = [HICON::default(); 4];
    for (slot, glyph) in out.iter_mut().zip(g) {
        *slot = make_icon(w, h, glyph)?;
    }
    Some(out)
}

/// One white glyph on transparent, as a 32bpp alpha icon. Edges are 4x4 supersampled, so the
/// triangles don't come out as staircases.
fn make_icon(w: i32, h: i32, glyph: Glyph) -> Option<HICON> {
    const SS: i32 = 4;
    let mut pixels = vec![0u32; (w * h) as usize];
    for py in 0..h {
        for px in 0..w {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / w as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / h as f32;
                    hits += glyph(x, y) as i32;
                }
            }
            // Premultiplied white: every channel equals the coverage.
            let a = (hits * 255 / (SS * SS)) as u32;
            pixels[(py * w + px) as usize] = a << 24 | a << 16 | a << 8 | a;
        }
    }

    icon_from_bgra(&pixels, w, h)
}

/// Premultiplied BGRA (`0xAARRGGBB` per `u32`, little-endian, so B,G,R,A in memory) to an HICON.
fn icon_from_bgra(pixels: &[u32], w: i32, h: i32) -> Option<HICON> {
    if pixels.len() != (w as usize) * (h as usize) {
        return None;
    }
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    unsafe {
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());
        // The alpha channel does the masking; CreateIconIndirect still wants a mask bitmap, and a
        // zeroed one leaves it alone. 1bpp scanlines are WORD aligned.
        let mask_bytes = vec![0u8; (((w + 15) / 16) * 2 * h) as usize];
        let mask = CreateBitmap(w, h, 1, 1, Some(mask_bytes.as_ptr().cast()));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        // CreateIconIndirect copies both bitmaps.
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }
}

/// Push a custom app icon at the taskbar button (#173).
///
/// This exists because `WebviewWindow::set_icon` cannot reach it: Tauri routes that to tao's
/// `set_window_icon`, which only ever sends `WM_SETICON`/`ICON_SMALL` (Alt-Tab and the small
/// titlebar icon). `ICON_BIG` is what the taskbar reads, tao leaves it unset, and its window class
/// registers a null `hIcon`, so Windows falls back to the icon compiled into the .exe. Nothing but
/// this message changes the button.
pub fn set_big_icon(window: &tauri::WebviewWindow, img: &tauri::image::Image<'_>) {
    let Ok(hwnd) = window.hwnd() else { return };
    let (w, h) = (img.width() as i32, img.height() as i32);
    let pixels = crate::appicon::premultiplied_bgra(img.rgba());
    let Some(icon) = icon_from_bgra(&pixels, w, h) else { return };
    unsafe {
        SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as _)), Some(LPARAM(icon.0 as _)));
    }
    // The window owns the handle until it is replaced. Freeing the one it just let go keeps a user
    // who tries five icons in a row from leaking five of them.
    let old = PREV_BIG.swap(icon.0 as isize, Ordering::Relaxed);
    if old != 0 {
        unsafe {
            let _ = DestroyIcon(HICON(old as _));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flipped inequality renders a blank button, which on Windows reads as the whole toolbar
    /// being broken. Cheap sanity bound on how much of the box each glyph covers.
    #[test]
    fn glyphs_have_ink() {
        let glyphs: [(&str, Glyph); 4] = [
            ("prev", prev_glyph),
            ("play", play_glyph),
            ("pause", pause_glyph),
            ("next", next_glyph),
        ];
        for (name, glyph) in glyphs {
            let ink = (0..16)
                .flat_map(|py| (0..16).map(move |px| (px, py)))
                .filter(|&(px, py)| glyph((px as f32 + 0.5) / 16.0, (py as f32 + 0.5) / 16.0))
                .count();
            assert!((20..200).contains(&ink), "{name}: {ink} lit pixels of 256");
        }
    }
}
