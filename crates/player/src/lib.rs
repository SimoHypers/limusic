//! libmpv wrapper. context/14. YouTube-agnostic: takes a fully-resolved URL + headers, never
//! a videoId. The orchestrator feeds it a 1-track lookahead, which goes into mpv's own playlist
//! for a gapless transition, or onto a second mpv instance when crossfading is on: one file at a
//! time per instance, so an overlap needs a second deck (see [`Decks`]).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use libmpv2::events::{Event, EventContext, PropertyData};
use libmpv2::{Format, Mpv};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("mpv: {0}")]
    Mpv(#[from] libmpv2::Error),
    /// mpv refused a chain carrying the pitch filter, which means this libmpv was built without
    /// librubberband. Its own answer is `Raw(-9)`, so it needs saying in words: this one reaches
    /// the user as a toast.
    #[error("Pitch shifting isn't available in this build")]
    NoPitchFilter,
}

/// Events pumped from mpv's event thread. context/14 §player surface.
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
    Error(String),
}

/// mpv end-file reasons (from `mpv_end_file_reason`).
const EOF: i32 = 0;

/// User-facing message for a failed track — raw mpv codes ("Raw(-13)") mean nothing to users.
fn friendly_error(e: &libmpv2::Error) -> String {
    use libmpv2::mpv_error;
    match e {
        libmpv2::Error::Loadfile { error } => friendly_error(error),
        libmpv2::Error::Raw(code) => match *code {
            // Not necessarily YouTube: mpv reads from the loopback audio proxy, so this also
            // covers the proxy failing to reach googlevideo. Say only what is known.
            mpv_error::LoadingFailed => {
                "Couldn't load this track. The stream link was refused.".to_owned()
            }
            mpv_error::NothingToPlay => "This stream contains no playable audio".to_owned(),
            mpv_error::UnknownFormat => "Unrecognized audio format".to_owned(),
            mpv_error::AoInitFailed => "Couldn't start audio output".to_owned(),
            other => format!("Playback failed (mpv error {other})"),
        },
        other => format!("Playback failed ({other})"),
    }
}

/// Did this end-file error come from the audio output rather than the stream? Windows reports it
/// after a sleep/resume (WASAPI re-enumerates and the old device is gone), Linux and macOS after a
/// device is unplugged or the sound server restarts.
fn is_ao_init_failed(e: &libmpv2::Error) -> bool {
    use libmpv2::mpv_error;
    match e {
        libmpv2::Error::Loadfile { error } => is_ao_init_failed(error),
        libmpv2::Error::Raw(code) => *code == mpv_error::AoInitFailed,
        _ => false,
    }
}

/// The player. Wraps `Arc<Mpv>` (Send+Sync); an event loop runs on a dedicated OS thread per deck
/// and pumps [`PlayerEvent`]s into a channel taken once via [`Player::take_events`].
pub struct Player {
    decks: Arc<Decks>,
    events: Option<UnboundedReceiver<PlayerEvent>>,
    /// `(loudness gain dB, pitch semitones)`. mpv's `af` is one global chain, so the two things
    /// that write to it have to be re-applied together: a bare `set_property("af", ...)` from
    /// either one would drop the other's filter.
    af: Mutex<(Option<f64>, i32)>,
}

/// One mpv instance plays one file at a time, so an *overlap* needs a second decoder and a second
/// audio output. Crossfading keeps two: the active deck is what the app hears, the idle one holds
/// the next track loaded and paused, and the fade swaps them.
///
/// The second deck is built on the first preload and never before, so nobody who leaves crossfade
/// off pays for it. With the setting off, `b` stays empty and every path below is the single-mpv
/// one it always was (the lookahead goes into mpv's own playlist, gaplessly).
struct Decks {
    a: Arc<Mpv>,
    b: OnceLock<Arc<Mpv>>,
    /// Which deck the app is listening to. Events from the other one are dropped.
    active: AtomicUsize,
    /// Crossfade length in milliseconds; 0 is off.
    crossfade_ms: AtomicU64,
    /// The volume slider's percent, so a fade can scale the user's own level instead of
    /// overwriting it.
    volume: AtomicI64,
    /// A track is loaded and paused on the idle deck, waiting to be faded in. Published by the
    /// idle deck's own `FileLoaded`, never by `preload`: `loadfile` returns long before mpv has
    /// the stream open, and a fade into a deck that is still opening plays silence over the
    /// track going out.
    preloaded: AtomicBool,
    /// Bumped whenever the preload is cancelled. A `loadfile` already in flight when that happens
    /// still reaches `FileLoaded` a moment later; comparing this against `preload_armed` is how
    /// that late event knows it is loading a track nobody wants any more, which used to hand the
    /// next crossfade whatever the user had just skipped away from.
    preload_gen: AtomicU64,
    /// The generation the track currently loading on the idle deck was started under.
    preload_armed: AtomicU64,
    /// Written only under the `pending` lock, so an `enqueue` racing the end of a fade cannot
    /// read "still fading", stash its URL, and have the ramp thread take `pending` a moment
    /// earlier: that leaves the URL stranded, loaded by nobody, while the app has already
    /// recorded a lookahead. Read without the lock on the event thread, where it is a hint.
    fading: AtomicBool,
    /// Bumped to cancel a fade in progress. The ramp thread captures it when it starts and stops
    /// as soon as the value no longer matches, so a pause, a seek or a skip mid-fade doesn't keep
    /// writing volumes to decks that have moved on.
    fade_gen: AtomicU64,
    /// A preload that arrived mid-fade. The idle deck is the one still fading *out*, so loading
    /// over it would cut the fade short: the ramp thread picks this up when it is done.
    pending: Mutex<Option<String>>,
    /// repeat-one. mpv restarts the file instead of ending it, so such a track must never fade
    /// into the one behind it. Mirrored here because the trigger runs on the event thread, where
    /// asking mpv for a property can stall the pump.
    loop_file: AtomicBool,
    cache_dir: String,
    tx: UnboundedSender<PlayerEvent>,
}

impl Decks {
    fn mpv(&self, deck: usize) -> Option<&Arc<Mpv>> {
        if deck == 0 {
            Some(&self.a)
        } else {
            self.b.get()
        }
    }

    fn active_mpv(&self) -> &Arc<Mpv> {
        self.mpv(self.active.load(Ordering::SeqCst)).unwrap_or(&self.a)
    }

    fn idle_deck(&self) -> usize {
        1 - self.active.load(Ordering::SeqCst)
    }

    fn crossfade_secs(&self) -> f64 {
        self.crossfade_ms.load(Ordering::Relaxed) as f64 / 1000.0
    }
}

/// Build one mpv instance, configured. Both decks go through here, so a crossfade deck is not a
/// lesser player: same cache, same reconnect options, same log level.
fn new_mpv(cache_dir: &str) -> Result<Mpv, Error> {
    // Mirror the Phase-0 spike: create, then set_property (setting some options during the
    // pre-init phase returns PROPERTY_NOT_FOUND on this mpv build).
    let mpv = Mpv::new()?;
    mpv.set_property("vid", "no")?; // audio only
                                    // The app resolves every URL itself; mpv shelling out to youtube-dl is never right and buries
                                    // the real failure. A stream that 403s went "Stream ends prematurely" -> ytdl_hook ->
                                    // "youtube-dl failed: not found" -> "Failed to recognize file format", so the user was told
                                    // their audio format was wrong when the download had been refused (issue #292).
    mpv.set_property("ytdl", "no")?;
    mpv.set_property("gapless-audio", "yes")?;
    // mpv's native Matroska demuxer floors WebM DiscardPadding nanoseconds to Opus samples. For
    // values one nanosecond below a sample boundary (YouTube emits 6,833,333 ns), that leaves one
    // unwanted sample at EOF and an audible pop between otherwise-continuous tracks. FFmpeg's
    // demuxer rounds the padding correctly without altering the stream bytes.
    mpv.set_property("demuxer", "lavf")?;
    // Open the appended remote entry while the current track is still playing. `gapless-audio`
    // keeps the output device alive, but on its own does not guarantee that the next HTTP stream
    // and demuxer are ready before the buffered tail reaches the playlist boundary.
    mpv.set_property("prefetch-playlist", "yes")?;
    mpv.set_property("cache", "yes")?;
    mpv.set_property("cache-on-disk", "yes")?;
    mpv.set_property("demuxer-cache-dir", cache_dir)?;
    // The demuxer runs at mpv's browser-sized defaults otherwise: 150 MiB forward and 50 MiB
    // back, per open file, and the gapless lookahead keeps two open across every transition.
    //
    // Note what these bound. `cache-on-disk` is on above, and mpv's manual is explicit that in
    // that mode the payload lives in the cache file and these limits apply to *packet
    // metadata* only, "typically 50 MB per hour of media". So 64 MiB is not "several tracks of
    // audio bytes", it is roughly 80 minutes of media before mpv starts pruning metadata. Big
    // enough that a seek anywhere inside an hour-long mix lands in the cached range once the
    // demuxer has had a head start, which turns those seeks into instant offline ones rather
    // than ones that have to open a fresh HTTP request (issue #188).
    //
    // The *back* buffer is the same size, not mpv's skimpy default. It is what makes a
    // backward seek instant: at the old 8 MiB mpv had long since pruned a position the user
    // had already heard, so scrubbing back forced a fresh network read and an audible stall
    // (the log showed `Enter buffering ... waited 0.76 secs` after a seek into an
    // already-played range). 64 MiB keeps roughly an entire mix, so backward seeks stay
    // offline. It is packet metadata, so the memory cost is the same order as the forward cap.
    mpv.set_property("demuxer-max-bytes", 64 * 1024 * 1024_i64)?;
    mpv.set_property("demuxer-max-back-bytes", 64 * 1024 * 1024_i64)?;
    // mpv enters "buffering" whenever a seek needs the network, and by default resumes only
    // once a full second of audio is buffered (`cache-pause-wait`, default 1). That second is
    // most of the "wait for it to start" after a seek; the connection answers in a fraction of
    // it. Resume on a shorter buffer and let the demuxer keep filling behind playback. mpv
    // still buffers (it pauses if the cache empties and the device underruns), so this shrinks
    // the safety margin to start sooner, it does not remove the guard.
    mpv.set_property("cache-pause-wait", 0.3)?;
    // ffmpeg's HTTP reader retries nothing by default: one dropped connection, one transient
    // error, and the track dies outright (mpv reports end-file with an error, which the app
    // turns into a skip). Seeking in a long stream is where that bites, because a seek past
    // the cached range opens a *fresh* request and gets no second chance. Issue #188.
    //
    // The retries are deliberately bounded. `reconnect_delay_max=5` alone means the loop is
    // infinite (ffmpeg's `reconnect_max_retries` defaults to -1 = unlimited), and a connection
    // that keeps dying at the same byte offset then hangs the demuxer forever: the app's log
    // shows "Will reconnect at ..." with repeated audio underruns and never a track-failed
    // event, so its retry-with-a-fresh-URL recovery never runs and playback is silently dead.
    // A handful of attempts surfaces the error and lets the app re-resolve (issue #188 followup).
    //
    // `short_seek_size=1` disables ffmpeg's "soft-seek" optimization. When a seek lands within
    // `short_seek` bytes of the end of the current response range, the HTTP reader drains the
    // rest of the body instead of issuing a new Range request ("Soft-seeking to offset ... by
    // draining N remaining byte(s)"). `short_seek` is the TLS/TCP stack's `SO_RCVBUF`, and on
    // Windows that reports a huge auto-tuned window, so a past-cache seek into a long
    // googlevideo response - whose initial `Range: bytes=0-` puts the range end at EOF - is
    // classified as "short" and ffmpeg tries to drain the remaining ~120 MB at dial-up speed.
    // Playback then pins at the seek target forever (issue #188). A threshold of 1 forces every
    // real seek to close the connection and open a fresh Range request, which googlevideo
    // answers with a 206.
    mpv.set_property(
            "stream-lavf-o",
            "reconnect=1,reconnect_streamed=1,reconnect_on_network_error=1,reconnect_delay_max=2,reconnect_max_retries=6,reconnect_delay_total_max=30,short_seek_size=1",
        )?;
    request_mpv_log(&mpv);
    Ok(mpv)
}

/// Observe the properties the event loop derives everything from, and run it on its own thread.
fn spawn_deck_events(mpv: &Arc<Mpv>, deck: usize, decks: Arc<Decks>) -> Result<(), Error> {
    let ev = EventContext::new(mpv.ctx);
    ev.disable_deprecated_events().ok();
    ev.observe_property("time-pos", Format::Double, 0)?;
    ev.observe_property("duration", Format::Double, 1)?;
    ev.observe_property("pause", Format::Flag, 2)?;
    ev.observe_property("idle-active", Format::Flag, 3)?;
    std::thread::Builder::new()
        .name(format!("mpv-events-{deck}"))
        .spawn(move || event_loop(ev, deck, decks))
        .expect("spawn mpv event thread");
    Ok(())
}

impl Player {
    /// Create a player with a disk audio cache under `cache_dir` (the audio-bytes tier, context/14).
    pub fn new(cache_dir: &str) -> Result<Self, Error> {
        // libmpv requires LC_NUMERIC=="C" to parse internal option values; Tauri/GTK's init
        // resets the process locale from the system locale first, which makes mpv_create()
        // return null (ponytail: locale reset only, revisit if other LC_* categories start
        // tripping mpv too).
        //
        // Here and not in `new_mpv`: the crossfade deck is built lazily, on whatever thread a
        // lookahead happened to land on, and `setlocale` is not thread-safe. By then this has
        // already run, on the thread that builds the player.
        unsafe {
            libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr());
        }
        let a = Arc::new(new_mpv(cache_dir)?);
        let (tx, rx) = unbounded_channel();
        let decks = Arc::new(Decks {
            a: a.clone(),
            b: OnceLock::new(),
            active: AtomicUsize::new(0),
            crossfade_ms: AtomicU64::new(0),
            volume: AtomicI64::new(100),
            preloaded: AtomicBool::new(false),
            preload_gen: AtomicU64::new(0),
            preload_armed: AtomicU64::new(0),
            fading: AtomicBool::new(false),
            fade_gen: AtomicU64::new(0),
            pending: Mutex::new(None),
            loop_file: AtomicBool::new(false),
            cache_dir: cache_dir.to_owned(),
            tx,
        });
        spawn_deck_events(&a, 0, decks.clone())?;
        Ok(Player { decks, events: Some(rx), af: Mutex::new((None, 0)) })
    }

    /// The deck the app is hearing. Every command below acts on this one.
    fn mpv(&self) -> &Arc<Mpv> {
        self.decks.active_mpv()
    }

    /// The other deck, built on first use (crossfade is off for most people, and an unused mpv
    /// instance is still an mpv instance).
    fn idle_mpv(&self) -> Result<Arc<Mpv>, Error> {
        let deck = self.decks.idle_deck();
        if let Some(m) = self.decks.mpv(deck) {
            return Ok(m.clone());
        }
        let m = Arc::new(new_mpv(&self.decks.cache_dir)?);
        spawn_deck_events(&m, deck, self.decks.clone())?;
        let _ = self.decks.b.set(m);
        Ok(self.decks.b.get().expect("deck b just set").clone())
    }

    /// Take the event receiver (once).
    pub fn take_events(&mut self) -> Option<UnboundedReceiver<PlayerEvent>> {
        self.events.take()
    }

    /// Load and play a fresh URL, replacing the playlist. context/14.
    ///
    /// `start` lands playback at that position in seconds from the first sample, via `loadfile`'s
    /// `start=` option. It exists because a `seek` issued right after `loadfile` is *not* queued by
    /// mpv: the file isn't loaded yet, the command fails with `MPV_ERROR_COMMAND`, and a caller that
    /// ignores the error (as `state::start_current` did) silently plays from 0 instead of the
    /// requested position, so every resume and every failed-track retry restarted at the top
    /// (issue #188). `start=` is applied as part of the load, so there is no window to lose it.
    pub fn load(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        gain_db: Option<f64>,
        start: Option<f64>,
    ) -> Result<(), Error> {
        // Whatever was queued up on the other deck is not what the user asked for. Cut a fade
        // in progress too: a skip during the last seconds of a track should not leave the old one
        // still fading under the new one.
        self.drop_preload(true);
        // A paused outgoing file blips if the caller's play() unpauses it before mpv has unloaded
        // it (issue #306). `loadfile replace` and `stop` only set a flag: mpv's core thread tears
        // the file down later, and calls made back to back all run before it gets the chance, so
        // `pause=false` used to land on the old file every time. Stop it and wait until mpv has
        // really let go: `path` goes unavailable only after the teardown, which flushes the audio
        // output. (`idle-active` can't say this, it flips the moment `stop` is issued.) Playing
        // track changes are left alone, the old track is audible there anyway.
        // ponytail: bounded poll, a few ms in practice; wait on the deck's EndFile event if 50 ms
        // ever turns out too short.
        if self.mpv().get_property::<bool>("pause").unwrap_or(false) {
            self.mpv().command("stop", &[])?;
            for _ in 0..50 {
                if self.mpv().get_property::<String>("path").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        self.apply_headers(headers)?;
        self.set_gain(gain_db)?;
        let args = loadfile_args(url, start);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.mpv().command("loadfile", &refs)?;
        Ok(())
    }

    /// Append the next track for a gapless transition (the 1-track lookahead). context/14.
    ///
    /// Note: mpv's `http-header-fields`/`user-agent` are global properties, so appended tracks
    /// inherit the currently-set headers. Phase 1 direct-URL clients need no per-track cookies,
    /// so this is fine; per-track header divergence is a Phase 2+ concern (WEB_REMIX `&pot=`).
    pub fn enqueue(&self, url: &str) -> Result<(), Error> {
        if self.decks.crossfade_ms.load(Ordering::Relaxed) == 0 {
            self.mpv().command("loadfile", &[&quoted(url), "append"])?;
            return Ok(());
        }
        // Crossfading needs the next track on the *other* deck: an appended playlist entry can
        // only ever start once this one has stopped, which is the opposite of an overlap.
        //
        // Under the lock, because the end of a fade clears `fading` and takes `pending` under the
        // same one. Either this hands the URL over and the ramp thread picks it up, or the fade
        // has already finished and it is preloaded here; never neither.
        {
            let mut pending = self.decks.pending.lock().unwrap();
            if self.decks.fading.load(Ordering::Acquire) {
                *pending = Some(url.to_owned());
                return Ok(());
            }
        }
        let idle = self.idle_mpv()?;
        preload(&self.decks, &idle, url)
    }

    /// Crossfade length, or `None`/under a second to turn it off. Applies from the next track
    /// change; whatever is already loaded keeps the transition it was primed with.
    pub fn set_crossfade(&self, secs: Option<f64>) {
        let ms = secs
            .filter(|s| s.is_finite() && *s >= 1.0)
            .map_or(0, |s| (s.min(10.0) * 1000.0) as u64);
        self.decks.crossfade_ms.store(ms, Ordering::Relaxed);
        if ms == 0 {
            // Otherwise a track loaded onto the idle deck stays there for the rest of the
            // session, holding a demuxer and a disk cache open on a URL that expires. The fade
            // the user is *hearing* is left to finish: turning the setting off is about the next
            // transition, not this one.
            self.drop_preload(false);
        }
    }

    /// Forget the track waiting on the idle deck. A no-op without crossfading, where the lookahead
    /// lives in mpv's own playlist and `playlist-clear` is what drops it.
    ///
    /// `cut_fade` also stops a fade-out in progress. Off for a queue edit (the transition the user
    /// is hearing is not what changed), on for an explicit load.
    fn drop_preload(&self, cut_fade: bool) {
        *self.decks.pending.lock().unwrap() = None;
        // Before anything else: a `loadfile` still in flight must not publish itself after this.
        self.decks.preload_gen.fetch_add(1, Ordering::SeqCst);
        if cut_fade {
            self.cancel_fade();
        }
        if self.decks.preloaded.swap(false, Ordering::AcqRel) {
            if let Some(m) = self.decks.mpv(self.decks.idle_deck()) {
                let _ = m.command("stop", &[]);
            }
        }
    }

    /// Cut a fade in progress short. The ramp thread notices within a step and does the teardown
    /// (see [`finish_fade`]): stops the outgoing deck and brings the incoming one, which is what
    /// the user is listening to, up to full level instead of leaving it wherever the ramp was.
    ///
    /// The outgoing deck is paused here rather than left to that thread, so nothing of the old
    /// track is audible past this call.
    fn cancel_fade(&self) {
        if !self.decks.fading.load(Ordering::Acquire) {
            return;
        }
        self.decks.fade_gen.fetch_add(1, Ordering::SeqCst);
        if let Some(m) = self.decks.mpv(self.decks.idle_deck()) {
            let _ = m.set_property("pause", true);
        }
    }

    /// Clear the mpv playlist (e.g. when the user jumps to a new track).
    pub fn clear_playlist(&self) -> Result<(), Error> {
        self.mpv().command("playlist-clear", &[])?;
        self.drop_preload(false);
        Ok(())
    }

    /// Stop playback outright and empty the playlist: mpv goes idle and stays there.
    ///
    /// Not [`Self::clear_playlist`], which keeps the entry mpv is on. This exists for the audio
    /// device disappearing under a loaded file: mpv ends that file with an error and, with no
    /// `keep-open` and `prefetch-playlist` on, walks straight into the appended gapless lookahead
    /// (already open) and tries it too. By the time the app hears about the failure that next
    /// entry is the current one, so `playlist-clear` would leave it playing, which is a track the
    /// user never asked for starting by itself. Issue #267.
    pub fn stop(&self) -> Result<(), Error> {
        self.drop_preload(true);
        self.mpv().command("stop", &[])?;
        Ok(())
    }

    /// True when mpv has nothing loaded (playlist exhausted or the last load failed). The
    /// orchestrator uses this after a track ends/fails to tell "gaplessly advanced into the
    /// lookahead" apart from "stalled — load the next track explicitly".
    pub fn is_idle(&self) -> bool {
        self.mpv().get_property::<bool>("idle-active").unwrap_or(true)
    }

    pub fn play(&self) -> Result<(), Error> {
        self.mpv().set_property("pause", false)?;
        Ok(())
    }

    /// Pause. Cancels a fade first: `active` swaps to the incoming deck the moment an overlap
    /// starts, so pausing only that one would leave the previous track playing out underneath the
    /// silence for the rest of the fade. Every pause in the app comes through here, media keys and
    /// MPRIS included.
    pub fn pause(&self) -> Result<(), Error> {
        self.cancel_fade();
        self.mpv().set_property("pause", true)?;
        Ok(())
    }

    /// Play/pause. Same reason as [`Self::pause`]: only one deck would answer the cycle. (A fade
    /// can't be running while paused, so this is a no-op on the way back up.)
    pub fn toggle(&self) -> Result<(), Error> {
        self.cancel_fade();
        self.mpv().command("cycle", &["pause"])?;
        Ok(())
    }

    /// Loop the current file seamlessly (repeat-one). mpv restarts the file at EOF *without*
    /// emitting end-file, so the queue logic upstream never advances while this is on — by design.
    pub fn set_loop_file(&self, on: bool) -> Result<(), Error> {
        self.decks.loop_file.store(on, Ordering::Relaxed);
        self.mpv().set_property("loop-file", if on { "inf" } else { "no" })?;
        Ok(())
    }

    /// Absolute seek in seconds.
    ///
    /// Logged, with the end of mpv's cached range, because that one number splits the two kinds of
    /// seek: inside the cache it is instant and never touches the network, past it mpv has to open
    /// a *fresh* HTTP request for the new offset. A report that says "seeking hangs" is answered by
    /// which of those it was, and nothing used to record it. Issue #188.
    pub fn seek(&self, position_secs: f64) -> Result<(), Error> {
        // Scrubbing is aimed at the track that is playing, which during an overlap is the
        // incoming deck. The outgoing one has nothing to do with the new position.
        self.cancel_fade();
        // `demuxer-cache-time` is the *end* of the cached range, so this only catches a forward
        // seek past it. A backward seek can need the network too (mpv prunes behind the reader);
        // the mpv log is what says which, when `LIMUSIC_MPV_LOG` is on.
        let cached_to = self.mpv().get_property::<f64>("demuxer-cache-time").ok();
        let past_cache_end = cached_to.map_or(true, |c| position_secs > c);
        tracing::info!(to = position_secs, cached_to, past_cache_end, "seek");
        self.mpv().command("seek", &[&position_secs.to_string(), "absolute"])?;
        Ok(())
    }

    /// Set output volume (0–100). The slider percent is perceptual, not mpv's raw scale:
    /// mpv cubes its `volume` property (gain = (v/100)³), which makes a 10-step drag near
    /// the bottom jump ~18 dB while the same drag near the top moves ~3 dB. Map the percent
    /// onto a 60 dB loudness range instead (see [`perceptual_to_mpv`]), so steps stay roughly
    /// the same size and the bottom of the slider is actually quiet rather than just near-floor.
    pub fn set_volume(&self, volume: i64) -> Result<(), Error> {
        self.mpv().set_property("volume", perceptual_to_mpv(volume))?;
        // Remembered because a crossfade scales it on both decks, and because the deck that
        // fades in is not the one this was last set on.
        self.decks.volume.store(volume, Ordering::Relaxed);
        Ok(())
    }

    /// Current volume level (0-100).
    pub fn volume(&self) -> i64 {
        self.decks.volume.load(Ordering::Relaxed)
    }

    /// Route the audio bytes through a proxy (the app's `proxy` setting). Call before the first
    /// [`Self::load`].
    ///
    /// The setting used to reach the InnerTube client only, so in a region that needs a proxy the
    /// orchestrator resolved a stream URL and mpv then sat at 0:00 while ffmpeg retried a
    /// googlevideo connection it could not open (#241).
    ///
    /// ffmpeg only speaks `http://` proxies (it CONNECTs through them for https URLs too), so
    /// anything else is refused here rather than handed over: mpv would accept the string and
    /// ffmpeg would silently stream direct. That includes `https://`, which looks supported and is
    /// not: `libavformat` gates proxying on a literal `http://` prefix (`av_strstart` in http.c and
    /// tls.c), so an https proxy is dropped without a word.
    ///
    /// Only the scheme is logged: a proxy URI can carry credentials in its userinfo and the warning
    /// lands in `limusic.log`, which is what users attach to bug reports.
    pub fn set_http_proxy(&self, proxy: Option<&str>) -> Result<(), Error> {
        let p = proxy.unwrap_or("").trim();
        let usable = p.is_empty() || p.starts_with("http://");
        if !usable {
            let scheme = p.split_once("://").map_or("(none)", |(s, _)| s);
            tracing::warn!(scheme, "mpv only speaks http:// proxies, audio will stream direct");
            return Ok(());
        }
        self.mpv().set_property("http-proxy", p)?;
        Ok(())
    }

    fn apply_headers(&self, headers: &HashMap<String, String>) -> Result<(), Error> {
        // User-Agent has its own mpv property; everything else joins http-header-fields.
        if let Some(ua) = headers.get("User-Agent").or_else(|| headers.get("user-agent")) {
            self.mpv().set_property("user-agent", ua.as_str())?;
        }
        let fields: String = headers
            .iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("user-agent"))
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join(",");
        self.mpv().set_property("http-header-fields", fields.as_str())?;
        Ok(())
    }

    /// Apply a per-track loudness gain (dB) as an mpv `volume` audio filter. context/14. Kept
    /// YouTube-agnostic: the caller computes the gain from `loudnessDb` (see `state::loudness_gain`);
    /// this just applies whatever dB it's handed.
    ///
    /// `af` is a **global** mpv property, not a per-playlist-entry one, so a gaplessly-advanced
    /// track keeps whatever the last [`Self::load`] set. The orchestrator has to call this itself
    /// on a gapless advance or every track after the first plays at the first track's gain.
    // ponytail: set on advance, so the head of a gapless track carries the old gain for the event
    // round-trip (a few ms) and the filter chain reinits mid-stream. If that ever clicks audibly,
    // keep one labelled filter (`af=@gain:lavfi=[volume=0dB]`) and retune it with `af-command`.
    pub fn set_gain(&self, gain_db: Option<f64>) -> Result<(), Error> {
        self.af.lock().unwrap().0 = gain_db;
        self.apply_af()
    }

    /// Tempo, 0.25–2.0. Pitch is unaffected: `audio-pitch-correction` (mpv's default) time-stretches
    /// rather than resamples, so this is Metrolist's `PlaybackParameters.speed` exactly.
    pub fn set_speed(&self, speed: f64) -> Result<(), Error> {
        self.mpv().set_property("speed", speed.clamp(0.25, 2.0))?;
        Ok(())
    }

    /// Pitch shift in semitones, −12..=12 (one octave either way), via the rubberband filter.
    /// Independent of [`Self::set_speed`]: rubberband takes over the time-stretch mpv would
    /// otherwise do with scaletempo2, and shifts pitch on top of it.
    // ponytail: native `rubberband` only. A libmpv built without librubberband errors out and the
    // command surfaces that to the user; wire the `lavfi=[rubberband=pitch=...]` fallback if a
    // Windows/macOS build ever turns up without it.
    pub fn set_pitch(&self, semitones: i32) -> Result<(), Error> {
        let wanted = semitones.clamp(-12, 12);
        let previous = std::mem::replace(&mut self.af.lock().unwrap().1, wanted);
        if let Err(e) = self.apply_af() {
            // No librubberband in this build: mpv rejects the *whole* chain, loudness gain
            // included, so put the old value back rather than leave every later set_gain failing.
            // (mpv never applied the bad chain, so this restores what is already playing.)
            self.af.lock().unwrap().1 = previous;
            let _ = self.apply_af();
            return Err(if wanted == 0 { e } else { Error::NoPitchFilter });
        }
        Ok(())
    }

    fn apply_af(&self) -> Result<(), Error> {
        let (gain_db, semitones) = *self.af.lock().unwrap();
        self.mpv().set_property("af", af_chain(gain_db, semitones).as_str())?;
        Ok(())
    }
}

/// The whole `af` chain: loudness gain, then pitch. Empty when neither is in play, so the default
/// path stays exactly the filterless one it was before pitch existed.
///
/// A positive gain carries a limiter: lifting a quiet track pushes its peaks past full scale, and
/// without one they would clip at the output. Its threshold is full scale (`limit=1`), so it only
/// touches what would genuinely clip: a lower ceiling pulls down every peak of every boosted
/// track, which is audible gain-riding on music that was never going to clip (#298, #300).
/// `level=disabled` stops alimiter from re-normalizing the result, which would undo the gain.
/// `latency=1` is essential for gapless playback: alimiter looks 5 ms ahead, and latency
/// compensation trims that initial delay and drains the same number of buffered samples at EOF
/// instead of dropping the outgoing tail when mpv rebuilds the graph for the next playlist entry.
fn af_chain(gain_db: Option<f64>, semitones: i32) -> String {
    let mut chain = Vec::new();
    match gain_db {
        Some(g) if g > 0.0 => {
            chain.push(format!("lavfi=[volume={g}dB,alimiter=limit=1:level=disabled:latency=1]"))
        }
        Some(g) => chain.push(format!("lavfi=[volume={g}dB]")),
        None => {}
    }
    if semitones != 0 {
        // Semitones → frequency multiplier (equal temperament).
        chain.push(format!(
            "{}=pitch-scale={}",
            pitch_filter(),
            2f64.powf(semitones as f64 / 12.0)
        ));
    }
    chain.join(",")
}

/// Test seam. Set it to reproduce a libmpv built without librubberband: mpv then rejects the whole
/// `af` chain, loudness gain included, which is the failure [`Player::set_pitch`] rolls back from.
/// A machine that has the filter can't reach that path any other way. Not compiled into the app.
#[cfg(test)]
static NO_RUBBERBAND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn pitch_filter() -> &'static str {
    #[cfg(test)]
    if NO_RUBBERBAND.load(std::sync::atomic::Ordering::Relaxed) {
        return "rubberband_this_build_does_not_have";
    }
    "rubberband"
}

/// The env var that turns mpv's own log on, and the level it is given. mpv's names, so: `no`,
/// `fatal`, `error`, `warn`, `info`, `v`, `debug`, `trace`.
const MPV_LOG_ENV: &str = "LIMUSIC_MPV_LOG";

/// Ask mpv for its own log messages, which arrive as [`Event::LogMessage`] and are forwarded into
/// `tracing` by [`event_loop`].
///
/// Deliberately not mpv's `log-file` option. The app's log is the one the diagnostics button
/// collects *and redacts*, and an mpv log holds the full signed googlevideo URL; and a separate
/// file cannot be read against the app's own lines, which is the whole question when a report says
/// "it hung when I seeked". Interleaved and redacted beats complete and unpasteable.
///
/// `warn` by default: the levels that would answer a question like that (`v` shows demuxer seeks,
/// HTTP opens and cache state) run to thousands of lines a minute and have no business in a
/// shipped user's log file. Set [`MPV_LOG_ENV`] for a reproduction run.
///
/// Best-effort: a machine that can't turn the log on still plays music.
fn request_mpv_log(mpv: &Mpv) -> bool {
    let level = std::env::var(MPV_LOG_ENV).unwrap_or_else(|_| "warn".to_owned());
    request_mpv_log_at(mpv, &level)
}

/// The half of [`request_mpv_log`] that doesn't read the environment, so a test can name a level.
fn request_mpv_log_at(mpv: &Mpv, level: &str) -> bool {
    let Ok(level_c) = std::ffi::CString::new(level) else { return false };
    // SAFETY: `mpv.ctx` is the live handle, and mpv copies the level string during the call.
    let rc = unsafe { libmpv2_sys::mpv_request_log_messages(mpv.ctx.as_ptr(), level_c.as_ptr()) };
    if rc < 0 {
        tracing::warn!(rc, level, "mpv refused this log level ({MPV_LOG_ENV})");
    }
    rc >= 0
}

fn event_loop(mut ev: EventContext, deck: usize, decks: Arc<Decks>) {
    let tx = decks.tx.clone();
    // Only the deck the app is listening to gets to speak. The other one is either silent or
    // fading out under the new track, and its position, duration and end-of-file are not what
    // the app is playing.
    let live = || decks.active.load(Ordering::SeqCst) == deck;
    let mut duration = 0.0f64;
    // Playback state is derived from two properties, never polled: mpv answers `mpv_get_property`
    // synchronously on its core lock, so asking it from the app's async event pump can stall that
    // pump exactly when mpv is busiest (a gapless transition opening the next stream) — and a
    // stalled pump stops draining mpv's events, so track-end is never handled and playback wedges.
    // These arrive as events; nothing has to ask.
    //
    // mpv reports the initial value of an observed property immediately, so both are seeded here
    // before anything is loaded: `pause: false`, `idle-active: true` ⇒ not playing.
    let mut paused = false;
    let mut idle = true;
    let mut playing = false;
    loop {
        match ev.wait_event(1.0) {
            Some(Ok(event)) => {
                let out = match event {
                    Event::PropertyChange {
                        name: "time-pos",
                        change: PropertyData::Double(p),
                        ..
                    } => {
                        // Not while paused: mpv reports `time-pos` on a seek too, and a seek
                        // into the last seconds of a paused track must not start the next one.
                        if live() && !paused && !idle {
                            maybe_crossfade(&decks, deck, p, duration);
                        }
                        Some(PlayerEvent::Position(p))
                    }
                    Event::PropertyChange {
                        name: "duration",
                        change: PropertyData::Double(d),
                        ..
                    } => {
                        duration = d;
                        Some(PlayerEvent::Duration(d))
                    }
                    Event::PropertyChange {
                        name: "pause", change: PropertyData::Flag(p), ..
                    } => {
                        paused = p;
                        None
                    }
                    Event::PropertyChange {
                        name: "idle-active",
                        change: PropertyData::Flag(i),
                        ..
                    } => {
                        idle = i;
                        None
                    }
                    // mpv's own log, at whatever level `request_mpv_log` asked for. Its `prefix`
                    // is the subsystem ("mkv", "ffmpeg/demuxer", "cplayer"), which is the part
                    // that says where a stall is.
                    Event::LogMessage { prefix, level, text, .. } => {
                        let text = text.trim_end();
                        // Everything below `warn` lands at `info`, not `debug`: the app's default
                        // filter is `info`, so a `debug!` here would be silently dropped and
                        // setting `LIMUSIC_MPV_LOG` would appear to do nothing. mpv only sends
                        // these levels when that variable asked for them, so they are never noise.
                        match level {
                            "fatal" | "error" => {
                                tracing::error!(target: "mpv", "[{prefix}] {text}")
                            }
                            "warn" => tracing::warn!(target: "mpv", "[{prefix}] {text}"),
                            _ => tracing::info!(target: "mpv", "[{prefix}] {text}"),
                        }
                        None
                    }
                    // The idle deck has its file open, so the fade has something to bring up.
                    // A cancelled preload gets here too (mpv delivers the event before the
                    // `stop`), which is what the generation check is for.
                    Event::FileLoaded => {
                        if !live()
                            && decks.preload_armed.load(Ordering::SeqCst)
                                == decks.preload_gen.load(Ordering::SeqCst)
                        {
                            decks.preloaded.store(true, Ordering::Release);
                        }
                        None
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
                    // Receiver dropped ⇒ player gone ⇒ stop the thread. A crossfade sends its own
                    // `TrackEnded` when the overlap *starts*, so the outgoing deck's real one a few
                    // seconds later is dropped here rather than advancing the queue twice.
                    if live() && tx.send(e).is_err() {
                        break;
                    }
                }
                // A gapless advance never touches either property, so no spurious stop/start is
                // emitted between tracks.
                let now = !paused && !idle;
                if now != playing {
                    playing = now;
                    if live() && tx.send(PlayerEvent::Playing(now)).is_err() {
                        break;
                    }
                }
            }
            Some(Err(e)) => {
                // libmpv2 routes MPV_EVENT_END_FILE with an error (dead URL, 403, bad format)
                // through here instead of Event::EndFile — in our usage (no async get/set/command
                // replies) an Err from wait_event *is* a failed track.
                if !live() {
                    // Not every silent deck is a preload: `start_crossfade` flips `active` to the
                    // incoming deck the moment an overlap starts, so for the length of the fade
                    // the deck going *out* is the one that isn't live. Its stream dying there is
                    // the end of a track the user already heard, and reporting it as a lookahead
                    // failure would clear the *next* track's preload and evict its cached URL.
                    // A preload can't be the one erroring here: one arriving mid-fade is held in
                    // `pending` until `finish_fade`, which clears `fading` before it is loaded.
                    if decks.fading.load(Ordering::Acquire) {
                        tracing::warn!(deck, error = %friendly_error(&e), "outgoing track died mid-fade");
                        continue;
                    }
                    // The preload died before anyone heard it. Reporting it would skip the track
                    // that is actually playing; dropping the preload instead means this track
                    // reaches its own end normally and the app loads the next one explicitly
                    // (`state::on_track_ended` asks `is_idle` for exactly this case). It is still
                    // reported, as its own event, so the dead URL is evicted from the cache. `let _`,
                    // not `break`: this loop is what the playing track depends on.
                    tracing::warn!(deck, error = %friendly_error(&e), "crossfade preload failed");
                    decks.preloaded.store(false, Ordering::Release);
                    let _ = tx.send(PlayerEvent::LookaheadFailed(friendly_error(&e)));
                    continue;
                }
                let ev = if is_ao_init_failed(&e) {
                    PlayerEvent::AudioDeviceLost
                } else {
                    PlayerEvent::TrackFailed(friendly_error(&e))
                };
                if tx.send(ev).is_err() {
                    break;
                }
            }
            None => {}
        }
    }
}

/// How often a crossfade retouches both decks' volumes.
const FADE_STEP: Duration = Duration::from_millis(50);

/// Properties a fresh deck has to inherit from the one playing. mpv keeps these per instance, so
/// a preload that skipped them would stream past the user's proxy, without their headers, at 1x,
/// with no loudness gain and no pitch shift.
const INHERITED: [&str; 5] = ["user-agent", "http-header-fields", "http-proxy", "speed", "af"];

/// Load `url` on the idle deck, paused and silent, ready for [`start_crossfade`] to bring it up.
fn preload(decks: &Decks, mpv: &Arc<Mpv>, url: &str) -> Result<(), Error> {
    let src = decks.active_mpv();
    for key in INHERITED {
        if let Ok(v) = src.get_property::<String>(key) {
            let _ = mpv.set_property(key, v.as_str());
        }
    }
    mpv.set_property("pause", true)?;
    mpv.set_property("volume", 0.0)?;
    // Not `preloaded = true`: that waits for this deck's `FileLoaded` (see the field). Armed
    // before the load, so a `drop_preload` landing during it invalidates this generation.
    decks.preload_armed.store(decks.preload_gen.load(Ordering::SeqCst), Ordering::SeqCst);
    mpv.command("loadfile", &[&quoted(url), "replace"])?;
    Ok(())
}

/// The effective fade length when `pos` is close enough to the end of the track to start one,
/// else `None`.
///
/// Two clamps, and both have been wrong at some point. A fade longer than half the track would
/// begin before the previous one had finished, so a 10 s setting fades a 30 s interlude for 10 s
/// and a 12 s one for 6. And a fade can only be as long as what is actually left: a lookahead
/// that resolves slowly arrives with 3 s to go, and a 5 s ramp then has the incoming track still
/// climbing 2 s after the outgoing one hit its own end.
fn fade_due(setting: f64, pos: f64, duration: f64) -> Option<f64> {
    if !(setting > 0.0 && pos.is_finite() && duration.is_finite() && duration > 1.0) {
        return None;
    }
    let left = duration - pos;
    let fade = setting.min(duration / 2.0);
    (left <= fade).then(|| fade.min(left).max(0.0))
}

/// Called on every position tick of the active deck. Everything has to line up: crossfading is on,
/// the next track is already loaded and paused on the other deck, no fade is running, and this
/// track is not looping (repeat-one never ends, so it must never fade into anything).
fn maybe_crossfade(decks: &Arc<Decks>, deck: usize, pos: f64, duration: f64) {
    if decks.loop_file.load(Ordering::Relaxed)
        || decks.fading.load(Ordering::Acquire)
        || !decks.preloaded.load(Ordering::Acquire)
    {
        return;
    }
    let Some(fade) = fade_due(decks.crossfade_secs(), pos, duration) else { return };
    start_crossfade(decks, deck, fade);
}

/// Begin the overlap: swap which deck the app hears, unpause the preloaded one, and ramp both
/// volumes from a worker thread.
///
/// `TrackEnded` is sent at the *start* of the fade, because that is when the new track becomes the
/// one being heard: the app advances its queue there, so the UI, the media keys and the scrobbler
/// change over with the audio rather than seconds late. The outgoing deck's real end-of-file is
/// dropped by [`event_loop`], which by then is no longer the live deck.
fn start_crossfade(decks: &Arc<Decks>, from: usize, fade: f64) {
    let (Some(out), Some(incoming)) = (decks.mpv(from), decks.mpv(1 - from)) else { return };
    let (out, incoming) = (out.clone(), incoming.clone());
    // Tempo is per instance and is set on whatever deck was active at the time (`set_speed`, the
    // tempo dialog), so a track preloaded before the user touched it would come in at 1x.
    if let Ok(speed) = out.get_property::<String>("speed") {
        let _ = incoming.set_property("speed", speed.as_str());
    }
    decks.preloaded.store(false, Ordering::Release);
    // Read before `fading` goes up, which is what lets a cancel bump it: taken afterwards, a
    // cancel landing in between would be captured as the current value and never noticed.
    let gen = decks.fade_gen.load(Ordering::SeqCst);
    decks.fading.store(true, Ordering::Release);
    decks.active.store(1 - from, Ordering::SeqCst);
    let _ = incoming.set_property("pause", false);
    let _ = decks.tx.send(PlayerEvent::TrackEnded);
    // mpv reported the incoming track's duration when it was *preloaded*, while this deck was
    // still the inactive one, so that event was dropped and nothing repeats it: a property change
    // is only sent when the property changes. Without this the player bar keeps the outgoing
    // track's length for the whole of the next song. After `TrackEnded`, which resets the app's
    // stored duration on its way through the queue advance.
    //
    // It also bounds the fade: an overlap longer than half the *incoming* track would still be
    // climbing past that track's own end.
    let mut fade = fade;
    if let Ok(secs) = incoming.get_property::<f64>("duration") {
        if secs.is_finite() && secs > 0.0 {
            let _ = decks.tx.send(PlayerEvent::Duration(secs));
            fade = fade.min(secs / 2.0);
        }
    }
    tracing::info!(from, fade, "crossfading");
    let (ramp, fade_out, fade_in) = (decks.clone(), out.clone(), incoming.clone());
    let spawned = std::thread::Builder::new().name("mpv-crossfade".into()).spawn(move || {
        let (decks, out, incoming) = (ramp, fade_out, fade_in);
        let steps = ((fade / FADE_STEP.as_secs_f64()).round() as i64).max(1);
        let mut cancelled = false;
        for step in 1..=steps {
            std::thread::sleep(FADE_STEP);
            // A pause, a seek or a skip landed. Stop writing volumes: by now one of these decks
            // is holding a track nobody asked for.
            if decks.fade_gen.load(Ordering::SeqCst) != gen {
                cancelled = true;
                break;
            }
            let turn = step as f64 / steps as f64 * std::f64::consts::FRAC_PI_2;
            let vol = decks.volume.load(Ordering::Relaxed);
            let _ = out.set_property("volume", fade_volume(vol, turn.cos()));
            let _ = incoming.set_property("volume", fade_volume(vol, turn.sin()));
        }
        // A lookahead that arrived mid-fade was held back rather than loaded over the track that
        // was still fading out. Now there is a free deck for it.
        if let Some(url) = finish_fade(&decks, &out, &incoming, cancelled) {
            // Unless crossfading was switched off while this ran, in which case the next
            // transition is mpv's own gapless one and a loaded deck would just sit there.
            if decks.crossfade_ms.load(Ordering::Relaxed) > 0 {
                if let Some(m) = decks.mpv(decks.idle_deck()).cloned() {
                    if let Err(e) = preload(&decks, &m, &url) {
                        tracing::warn!(error = %e, "deferred crossfade preload failed");
                    }
                }
            }
        }
    });
    if let Err(e) = spawned {
        // No thread, no ramp. The decks have already swapped, so tearing down as if the fade had
        // been cancelled puts the new track straight at full level and stops the old one: it steps
        // over instead of fading, rather than playing the rest of the song at the silence it was
        // preloaded with.
        tracing::warn!(error = %e, "couldn't spawn the crossfade thread");
        finish_fade(decks, &out, &incoming, true);
    }
}

/// End a fade, however it ended. Stops the deck that was going out and puts its volume back where
/// the user has it, so it is ready to be the next preload.
///
/// A `cancelled` fade also has to bring the *incoming* deck up to full level: it was cut mid-ramp
/// and it is the one the user is listening to. Without that, skipping one second into a five
/// second fade leaves the new track at a third of its volume, swelling for the next four seconds.
///
/// Returns a lookahead that arrived mid-fade, if one did. Clearing `fading` and taking `pending`
/// happen under the same lock that [`Player::enqueue`] reads `fading` under, so a URL can't slip
/// between the two and be loaded by nobody.
fn finish_fade(
    decks: &Decks,
    out: &Arc<Mpv>,
    incoming: &Arc<Mpv>,
    cancelled: bool,
) -> Option<String> {
    let vol = decks.volume.load(Ordering::Relaxed);
    let _ = out.command("stop", &[]);
    let _ = out.set_property("volume", perceptual_to_mpv(vol));
    if cancelled {
        let _ = incoming.set_property("volume", perceptual_to_mpv(vol));
    }
    let mut pending = decks.pending.lock().unwrap();
    decks.fading.store(false, Ordering::Release);
    pending.take()
}

/// mpv `volume` for one side of a crossfade: the user's own level scaled by the amplitude factor
/// `amp`. mpv's gain is (volume/100)³, so the cube root puts `amp` on the amplitude scale rather
/// than on the property's. The two sides pass cos and sin of the same quarter turn, which keeps
/// cos²+sin²=1: equal power across the overlap, so the middle of a fade doesn't dip.
fn fade_volume(percent: i64, amp: f64) -> f64 {
    perceptual_to_mpv(percent) * amp.clamp(0.0, 1.0).cbrt()
}

/// Quote a filename/URL for mpv's command parser.
///
/// libmpv2's `command` builds one space-joined string and hands it to `mpv_command_string`, which
/// splits it back apart on whitespace. So `loadfile /music/My music/a, b.mp3 replace` reaches mpv
/// as six arguments and fails with INVALID_PARAMETER (-4) — which is every local file whose path
/// has a space in it. Inside double quotes mpv only treats `\` specially, so escaping those two
/// characters is the whole job (verified against libmpv: quotes, commas, `$` and backslashes all
/// round-trip byte for byte through `playlist/0/filename`).
fn quoted(arg: &str) -> String {
    format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The `loadfile` argument list for [`Player::load`], with an optional start position passed
/// through the file-local `start=` option. Split out from the FFI call so the argument list is
/// testable without libmpv, the same way `quoted` is. `loadfile`'s third positional (`index`) must
/// be supplied before the options string, so `-1` (auto) rides along whenever `start` is present.
/// A non-finite or non-positive start is dropped: `start=0` is the default anyway, and a NaN would
/// poison the load.
fn loadfile_args(url: &str, start: Option<f64>) -> Vec<String> {
    let mut args = vec![quoted(url), "replace".to_owned()];
    if let Some(pos) = start.filter(|p| p.is_finite() && *p > 0.0) {
        args.push("-1".to_owned());
        args.push(quoted(&format!("start={pos}")));
    }
    args
}

/// Slider percent → mpv `volume` value, over a 60 dB range. mpv applies gain = (v/100)³,
/// i.e. 60·log10(v/100) dB, so v = 100·10^(−(1−s/100)^1.5) yields −60·(1−s/100)^1.5 dB:
/// 50% is −21 dB, 25% is −39 dB, 1% is −59 dB. 0 stays a hard mute.
///
/// The 1.5 exponent buys the low end its range without moving anyone's saved setting much
/// (the old linear-in-dB curve put 50% at −20 dB, this one at −21). Steps are 0.9 dB at the
/// quiet end and 0.3 dB near the top, which is the right way round: fine control is wanted
/// where a dB is loud, and the bottom of the slider needs to reach somewhere quiet.
fn perceptual_to_mpv(percent: i64) -> f64 {
    if percent <= 0 {
        return 0.0;
    }
    100.0 * 10f64.powf(-(1.0 - percent.min(100) as f64 / 100.0).powf(1.5))
}

#[cfg(test)]
mod tests {
    use super::{af_chain, is_ao_init_failed, loadfile_args, perceptual_to_mpv, quoted};

    #[test]
    fn gain_and_pitch_share_one_chain() {
        // The bug this exists for: either setter clobbering the other's filter.
        assert_eq!(af_chain(None, 0), "");
        assert_eq!(af_chain(Some(-3.5), 0), "lavfi=[volume=-3.5dB]");
        assert_eq!(
            af_chain(Some(4.0), 0),
            "lavfi=[volume=4dB,alimiter=limit=1:level=disabled:latency=1]"
        );
        assert_eq!(af_chain(None, 12), "rubberband=pitch-scale=2");
        assert_eq!(af_chain(Some(-6.0), -12), "lavfi=[volume=-6dB],rubberband=pitch-scale=0.5");
        // One semitone up is the twelfth root of two.
        assert!(af_chain(None, 1).ends_with("1.0594630943592953"));
    }

    /// Everything above is string-building; this drives a real libmpv and reads `af` back out of
    /// it, because the questions that matter ("is the gain still in the chain", "what does mpv keep
    /// when it rejects a chain") are answered by mpv, not by us. Nothing is played, so no audio
    /// device is opened. One test rather than four: `NO_RUBBERBAND` is process-global and cargo
    /// runs tests in parallel.
    #[test]
    fn mpv_keeps_the_gain_through_pitch_changes_and_failures() {
        use super::{Error, Player, NO_RUBBERBAND};
        use std::sync::atomic::Ordering;

        let dir = std::env::temp_dir().join("limusic-af-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = Player::new(dir.to_str().unwrap()).expect("libmpv");
        let af = || p.mpv().get_property::<String>("af").unwrap();

        // `stream-lavf-o` is the one option here mpv could reject outright (it isn't a plain
        // flag), and `new` is infallible-by-expect at the call site, so a rejection would be a
        // panic on launch. Read it back: the reconnect settings are what keeps a seek in a long
        // stream from killing the track.
        let lavf = p.mpv().get_property::<String>("stream-lavf-o").unwrap();
        assert!(lavf.contains("reconnect=1"), "reconnect options missing: {lavf}");
        assert!(lavf.contains("reconnect_on_network_error=1"), "{lavf}");
        // Without this ffmpeg soft-seeks (drains the response body) instead of opening a new
        // connection, which stalls a deep seek in a long stream (issue #188).
        assert!(lavf.contains("short_seek_size=1"), "soft-seek guard missing: {lavf}");
        // The retries have to be *bounded*. `reconnect_delay_max=5` alone is an infinite loop
        // (ffmpeg's `reconnect_max_retries` defaults to -1): a connection that dies at the same
        // byte offset hangs the demuxer forever and the app's track-failed recovery never runs
        // (issue #188: "Will reconnect at ..." with audio underruns and no error). A cap is what
        // turns that into an error the app can recover from instead of a silent stall.
        assert!(lavf.contains("reconnect_max_retries"), "retry cap missing: {lavf}");
        assert!(
            !lavf.contains("reconnect_max_retries=-1"),
            "retry cap must not be unlimited: {lavf}"
        );

        // The proxy has to reach the audio bytes, not just the API calls (#241), and a proxy mpv
        // takes but ffmpeg ignores is worse than none: it looks applied and streams direct.
        p.set_http_proxy(Some("http://127.0.0.1:8080")).unwrap();
        assert_eq!(p.mpv().get_property::<String>("http-proxy").unwrap(), "http://127.0.0.1:8080");
        p.set_http_proxy(Some("socks5://127.0.0.1:1080")).unwrap();
        assert_eq!(
            p.mpv().get_property::<String>("http-proxy").unwrap(),
            "http://127.0.0.1:8080",
            "a socks proxy must not replace a usable one"
        );
        p.set_http_proxy(Some("https://127.0.0.1:8443")).unwrap();
        assert_eq!(
            p.mpv().get_property::<String>("http-proxy").unwrap(),
            "http://127.0.0.1:8080",
            "ffmpeg gates proxying on a literal http:// prefix, so https must be refused too"
        );
        p.set_http_proxy(None).unwrap();
        assert_eq!(p.mpv().get_property::<String>("http-proxy").unwrap(), "");

        // Gapless audio only keeps the output device open. The appended remote file also has to
        // be opened before the current track reaches EOF, or its HTTP/demux startup becomes an
        // audible pause at the boundary.
        assert!(
            p.mpv().get_property::<bool>("prefetch-playlist").unwrap(),
            "gapless lookahead prefetch is disabled"
        );
        assert_eq!(
            p.mpv().get_property::<String>("demuxer").unwrap(),
            "lavf",
            "native Matroska demuxing reintroduces a boundary sample"
        );

        // Seek latency. A 12 MiB back buffer was pruned well before a long mix ended, so a backward
        // seek hit the network and stalled; the default 1 s buffering gate is most of the rest of
        // the post-seek wait. Read both back so a silently-rejected value fails here.
        assert_eq!(
            p.mpv().get_property::<i64>("demuxer-max-back-bytes").unwrap(),
            64 * 1024 * 1024,
            "back buffer reverted to mpv's default"
        );
        // mpv stores this as a float, so the read-back is 0.30000001..., not 0.3 exactly.
        let cpw: f64 = p.mpv().get_property("cache-pause-wait").unwrap_or(-1.0);
        assert!((cpw - 0.3).abs() < 1e-6, "buffering gate reverted to mpv's default, got {cpw}");

        // The mpv log request is a raw FFI call libmpv2 doesn't wrap, and the whole point of it is
        // that someone reproducing a bug gets lines out of a shipped build. Check mpv takes the
        // level a reproduction run would ask for, and that a wrong one is reported rather than
        // silently doing nothing.
        use super::request_mpv_log_at;
        assert!(request_mpv_log_at(p.mpv(), "v"), "mpv refused the verbose log level");
        assert!(request_mpv_log_at(p.mpv(), "warn"), "mpv refused the default log level");
        assert!(!request_mpv_log_at(p.mpv(), "louder"), "a bogus level must not report success");

        // 1. Loudness normalization, then a pitch round trip. The gain has to survive both steps.
        p.set_gain(Some(-7.7)).unwrap();
        assert!(af().contains("volume=-7.7dB"), "gain missing: {}", af());
        p.set_pitch(2).unwrap();
        assert!(af().contains("volume=-7.7dB"), "pitch dropped the gain: {}", af());
        assert!(af().contains("rubberband"), "pitch missing: {}", af());
        p.set_pitch(0).unwrap();
        assert!(af().contains("volume=-7.7dB"), "reset dropped the gain: {}", af());
        assert!(!af().contains("rubberband"), "pitch 0 left a filter behind: {}", af());

        // 2. Gapless advance: the orchestrator retunes the gain for the next track (state.rs, the
        // `lookahead_gain` take). A pitch the user set must not fall out of the chain when it does.
        p.set_pitch(-5).unwrap();
        p.set_gain(Some(-2.5)).unwrap();
        assert!(af().contains("volume=-2.5dB"), "retune missed: {}", af());
        assert!(af().contains("rubberband"), "retune dropped the pitch: {}", af());
        p.set_pitch(0).unwrap();

        // 3. A libmpv without librubberband. mpv rejects the chain wholesale, so this is also the
        // case where loudness normalization could silently disappear.
        let before = af();
        NO_RUBBERBAND.store(true, Ordering::Relaxed);
        let err = p.set_pitch(3).unwrap_err();
        NO_RUBBERBAND.store(false, Ordering::Relaxed);
        // The user is told, in words. mpv's own answer is `Raw(-9)`, which says nothing.
        assert!(matches!(err, Error::NoPitchFilter), "rejection must surface: {err}");
        assert_eq!(err.to_string(), "Pitch shifting isn't available in this build");
        // mpv never applied the bad chain, and the rollback re-applied the good one either way.
        assert_eq!(af(), before, "a rejected pitch changed the live chain");
        assert!(af().contains("volume=-2.5dB"), "normalization lost: {}", af());
        // And the rolled-back state is clean: the next per-track retune is gain-only, not a
        // permanently poisoned chain that fails from here on.
        p.set_gain(Some(-4.0)).unwrap();
        let after = af(); // mpv hands the chain back in its own escaped form, hence `contains`
        assert!(after.contains("volume=-4dB"), "retune after a rejection failed: {after}");
        assert!(!after.contains("rubberband"), "stored pitch survived the rollback: {after}");

        // 4. A boosted track: mpv (and its ffmpeg) must accept the limiter, or the gain is lost.
        p.set_gain(Some(6.0)).unwrap();
        assert!(af().contains("alimiter"), "boost went in without its limiter: {}", af());
    }

    /// Issue #306: loading over a paused track must not return while that track is still loaded,
    /// because the caller's play() comes next and would unpause it for a moment. Without the wait
    /// in `load` this failed on every run here: mpv applies back-to-back calls before its core
    /// thread gets to unload anything.
    #[test]
    fn load_over_a_paused_track_unloads_it_first() {
        use super::Player;
        use std::collections::HashMap;
        use std::time::{Duration, Instant};

        let dir = std::env::temp_dir().join("limusic-load-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = Player::new(dir.to_str().unwrap()).expect("libmpv");
        p.mpv().set_property("ao", "null").unwrap(); // no audio device needed
        let path = || p.mpv().get_property::<String>("path").ok();
        let old = "av://lavfi:sine=f=440:d=30";

        p.load(old, &HashMap::new(), None, None).unwrap();
        p.play().unwrap();
        let t = Instant::now();
        while path().as_deref() != Some(old) && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(path().as_deref(), Some(old), "the first track never loaded");
        p.pause().unwrap();

        for _ in 0..10 {
            p.load("av://lavfi:sine=f=1000:d=30", &HashMap::new(), Some(-3.0), None).unwrap();
            assert_ne!(path().as_deref(), Some(old), "load returned with the paused track loaded");
            p.load(old, &HashMap::new(), None, None).unwrap();
            let t = Instant::now();
            while path().as_deref() != Some(old) && t.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[test]
    fn loadfile_start_is_a_file_local_option() {
        // No start: the plain 2-argument loadfile, unchanged.
        assert_eq!(loadfile_args("u", None), vec!["\"u\"", "replace"]);
        // Anything at or below 0 is the default position, so it is not worth the option, and a
        // NaN or infinity must never reach mpv.
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(loadfile_args("u", Some(bad)), vec!["\"u\"", "replace"], "start={bad}");
        }
        // The `index` positional has to be present for `options` to be read.
        assert_eq!(
            loadfile_args("u", Some(605.0)),
            vec!["\"u\"", "replace", "-1", "\"start=605\""]
        );
        // Fractional resume positions are what `state::pending_seek` actually carries.
        assert_eq!(
            loadfile_args("u", Some(8.6155624669999)),
            vec!["\"u\"", "replace", "-1", "\"start=8.6155624669999\""]
        );
    }

    #[test]
    fn paths_survive_mpvs_command_parser() {
        // The bug this exists for: a space used to end the argument.
        assert_eq!(quoted("/music/My music/a, b.mp3"), "\"/music/My music/a, b.mp3\"");
        // Only backslash and double quote mean anything inside the quotes.
        assert_eq!(quoted(r#"/m/say "hi".mp3"#), r#""/m/say \"hi\".mp3""#);
        assert_eq!(quoted(r"C:\Music\x.mp3"), r#""C:\\Music\\x.mp3""#);
        // A stream URL is unchanged apart from the wrapper.
        assert_eq!(quoted("https://x/y?a=1&b=2"), "\"https://x/y?a=1&b=2\"");
    }

    /// The two halves of the crossfade decision: when it starts, and how loud each deck is
    /// while it runs. Both are pure, and both have a failure mode you can hear.
    #[test]
    fn crossfade_timing_and_equal_power() {
        use super::{fade_due, fade_volume};

        // A 5 s fade on a 3 minute track starts with 5 s to go, not before.
        assert_eq!(fade_due(5.0, 174.9, 180.0), None);
        assert_eq!(fade_due(5.0, 175.0, 180.0), Some(5.0));
        // Started late (a preload that only just landed): fade for what's left, not the setting.
        assert_eq!(fade_due(5.0, 179.5, 180.0), Some(0.5));
        // Off.
        assert_eq!(fade_due(0.0, 179.0, 180.0), None);
        // A fade longer than half the track is clamped, so it can't start before the previous
        // one has finished (a 10 s setting on a 12 s interlude).
        assert_eq!(fade_due(10.0, 5.0, 12.0), None);
        assert_eq!(fade_due(10.0, 6.0, 12.0), Some(6.0));
        // And it can only be as long as what's left. A lookahead that resolved slowly arrives
        // with 3 s to go: fade for 3, not 5, or the incoming track is still climbing 2 s after
        // the outgoing one has hit its own end.
        assert_eq!(fade_due(5.0, 177.0, 180.0), Some(3.0));
        // Right at (or past) the end there is nothing left to fade, and a negative length would
        // be worse than a cut.
        assert_eq!(fade_due(5.0, 180.0, 180.0), Some(0.0));
        assert_eq!(fade_due(5.0, 181.0, 180.0), Some(0.0));
        // Nothing mpv hasn't reported a real duration for: a live stream is 0, and `time-pos`
        // is NaN between files. Neither may trigger a fade.
        assert_eq!(fade_due(5.0, 10.0, 0.0), None);
        assert_eq!(fade_due(5.0, f64::NAN, 180.0), None);
        assert_eq!(fade_due(5.0, 10.0, f64::INFINITY), None);

        // Equal power: the two sides sum to the same power all the way through, so the middle of
        // an overlap is as loud as either end. (A linear-in-dB fade dips ~21 dB there.)
        let power = |t: f64| {
            let turn = t * std::f64::consts::FRAC_PI_2;
            let amp = |v: f64| (v / 100.0).powi(3); // mpv's own curve: volume → amplitude
            amp(fade_volume(100, turn.cos())).powi(2) + amp(fade_volume(100, turn.sin())).powi(2)
        };
        for step in 0..=10 {
            let p = power(step as f64 / 10.0);
            assert!((p - 1.0).abs() < 1e-9, "power dipped to {p} at {step}/10");
        }
        // It scales the user's own volume rather than jumping to full.
        assert_eq!(fade_volume(40, 1.0), perceptual_to_mpv(40));
        assert_eq!(fade_volume(40, 0.0), 0.0);
    }

    #[test]
    fn volume_curve() {
        let db = |s| 60.0 * (perceptual_to_mpv(s) / 100.0).log10();
        assert_eq!(perceptual_to_mpv(0), 0.0); // hard mute, not just very quiet
        assert_eq!(perceptual_to_mpv(100), 100.0);
        assert!((db(50) + 21.21).abs() < 0.01);
        // The point of the curve: 1% has somewhere to go. The old 40 dB range bottomed out
        // here, which left anyone listening quietly pinned to the floor.
        assert!((db(1) + 59.10).abs() < 0.01);
        // Monotonic, and finer steps at the loud end than the quiet one.
        assert!((1..=100).all(|s| perceptual_to_mpv(s) > perceptual_to_mpv(s - 1)));
        assert!(db(100) - db(99) < db(2) - db(1));
    }

    #[test]
    fn audio_device_errors_are_not_dead_streams() {
        use libmpv2::{mpv_error, Error};
        // The branch that decides whether a PC waking from sleep holds its place or walks the
        // whole queue playing every track it touches (issue #267).
        assert!(is_ao_init_failed(&Error::Raw(mpv_error::AoInitFailed)));
        assert!(is_ao_init_failed(&Error::Loadfile {
            error: std::rc::Rc::new(Error::Raw(mpv_error::AoInitFailed))
        }));
        // A dead or expired stream URL still has to reach the fallback clients.
        assert!(!is_ao_init_failed(&Error::Raw(mpv_error::LoadingFailed)));
        assert!(!is_ao_init_failed(&Error::Raw(mpv_error::NothingToPlay)));
    }
}
