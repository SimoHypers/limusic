//! Multi-platform audio player wrapper. context/14.
//!
//! YouTube-agnostic: takes a fully-resolved URL + headers, never a videoId.
//! On Desktop (Linux/Windows/macOS), uses libmpv (`desktop.rs`).
//! On Android, delegates to the Android media playback engine (`android.rs`).

#[cfg(not(target_os = "android"))]
pub(crate) mod video;
#[cfg(not(target_os = "android"))]
pub use video::{GlDisplay, Thumbnail, VideoRenderer};

#[cfg(target_os = "android")]
pub enum GlDisplay {}

#[cfg(target_os = "android")]
#[derive(Debug, Clone)]
pub struct Thumbnail {
    pub pts: f64,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[cfg(target_os = "android")]
pub struct VideoRenderer;

#[cfg(target_os = "android")]
impl VideoRenderer {
    pub fn render(&mut self, _fbo: i32, _width: i32, _height: i32) -> Result<(), Error> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(not(target_os = "android"))]
    #[error("mpv: {0}")]
    Mpv(#[from] libmpv2::Error),
    #[cfg(target_os = "android")]
    #[error("android player error: {0}")]
    Android(String),
    /// mpv refused a chain carrying the pitch filter, which means this libmpv was built without
    /// librubberband. Its own answer is `Raw(-9)`, so it needs saying in words: this one reaches
    /// the user as a toast.
    #[error("Pitch shifting isn't available in this build")]
    NoPitchFilter,
}

/// Events pumped from player's event thread. context/14 §player surface.
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Position(f64),
    Duration(f64),
    /// Playback started or stopped, emitted only on a real change.
    ///
    /// Derived from mpv's `pause` **and** `idle-active`, because `pause` alone is a trap: it starts
    /// out `false` and a `loadfile` doesn't touch it, so starting a track sets `false` → `false`
    /// and fires **no** property event at all. `idle-active` is the one that actually flips when a
    /// file starts (and when the playlist runs dry). Anything reading playback state off `pause`
    /// alone never hears that a track began, and only recovers on a manual pause/unpause.
    Playing(bool),
    /// mpv's `pause` flag changed: someone pressed pause or play. Unlike [`PlayerEvent::Playing`]
    /// this says nothing when the playlist runs dry or a new file starts, which is what a Listen
    /// Together host has to broadcast: a track change is announced on its own, and a "stopped"
    /// from the gap between two tracks reaches the room after the next one has started.
    Paused(bool),
    /// One track finished normally (EOF) — orchestrator advances the queue.
    TrackEnded,
    /// One track died (end-file with error, e.g. its URL 403'd). mpv may have auto-advanced
    /// into the next playlist entry or gone idle — the orchestrator asks [`Player::is_idle`].
    TrackFailed(String),
    /// mpv could not open the audio device, so the file ended with an error that has nothing to do
    /// with the stream: the device went away under it (sleep/resume, unplugged, a driver restart).
    /// Kept apart from [`PlayerEvent::TrackFailed`] because the two want opposite handling, see
    /// `AppState::on_audio_device_lost`.
    AudioDeviceLost,
    /// The crossfade preload deck failed to open the *next* track (dead or 403 URL). Kept apart
    /// from [`PlayerEvent::TrackFailed`] because reporting it as that would skip the track the
    /// user is hearing; what is owed is evicting the next track's cached URL, see
    /// `AppState::on_lookahead_failed`.
    LookaheadFailed(String),
    /// mpv could not open the music video for the audio file it names (exactly as mpv was handed
    /// it). The sound is unaffected; the app decides whether another stream for the picture is
    /// worth trying. The open is a network round trip, so a skip or a crossfade can land first,
    /// and the file named is then no longer the one playing.
    VideoFailed(String),
    Error(String),
}

#[cfg(not(target_os = "android"))]
mod desktop;
#[cfg(not(target_os = "android"))]
pub use desktop::Player;
#[cfg(not(target_os = "android"))]
pub(crate) use desktop::{quoted, Decks};

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use android::{android_set_metadata, Player};
