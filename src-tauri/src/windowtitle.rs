//! The main window's title follows the current track, so the OS window list names the song: the
//! Windows taskbar hover preview prints the window title above the thumbnail, and Alt-Tab and a
//! Linux taskbar read the same string.
//!
//! Only "main". The mini player is `skip_taskbar` and keeps its creation title because compositor
//! window rules match on it (#362, mini.rs).

use innertube::SongItem;
use tauri::{AppHandle, Manager};

/// `None` means nothing is current, so the title is the app's own name.
fn title(item: Option<&SongItem>) -> String {
    let Some(item) = item else { return "Limusic".to_owned() };
    let artists = item.artists.trim();
    if artists.is_empty() {
        item.title.clone()
    } else {
        format!("{} — {artists}", item.title)
    }
}

pub fn apply(app: &AppHandle, item: Option<&SongItem>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_title(&title(item));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str, artists: &str) -> SongItem {
        SongItem { title: title.into(), artists: artists.into(), ..Default::default() }
    }

    #[test]
    fn names_the_track_and_its_artists() {
        assert_eq!(title(Some(&song("Blue Monday", "New Order"))), "Blue Monday — New Order");
    }

    #[test]
    fn no_artists_leaves_the_track_alone() {
        assert_eq!(title(Some(&song("Blue Monday", ""))), "Blue Monday");
    }

    #[test]
    fn blank_artists_count_as_none() {
        assert_eq!(title(Some(&song("Blue Monday", "   "))), "Blue Monday");
    }

    #[test]
    fn nothing_current_is_the_app_name() {
        assert_eq!(title(None), "Limusic");
    }
}
