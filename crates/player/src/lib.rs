//! libmpv wrapper. context/14. YouTube-agnostic: takes a fully-resolved URL + headers, never
//! a videoId. Gapless via mpv's internal playlist (1-track lookahead fed by the orchestrator).

use std::collections::HashMap;
use std::sync::Arc;

use libmpv2::events::{Event, EventContext, PropertyData};
use libmpv2::{Format, Mpv};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("mpv: {0}")]
    Mpv(#[from] libmpv2::Error),
}

/// Events pumped from mpv's event thread. context/14 §player surface.
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Position(f64),
    Duration(f64),
    Playing,
    Paused,
    /// One track finished normally (EOF) — orchestrator advances the queue.
    TrackEnded,
    /// One track died (end-file with error, e.g. its URL 403'd). mpv may have auto-advanced
    /// into the next playlist entry or gone idle — the orchestrator asks [`Player::is_idle`].
    TrackFailed(String),
    Error(String),
}

/// mpv end-file reasons (from `mpv_end_file_reason`).
const EOF: i32 = 0;

/// The player. Wraps `Arc<Mpv>` (Send+Sync); the event loop runs on a dedicated OS thread and
/// pumps [`PlayerEvent`]s into a channel taken once via [`Player::take_events`].
pub struct Player {
    mpv: Arc<Mpv>,
    events: Option<UnboundedReceiver<PlayerEvent>>,
}

impl Player {
    /// Create a player with a disk audio cache under `cache_dir` (the audio-bytes tier, context/14).
    pub fn new(cache_dir: &str) -> Result<Self, Error> {
        // libmpv requires LC_NUMERIC=="C" to parse internal option values; Tauri/GTK's init
        // resets the process locale from the system locale first, which makes mpv_create()
        // return null (ponytail: locale reset only, revisit if other LC_* categories start
        // tripping mpv too).
        unsafe {
            libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr());
        }

        // Mirror the Phase-0 spike: create, then set_property (setting some options during the
        // pre-init phase returns PROPERTY_NOT_FOUND on this mpv build).
        let mpv = Mpv::new()?;
        mpv.set_property("vid", "no")?; // audio only
        mpv.set_property("gapless-audio", "yes")?;
        mpv.set_property("cache", "yes")?;
        mpv.set_property("cache-on-disk", "yes")?;
        mpv.set_property("demuxer-cache-dir", cache_dir)?;
        let mpv = Arc::new(mpv);

        let (tx, rx) = unbounded_channel();
        let ev = EventContext::new(mpv.ctx);
        ev.disable_deprecated_events().ok();
        ev.observe_property("time-pos", Format::Double, 0)?;
        ev.observe_property("duration", Format::Double, 1)?;
        ev.observe_property("pause", Format::Flag, 2)?;

        std::thread::Builder::new()
            .name("mpv-events".into())
            .spawn(move || event_loop(ev, tx))
            .expect("spawn mpv event thread");

        Ok(Player { mpv, events: Some(rx) })
    }

    /// Take the event receiver (once).
    pub fn take_events(&mut self) -> Option<UnboundedReceiver<PlayerEvent>> {
        self.events.take()
    }

    /// Load and play a fresh URL, replacing the playlist. context/14.
    pub fn load(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        gain_db: Option<f64>,
    ) -> Result<(), Error> {
        self.apply_headers(headers)?;
        self.apply_gain(gain_db)?;
        self.mpv.command("loadfile", &[url, "replace"])?;
        Ok(())
    }

    /// Append the next track for a gapless transition (the 1-track lookahead). context/14.
    ///
    /// Note: mpv's `http-header-fields`/`user-agent` are global properties, so appended tracks
    /// inherit the currently-set headers. Phase 1 direct-URL clients need no per-track cookies,
    /// so this is fine; per-track header divergence is a Phase 2+ concern (WEB_REMIX `&pot=`).
    pub fn enqueue(&self, url: &str) -> Result<(), Error> {
        self.mpv.command("loadfile", &[url, "append"])?;
        Ok(())
    }

    /// Clear the mpv playlist (e.g. when the user jumps to a new track).
    pub fn clear_playlist(&self) -> Result<(), Error> {
        self.mpv.command("playlist-clear", &[])?;
        Ok(())
    }

    /// True when mpv has nothing loaded (playlist exhausted or the last load failed). The
    /// orchestrator uses this after a track ends/fails to tell "gaplessly advanced into the
    /// lookahead" apart from "stalled — load the next track explicitly".
    pub fn is_idle(&self) -> bool {
        self.mpv.get_property::<bool>("idle-active").unwrap_or(true)
    }

    pub fn play(&self) -> Result<(), Error> {
        self.mpv.set_property("pause", false)?;
        Ok(())
    }

    pub fn pause(&self) -> Result<(), Error> {
        self.mpv.set_property("pause", true)?;
        Ok(())
    }

    pub fn toggle(&self) -> Result<(), Error> {
        self.mpv.command("cycle", &["pause"])?;
        Ok(())
    }

    /// Absolute seek in seconds.
    pub fn seek(&self, position_secs: f64) -> Result<(), Error> {
        self.mpv
            .command("seek", &[&position_secs.to_string(), "absolute"])?;
        Ok(())
    }

    /// Set output volume (0–100, mpv accepts >100 for amplification).
    pub fn set_volume(&self, volume: i64) -> Result<(), Error> {
        self.mpv.set_property("volume", volume)?;
        Ok(())
    }

    fn apply_headers(&self, headers: &HashMap<String, String>) -> Result<(), Error> {
        // User-Agent has its own mpv property; everything else joins http-header-fields.
        if let Some(ua) = headers.get("User-Agent").or_else(|| headers.get("user-agent")) {
            self.mpv.set_property("user-agent", ua.as_str())?;
        }
        let fields: String = headers
            .iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("user-agent"))
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join(",");
        self.mpv.set_property("http-header-fields", fields.as_str())?;
        Ok(())
    }

    /// Apply per-track loudness gain. context/14.
    // ponytail: `<gain>dB` volume filter, not full target-LUFS math — Phase 1 loudness is
    // non-blocking (PHASE1-PROMPT). Upgrade to the file-07 VolumeNormalization formula in Phase 4.
    fn apply_gain(&self, gain_db: Option<f64>) -> Result<(), Error> {
        match gain_db {
            Some(g) => self
                .mpv
                .set_property("af", format!("lavfi=[volume={g}dB]").as_str())?,
            None => self.mpv.set_property("af", "")?,
        }
        Ok(())
    }
}

fn event_loop(mut ev: EventContext, tx: tokio::sync::mpsc::UnboundedSender<PlayerEvent>) {
    loop {
        match ev.wait_event(1.0) {
            Some(Ok(event)) => {
                let out = match event {
                    Event::PropertyChange { name: "time-pos", change: PropertyData::Double(p), .. } => {
                        Some(PlayerEvent::Position(p))
                    }
                    Event::PropertyChange { name: "duration", change: PropertyData::Double(d), .. } => {
                        Some(PlayerEvent::Duration(d))
                    }
                    Event::PropertyChange { name: "pause", change: PropertyData::Flag(paused), .. } => {
                        Some(if paused { PlayerEvent::Paused } else { PlayerEvent::Playing })
                    }
                    Event::EndFile(reason) => match reason as i32 {
                        EOF => Some(PlayerEvent::TrackEnded),
                        // STOP/QUIT/REDIRECT are deliberate (loadfile replace, shutdown) — ignore.
                        // ERROR never reaches this arm: libmpv2 surfaces end-file-with-error as
                        // Err from wait_event (see below).
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(e) = out {
                    // Receiver dropped ⇒ player gone ⇒ stop the thread.
                    if tx.send(e).is_err() {
                        break;
                    }
                }
            }
            Some(Err(e)) => {
                // libmpv2 routes MPV_EVENT_END_FILE with an error (dead URL, 403, bad format)
                // through here instead of Event::EndFile — in our usage (no async get/set/command
                // replies) an Err from wait_event *is* a failed track.
                if tx.send(PlayerEvent::TrackFailed(e.to_string())).is_err() {
                    break;
                }
            }
            None => {}
        }
    }
}
