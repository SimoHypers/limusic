//! "Now playing" notifications on a track change (issue #94), behind the `track_notifications`
//! setting. Off by default: Linux desktops already show the MPRIS widget, and a notification per
//! song is a preference, not something everyone wants.
//!
//! Only while no Limusic window has focus, unless the song started on its own and the user opted
//! into `track_notifications_focused`: a song they just picked in the app needs no toast, but one
//! that rolls over while Limusic sits in front may go unseen. Best-effort like `media.rs`: a
//! missing notification daemon is a `debug!` line.

#[cfg(all(unix, not(target_os = "macos")))]
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// The last song's notification, closed before the next one is shown so a run of skips leaves one
/// up, not a stack. Not `replaces_id`: Plasma updates an expired notification in place in its
/// history and never pops it up again, so only the first skip was visible.
#[cfg(all(unix, not(target_os = "macos")))]
static PREV: Mutex<Option<notify_rust::NotificationHandle>> = Mutex::new(None);

pub fn track_changed(app: &AppHandle, title: &str, artists: &str, even_focused: bool) {
    if !even_focused && app.webview_windows().values().any(|w| w.is_focused().unwrap_or(false)) {
        tracing::debug!("notification skipped: a Limusic window has focus");
        return;
    }
    let mut n = notify_rust::Notification::new();
    n.summary(title).body(artists).appname("Limusic");
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // The binary name is the icon name the .deb/.rpm install. An AppImage has no themed icon,
        // so there the daemon shows its generic one.
        n.auto_icon().hint(notify_rust::Hint::SuppressSound(true));
    }
    // A toast is only delivered for an AppUserModelID that a Start menu shortcut registers, and the
    // NSIS installer registers the bundle identifier. A dev build has no shortcut, so it keeps
    // notify-rust's default (PowerShell's).
    #[cfg(target_os = "windows")]
    if !tauri::is_dev() {
        n.app_id(&app.config().identifier);
    }
    // Which app macOS files the notification under. Errors after the first call (it is set once per
    // process), which is fine. A dev binary has no bundle, so it posts as Terminal.
    #[cfg(target_os = "macos")]
    let _ = notify_rust::set_application(if tauri::is_dev() {
        "com.apple.Terminal"
    } else {
        &app.config().identifier
    });
    #[cfg(target_os = "macos")]
    present_while_frontmost();
    // A blocking D-Bus / WinRT / AppKit call: keep it off the playback path.
    std::thread::spawn(move || {
        // Held across close + show, so two quick skips can't both close the same one.
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut prev = PREV.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(h) = prev.take() {
            h.close();
        }
        tracing::debug!("notification sent");
        match n.show() {
            #[cfg(all(unix, not(target_os = "macos")))]
            Ok(h) => *prev = Some(h),
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            Ok(_) => {}
            Err(e) => tracing::debug!("notification: {e}"),
        }
    });
}

/// AppKit hides a notification posted by the frontmost app unless the center's delegate answers
/// `shouldPresentNotification:` with YES, and mac-notification-sys's delegate doesn't implement it
/// (still true in 0.6.15). Add it to their class, so `track_changed` alone decides when a song
/// notifies. Untested on a real Mac (#94).
#[cfg(target_os = "macos")]
fn present_while_frontmost() {
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{sel, Encode};

    extern "C-unwind" fn yes(
        _: *mut AnyObject,
        _: Sel,
        _: *mut AnyObject,
        _: *mut AnyObject,
    ) -> Bool {
        Bool::YES
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Some(cls) = AnyClass::get(c"NotificationCenterDelegate") else {
            tracing::debug!("notification: mac-notification-sys delegate class not found");
            return;
        };
        let types = std::ffi::CString::new(format!("{}@:@@", Bool::ENCODING)).unwrap();
        type Present =
            extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> Bool;
        // SAFETY: the class exists and lacks this selector, `yes` matches the encoding in `types`,
        // and `Imp` is the type-erased pointer the runtime calls back through.
        unsafe {
            objc2::ffi::class_addMethod(
                cls as *const AnyClass as *mut AnyClass,
                sel!(userNotificationCenter:shouldPresentNotification:),
                std::mem::transmute::<Present, Imp>(yes),
                types.as_ptr(),
            );
        }
    });
}
