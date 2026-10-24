//! libmpv wrapper. context/14. YouTube-agnostic: takes a fully-resolved URL + headers, never
//! a videoId. The orchestrator feeds it a 1-track lookahead, which goes into mpv's own playlist
//! for a gapless transition, or onto a second mpv instance when crossfading is on: one file at a
//! time per instance, so an overlap needs a second deck (see [`Decks`]).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use libmpv2::events::{Event, EventContext, PropertyData};
use libmpv2::mpv_node::MpvNode;
use libmpv2::{Format, Mpv};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

mod video;
pub use video::{GlDisplay, Thumbnail, VideoRenderer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("mpv: {0}")]
    Mpv(#[from] libmpv2::Error),
    /// mpv refused a chain carrying the tempo/pitch filter, which means this libmpv was built without
    /// librubberband. Its own answer is `Raw(-9)`, so it needs saying in words: this one reaches
    /// the user as a toast.
    #[error("Tempo and pitch adjustment aren't available in this build")]
    NoPitchFilter,
    #[error("Reverb isn't available: {0}")]
    Reverb(String),
    #[error("Invalid playback effect values")]
    InvalidParams,
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
    /// mpv's `pause` flag changed: someone pressed pause or play. Unlike [`PlayerEvent::Playing`]
    /// this says nothing when the playlist runs dry or a new file starts, which is what a Listen
    /// Together host has to broadcast: a track change is announced on its own, and a "stopped"
    /// from the gap between two tracks reaches the room after the next one has started.
    Paused(bool),
    /// One track finished normally (EOF) — orchestrator advances the queue.
    TrackEnded,
    /// Repeat-one started the file over. `loop-file` sends no end-file, so this is the only sign
    /// a new pass began; the app counts it as a new play (#518).
    Looped,
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
}

#[derive(Clone, Copy)]
struct AudioFilters {
    // Gain belongs to each track; effects belong to the listening session.
    gain: [Option<f64>; 2],
    speed: f64,
    semitones: i32,
    reverb: Reverb,
}

/// Session reverb controls. Dry and wet are independent signal levels, not a crossfade.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reverb {
    pub enabled: bool,
    pub preset: u8, // 1..=6: small/medium/large room, medium/large hall, plate.
    pub dry: u8,
    pub wet: u8,
    pub decay: f64,      // Nominal RT60, seconds.
    pub pre_delay: u16,  // Additional delay of the wet signal, milliseconds.
    pub damping: u16,    // Wet high-cut frequency, Hz.
    pub reflections: u8, // Level of the IR's first 80 ms, relative to its later tail.
}

impl Default for Reverb {
    fn default() -> Self {
        Self {
            enabled: false,
            preset: 3,
            dry: 100,
            wet: 25,
            decay: 1.1,
            pre_delay: 0,
            damping: 8000,
            reflections: 100,
        }
    }
}

const REVERB_DECAYS: [f64; 6] = [0.4, 0.7, 1.1, 1.8, 3.2, 1.8];

const REVERB_PRESETS: [(&str, &[u8]); 6] = [
    ("small-room", include_bytes!("../assets/small-room.wav")),
    ("medium-room", include_bytes!("../assets/medium-room.wav")),
    ("large-room", include_bytes!("../assets/large-room.wav")),
    ("medium-hall", include_bytes!("../assets/medium-hall.wav")),
    ("large-hall", include_bytes!("../assets/large-hall.wav")),
    ("plate", include_bytes!("../assets/plate.wav")),
];

/// One mpv instance plays one file at a time, so an *overlap* needs a second decoder and a second
/// audio output. Crossfading keeps two: the active deck is what the app hears, the idle one holds
/// the next track loaded and paused, and the fade swaps them.
///
/// The second deck is built on the first preload and never before, so nobody who leaves crossfade
/// off pays for it. With the setting off, `b` stays empty and every path below is the single-mpv
/// one it always was (the lookahead goes into mpv's own playlist, gaplessly).
struct Decks {
    af: Mutex<AudioFilters>,
    /// Last committed tempo as f64 bits; the event pump must not wait on effect updates.
    speed_bits: AtomicU64,
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
    afir_normalization: OnceLock<&'static str>,
    tx: UnboundedSender<PlayerEvent>,
    /// Music videos waiting for (or attached to) the audio file they belong to. See `video.rs`.
    videos: Mutex<video::Videos>,
    /// Someone can see the picture, so the deck being heard decodes it.
    video_visible: AtomicBool,
    /// Set by the video renderer: deck b needs a render context of its own before it can show a
    /// picture, and only the GL thread can make one.
    on_new_deck: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// The native window mpv draws the picture into, 0 for none (the render API). See
    /// [`Player::set_video_window`].
    wid: AtomicI64,
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

    fn speed(&self) -> f64 {
        f64::from_bits(self.speed_bits.load(Ordering::Relaxed))
    }
}

/// Build one mpv instance, configured. Both decks go through here, so a crossfade deck is not a
/// lesser player: same cache, same reconnect options, same log level.
fn new_mpv(cache_dir: &str) -> Result<Mpv, Error> {
    // Mirror the Phase-0 spike: create, then set_property (setting some options during the
    // pre-init phase returns PROPERTY_NOT_FOUND on this mpv build).
    let mpv = Mpv::new()?;
    // No picture until the app asks for one (`set_video_visible`). When it does, it goes through
    // the render API to the app's own GL surface: pinned, because left to probe mpv opens a window
    // of its own. `video-timing-offset=0` because otherwise each render call blocks until its
    // frame's display time, and it is made on the app's UI thread. See video.rs.
    mpv.set_property("vid", "no")?;
    // The OSD off: the app has its own seek bar, and mpv otherwise draws its own into the picture
    // on every seek.
    // Best-effort, unlike everything around them: an mpv that refused one of these would otherwise
    // fail the whole player, and the music must never depend on the picture.
    for (key, value) in [
        ("vo", "libmpv"),
        ("hwdec", "auto-safe"),
        ("video-timing-offset", "0"),
        ("osd-level", "0"),
        ("osd-bar", "no"),
    ] {
        if let Err(e) = mpv.set_property(key, value) {
            tracing::warn!(key, error = %e, "mpv refused a video option");
        }
    }
    // The app resolves every URL itself; mpv shelling out to youtube-dl is never right and buries
    // the real failure. A stream that 403s went "Stream ends prematurely" -> ytdl_hook ->
    // "youtube-dl failed: not found" -> "Failed to recognize file format", so the user was told
    // their audio format was wrong when the download had been refused (issue #292).
    mpv.set_property("ytdl", "no")?;
    // What the system mixer calls the stream. mpv's own names are "mpv" and `<media title> - mpv`,
    // and the media title of a proxied stream is the last segment of its loopback URL, so KDE's
    // mixer listed "e09087d13ce71f6d - mpv" (issue #409).
    mpv.set_property("audio-client-name", "Limusic")?;
    mpv.set_property("title", "Limusic")?;
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
            af: Mutex::new(AudioFilters {
                gain: [None; 2],
                speed: 1.0,
                semitones: 0,
                reverb: Reverb::default(),
            }),
            speed_bits: AtomicU64::new(1.0_f64.to_bits()),
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
            afir_normalization: OnceLock::new(),
            tx,
            videos: Mutex::new(video::Videos::default()),
            video_visible: AtomicBool::new(false),
            on_new_deck: OnceLock::new(),
            wid: AtomicI64::new(0),
        });
        spawn_deck_events(&a, 0, decks.clone())?;
        Ok(Player { decks, events: Some(rx) })
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
        let wid = self.decks.wid.load(Ordering::SeqCst);
        if wid != 0 {
            // Safe to carry on without it: a refused `wid` leaves the render API output, which
            // has no context here and so shows nothing, rather than a window of mpv's own.
            if let Err(e) = video::embed(&m, wid) {
                tracing::warn!(error = %e, "video: the crossfade deck refused the video window");
            }
        }
        spawn_deck_events(&m, deck, self.decks.clone())?;
        let _ = self.decks.b.set(m);
        if let Some(wake) = self.decks.on_new_deck.get() {
            wake();
        }
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
        let args = loadfile_args(url, start, loadfile_has_index(self.mpv()));
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
        let mut af = self.decks.af.lock().unwrap();
        let deck = self.decks.active.load(Ordering::SeqCst);
        let previous = af.gain[deck];
        af.gain[deck] = gain_db;
        if let Err(e) = apply_filters(&self.decks, deck, &af) {
            af.gain[deck] = previous;
            return Err(e);
        }
        Ok(())
    }

    /// Tempo, 0.5–2.0. Rubber Band time-stretches without shifting pitch whenever speed != 1.
    pub fn set_speed(&self, speed: f64) -> Result<(), Error> {
        let af = *self.decks.af.lock().unwrap();
        self.set_playback_params(speed, af.semitones, af.reverb)
    }

    /// Pitch shift in semitones, −12..=12 (one octave either way), via the rubberband filter.
    /// Independent of [`Self::set_speed`]: the same Rubber Band filter handles both controls.
    pub fn set_pitch(&self, semitones: i32) -> Result<(), Error> {
        let af = *self.decks.af.lock().unwrap();
        self.set_playback_params(af.speed, semitones, af.reverb)
    }

    /// One serialized, rollback-safe update on both decks.
    pub fn set_playback_params(
        &self,
        speed: f64,
        semitones: i32,
        reverb: Reverb,
    ) -> Result<(), Error> {
        if !speed.is_finite()
            || !(0.5..=2.0).contains(&speed)
            || !(-12..=12).contains(&semitones)
            || !(1..=6).contains(&reverb.preset)
            || reverb.dry > 100
            || reverb.wet > 100
            || reverb.reflections > 100
            || !reverb.decay.is_finite()
            || !(0.1..=10.0).contains(&reverb.decay)
            || reverb.pre_delay > 200
            || !(500..=20000).contains(&reverb.damping)
        {
            return Err(Error::InvalidParams);
        }
        let mut af = self.decks.af.lock().unwrap();
        if reverb.enabled && reverb.wet != 0 {
            ensure_reverb_asset(&self.decks, reverb.preset)?;
        }
        let previous = *af;
        let wanted = AudioFilters { speed, semitones, reverb, ..previous };
        for deck in 0..2 {
            // Audible Reverb embeds a fixed tempo in lavfi; other speed updates retain stretch state.
            let result = if previous.semitones == semitones
                && previous.reverb == reverb
                && (semitones != 0 || (previous.speed == 1.0) == (speed == 1.0))
                && (!reverb.enabled || reverb.wet == 0 || previous.speed == speed)
            {
                self.decks
                    .mpv(deck)
                    .map_or(Ok(()), |m| m.set_property("speed", speed).map_err(Error::from))
            } else {
                apply_filters(&self.decks, deck, &wanted)
            };
            if let Err(e) = result {
                for rollback in 0..2 {
                    let _ = apply_filters(&self.decks, rollback, &previous);
                }
                return Err(e);
            }
        }
        *af = wanted;
        self.decks.speed_bits.store(speed.to_bits(), Ordering::Relaxed);
        Ok(())
    }

    pub fn speed(&self) -> f64 {
        self.decks.speed()
    }

    pub fn playback_params(&self) -> (f64, i32, Reverb) {
        let af = self.decks.af.lock().unwrap();
        (af.speed, af.semitones, af.reverb)
    }
}

fn reverb_path(decks: &Decks, preset: u8) -> std::path::PathBuf {
    // clear_caches removes audio files at the cache root; the embedded effect is not a stream.
    let name = REVERB_PRESETS[preset as usize - 1].0;
    std::path::Path::new(&decks.cache_dir).join("effects").join(format!("reverb-{name}-v1.wav"))
}

fn ensure_reverb_asset(decks: &Decks, preset: u8) -> Result<std::path::PathBuf, Error> {
    let path = reverb_path(decks, preset);
    let bytes = REVERB_PRESETS[preset as usize - 1].1;
    // Versioned assets need only a size check; valid extra WAV chunks can stay in place.
    if !std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() >= bytes.len() as u64) {
        let error = |e: std::io::Error| Error::Reverb(e.to_string());
        std::fs::create_dir_all(path.parent().unwrap()).map_err(error)?;
        let temporary = path.with_extension("wav.tmp");
        std::fs::write(&temporary, bytes).map_err(error)?;
        std::fs::rename(&temporary, &path).map_err(error)?;
    }
    Ok(path)
}

fn apply_filters(decks: &Decks, deck: usize, af: &AudioFilters) -> Result<(), Error> {
    let Some(mpv) = decks.mpv(deck) else { return Ok(()) };
    let mut chain = af_chain(af.gain[deck], 0, 1.0);
    if af.reverb.enabled {
        if !chain.is_empty() {
            chain.push(',');
        }
        if af.reverb.wet == 0 {
            // A silent wet path needs no room DSP; retain dry gain and peak protection.
            let graph = format!(
                "volume={},alimiter=limit=1:level=disabled:latency=1",
                f64::from(af.reverb.dry) / 100.0
            );
            chain.push_str(&format!("@reverb:lavfi=graph=%{}%{graph}:o=%9%threads=1", graph.len()));
        } else {
            let path = ensure_reverb_asset(decks, af.reverb.preset)?;
            let normalization = if let Some(option) = decks.afir_normalization.get() {
                *option
            } else {
                // Release tags and git builds do not have consistently parseable version strings.
                // Probe the actual option once without touching either playing deck.
                let option = choose_afir_normalization(|option| {
                    let graph = format!(
                        "[in]asplit[signal][ir];[ir]atrim=end_sample=1[impulse];\
                         [signal][impulse]afir={option}[out]"
                    );
                    validate_lavfi(&format!("lavfi=graph=%{}%{graph}", graph.len()))
                })
                .map_err(|e| Error::Reverb(e.to_string()))?;
                let _ = decks.afir_normalization.set(option);
                option
            };
            chain.push_str(&reverb_filter(&path, af.reverb, normalization, af.speed, af.semitones));
        }
    }
    // The audible room graph already stretches its input; never add a second Rubber Band.
    let stretch = if af.reverb.enabled && af.reverb.wet != 0 {
        String::new()
    } else {
        af_chain(None, af.semitones, af.speed)
    };
    if !stretch.is_empty() {
        if !chain.is_empty() {
            chain.push(',');
        }
        chain.push_str(&stretch);
    }
    set_filter_chain(mpv, &chain).map_err(|e| {
        if af.reverb.enabled {
            Error::Reverb(e.to_string())
        } else if af.semitones != 0 || af.speed != 1.0 {
            Error::NoPitchFilter
        } else {
            Error::Mpv(e)
        }
    })?;
    mpv.set_property("speed", af.speed)?;
    Ok(())
}

fn set_filter_chain(mpv: &Mpv, chain: &str) -> Result<(), libmpv2::Error> {
    // `af set` initializes the graph and reports failure; writing the `af` property does not.
    // Cold decks only store the chain; probe effect graphs before accepting them.
    let needs_probe = chain.contains("@reverb") || chain.contains(pitch_filter());
    if needs_probe && mpv.get_property::<MpvNode>("audio-out-params").is_err() {
        validate_lavfi(chain)?;
    }
    mpv.command("af", &["set", &quoted(chain)])
}

fn choose_afir_normalization(
    mut probe: impl FnMut(&str) -> Result<(), libmpv2::Error>,
) -> Result<&'static str, libmpv2::Error> {
    // New FFmpeg still accepts gtype but ignores it, so always try irnorm first.
    if probe("irnorm=-1").is_ok() {
        return Ok("irnorm=-1");
    }
    probe("gtype=none")?;
    Ok("gtype=none")
}

fn validate_lavfi(chain: &str) -> Result<(), libmpv2::Error> {
    #[cfg(test)]
    FILTER_PROBES.set(FILTER_PROBES.get() + 1);
    let probe = Mpv::new()?;
    probe.set_property("ao", "null")?;
    probe.set_property("vid", "no")?;
    probe.set_property("pause", true)?;
    probe.command("loadfile", &["av://lavfi:anullsrc=r=48000:cl=stereo", "replace"])?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while probe.get_property::<MpvNode>("audio-out-params").is_err() {
        if std::time::Instant::now() >= deadline {
            return Err(libmpv2::Error::Raw(libmpv2::mpv_error::Command));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    probe.command("af", &["set", &quoted(chain)])
}

fn reverb_filter(
    path: &std::path::Path,
    reverb: Reverb,
    normalization: &str,
    speed: f64,
    semitones: i32,
) -> String {
    // Two parsers: amovie's option value, then lavfi's graph. Escape both, including Windows
    // drive letters and cache directories containing apostrophes, spaces, or graph delimiters.
    let filename: String = path
        .to_string_lossy()
        .replace('\\', "/")
        .chars()
        .flat_map(|c| if "\\': ".contains(c) { vec!['\\', c] } else { vec![c] })
        .flat_map(|c| if "\\'[],;".contains(c) { vec!['\\', c] } else { vec![c] })
        .collect();
    // afir's dry/wet options are input/output gain, NOT a dry/wet blend. Mix explicitly.
    // Keep the song at unity gain and add reflections at an independent wet level.
    // Changing presets, seeking, or skipping rebuilds the room and resets its tail.
    // Stretch the song inside lavfi, before the room, so no native speed metadata enters lavfi.
    let pitch = lavfi_stretch(speed, semitones);
    let stretch = REVERB_DECAYS[reverb.preset as usize - 1] / reverb.decay;
    let rate = (48000.0 * stretch).round() as u32;
    let dry = f64::from(reverb.dry) / 100.0;
    let wet = f64::from(reverb.wet) / 100.0;
    let reflections = f64::from(reverb.reflections) / 100.0;
    let early = 0.08;
    let pre_delay = reverb.pre_delay;
    let energy = stretch.sqrt(); // Stretching the IR must not multiply its energy.
    let timing = if speed != 1.0 {
        format!(",asetpts=(PTS-STARTPTS)*{speed}+STARTPTS")
    } else {
        String::new()
    };
    let graph = format!(
        "amovie=filename={filename},asetrate={rate},aresample=48000,\
         aeval=exprs='val(ch)*if(lt(t,{early}),{reflections},1)':c=same,volume={energy}[ir];\
         [in]{pitch}aformat=sample_rates=48000:channel_layouts=stereo,asplit[dry][src];\
         [src][ir]afir=dry=1:wet=1:irfmt=input:irgain=1:{normalization}:minp=1024:maxp=8192,\
         lowpass=f={},adelay={}:all=1[wet];\
         [dry][wet]amix=inputs=2:weights='{dry} {wet}':normalize=0,\
         alimiter=limit=1:level=disabled:latency=1{timing}[out]",
        reverb.damping, pre_delay
    );
    // Length quoting protects mpv's parser from delimiters inside the lavfi graph.
    // 1024-sample initial blocks (~21 ms at 48 kHz) trade buffering for less CPU work.
    // Larger tail blocks save long-decay work.
    // One worker avoids coordinating channel jobs in this small stereo graph.
    let mut filter = format!("@reverb:lavfi=graph=%{}%{graph}:o=%9%threads=1", graph.len());
    if speed != 1.0 {
        // Consume mpv's speed command and restore source-clock metadata without another stretch.
        // scale*speed=1 with no overlap/search copies the graph's samples unchanged.
        filter.push_str(&format!(",scaletempo=scale={}:overlap=0:search=0", 1.0 / speed));
    }
    filter
}

fn lavfi_stretch(speed: f64, semitones: i32) -> String {
    if speed == 1.0 && semitones == 0 {
        return String::new();
    }
    // FFmpeg exposes no engine option; its integer window passes EngineFiner | WindowShort to
    // Rubber Band's C API. These R3 options match the native short/shifted settings.
    // Its real-time wrapper can truncate or pad the EOF block.
    format!(
        "{}=tempo={speed}:pitch={}:window={}:formant=shifted:\
         transients=mixed:pitchq=consistency:channels=together,",
        pitch_filter(),
        2f64.powf(semitones as f64 / 12.0),
        0x20100000
    )
}

/// Loudness gain, then tempo/pitch. Normal speed and zero Pitch bypass Rubber Band.
///
/// A positive gain carries a limiter: lifting a quiet track pushes its peaks past full scale, and
/// without one they would clip at the output. Its threshold is full scale (`limit=1`), so it only
/// touches what would genuinely clip: a lower ceiling pulls down every peak of every boosted
/// track, which is audible gain-riding on music that was never going to clip (#298, #300).
/// `level=disabled` stops alimiter from re-normalizing the result, which would undo the gain.
/// `latency=1` is essential for gapless playback: alimiter looks 5 ms ahead, and latency
/// compensation trims that initial delay and drains the same number of buffered samples at EOF
/// instead of dropping the outgoing tail when mpv rebuilds the graph for the next playlist entry.
fn af_chain(gain_db: Option<f64>, semitones: i32, speed: f64) -> String {
    let mut chain = Vec::new();
    match gain_db {
        Some(g) if g > 0.0 => {
            chain.push(format!("lavfi=[volume={g}dB,alimiter=limit=1:level=disabled:latency=1]"))
        }
        Some(g) => chain.push(format!("lavfi=[volume={g}dB]")),
        None => {}
    }
    if semitones != 0 || speed != 1.0 {
        // Semitones → frequency multiplier (equal temperament).
        // Short windows reduce processing latency; shifted formants follow the pitch change.
        chain.push(format!(
            "{}=window=short:formant=shifted:pitch-scale={}",
            pitch_filter(),
            2f64.powf(semitones as f64 / 12.0)
        ));
    }
    chain.join(",")
}

#[cfg(test)]
std::thread_local! {
    /// Test seam: force a missing filter so [`Player::set_pitch`] exercises chain rollback.
    /// Thread-local so parallel playback tests keep their real filters. Absent from the app.
    static NO_RUBBERBAND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Count validation players independently of parallel playback tests.
    static FILTER_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn pitch_filter() -> &'static str {
    #[cfg(test)]
    if NO_RUBBERBAND.get() {
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
    // The previous `time-pos`, to spot repeat-one going back to the top (`loop_wrapped`).
    let mut last_pos = f64::NAN;
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
                        if live()
                            && decks.loop_file.load(Ordering::Relaxed)
                            && loop_wrapped(last_pos, p, duration)
                        {
                            let _ = tx.send(PlayerEvent::Looped);
                        }
                        last_pos = p;
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
                        let changed = p != paused;
                        paused = p;
                        changed.then_some(PlayerEvent::Paused(p))
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
                        // A new file, so the old one's end is not this one's: a skip from a
                        // track's last seconds under repeat-one is not a wrap. Looping is a seek
                        // inside mpv and never loads the file again.
                        last_pos = f64::NAN;
                        video::file_loaded(&decks, deck);
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
    let mut af = decks.af.lock().unwrap();
    af.gain[decks.idle_deck()] = af.gain[decks.active.load(Ordering::SeqCst)];
    let src = decks.active_mpv();
    for key in INHERITED {
        if let Ok(v) = src.get_property::<String>(key) {
            let _ = mpv.set_property(key, v.as_str());
        }
    }
    mpv.set_property("pause", true)?;
    mpv.set_property("volume", 0.0)?;
    // Off the audio device until its fade starts (`start_crossfade` puts it back). A paused deck
    // with a file loaded still holds an output stream, so Linux mixers listed this one for the
    // whole track as a second app next to the one playing, with a volume slider of its own that
    // the user's changes never reached (issue #409). Linux only: Windows groups a process's
    // streams under one mixer entry and macOS has no per-app mixer.
    #[cfg(target_os = "linux")]
    mpv.set_property("ao", "null")?;
    // Not `preloaded = true`: that waits for this deck's `FileLoaded` (see the field). Armed
    // before the load, so a `drop_preload` landing during it invalidates this generation.
    decks.preload_armed.store(decks.preload_gen.load(Ordering::SeqCst), Ordering::SeqCst);
    mpv.command("loadfile", &[&quoted(url), "replace"])?;
    Ok(())
}

/// The effective fade length when `pos` is close enough to the end of the track to start one,
/// else `None`. All three arguments are in listening (wall-clock) seconds.
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

/// Whether a position tick is repeat-one wrapping: the previous tick was in the file's last stretch
/// and this one is back at its start. A seek from the end to the top reads the same, and counting
/// that as another play is right too.
fn loop_wrapped(last: f64, pos: f64, duration: f64) -> bool {
    const EDGE: f64 = 1.5;
    duration > 2.0 * EDGE && last >= duration - EDGE && pos < EDGE
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
    // mpv positions follow the source clock, whereas the volume worker sleeps in real time.
    let speed = decks.speed();
    let Some(fade) = fade_due(decks.crossfade_secs(), pos / speed, duration / speed) else {
        return;
    };
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
    let speed = decks.speed();
    decks.preloaded.store(false, Ordering::Release);
    // Read before `fading` goes up, which is what lets a cancel bump it: taken afterwards, a
    // cancel landing in between would be captured as the current value and never noticed.
    let gen = decks.fade_gen.load(Ordering::SeqCst);
    decks.fading.store(true, Ordering::Release);
    decks.active.store(1 - from, Ordering::SeqCst);
    video::deck_swapped(decks);
    // Back on the real device (see `preload`). mpv reopens the output from where the null one had
    // prefilled to, 0.2 s into the track, so seek back to the start: a preload never has a
    // `start=`. Measured on mpv 0.41: without the seek the incoming track lost its first 0.2 s.
    #[cfg(target_os = "linux")]
    {
        // Normally empty (device autodetection). Preserve an explicitly selected output too,
        // including the timed null output used by headless transition tests.
        let ao = out.get_property::<String>("ao").unwrap_or_default();
        if let Err(e) = incoming.set_property("ao", ao.as_str()) {
            tracing::warn!(error = %e, "crossfade deck refused its audio device, it will be silent");
        }
        let _ = incoming.command("seek", &["0", "absolute"]);
    }
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
            fade = fade.min(secs / speed / 2.0);
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
/// testable without libmpv, the same way `quoted` is. On mpv 0.38+ `loadfile`'s third positional
/// (`index`) must be supplied before the options string, so `-1` (auto) rides along whenever
/// `start` is present; older mpv reads the options as argument 3 (see [`loadfile_has_index`]).
/// A non-finite or non-positive start is dropped: `start=0` is the default anyway, and a NaN would
/// poison the load.
fn loadfile_args(url: &str, start: Option<f64>, has_index: bool) -> Vec<String> {
    let mut args = vec![quoted(url), "replace".to_owned()];
    if let Some(pos) = start.filter(|p| p.is_finite() && *p > 0.0) {
        if has_index {
            args.push("-1".to_owned());
        }
        args.push(quoted(&format!("start={pos}")));
    }
    args
}

/// Whether this libmpv's `loadfile` has the `index` positional that mpv 0.38 put in front of the
/// options. The AppImage bundles Ubuntu 24.04's libmpv 0.37, where a `-1` there is parsed as the
/// options string and fails the whole load with INVALID_PARAMETER, so every resume and retry
/// failed on the first press (issue #384). Asked of mpv's own `command-list` rather than parsed
/// out of `mpv-version`, which a git build (the Windows one) doesn't keep to a release number.
fn loadfile_has_index(mpv: &Mpv) -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| {
        let Some(cmds) = mpv.get_property::<MpvNode>("command-list").ok().and_then(MpvNode::array)
        else {
            return true; // what every current mpv speaks
        };
        let is_named = |node: MpvNode, name: &str| {
            node.map().is_some_and(|mut m| m.any(|(k, v)| k == "name" && v.str() == Some(name)))
        };
        cmds.filter(|c| is_named(c.clone(), "loadfile")).any(|c| {
            let args = c.map().and_then(|mut m| m.find(|(k, _)| k == "args"));
            args.and_then(|(_, a)| a.array()).is_some_and(|mut a| a.any(|a| is_named(a, "index")))
        })
    })
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
mod transition_tests;

#[cfg(test)]
mod tests {
    fn reverb(preset: u8) -> super::Reverb {
        if preset == 0 {
            return super::Reverb::default();
        }
        super::Reverb {
            enabled: true,
            preset,
            decay: super::REVERB_DECAYS.get(preset as usize - 1).copied().unwrap_or(1.1),
            ..super::Reverb::default()
        }
    }
    use super::{
        af_chain, is_ao_init_failed, loadfile_args, loop_wrapped, perceptual_to_mpv, quoted,
    };

    #[test]
    fn repeat_one_wrap_is_end_to_start_only() {
        assert!(loop_wrapped(199.9, 0.02, 200.0));
        assert!(!loop_wrapped(f64::NAN, 0.0, 200.0), "the first tick of a file");
        assert!(!loop_wrapped(120.0, 0.0, 200.0), "a seek back from the middle");
        assert!(!loop_wrapped(199.9, 30.0, 200.0), "a seek back from the end, not to the top");
        assert!(!loop_wrapped(199.9, 0.0, 0.0), "no duration yet");
        assert!(!loop_wrapped(2.0, 0.0, 2.5), "too short to tell the end from the start");
    }

    #[test]
    fn afir_normalization_probes_options_in_compatibility_order() {
        use super::choose_afir_normalization;
        use libmpv2::{mpv_error, Error};
        let mut probes = Vec::new();
        assert_eq!(
            choose_afir_normalization(|option| {
                probes.push(option.to_owned());
                Ok(())
            })
            .unwrap(),
            "irnorm=-1"
        );
        assert_eq!(probes, ["irnorm=-1"]);

        probes.clear();
        assert_eq!(
            choose_afir_normalization(|option| {
                probes.push(option.to_owned());
                if option == "gtype=none" {
                    Ok(())
                } else {
                    Err(Error::Raw(mpv_error::Command))
                }
            })
            .unwrap(),
            "gtype=none"
        );
        assert_eq!(probes, ["irnorm=-1", "gtype=none"]);
        assert!(choose_afir_normalization(|_| Err(Error::Raw(mpv_error::Command))).is_err());
    }

    #[test]
    fn gain_and_pitch_share_one_chain() {
        // The bug this exists for: either setter clobbering the other's filter.
        assert_eq!(af_chain(None, 0, 1.0), "");
        assert_eq!(af_chain(Some(-3.5), 0, 1.0), "lavfi=[volume=-3.5dB]");
        assert_eq!(
            af_chain(Some(4.0), 0, 1.0),
            "lavfi=[volume=4dB,alimiter=limit=1:level=disabled:latency=1]"
        );
        assert!(af_chain(None, 12, 1.0).ends_with("pitch-scale=2"));
        let chain = af_chain(Some(-6.0), -12, 1.0);
        assert!(chain.starts_with("lavfi=[volume=-6dB],"));
        assert!(chain.ends_with("pitch-scale=0.5"));
        // One semitone up is the twelfth root of two.
        assert!(af_chain(None, 1, 1.0).ends_with("1.0594630943592953"));
    }

    #[test]
    fn cold_gain_updates_skip_filter_probes() {
        use super::{Player, Reverb, FILTER_PROBES};

        let dir = tempfile::tempdir().unwrap();
        let p = Player::new(dir.path().to_str().unwrap()).unwrap();
        assert!(p.mpv().get_property::<super::MpvNode>("audio-out-params").is_err());
        let before = FILTER_PROBES.get();
        for gain in [Some(-7.7), Some(6.0), None] {
            p.set_gain(gain).unwrap();
        }
        assert_eq!(FILTER_PROBES.get(), before, "gain-only updates started a temporary mpv");

        // Cold effect chains still need validation even though no track is decoded yet.
        p.set_pitch(1).unwrap();
        assert_eq!(FILTER_PROBES.get(), before + 1, "Rubber Band was not probed");
        let reverb = Reverb { enabled: true, wet: 0, ..Reverb::default() };
        p.set_playback_params(1.0, 0, reverb).unwrap();
        assert_eq!(FILTER_PROBES.get(), before + 2, "reverb was not probed");
    }

    #[test]
    fn reverb_assets_are_reused_and_recreated_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let p = super::Player::new(dir.path().to_str().unwrap()).unwrap();
        p.set_playback_params(1.0, 0, reverb(3)).unwrap();
        let asset = super::reverb_path(&p.decks, 3);
        let embedded = super::REVERB_PRESETS[2].1;
        assert!(std::fs::read(&asset).unwrap() == embedded);
        p.set_playback_params(1.0, 0, reverb(0)).unwrap();

        // Valid extra WAV padding must not trigger a content comparison and rewrite.
        let mut cached = embedded.to_vec();
        cached.extend_from_slice(b"JUNK\0\0\0\0");
        let size = (cached.len() as u32 - 8).to_le_bytes();
        cached[4..8].copy_from_slice(&size);
        std::fs::write(&asset, &cached).unwrap();
        p.set_playback_params(0.8, 0, reverb(3)).unwrap();
        assert!(std::fs::read(&asset).unwrap() == cached, "existing IR was rewritten");
        p.set_playback_params(1.0, 0, reverb(0)).unwrap();

        std::fs::remove_file(&asset).unwrap();
        p.set_playback_params(1.0, 0, reverb(3)).unwrap();
        assert!(std::fs::read(&asset).unwrap() == embedded, "missing IR was not restored");

        std::fs::remove_file(&asset).unwrap();
        p.set_gain(Some(-3.0)).unwrap();
        assert!(std::fs::read(&asset).unwrap() == embedded, "gain update did not restore the IR");

        std::fs::write(&asset, &embedded[..44]).unwrap();
        p.set_playback_params(0.8, 0, reverb(3)).unwrap();
        assert!(std::fs::read(&asset).unwrap() == embedded, "truncated IR was not repaired");
    }

    /// Everything above is string-building; this drives a real libmpv and reads `af` back out of
    /// it, because the questions that matter ("is the gain still in the chain", "what does mpv keep
    /// when it rejects a chain") are answered by mpv, not by us. Nothing is played, so no audio
    /// device is opened. Exercise changes and rollback on the same player instance.
    #[test]
    fn mpv_keeps_the_gain_through_pitch_changes_and_failures() {
        use super::{Error, Player, NO_RUBBERBAND};

        let dir = std::env::temp_dir().join("limusic-af-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = Player::new(dir.to_str().unwrap()).expect("libmpv");
        let af = || p.mpv().get_property::<String>("af").unwrap();
        assert_eq!(p.decks.speed(), 1.0);

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
        NO_RUBBERBAND.set(true);
        let err = p.set_pitch(3).unwrap_err();
        NO_RUBBERBAND.set(false);
        // The user is told, in words. mpv's own answer is `Raw(-9)`, which says nothing.
        assert!(matches!(err, Error::NoPitchFilter), "rejection must surface: {err}");
        assert_eq!(err.to_string(), "Tempo and pitch adjustment aren't available in this build");
        // mpv never applied the bad chain, and the rollback re-applied the good one either way.
        assert_eq!(af(), before, "a rejected pitch changed the live chain");
        assert!(af().contains("volume=-2.5dB"), "normalization lost: {}", af());
        // And the rolled-back state is clean: the next per-track retune is gain-only, not a
        // permanently poisoned chain that fails from here on.
        p.set_gain(Some(-4.0)).unwrap();
        let after = af(); // mpv hands the chain back in its own escaped form, hence `contains`
        assert!(after.contains("volume=-4dB"), "retune after a rejection failed: {after}");
        assert!(!after.contains("rubberband"), "stored pitch survived the rollback: {after}");

        // A refused tempo update must leave the live settings and filter chain intact.
        let before = af();
        NO_RUBBERBAND.set(true);
        let err = p.set_speed(0.8).unwrap_err();
        NO_RUBBERBAND.set(false);
        assert!(matches!(err, Error::NoPitchFilter));
        assert_eq!(p.playback_params(), (1.0, 0, reverb(0)));
        assert_eq!(p.decks.speed(), 1.0, "rejected tempo changed crossfade timing");
        assert_eq!(p.mpv().get_property::<f64>("speed").unwrap(), 1.0);
        assert_eq!(af(), before, "rejected tempo changed the live chain");

        // 4. A boosted track: mpv (and its ffmpeg) must accept the limiter, or the gain is lost.
        p.set_gain(Some(6.0)).unwrap();
        assert!(af().contains("alimiter"), "boost went in without its limiter: {}", af());

        // Reverb joins the same chain; retuning a track must preserve both session effects.
        p.set_playback_params(0.8, -3, reverb(3)).unwrap();
        assert_eq!(p.decks.speed(), 0.8);
        p.set_gain(Some(-3.0)).unwrap();
        assert!(
            af().contains("afir") && af().contains("rubberband") && af().contains("volume=-3dB")
        );
        let before = af();
        NO_RUBBERBAND.set(true);
        let rejected = p.set_playback_params(1.2, 4, reverb(5));
        NO_RUBBERBAND.set(false);
        assert!(rejected.is_err());
        assert_eq!(p.playback_params(), (0.8, -3, reverb(3)));
        assert_eq!(p.decks.speed(), 0.8, "rejected effects changed crossfade timing");
        assert_eq!(af(), before, "a refused effect changed the existing chain");
        assert_eq!(p.mpv().get_property::<f64>("speed").unwrap(), 0.8);
        for bad in [f64::NAN, f64::INFINITY, 0.0, 0.49, 3.0] {
            assert!(p.set_playback_params(bad, 0, reverb(0)).is_err());
        }
        p.set_playback_params(0.5, 0, reverb(0)).unwrap();
        assert_eq!(p.playback_params().0, 0.5);
        p.set_playback_params(0.8, -3, reverb(3)).unwrap();
        assert!(p.set_playback_params(1.0, 13, reverb(0)).is_err());
        assert!(p.set_playback_params(1.0, 0, reverb(7)).is_err());
        for bad in [
            super::Reverb { preset: 0, ..reverb(3) },
            super::Reverb { dry: 101, ..reverb(3) },
            super::Reverb { wet: 101, ..reverb(3) },
            super::Reverb { decay: f64::NAN, ..reverb(3) },
            super::Reverb { decay: 0.0, ..reverb(3) },
            super::Reverb { decay: 10.1, ..reverb(3) },
            super::Reverb { pre_delay: 201, ..reverb(3) },
            super::Reverb { damping: 499, ..reverb(3) },
            super::Reverb { damping: 20001, ..reverb(3) },
            super::Reverb { reflections: 101, ..reverb(3) },
        ] {
            assert!(p.set_playback_params(1.0, 0, bad).is_err());
            assert_eq!(p.playback_params(), (0.8, -3, reverb(3)));
        }
        p.set_playback_params(1.0, 0, reverb(0)).unwrap();
        assert!(!af().contains("afir") && !af().contains("rubberband"));
    }

    #[test]
    fn reverb_playback_clock_survives_seeks_and_live_controls() {
        use super::Player;
        use std::time::{Duration, Instant};

        let dir = std::env::temp_dir().join("limusic-reverb-clock-test");
        std::fs::create_dir_all(&dir).unwrap();
        let track = dir.join("tone.wav");
        // A seekable stereo/48 kHz fixture; the lavfi source demuxer cannot seek.
        let size = 60_u32 * 48000 * 4;
        let mut wav = Vec::with_capacity(size as usize + 44);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(size + 36).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x02\0");
        wav.extend_from_slice(&48000_u32.to_le_bytes());
        wav.extend_from_slice(&192000_u32.to_le_bytes());
        wav.extend_from_slice(b"\x04\0\x10\0data");
        wav.extend_from_slice(&size.to_le_bytes());
        for i in 0..60 * 48000 {
            let sample =
                (2000.0 * (i as f64 * 440.0 * std::f64::consts::TAU / 48000.0).sin()) as i16;
            wav.extend_from_slice(&sample.to_le_bytes());
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(&track, wav).unwrap();
        let p = Player::new(dir.to_str().unwrap()).unwrap();
        p.mpv().set_property("ao", "null").unwrap();
        p.mpv().set_property("ao-null-untimed", false).unwrap();
        p.set_playback_params(0.8, -1, reverb(3)).unwrap();
        p.load(track.to_str().unwrap(), &Default::default(), None, None).unwrap();
        p.play().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while p.mpv().get_property::<f64>("time-pos").unwrap_or(0.0) < 0.1 {
            assert!(Instant::now() < deadline, "playback never started");
            std::thread::sleep(Duration::from_millis(20));
        }
        // Cover a slow-tempo seek, then a live tempo/pitch change with reverb still active.
        for (speed, pitch, room, seek) in
            [(0.8, -1, reverb(3), Some(10.0)), (1.2, 0, reverb(3), None)]
        {
            p.set_playback_params(speed, pitch, room).unwrap();
            if let Some(position) = seek {
                p.seek(position).unwrap();
            }
            // Settle the output buffer before measuring over two seconds, not a single chunk.
            std::thread::sleep(Duration::from_secs(1));
            let start = p.mpv().get_property::<f64>("time-pos").unwrap();
            if let Some(position) = seek {
                assert!((start - position - speed).abs() < 0.3, "seek landed at {start}");
            }
            let wall = Instant::now();
            std::thread::sleep(Duration::from_secs(2));
            let end = p.mpv().get_property::<f64>("time-pos").unwrap();
            let rate = (end - start) / wall.elapsed().as_secs_f64();
            eprintln!("tempo {speed}, pitch {pitch}, room {room:?}: clock rate {rate:.4}");
            assert!((rate - speed).abs() < 0.05, "wrong playback rate {rate}, expected {speed}");
            assert_eq!(p.mpv().get_property::<f64>("speed").unwrap(), speed);
        }
        p.mpv().command("stop", &[]).unwrap();
    }

    #[test]
    fn reverb_renders_a_tail_and_updates_both_decks() {
        use super::Player;
        use std::collections::HashMap;
        use std::time::{Duration, Instant};
        // Logged mpv errors do not become PlayerEvent failures; capture this regression explicitly.
        use tracing_subscriber::fmt::MakeWriter;
        static LOG: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(|| LOG.make_writer())
            .finish();
        tracing::subscriber::set_global_default(subscriber).unwrap();
        // Exercise BOTH escaping levels and mpv's byte-length quoting, including non-ASCII.
        let dir = std::env::temp_dir().join("limusic-reverb é ' [room],; test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = Player::new(dir.to_str().unwrap()).unwrap();
        p.mpv().set_property("ao", "pcm").unwrap();
        p.mpv().set_property("ao-pcm-waveheader", false).unwrap();
        p.mpv().set_property("audio-format", "s16").unwrap();
        p.mpv().set_property("audio-samplerate", 48000_i64).unwrap();
        p.mpv().set_property("audio-channels", "stereo").unwrap();
        p.mpv().set_property("gapless-audio", "no").unwrap();
        p.mpv().set_property("keep-open", true).unwrap();
        let mut render_index = 0;
        let tempo = std::cell::Cell::new(1.0);
        let padding = std::cell::Cell::new(4.0);
        let mut render = |controls: super::Reverb| {
            let output = dir.join(format!("render-{render_index}.pcm"));
            render_index += 1;
            p.mpv().set_property("ao-pcm-file", output.to_str().unwrap()).unwrap();
            p.set_playback_params(tempo.get(), 0, controls).unwrap();
            // Seeded broadband audio followed by silence checks actual level AND decay.
            p.pause().unwrap();
            p.load(
                &format!(
                    "av://lavfi:anoisesrc=color=pink:amplitude=0.05:seed=42:d=0.4,apad=pad_dur={}",
                    padding.get()
                ),
                &HashMap::new(),
                None,
                None,
            )
            .unwrap();
            p.play().unwrap(); // keep-open pauses at EOF; every render starts playing.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !p.mpv().get_property::<bool>("eof-reached").unwrap_or(false) {
                assert!(
                    Instant::now() < deadline,
                    "{controls:?} never reached EOF (position={:?}, idle={:?})",
                    p.mpv().get_property::<f64>("time-pos"),
                    p.mpv().get_property::<bool>("idle-active")
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            p.mpv().command("stop", &[]).unwrap();
            while p.mpv().get_property::<String>("path").is_ok() {
                assert!(Instant::now() < deadline, "render did not close");
                std::thread::sleep(Duration::from_millis(10));
            }
            let bytes = std::fs::read(&output).unwrap();
            let samples: Vec<i16> =
                bytes.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]])).collect();
            assert!(
                samples.len() >= (48000.0 * 4.0 / tempo.get()) as usize,
                "render produced too little audio"
            );
            assert!(samples.iter().all(|s| s.unsigned_abs() < 32767), "render clipped");
            samples
        };
        let rms = |samples: &[i16], start: f64, end: f64| {
            let window = &samples[(start * 48000.0) as usize * 2..(end * 48000.0) as usize * 2];
            (window.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / window.len() as f64).sqrt()
        };
        let dry = render(reverb(0));
        let mut late = Vec::new();
        for preset in 1..=6 {
            let samples = render(reverb(preset));
            let ratio = rms(&samples, 0.1, 0.4) / rms(&dry, 0.1, 0.4);
            assert!(ratio >= 0.95, "preset {preset} changed song level: {ratio}");
            let tail = rms(&samples, 0.45, 0.6);
            assert!(tail > 1.0, "preset {preset} has no tail: {tail}");
            late.push(rms(&samples, 1.0, 1.3));
            eprintln!(
                "preset {preset}: song RMS ratio={ratio:.3}, tail={tail:.1}, late={:.1}",
                late.last().unwrap()
            );
        }
        assert!(late[4] > late[0] * 10.0, "large hall must linger longer than small room");

        // Controls must change their own signal path, without requiring a listening judgment.
        let half_dry = render(super::Reverb { dry: 50, wet: 0, ..reverb(3) });
        assert!((rms(&half_dry, 0.1, 0.4) / rms(&dry, 0.1, 0.4) - 0.5).abs() < 0.01);
        let muted = render(super::Reverb { dry: 0, wet: 0, ..reverb(3) });
        assert!(muted.iter().all(|s| *s == 0), "zero dry/wet must mute both paths");
        let wet_controls = super::Reverb { dry: 0, wet: 100, ..reverb(3) };
        let wet = render(wet_controls);
        assert!(rms(&wet, 0.01, 0.05) > 1.0, "wet-only output lost its reflections");
        let delayed = render(super::Reverb { pre_delay: 100, ..wet_controls });
        assert_eq!(rms(&delayed, 0.0, 0.1), 0.0, "pre-delay must delay only the wet path");
        assert!(rms(&delayed, 0.11, 0.15) > 1.0);
        let no_reflections = render(super::Reverb { reflections: 0, ..wet_controls });
        assert_eq!(
            rms(&no_reflections, 0.0, 0.07),
            0.0,
            "early reflection control did not silence the early IR"
        );
        assert!(rms(&no_reflections, 0.45, 0.6) > 1.0, "early control removed the late tail");
        let short = render(super::Reverb { decay: 0.3, ..wet_controls });
        let long = render(super::Reverb { decay: 3.0, ..wet_controls });
        assert!(
            rms(&long, 1.0, 1.3) > rms(&short, 1.0, 1.3) + 1.0,
            "decay did not extend the tail"
        );
        let dark = render(super::Reverb { damping: 500, ..wet_controls });
        let bright = render(super::Reverb { damping: 20000, ..wet_controls });
        let high_energy = |samples: &[i16]| {
            samples[9600..38400]
                .windows(3)
                .map(|s| (f64::from(s[2]) - f64::from(s[0])).powi(2))
                .sum::<f64>()
        };
        assert!(
            high_energy(&dark) < high_energy(&bright) * 0.5,
            "damping did not reduce high frequencies"
        );

        // Reverb follows stretching: its envelope and pre-delay use listening seconds.
        let tail_drop = |samples: &[i16], end: f64| {
            20.0 * (rms(samples, end + 0.6, end + 0.75) / rms(samples, end + 0.1, end + 0.25))
                .log10()
        };
        let reference_drop = tail_drop(&wet, 0.4);
        for speed in [0.5, 0.8, 1.2, 2.0] {
            tempo.set(speed);
            let samples = render(wet_controls);
            let duration = samples.len() as f64 / 48000.0 / 2.0;
            // Rubber Band rounds its real-time EOF flush to processing blocks:
            // https://breakfastquay.com/rubberband/integration.html
            // Ubuntu 24.04 renders 2.246667 s at 2x: 46.667 ms extra output (93.333 ms
            // in source time). Only that upper bound grows; other speeds and truncation
            // retain the original 80 ms source-time limit. The isolated stage comparison
            // checks that reverb and the speed adapter add no samples of their own.
            let error = duration * speed - 4.4;
            let max_extra = if speed == 2.0 { 0.10 } else { 0.08 };
            assert!(error > -0.08 && error < max_extra, "tempo {speed}: wrong duration {duration}");
            if speed == 2.0 {
                // A fixed EOF allowance must not hide an incorrect stretching rate.
                // Extra source silence should add exactly its scaled duration, apart
                // from block rounding, even if the graph has a fixed buffered tail.
                padding.set(8.0);
                let longer = render(wet_controls);
                padding.set(4.0);
                let added = (longer.len() as f64 - samples.len() as f64) / 48000.0 / 2.0;
                assert!(
                    (added - 4.0 / speed).abs() < 0.04,
                    "tempo {speed}: added duration {added}"
                );
            }
            let drop = tail_drop(&samples, 0.4 / speed);
            eprintln!("tempo {speed}: tail drop {drop:.2} dB, reference {reference_drop:.2} dB");
            assert!((drop - reference_drop).abs() < 5.0, "tempo {speed} changed room decay");
            let delayed = render(super::Reverb { pre_delay: 100, ..wet_controls });
            let added = (delayed.len() as f64 - samples.len() as f64) / 48000.0 / 2.0;
            assert!((added - 0.1).abs() < 0.01, "tempo {speed} changed pre-delay: {added}");
        }
        tempo.set(1.0);

        let other = p.idle_mpv().unwrap();
        other.set_property("ao", "null").unwrap();
        super::preload(&p.decks, &other, "av://lavfi:sine=f=440:d=3").unwrap();
        // During an overlap, the outgoing and incoming tracks keep their own normalization.
        p.set_gain(Some(-6.0)).unwrap();
        p.decks.active.store(1, std::sync::atomic::Ordering::SeqCst);
        p.set_gain(Some(-2.5)).unwrap();
        p.set_playback_params(0.8, -1, reverb(2)).unwrap();
        for deck in 0..2 {
            let m = p.decks.mpv(deck).unwrap();
            let chain = m.get_property::<String>("af").unwrap();
            assert!(chain.contains("afir"));
            assert!(chain.contains("rubberband"));
            assert!(chain.contains(if deck == 0 { "volume=-6dB" } else { "volume=-2.5dB" }));
            assert_eq!(m.get_property::<f64>("speed").unwrap(), 0.8);
        }
        p.set_pitch(0).unwrap();
        p.set_speed(1.2).unwrap();
        for deck in 0..2 {
            let m = p.decks.mpv(deck).unwrap();
            let chain = m.get_property::<String>("af").unwrap();
            assert!(chain.contains("afir") && chain.contains("rubberband"));
            assert!(chain.contains(if deck == 0 { "volume=-6dB" } else { "volume=-2.5dB" }));
            assert_eq!(m.get_property::<f64>("speed").unwrap(), 1.2);
        }
        p.set_speed(1.0).unwrap();
        for deck in 0..2 {
            let chain = p.decks.mpv(deck).unwrap().get_property::<String>("af").unwrap();
            assert!(chain.contains("afir"));
            assert!(!chain.contains("rubberband") && !chain.contains("scaletempo"));
        }
        p.set_playback_params(1.0, 0, reverb(0)).unwrap();
        for deck in 0..2 {
            assert!(!p
                .decks
                .mpv(deck)
                .unwrap()
                .get_property::<String>("af")
                .unwrap()
                .contains("afir"));
        }

        // An unwritable asset must not change the controls or poison the next filter update.
        let asset = super::reverb_path(&p.decks, 3);
        std::fs::remove_file(&asset).unwrap();
        std::fs::create_dir(&asset).unwrap();
        assert!(p.set_playback_params(0.8, 0, reverb(3)).is_err());
        assert_eq!(p.playback_params(), (1.0, 0, reverb(0)));
        std::fs::remove_dir(&asset).unwrap();
        p.set_playback_params(1.0, 0, reverb(3)).unwrap();
        let log = String::from_utf8(LOG.lock().unwrap().clone()).unwrap();
        assert!(!log.contains("speed changing filters before libavfilter"), "{log}");
        assert!(!log.contains("Channel layout is not set"), "{log}");
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
        assert_eq!(loadfile_args("u", None, true), vec!["\"u\"", "replace"]);
        // Anything at or below 0 is the default position, so it is not worth the option, and a
        // NaN or infinity must never reach mpv.
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                loadfile_args("u", Some(bad), true),
                vec!["\"u\"", "replace"],
                "start={bad}"
            );
        }
        // On mpv 0.38+ the `index` positional has to be present for `options` to be read, and
        // before 0.38 it doesn't exist (issue #384).
        assert_eq!(
            loadfile_args("u", Some(605.0), true),
            vec!["\"u\"", "replace", "-1", "\"start=605\""]
        );
        assert_eq!(
            loadfile_args("u", Some(605.0), false),
            vec!["\"u\"", "replace", "\"start=605\""]
        );
        // Fractional resume positions are what `state::pending_seek` actually carries.
        assert_eq!(
            loadfile_args("u", Some(8.6155624669999), true),
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
    fn crossfade_does_not_wait_for_effect_updates() {
        use super::{maybe_crossfade, Ordering, Player};
        use std::sync::mpsc;

        let dir = tempfile::tempdir().unwrap();
        let p = Player::new(dir.path().to_str().unwrap()).unwrap();
        p.mpv().set_property("ao", "null").unwrap();
        let incoming = p.idle_mpv().unwrap();
        incoming.set_property("ao", "null").unwrap();
        p.set_speed(0.8).unwrap();
        p.set_crossfade(Some(1.5));
        // An effects update may have reached the incoming deck before it commits its tempo.
        incoming.set_property("speed", 1.2).unwrap();
        for (position, active) in [(0.0, 0), (9.0, 1)] {
            p.decks.preloaded.store(true, Ordering::Release);
            let effects = p.decks.af.lock().unwrap();
            let decks = p.decks.clone();
            let (tx, rx) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                maybe_crossfade(&decks, 0, position, 10.0);
                tx.send(()).unwrap();
            });
            let finished = rx.recv_timeout(std::time::Duration::from_secs(1));
            // Release before joining so a regression fails instead of hanging the test suite.
            drop(effects);
            worker.join().unwrap();
            assert!(finished.is_ok(), "crossfade blocked on effects at position {position}");
            assert_eq!(p.decks.active.load(Ordering::SeqCst), active);
            assert_eq!(incoming.get_property::<f64>("speed").unwrap(), 1.2);
        }
        p.stop().unwrap();
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

    /// A Listen Together host broadcasts `Paused`, so a track running out must not produce one:
    /// it reached the room after the next track had started, with the old track's end as the
    /// position, and guests seeked the new track past its own end.
    #[test]
    fn running_out_is_not_a_pause() {
        use super::{Player, PlayerEvent};
        use std::time::{Duration, Instant};

        let dir = std::env::temp_dir().join("limusic-pause-test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Player::new(dir.to_str().unwrap()).expect("libmpv");
        p.mpv().set_property("ao", "null").unwrap();
        let mut rx = p.take_events().unwrap();
        let mut until = |done: fn(&PlayerEvent) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut seen = Vec::new();
            while Instant::now() < deadline {
                match rx.try_recv() {
                    Ok(e) => {
                        let stop = done(&e);
                        seen.push(e);
                        if stop {
                            return seen;
                        }
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            panic!("timed out, saw {seen:?}");
        };

        p.load("av://lavfi:sine=duration=0.3", &Default::default(), None, None).unwrap();
        let _ = p.play();
        // Through the end of the file and mpv going idle, which is the `Playing(false)` that used
        // to be broadcast as a pause.
        let seen = until(|e| matches!(e, PlayerEvent::Playing(false)));
        assert!(seen.iter().any(|e| matches!(e, PlayerEvent::TrackEnded)), "{seen:?}");
        assert!(!seen.iter().any(|e| matches!(e, PlayerEvent::Paused(_))), "{seen:?}");

        p.pause().unwrap();
        until(|e| matches!(e, PlayerEvent::Paused(true)));
    }

    /// A resume rides `loadfile`'s `start=`, whose position in the argument list moved in mpv
    /// 0.38. The AppImage bundles 0.37, so run this against that library too (issue #384):
    /// `LD_LIBRARY_PATH=<extracted AppImage>/usr/lib cargo test -p player start_lands`.
    #[test]
    fn start_lands_on_this_libmpv() {
        use super::Player;
        use std::time::{Duration, Instant};

        let dir = std::env::temp_dir().join("limusic-start-test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = Player::new(dir.to_str().unwrap()).expect("libmpv");
        p.mpv().set_property("ao", "null").unwrap();
        // time-pos can precede mpv's initial start seek. Pausing makes this wait meaningful:
        // an ignored start option cannot accidentally pass by playing from zero to two seconds.
        p.mpv().set_property("pause", true).unwrap();
        p.load("av://lavfi:sine=duration=5", &Default::default(), None, Some(2.0)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let pos = loop {
            if let Ok(pos) = p.mpv().get_property::<f64>("time-pos") {
                if (1.9..3.0).contains(&pos) {
                    break pos;
                }
            }
            assert!(Instant::now() < deadline, "file never loaded");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!((1.9..3.0).contains(&pos), "started at {pos}");
    }
}
