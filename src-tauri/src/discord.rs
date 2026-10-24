//! Discord Rich Presence via Discord's local IPC socket.
//!
//! Metrolist hand-rolls an OAuth2+PKCE login and a raw Gateway client because Android has no
//! Discord app to talk to — it has to authenticate *as the user* and push `PRESENCE_UPDATE`
//! itself. On desktop that whole stack is unnecessary: the running Discord client exposes an IPC
//! socket that accepts a presence payload directly. So there is no login, no token store, no
//! heartbeat, no reconnect strategy, and no `external-assets` header spoof — the local client does
//! all of it, and the identity is simply whoever is signed into Discord.
//!
//! Mirrors `media.rs`: the IPC client is blocking and stateful, so it gets an owner thread fed by a
//! channel. Everything here is best-effort — Discord not running, or quitting mid-song, is a
//! `debug!` line, never an error the user sees (context/16 fail-soft, same as the OS media widget).
//!
//! Presence is shown **only while actually playing**, unless the user turns on "show when paused"
//! (issue #226). Even then, a card needs playback to have started at least once this session: the
//! queue restored on launch is paused too, and a frozen "Listening to…" for a track nobody pressed
//! play on is worse than no card. A paused card also carries no timeline, because Discord has no
//! paused state and its progress bar would keep running.
//!
//! Timeline correctness hinges on three rules:
//! 1. A track change resets the thread's position to 0 — the app only ever starts tracks from the
//!    top, and trusting the previous track's stale position put every new song minutes in.
//! 2. Position messages carry the `Instant` they were *sent*, and the thread age-corrects — the
//!    event pump can lag seconds behind mpv (gapless advance resolves the next track over the
//!    network before draining more events), and an un-aged position "corrects" the timeline
//!    backwards.
//! 3. Only real mpv pause-flag transitions change `playing` — mpv fires `time-pos` on seeks while
//!    paused, so a position tick must never be treated as proof of playback.
//!
//! The socket is opened as soon as presence is enabled, not lazily on the first card — connecting
//! on demand meant the first song of a session waited out a connect round-trip (and a single failed
//! attempt stalled it for the whole retry interval) before anything showed up.
//!
//! Sending is rate-limited, because Discord silently drops presence updates that arrive too close
//! together — and a dropped update is invisible (the socket ACKs it). Four rules keep state from
//! getting stranded behind that: a **trailing-edge floor** (when a push is due but too soon, the
//! loop sleeps until the floor expires and then sends whatever is current — never discards it),
//! a short **grace** on a brand-new track so its length can land before the first push, a
//! **frame window** (`MAX_FRAMES` per `FRAME_WINDOW`) ensuring we do not exceed Discord's rate limits,
//! and a **track settle** (`TRACK_SETTLE`) that collapses rapid skip bursts into one push. Without
//! the grace, a track change pushed a bar-less card and needed a second push milliseconds later,
//! which Discord dropped — leaving the card stuck as an elapsed counter with no progress bar.

use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use innertube::SongItem;

/// Discord application id (a snowflake — digits only). **Must be set before rich presence does
/// anything.** Register an app named "Limusic" at <https://discord.com/developers/applications> and
/// paste its Application ID here — the app's *name* is what renders after "Listening to", and its
/// icon is the fallback artwork. Nothing else in the portal needs configuring: no bot user, no
/// OAuth redirect, no client secret. (Metrolist needs all of that only because Android has no
/// Discord client.)
const APP_ID: &str = "1525891596804161727";

const SONG_URL: &str = "https://music.youtube.com/watch?v=";
const ARTIST_URL: &str = "https://music.youtube.com/channel/";
const ALBUM_URL: &str = "https://music.youtube.com/browse/";
const REPO_URL: &str = "https://github.com/SimoHypers/limusic";
/// The small "provider badge" Discord draws in the corner of the artwork, the way Spotify's and
/// Apple Music's cards do (issue #226). Served straight from the repo so there is no asset to
/// register in the Discord developer portal and nothing to keep in sync at release time; if this
/// path ever moves, the badge silently stops rendering and nothing else breaks.
const BADGE_URL: &str =
    "https://raw.githubusercontent.com/SimoHypers/limusic/master/src-tauri/icons/128x128.png";

/// Reconnect backoff while enabled but unconnected (Discord not running, or it quit). Starts short
/// — Discord may simply be slower to start than we are — and eases off so a permanently-absent
/// Discord costs one wakeup every `CONNECT_RETRY_MAX`.
const CONNECT_RETRY_MIN: Duration = Duration::from_secs(1);
const CONNECT_RETRY_MAX: Duration = Duration::from_secs(15);
/// How long to park when nothing is pending. Any message wakes the thread anyway.
const IDLE_TICK: Duration = Duration::from_secs(15);
/// A position this far off the timeline we last pushed means the user seeked, so the progress bar
/// needs re-pushing. Anything smaller is just clock/tick noise.
const SEEK_DRIFT_MS: i64 = 2_000;
/// Never send two updates inside this window — Discord drops the second, and does so silently.
/// Pending state is not discarded: the loop wakes when the floor expires and sends the latest.
const SEND_FLOOR: Duration = Duration::from_millis(1_500);
/// A playing card is re-sent after this long so a silently lost activity heals itself (45 s:
/// about two frame windows, so at most one refresh falls in any window and it takes at most one
/// slot, yet a lost activity is repaired within a minute).
const REFRESH_INTERVAL: Duration = Duration::from_secs(45);
/// How long a new track waits for mpv to report its length before we give up and push a card with
/// no progress bar. Collapses the track-change burst (track + length + play state) into one push.
const DURATION_GRACE: Duration = Duration::from_millis(800);
/// Discord gateway docs state: "Clients may only update their game status 5 times per 20 seconds"
/// (<https://docs.discord.com/developers/events/gateway-events>, Activity Object section). The local
/// client forwards our IPC updates to the gateway so this limit applies to us, and updates beyond it
/// are dropped silently.
const MAX_FRAMES: usize = 5;
/// The sliding window MAX_FRAMES is counted in (21 s: 20 s plus one second of slack).
const FRAME_WINDOW: Duration = Duration::from_secs(21);
/// A skip burst arrives as track changes within about a second of each other; waiting this long
/// after a change (when a card is already up) collapses the burst into one push (1 s: long enough
/// to span the gap between presses in a skip burst, short enough that a single skip still shows
/// up almost immediately).
const TRACK_SETTLE: Duration = Duration::from_millis(1000);
/// Floor for any computed wait — handing `recv_timeout` a zero duration would spin.
const MIN_WAIT: Duration = Duration::from_millis(10);
/// Discord rejects `details`/`state`/`large_text` outside 2–128 characters.
const MAX_FIELD: usize = 128;
/// Discord rejects asset URLs longer than this — better a text-only card than a rejected payload.
const MAX_ASSET_URL: usize = 256;
/// Discord rejects a button whose URL is longer than this, and with it the whole payload.
const MAX_BUTTON_URL: usize = 512;

/// How the card is assembled, from the user's Discord settings tab. Persisted as one JSON blob in
/// the `discord_rpc_config` setting, so a new knob is a field here plus a control in the UI rather
/// than another row in `UI_SETTINGS`. Every field is a string the UI also understands, because the
/// settings tab renders a live preview of this exact card and the two have to agree.
///
/// Unknown or missing fields fall back to [`Default`], which reproduces the card Limusic showed
/// before any of this was configurable, with one deliberate exception: [`Self::badge`] is on, so an
/// existing install that never opens the tab gains the corner icon the way Spotify's and Apple
/// Music's cards have one.
#[derive(Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct RpcConfig {
    /// Which slot Discord renders after "Listening to": `app` | `line1` | `line2`.
    status_line: String,
    /// Replaces the registered application name in that slot. Empty keeps "Limusic".
    app_name: String,
    /// What the card's first line (`details`) carries: `title` | `artist` | `album`.
    line1: String,
    /// The second line (`state`): `title` | `artist` | `album` | `artist_album` | `off`.
    line2: String,
    /// Show the album artwork at all. Also gates [`Self::line3`], which Discord only renders as
    /// part of the assets block.
    cover: bool,
    /// The card's third line: `album` | `title` | `artist` | `off`. This is `assets.large_text`,
    /// which Discord draws under line 2 *and* reuses as the artwork's hover text — it is not a
    /// tooltip-only field, which is what it looked like until a real card was inspected.
    line3: String,
    /// Make line 1 / line 2 / the artwork open the thing they name. The target follows the content:
    /// a line showing the album links the album, one showing the artist links the artist.
    link_line1: bool,
    link_line2: bool,
    link_cover: bool,
    /// Show the elapsed/remaining progress bar. Ignored while paused: Discord has no paused state,
    /// so a bar left on a paused card keeps advancing and lies.
    timestamps: bool,
    /// Draw the Limusic badge in the corner of the artwork. Needs [`Self::cover`]: Discord renders
    /// `small_image` as a badge *over* the large one, and on its own it would become the artwork.
    badge: bool,
    /// Keep the card up while paused. Gated on playback having started at least once this session,
    /// so the queue restored at launch does not post a card for a track nobody played.
    show_paused: bool,
    /// Say only that music is playing: no title, artist, album, artwork, bar or buttons. Overrides
    /// everything else on this struct, which is why the settings tab hides the rest when it is on.
    hide_details: bool,
    /// The two buttons, in card order: `listen` | `album` | `artist` | `app` | `off`.
    button1: String,
    button2: String,
}

impl Default for RpcConfig {
    fn default() -> Self {
        RpcConfig {
            status_line: "line2".into(),
            app_name: String::new(),
            line1: "title".into(),
            line2: "artist".into(),
            cover: true,
            line3: "album".into(),
            link_line1: false,
            link_line2: false,
            link_cover: false,
            timestamps: true,
            badge: true,
            show_paused: false,
            hide_details: false,
            button1: "listen".into(),
            button2: "app".into(),
        }
    }
}

impl RpcConfig {
    /// Parse the stored blob. Anything unparseable is the default card, never an error the user
    /// sees: a corrupt setting must not take rich presence down with it.
    pub fn parse(json: Option<&str>) -> Self {
        json.and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default()
    }
}

enum Msg {
    Track(Box<Track>),
    /// The playing track's album, learned after it started. Unlike `Track`, keeps the timeline.
    Album {
        video_id: String,
        album: String,
    },
    Duration(f64),
    /// A position tick. `at` is when the value was read — the thread ages it before use.
    Position {
        pos: f64,
        at: Instant,
    },
    /// A real play/pause transition (mpv's pause flag), never inferred from position ticks.
    Playing(bool),
    Enabled(bool),
    Config(Box<RpcConfig>),
}

#[derive(Clone)]
struct Track {
    video_id: String,
    title: String,
    artists: String,
    artist_id: Option<String>,
    album: Option<String>,
    album_id: Option<String>,
    thumbnail: Option<String>,
    /// A file on this machine: it has no YouTube page, and its "id" is a path. Nothing about it may
    /// end up in a URL on the user's profile.
    local: bool,
}

/// App-side handle to the presence thread. `None` when the thread couldn't be spawned; every push
/// is then a no-op.
pub struct DiscordHandle {
    tx: Sender<Msg>,
}

impl DiscordHandle {
    pub fn set_track(&self, item: &SongItem) {
        let local = crate::local::is_local_song(&item.video_id);
        let _ = self.tx.send(Msg::Track(Box::new(Track {
            video_id: item.video_id.clone(),
            title: item.title.clone(),
            artists: item.artists.clone(),
            artist_id: item.artist_id.clone(),
            album: item.album.clone(),
            album_id: item.album_id.clone(),
            // Upscale here (stored thumbs are often 60px) and drop over-long URLs outright.
            // A local track's artwork is a path on this machine: Discord can't fetch it, and it
            // would put the user's directory layout on their profile. Send none.
            thumbnail: item.thumbnail.as_deref().filter(|_| !local).and_then(discord_thumb),
            local,
        })));
    }

    /// Same as `LastfmHandle::set_album`: a card played from search has no album until the radio
    /// behind it arrives (#309), and `set_track` would reset the card's timeline.
    pub fn set_album(&self, video_id: &str, album: &str) {
        let _ = self.tx.send(Msg::Album { video_id: video_id.to_owned(), album: album.to_owned() });
    }

    /// Convert mpv's track seconds to listening seconds for Discord's wall-clock timeline.
    pub fn set_duration(&self, secs: f64, speed: f64) {
        let _ = self.tx.send(Msg::Duration(secs / speed));
    }

    /// A raw position tick. Carries no play/pause authority — mpv also ticks on paused seeks.
    pub fn set_position(&self, pos: f64, speed: f64) {
        let _ = self.tx.send(Msg::Position { pos: pos / speed, at: Instant::now() });
    }

    pub fn set_playing(&self, playing: bool) {
        let _ = self.tx.send(Msg::Playing(playing));
    }

    pub fn set_enabled(&self, on: bool) {
        let _ = self.tx.send(Msg::Enabled(on));
    }

    /// Apply a new card layout. Re-pushes the current track immediately (subject to the send
    /// floor), so the settings tab's preview and the real card can't disagree.
    pub fn set_config(&self, cfg: RpcConfig) {
        let _ = self.tx.send(Msg::Config(Box::new(cfg)));
    }
}

/// Spawn the presence owner thread. `enabled` is the persisted `discord_rpc` setting — when off,
/// the thread parks on the channel and never opens a socket.
pub fn spawn(enabled: bool, cfg: RpcConfig) -> Option<DiscordHandle> {
    // Without a real app id there is nothing to connect *as*. Say so once, loudly, rather than
    // leaving a settings toggle that silently does nothing.
    if APP_ID.is_empty() || !APP_ID.bytes().all(|b| b.is_ascii_digit()) {
        tracing::warn!(APP_ID, "discord rich presence disabled: APP_ID is not a Discord app id");
        return None;
    }
    let (tx, rx) = channel::<Msg>();
    match std::thread::Builder::new()
        .name("discord-rpc".into())
        .spawn(move || run(rx, Presence::new(enabled, cfg)))
    {
        Ok(_) => Some(DiscordHandle { tx }),
        Err(e) => {
            tracing::warn!(error = %e, "discord-rpc thread spawn failed");
            None
        }
    }
}

fn run(rx: Receiver<Msg>, mut p: Presence) {
    // Sync once up front: when presence is already enabled at launch this opens the socket now,
    // rather than on the first song — the loop below would otherwise just block on `recv`.
    let mut wait = p.sync();
    loop {
        // Block for the first message, then drain everything queued behind it so a burst
        // (Track + Duration + Playing on a track change) becomes one push, not three — and so a
        // backlog applies in order before we decide what to show.
        match rx.recv_timeout(wait) {
            Ok(msg) => {
                p.apply(msg);
                loop {
                    match rx.try_recv() {
                        Ok(msg) => p.apply(msg),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            p.disconnect();
                            return;
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // Sender dropped: the app is shutting down. Clear the presence on the way out.
            Err(RecvTimeoutError::Disconnected) => {
                p.disconnect();
                return;
            }
        }
        wait = p.sync();
    }
}

/// What the thread should do right now. Pure — no socket, no state mutation — so the timing rules
/// (floor, grace, dedup) are unit-testable without a running Discord.
#[derive(Debug, PartialEq)]
enum Act {
    /// Nothing pending — sleep until the next reconnect tick.
    Idle,
    /// Something is pending but not allowed yet. Sleep this long, then re-decide.
    Wait(Duration),
    Push,
    Clear,
}

/// What we want Discord to show, what we last actually pushed, and the socket in between.
struct Presence {
    enabled: bool,
    cfg: RpcConfig,
    client: Option<DiscordIpcClient>,
    last_connect_try: Option<Instant>,
    /// Grows on each failed connect, resets on success. See [`CONNECT_RETRY_MIN`].
    connect_backoff: Duration,
    // desired
    track: Option<Track>,
    /// When the current track arrived — the duration grace runs from here.
    track_at: Instant,
    duration: f64,
    playing: bool,
    /// Has anything actually played this session? The restored queue arrives paused, and
    /// "show when paused" must not resurrect it into a card nobody asked for. Never reset on a
    /// track change: a gapless advance sends no play-state message (mpv's pause flag never moved),
    /// so clearing it there would silently disable the setting mid-queue.
    played: bool,
    /// Last known position + when it was read. `estimate()` ages it while playing.
    pos: f64,
    pos_at: Instant,
    // pushed — `Some` iff a card is currently shown (cards exist only while playing)
    sent: Option<Sent>,
    /// The layout changed since the last push. Kept apart from `sent`, which has to stay the
    /// record of what Discord is showing: dropping `sent` to force a re-push also told `plan()`
    /// there was nothing to take down, so turning "show while paused" off while paused left the
    /// old card up until the next song.
    cfg_dirty: bool,
    last_send: Option<Instant>,
    sends: VecDeque<Instant>,
}

struct Sent {
    video_id: String,
    /// Was the card pushed as a playing one? A paused card carries no timeline, so resuming has to
    /// re-push even though nothing about the track changed.
    playing: bool,
    /// Epoch millis the pushed timeline started at — `now - start_ms` is where Discord's bar is.
    start_ms: i64,
    /// The length the pushed bar was built from; 0 when the card went out without one.
    duration: f64,
}

impl Presence {
    fn new(enabled: bool, cfg: RpcConfig) -> Self {
        Presence {
            enabled,
            cfg,
            client: None,
            last_connect_try: None,
            connect_backoff: CONNECT_RETRY_MIN,
            track: None,
            track_at: Instant::now(),
            duration: 0.0,
            playing: false,
            played: false,
            pos: 0.0,
            pos_at: Instant::now(),
            sent: None,
            cfg_dirty: false,
            last_send: None,
            sends: VecDeque::new(),
        }
    }

    /// Best current position: the last tick, aged by wall time while playing. Ticks arrive
    /// throttled (~1s) and can queue behind slow event-pump work, so the raw value alone lies.
    fn estimate(&self) -> f64 {
        if self.playing {
            self.pos + self.pos_at.elapsed().as_secs_f64()
        } else {
            self.pos
        }
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Track(t) => {
                self.track = Some(*t);
                self.track_at = Instant::now();
                // Every track starts from the top in this app; the previous track's position must
                // not leak into the new card. (The one nonzero start — queue restore — arrives
                // paused, where no card is shown, and its real position ticks in before play.)
                self.pos = 0.0;
                self.pos_at = Instant::now();
                self.duration = 0.0; // length unknown until mpv reports it (grace waits for it)
            }
            // Dirty, since `wants_push` compares the track id and this one hasn't changed.
            Msg::Album { video_id, album } => {
                if let Some(t) = self.track.as_mut().filter(|t| t.video_id == video_id) {
                    t.album = Some(album);
                    self.cfg_dirty = true;
                }
            }
            Msg::Duration(secs) => self.duration = secs,
            Msg::Position { pos, at } => {
                self.pos = pos;
                self.pos_at = at;
            }
            Msg::Playing(on) => {
                self.played |= on;
                if self.playing != on {
                    // Fold the accrued playtime into `pos` before flipping, so pausing freezes the
                    // estimate and resuming restarts the clock from the frozen value.
                    self.pos = self.estimate();
                    self.pos_at = Instant::now();
                    self.playing = on;
                }
            }
            Msg::Enabled(on) => {
                if self.enabled != on {
                    self.enabled = on;
                    if !on {
                        self.disconnect();
                    }
                }
            }
            // The dirty flag is what makes the change visible: `wants_push` compares the *track*,
            // not the rendered card, so an unchanged track would otherwise keep the old layout on
            // screen until the next song.
            Msg::Config(cfg) => {
                if self.cfg != *cfg {
                    self.cfg = *cfg;
                    self.cfg_dirty = true;
                }
            }
        }
    }

    /// Reconcile Discord with the desired state, returning how long to sleep before re-deciding.
    fn sync(&mut self) -> Duration {
        if !self.enabled {
            return IDLE_TICK;
        }
        // Hold the socket open for as long as presence is on, so a track change finds it already
        // there. Internally throttled by the backoff, so this is a no-op once connected.
        self.ensure_connected();

        let wait = match self.plan() {
            Act::Idle => IDLE_TICK,
            Act::Wait(d) => d,
            Act::Push => {
                if self.ensure_connected() {
                    self.push_card();
                }
                IDLE_TICK
            }
            Act::Clear => {
                self.clear_shown();
                IDLE_TICK
            }
        };
        // Still down? Wake for the next connect attempt, whichever comes first.
        match self.connect_backoff_remaining() {
            Some(rem) if self.client.is_none() => wait.min(rem.max(MIN_WAIT)),
            _ => wait,
        }
    }

    /// A card while playing, nothing otherwise — subject to the duration grace and the send floor.
    fn plan(&self) -> Act {
        if !self.enabled {
            return Act::Idle;
        }
        let want_card =
            self.track.is_some() && (self.playing || (self.cfg.show_paused && self.played));
        if want_card {
            if !self.wants_push() {
                return Act::Idle;
            }
            if self.sent.is_some() && self.is_new_card() {
                if let Some(rem) = TRACK_SETTLE.checked_sub(self.track_at.elapsed()) {
                    return Act::Wait(rem.max(MIN_WAIT));
                }
            }
            // A new track whose length mpv hasn't reported yet: hold the first push briefly. Push
            // now and we'd show a bar-less card, then need a second push the moment the length
            // lands — which Discord drops for arriving too soon, stranding the card without its bar.
            if self.duration <= 0.0 && self.is_new_card() {
                if let Some(rem) = DURATION_GRACE.checked_sub(self.track_at.elapsed()) {
                    return Act::Wait(rem.max(MIN_WAIT));
                }
            }
        } else if self.sent.is_none() {
            return Act::Idle; // nothing shown, nothing to take down
        }
        // Trailing edge: too soon to send, so come back when the floor expires — and send whatever
        // is current *then*. Never drop the update; a dropped one is invisible (the socket ACKs it).
        if let Some(rem) = self.floor_remaining() {
            return Act::Wait(rem.max(MIN_WAIT));
        }
        if want_card {
            Act::Push
        } else {
            Act::Clear
        }
    }

    /// Would Discord's card differ from what we last pushed? This is what keeps the ~1s position
    /// ticks from becoming one presence update per second.
    fn wants_push(&self) -> bool {
        let Some(track) = &self.track else { return false };
        let Some(sent) = &self.sent else { return true };
        if self.cfg_dirty {
            return true;
        }
        if sent.video_id != track.video_id {
            return true;
        }
        // Pausing or resuming changes the card itself (the timeline goes away and comes back), so
        // it has to re-push even though the track did not change.
        if sent.playing != self.playing {
            return true;
        }
        // mpv reported the length, or refined it (streams get probed, then corrected) — the bar's
        // end moves. Also the retry path when a first push went out before the length landed.
        if (self.duration - sent.duration).abs() > 1.0 {
            return true;
        }
        // A paused card has no timeline to drift against, and `now_ms()` keeps moving while the
        // frozen position does not — without this the seek check would re-push once per tick.
        if !self.playing {
            return false;
        }
        // A playing card is re-sent so Discord recovers from a silently dropped update; a paused
        // card is not, there is nothing to recover and it would send all night.
        if self.last_send.is_some_and(|t| t.elapsed() >= REFRESH_INTERVAL) {
            return true;
        }
        // Seek detection: our aged position no longer matches the timeline we pushed. Doing it
        // here, off the pushed timestamps, catches every seek — UI, media key, or Listen Together —
        // for free, instead of hooking each caller of `player.seek`.
        let drift_ms = (self.estimate() * 1000.0) as i64 - (now_ms() - sent.start_ms);
        drift_ms.abs() > SEEK_DRIFT_MS
    }

    /// No card up, or the one up is for a different track.
    fn is_new_card(&self) -> bool {
        match (&self.sent, &self.track) {
            (Some(sent), Some(track)) => sent.video_id != track.video_id,
            _ => true,
        }
    }

    fn note_frame(&mut self) {
        let now = Instant::now();
        self.sends.push_back(now);
        while let Some(&t) = self.sends.front() {
            if now.duration_since(t) > FRAME_WINDOW {
                self.sends.pop_front();
            } else {
                break;
            }
        }
        self.last_send = Some(now);
    }

    /// Time left on the send floor, or `None` when we're clear to send.
    fn floor_remaining(&self) -> Option<Duration> {
        let floor = SEND_FLOOR.checked_sub(self.last_send?.elapsed());
        let window = if self.sends.len() >= MAX_FRAMES {
            self.sends.front().and_then(|&oldest| FRAME_WINDOW.checked_sub(oldest.elapsed()))
        } else {
            None
        };
        match (floor, window) {
            (Some(f), Some(w)) => Some(f.max(w)),
            (Some(f), None) => Some(f),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        }
    }

    fn push_card(&mut self) {
        // Cloned, not borrowed, so the socket below can be taken out of `self`. Only runs on a real
        // change (`needs_push` gates it), never on a plain position tick.
        let (Some(track), Some(mut client)) = (self.track.clone(), self.client.take()) else {
            return;
        };

        // Timestamps are epoch MILLIseconds (the crate's documented contract — Discord happens to
        // normalize second-scale values, but that's their heuristic, not the API).
        let pos = self.estimate().max(0.0);
        let start_ms = now_ms() - (pos * 1000.0) as i64;
        let end_ms = (self.duration > 0.0).then(|| start_ms + (self.duration * 1000.0) as i64);

        let mut ts = activity::Timestamps::new().start(start_ms);
        if let Some(end) = end_ms {
            ts = ts.end(end);
        }
        let cfg = self.cfg.clone();
        let mut act = activity::Activity::new().activity_type(activity::ActivityType::Listening);
        if !cfg.app_name.is_empty() {
            act = act.name(field(&cfg.app_name));
        }

        // "Hide details": the whole point is that nothing about the track leaves this machine, so
        // it short-circuits before any of it is attached. What friends see is the bare
        // "Listening to Limusic" line, which is the only slot left to display.
        if cfg.hide_details {
            act = act.status_display_type(activity::StatusDisplayType::Name);
        } else {
            // `details` is the one line Discord will not render a card without, so a slot whose
            // content this track lacks (an album-less single) falls back to the title — and the link
            // follows that fallback, or the line renders as the title and opens nothing.
            let line1 = text_for(&cfg.line1, &track);
            let line1_slot = if line1.is_some() { cfg.line1.as_str() } else { "title" };
            act = act.details(field(&line1.unwrap_or_else(|| track.title.clone())));
            // No bar on a paused card: Discord has no paused state, so the one we sent would keep
            // running and show the track finishing while it sits still.
            if cfg.timestamps && self.playing {
                act = act.timestamps(ts);
            }
            act = act.status_display_type(match cfg.status_line.as_str() {
                "app" => activity::StatusDisplayType::Name,
                "line1" => activity::StatusDisplayType::Details,
                _ => activity::StatusDisplayType::State,
            });
            if cfg.link_line1 {
                if let Some(url) = link_for(line1_slot, &track) {
                    act = act.details_url(url);
                }
            }
            if let Some(line2) = text_for(&cfg.line2, &track) {
                act = act.state(field(&line2));
                if cfg.link_line2 {
                    if let Some(url) = link_for(&cfg.line2, &track) {
                        act = act.state_url(url);
                    }
                }
            }
            // Unlike the Gateway, the IPC client accepts a plain https URL here and proxies it itself —
            // no `external-assets` round-trip. Artwork is best-effort: no thumbnail is just a
            // text-only presence.
            if let Some(url) = track.thumbnail.clone().filter(|_| cfg.cover) {
                let mut assets = activity::Assets::new().large_image(url);
                if let Some(line3) = text_for(&cfg.line3, &track) {
                    assets = assets.large_text(field(&line3));
                }
                // The artwork stands for the release, so it links the album when there is one and the
                // song otherwise.
                if cfg.link_cover {
                    if let Some(u) = link_for("album", &track).or_else(|| link_for("title", &track))
                    {
                        assets = assets.large_url(u);
                    }
                }
                // Only ever alongside the artwork: `small_image` on its own is not a badge, it becomes
                // the card's image.
                if cfg.badge {
                    let name = if cfg.app_name.is_empty() { "Limusic" } else { &cfg.app_name };
                    assets = assets.small_image(BADGE_URL).small_text(field(name));
                }
                act = act.assets(assets);
            }
            act = act.buttons(
                [&cfg.button1, &cfg.button2].iter().filter_map(|k| button_for(k, &track)).collect(),
            );
        }

        // The floor is charged for every frame we put on the wire, accepted or not.
        self.note_frame();
        self.cfg_dirty = false;
        let log_id = if track.local { "<local>" } else { track.video_id.as_str() };
        if client.set_activity(act).is_ok() && check_response(&mut client, "set_activity") {
            // Recorded even if Discord rejected the payload (warn-logged in check_response):
            // retrying an identical rejected frame in a loop helps nobody; the next real change
            // sends a fresh one.
            tracing::info!(
                video_id = %log_id,
                playing = self.playing,
                has_duration = self.duration > 0.0,
                pos_secs = pos,
                "discord presence pushed"
            );
            self.sent = Some(Sent {
                video_id: track.video_id,
                playing: self.playing,
                start_ms,
                duration: self.duration,
            });
            self.client = Some(client);
        } else {
            // Broken socket — Discord quit. Drop it; the reconnect tick picks it back up.
            tracing::warn!(video_id = %log_id, "discord presence push failed");
            self.sent = None;
        }
    }

    /// Take the card down if one is up. No card + no socket is a no-op — never connect just to
    /// clear (a fresh socket shows nothing anyway).
    fn clear_shown(&mut self) {
        if self.sent.take().is_none() {
            return;
        }
        if let Some(mut client) = self.client.take() {
            self.note_frame();
            if client.clear_activity().is_ok() && check_response(&mut client, "clear_activity") {
                tracing::info!("discord presence cleared");
                self.client = Some(client);
            } else {
                tracing::warn!("discord presence clear failed");
            }
        }
    }

    /// Time left before the next connect attempt is allowed, or `None` when we may try now.
    fn connect_backoff_remaining(&self) -> Option<Duration> {
        self.connect_backoff.checked_sub(self.last_connect_try?.elapsed())
    }

    fn ensure_connected(&mut self) -> bool {
        if self.client.is_some() {
            return true;
        }
        if self.connect_backoff_remaining().is_some() {
            return false; // too soon — the loop is already scheduled to wake for this
        }
        self.last_connect_try = Some(Instant::now());
        let mut client = DiscordIpcClient::new(APP_ID);
        match client.connect() {
            Ok(()) => {
                tracing::info!("discord rich presence connected");
                self.client = Some(client);
                self.connect_backoff = CONNECT_RETRY_MIN;
                self.sent = None; // fresh socket shows nothing — re-push everything
                true
            }
            // Not an error: Discord simply isn't running. Ease off, so a machine that never runs
            // Discord isn't probing a nonexistent socket every second forever.
            Err(e) => {
                tracing::debug!(error = %e, backoff = ?self.connect_backoff, "discord not available");
                self.connect_backoff = (self.connect_backoff * 2).min(CONNECT_RETRY_MAX);
                false
            }
        }
    }

    fn disconnect(&mut self) {
        self.clear_shown();
        if let Some(mut client) = self.client.take() {
            let _ = client.close();
        }
        self.sent = None;
        self.last_connect_try = None;
        self.connect_backoff = CONNECT_RETRY_MIN;
    }
}

/// Read the response frame Discord sends for every command. The crate's `set_activity` only
/// *writes* — without this read, rejected payloads look like success (and the dedup then pins the
/// stale card), and unread frames pile up in the socket buffer until the connection wedges.
/// Returns whether the socket is still usable. Blocking, but Discord answers commands promptly;
/// worst case a hung Discord stalls this dedicated thread, nothing else.
fn check_response(client: &mut DiscordIpcClient, what: &str) -> bool {
    match client.recv() {
        Ok((op, resp)) => {
            if op != 1 {
                tracing::warn!(what, op, "discord response opcode is not a normal frame");
            }
            if resp.get("evt").and_then(|v| v.as_str()) == Some("ERROR") {
                let msg =
                    resp.pointer("/data/message").and_then(|v| v.as_str()).unwrap_or("unknown");
                tracing::warn!(what, error = msg, "discord rejected the payload");
            }
            true
        }
        Err(e) => {
            tracing::debug!(what, error = %e, "discord response read failed — dropping socket");
            false
        }
    }
}

/// The text a configured slot (`title`, `artist`, `album`, `artist_album`, `off`) resolves to for
/// this track, or `None` when the track carries nothing for it — an album-less single with the
/// second line set to "Album" simply drops that line rather than showing a blank one.
fn text_for(slot: &str, t: &Track) -> Option<String> {
    let artist = (!t.artists.is_empty()).then(|| t.artists.clone());
    let album = t.album.clone().filter(|a| !a.is_empty());
    match slot {
        "title" => (!t.title.is_empty()).then(|| t.title.clone()),
        "artist" => artist,
        "album" => album,
        "artist_album" => match (artist, album) {
            (Some(a), Some(b)) => Some(format!("{a} — {b}")),
            (a, b) => a.or(b),
        },
        _ => None,
    }
}

/// Where a slot's text should point when the user made it clickable: the link follows the content,
/// so a line showing the album opens the album. `None` for a local file (no page to open, and its
/// id is a path on this machine) or when YouTube didn't link that entity on the row.
fn link_for(slot: &str, t: &Track) -> Option<String> {
    if t.local {
        return None;
    }
    match slot {
        "title" => (!t.video_id.is_empty()).then(|| format!("{SONG_URL}{}", t.video_id)),
        "artist" | "artist_album" => t.artist_id.as_ref().map(|id| format!("{ARTIST_URL}{id}")),
        "album" => t.album_id.as_ref().map(|id| format!("{ALBUM_URL}{id}")),
        _ => None,
    }
}

/// One configured button slot. The labels are deliberately English: they are Discord's content, not
/// the app's chrome, and the settings preview mirrors these same strings so what the user sees in
/// the preview is what their friends read.
fn button_for(kind: &str, t: &Track) -> Option<activity::Button<'static>> {
    let (label, url) = match kind {
        "listen" => ("Listen on YouTube Music", link_for("title", t)?),
        "album" => ("View album", link_for("album", t)?),
        "artist" => ("View artist", link_for("artist", t)?),
        "app" => ("Get Limusic", REPO_URL.to_owned()),
        _ => return None,
    };
    (url.len() <= MAX_BUTTON_URL).then(|| activity::Button::new(label, url))
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

/// Clamp a text field to Discord's 2–128 char window: truncate long values on a char boundary,
/// pad sub-2-char ones with braille blanks (renders empty, survives Discord's trimming) — either
/// violation rejects the whole payload.
fn field(s: &str) -> String {
    let mut out: String = match s.char_indices().nth(MAX_FIELD) {
        Some(_) => s.chars().take(MAX_FIELD - 1).chain(['…']).collect(),
        None => s.to_owned(),
    };
    while out.chars().count() < 2 {
        out.push('\u{2800}');
    }
    out
}

/// Ready a thumbnail URL for Discord's card: request a decent resolution (see
/// [`crate::media::cover_url`]) and refuse URLs over Discord's length limit.
fn discord_thumb(url: &str) -> Option<String> {
    let sized = crate::media::cover_url(url);
    (sized.len() <= MAX_ASSET_URL).then_some(sized)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `spawn` refuses to run without one, so a bad edit here silently disables the whole feature.
    #[test]
    fn app_id_is_a_snowflake() {
        assert!(
            !APP_ID.is_empty() && APP_ID.bytes().all(|b| b.is_ascii_digit()),
            "APP_ID must be a Discord application id (digits only) — got {APP_ID:?}"
        );
    }

    fn track(id: &str) -> Box<Track> {
        Box::new(Track {
            video_id: id.into(),
            title: "t".into(),
            artists: "a".into(),
            artist_id: None,
            album: None,
            album_id: None,
            thumbnail: None,
            local: false,
        })
    }

    fn playing(id: &str, pos: f64) -> Presence {
        let mut p = Presence::new(true, RpcConfig::default());
        p.apply(Msg::Track(track(id)));
        p.apply(Msg::Playing(true));
        p.apply(Msg::Position { pos, at: Instant::now() });
        p
    }

    /// Simulate a card already on screen for the current track, pushed long enough ago that the
    /// send floor is clear.
    fn sent_now(p: &mut Presence, pos_secs: i64) {
        p.sent = Some(Sent {
            video_id: p.track.as_ref().unwrap().video_id.clone(),
            playing: p.playing,
            start_ms: now_ms() - pos_secs * 1000,
            duration: p.duration,
        });
        p.last_send = Some(Instant::now() - Duration::from_secs(5));
        p.cfg_dirty = false;
    }

    #[test]
    fn tempo_adjusted_ticks_stay_on_discords_timeline() {
        let (tx, rx) = channel();
        let discord = DiscordHandle { tx };
        for (speed, duration, position) in
            [(0.5, 480.0, 120.0), (1.0, 240.0, 60.0), (1.25, 192.0, 48.0), (2.0, 120.0, 30.0)]
        {
            let mut p = playing("abc", 0.0);
            discord.set_duration(240.0, speed);
            p.apply(rx.recv().unwrap());
            discord.set_position(60.0, speed);
            p.apply(rx.recv().unwrap());
            assert_eq!(p.duration, duration);
            assert_eq!(p.pos, position);
            sent_now(&mut p, position as i64);
            assert!(!p.wants_push(), "tempo {speed} created a false seek");

            // Advance the source and the shown card by ten listening seconds, without sleeping.
            p.sent.as_mut().unwrap().start_ms -= 10_000;
            discord.set_position(60.0 + 10.0 * speed, speed);
            p.apply(rx.recv().unwrap());
            assert!(!p.wants_push(), "tempo {speed} made Discord drift during playback");
        }
    }

    /// An album learned mid-track (#309) re-pushes the shown card, but only for that track: one
    /// arriving for a track that has since been replaced is dropped.
    #[test]
    fn a_late_album_repushes_only_its_own_track() {
        let mut p = playing("seed", 5.0);
        p.duration = 200.0;
        sent_now(&mut p, 5);
        assert_eq!(p.plan(), Act::Idle);
        p.apply(Msg::Album { video_id: "other".into(), album: "Nope".into() });
        assert_eq!(p.track.as_ref().unwrap().album, None);
        assert_eq!(p.plan(), Act::Idle);
        p.apply(Msg::Album { video_id: "seed".into(), album: "After Hours".into() });
        assert_eq!(p.track.as_ref().unwrap().album.as_deref(), Some("After Hours"));
        assert_eq!(p.plan(), Act::Push);
    }

    /// The reported bug: a gapless advance pushed a card before mpv reported the new track's
    /// length, so it rendered as a bare elapsed counter with no progress bar — and the follow-up
    /// push carrying the length landed too soon after it and was dropped by Discord.
    ///
    /// The fix is to hold the first push of a new track (briefly) until the length lands, so one
    /// push carries both. Here the length arrives a beat later, as it does on a real advance.
    #[test]
    fn a_new_track_waits_for_its_length_then_pushes_once() {
        let mut p = playing("old", 200.0);
        p.duration = 210.0;
        sent_now(&mut p, 200);

        // Gapless advance: the track lands first, its length is still unknown.
        p.apply(Msg::Track(track("new")));
        assert!(
            matches!(p.plan(), Act::Wait(_)),
            "must hold the push while the length is unknown, got {:?}",
            p.plan()
        );
        p.track_at = Instant::now() - TRACK_SETTLE - Duration::from_millis(10);

        // mpv reports the length (state.rs hands it over the moment mpv transitions).
        p.apply(Msg::Duration(185.0));
        assert_eq!(p.plan(), Act::Push, "with the length known, push immediately");

        // That single push carries the bar; nothing further is pending.
        p.sent = Some(Sent {
            video_id: "new".into(),
            playing: true,
            start_ms: now_ms(),
            duration: 185.0,
        });
        p.last_send = Some(Instant::now());
        assert_eq!(p.plan(), Act::Idle, "one push per track change, not two");
    }

    /// If mpv never reports a length, the card must still go out — just without a bar.
    #[test]
    fn the_grace_expires_rather_than_hanging() {
        let mut p = playing("abc", 0.0);
        p.track_at = Instant::now() - DURATION_GRACE - Duration::from_millis(50);
        assert_eq!(p.plan(), Act::Push, "grace expired — push a bar-less card");
    }

    /// A length arriving after a bar-less card went out must still reach Discord.
    #[test]
    fn a_late_length_repushes_the_bar() {
        let mut p = playing("abc", 1.0);
        sent_now(&mut p, 1); // pushed with duration 0
        p.apply(Msg::Duration(185.0));
        assert!(p.wants_push(), "the bar's end appeared — push it");
        assert_eq!(p.plan(), Act::Push);
    }

    /// Two frames must never go out inside the floor — but the second is deferred, never dropped.
    #[test]
    fn the_send_floor_defers_rather_than_discards() {
        let mut p = playing("abc", 1.0);
        p.duration = 185.0;
        sent_now(&mut p, 1);
        p.last_send = Some(Instant::now()); // just sent
        p.apply(Msg::Position { pos: 120.0, at: Instant::now() }); // user scrubs

        match p.plan() {
            Act::Wait(d) => assert!(d <= SEND_FLOOR, "waits out the floor, got {d:?}"),
            other => panic!("expected a deferred push, got {other:?}"),
        }
        // Once the floor clears, the *pending* scrub still goes out.
        p.last_send = Some(Instant::now() - SEND_FLOOR - Duration::from_millis(10));
        assert_eq!(p.plan(), Act::Push, "the deferred update must not be lost");
    }

    /// The headline bug from the previous round: a track change must not inherit the previous
    /// track's position.
    #[test]
    fn track_change_resets_the_timeline() {
        let mut p = playing("old", 187.0);
        sent_now(&mut p, 187);
        p.apply(Msg::Track(track("new")));
        assert!(p.estimate() < 0.5, "new track starts at 0, got {}", p.estimate());
        assert!(p.wants_push(), "track change must push");
    }

    /// The whole point of `wants_push`: 1s position ticks must not become presence updates.
    #[test]
    fn steady_playback_does_not_push() {
        let mut p = playing("abc", 30.0);
        assert!(p.wants_push(), "first track must push");
        sent_now(&mut p, 30);

        p.apply(Msg::Position { pos: 31.0, at: Instant::now() });
        assert!(!p.wants_push(), "a tick on the pushed timeline must not push");
        assert_eq!(p.plan(), Act::Idle);

        p.apply(Msg::Position { pos: 120.0, at: Instant::now() });
        assert!(p.wants_push(), "a scrub must push");
    }

    /// A position that sat in a backlogged queue is aged before use, so the drift check compares
    /// like with like instead of "correcting" the timeline backwards.
    #[test]
    fn aged_positions_are_corrected() {
        let mut p = playing("abc", 0.0);
        sent_now(&mut p, 13);
        // A tick reading 10.0 that took 3s to arrive: the song is really at ~13s now.
        p.apply(Msg::Position { pos: 10.0, at: Instant::now() - Duration::from_secs(3) });
        let est = p.estimate();
        assert!((est - 13.0).abs() < 0.2, "expected ~13, got {est}");
        assert!(!p.wants_push(), "an aged-but-on-timeline tick must not push");
    }

    /// Pause freezes the estimate (folding accrued playtime), and no card is wanted while paused.
    #[test]
    fn pause_freezes_and_hides() {
        let mut p = playing("abc", 10.0);
        p.pos_at = Instant::now() - Duration::from_secs(3); // 3s of playback accrued
        p.apply(Msg::Playing(false));
        let frozen = p.estimate();
        assert!((frozen - 13.0).abs() < 0.2, "expected ~13 frozen, got {frozen}");
        assert!(!p.playing, "paused");
        // sync() clears the card when !playing — needs_push is only consulted while playing.
    }

    /// Pausing takes the card down; the clear is what `plan` asks for, not another push.
    #[test]
    fn pausing_clears_the_card() {
        let mut p = playing("abc", 30.0);
        p.duration = 185.0;
        sent_now(&mut p, 30);
        p.apply(Msg::Playing(false));
        assert_eq!(p.plan(), Act::Clear);
        p.sent = None; // as clear_shown would leave it
        assert_eq!(p.plan(), Act::Idle, "nothing shown, nothing to take down");
    }

    /// A failed connect must not stall the next attempt for the full retry interval — Discord is
    /// often just slower to start than we are.
    #[test]
    fn a_failed_connect_retries_soon_then_backs_off() {
        let mut p = Presence::new(true, RpcConfig::default());
        assert!(p.connect_backoff_remaining().is_none(), "never tried — try now");

        p.last_connect_try = Some(Instant::now()); // as ensure_connected does on failure
        let rem = p.connect_backoff_remaining().expect("throttled straight after a failure");
        assert!(rem <= CONNECT_RETRY_MIN, "first retry within {CONNECT_RETRY_MIN:?}, got {rem:?}");

        p.connect_backoff = CONNECT_RETRY_MAX; // after repeated failures
        p.last_connect_try = Some(Instant::now() - CONNECT_RETRY_MAX);
        assert!(p.connect_backoff_remaining().is_none(), "the backoff still lets retries through");
    }

    fn full_track() -> Track {
        Track {
            video_id: "vid".into(),
            title: "Tell Ur Girlfriend".into(),
            artists: "Lay Bankz".into(),
            artist_id: Some("UCartist".into()),
            album: Some("After 7".into()),
            album_id: Some("MPREalbum".into()),
            thumbnail: None,
            local: false,
        }
    }

    /// The defaults must reproduce the card that shipped before any of this was configurable, so
    /// an existing install that never opens the Discord tab sees nothing it did not ask for. The
    /// badge is the one deliberate addition.
    #[test]
    fn the_default_config_is_the_old_card() {
        let c = RpcConfig::default();
        let t = full_track();
        assert_eq!(text_for(&c.line1, &t).as_deref(), Some("Tell Ur Girlfriend"));
        assert_eq!(text_for(&c.line2, &t).as_deref(), Some("Lay Bankz"));
        assert_eq!(text_for(&c.line3, &t).as_deref(), Some("After 7"));
        assert!(c.timestamps && c.cover);
        assert!(c.badge, "the badge is on by default, and needs the artwork it sits on");
        assert!(!c.show_paused, "paused still clears the card unless asked otherwise");
        assert!(!c.hide_details);
        assert!(!c.link_line1 && !c.link_line2 && !c.link_cover, "links are opt-in");
        assert_eq!(c.status_line, "line2", "status shows the artist, as it always did");
        assert!(button_for(&c.button1, &t).is_some() && button_for(&c.button2, &t).is_some());
    }

    /// Links follow the *content* of a slot, so a line switched to the album opens the album.
    #[test]
    fn a_links_target_follows_what_the_line_shows() {
        let t = full_track();
        assert_eq!(link_for("title", &t).unwrap(), format!("{SONG_URL}vid"));
        assert_eq!(link_for("artist", &t).unwrap(), format!("{ARTIST_URL}UCartist"));
        assert_eq!(link_for("artist_album", &t).unwrap(), format!("{ARTIST_URL}UCartist"));
        assert_eq!(link_for("album", &t).unwrap(), format!("{ALBUM_URL}MPREalbum"));
        assert_eq!(link_for("off", &t), None);
    }

    /// A local file's "video id" is a path on this machine. No configuration may put it on the
    /// user's public profile, as a link or as a button.
    #[test]
    fn a_local_file_is_never_linked() {
        let t = Track { local: true, video_id: "/home/me/Music/x.flac".into(), ..full_track() };
        for slot in ["title", "artist", "album", "artist_album"] {
            assert_eq!(link_for(slot, &t), None, "{slot} must not link a local file");
        }
        for kind in ["listen", "album", "artist"] {
            assert!(button_for(kind, &t).is_none(), "{kind} must not link a local file");
        }
        assert!(button_for("app", &t).is_some(), "the repo button is always safe");
    }

    /// A slot the track has nothing for drops out rather than rendering blank — and `details`,
    /// which Discord will not draw a card without, falls back to the title.
    #[test]
    fn missing_metadata_drops_the_slot() {
        let t = Track { album: None, album_id: None, ..full_track() };
        assert_eq!(text_for("album", &t), None);
        assert_eq!(link_for("album", &t), None);
        assert_eq!(text_for("artist_album", &t).as_deref(), Some("Lay Bankz"));
        assert!(button_for("album", &t).is_none(), "no album, no album button");
    }

    /// "Show when paused" must not resurrect the queue restored at launch: that arrives paused for
    /// a track nobody pressed play on, and a card for it is the frozen "Listening to…" the whole
    /// paused-clears rule exists to avoid.
    #[test]
    fn show_when_paused_waits_for_real_playback() {
        let cfg = RpcConfig { show_paused: true, ..Default::default() };
        let mut p = Presence::new(true, cfg);
        // Launch: the queue is restored, position ticks in, nothing has played.
        p.apply(Msg::Track(track("abc")));
        p.apply(Msg::Position { pos: 42.0, at: Instant::now() });
        assert_eq!(p.plan(), Act::Idle, "a queue nobody started must not post a card");

        // The user presses play, then pauses. (The length has to be in, or the card would be
        // waiting out the duration grace rather than deciding anything about the pause.)
        p.apply(Msg::Playing(true));
        p.apply(Msg::Duration(185.0));
        assert_eq!(p.plan(), Act::Push);
        sent_now(&mut p, 0);
        p.apply(Msg::Playing(false));
        assert_eq!(p.plan(), Act::Push, "a real pause keeps the card, re-pushed without its bar");
    }

    /// With the setting off, pausing still takes the card down — the shipped behavior.
    #[test]
    fn pausing_still_clears_by_default() {
        let mut p = playing("abc", 30.0);
        sent_now(&mut p, 30);
        p.apply(Msg::Playing(false));
        assert_eq!(p.plan(), Act::Clear);
    }

    /// A paused card carries no timeline, so the seek check has nothing to compare against —
    /// `now_ms()` keeps moving while the frozen position does not, and without the guard every
    /// idle tick would look like a seek and re-push.
    #[test]
    fn a_paused_card_does_not_drift_into_a_push() {
        let cfg = RpcConfig { show_paused: true, ..Default::default() };
        let mut p = Presence::new(true, cfg);
        p.apply(Msg::Track(track("abc")));
        p.apply(Msg::Playing(true));
        p.apply(Msg::Position { pos: 30.0, at: Instant::now() });
        p.apply(Msg::Playing(false));
        // The card as it went out while paused, pushed a good while ago.
        p.sent = Some(Sent {
            video_id: "abc".into(),
            playing: false,
            start_ms: now_ms() - 600_000,
            duration: 185.0,
        });
        p.duration = 185.0;
        p.last_send = Some(Instant::now() - Duration::from_secs(5));
        assert!(!p.wants_push(), "ten minutes paused is not ten minutes of seeking");
        assert_eq!(p.plan(), Act::Idle);

        p.apply(Msg::Playing(true));
        assert!(p.wants_push(), "resuming must re-push: the bar has to come back");
    }

    /// Changing the layout has to re-render the card that is already up; `wants_push` only ever
    /// compares the track, so the config message is what forces it.
    #[test]
    fn a_config_change_repushes_the_current_card() {
        let mut p = playing("abc", 30.0);
        p.duration = 185.0;
        sent_now(&mut p, 30);
        assert!(!p.wants_push(), "nothing pending before the change");

        let cfg = RpcConfig { line2: "album".into(), ..Default::default() };
        p.apply(Msg::Config(Box::new(cfg.clone())));
        assert_eq!(p.plan(), Act::Push, "the new layout must reach Discord now");

        sent_now(&mut p, 30); // as push_card would leave it
        p.apply(Msg::Config(Box::new(cfg)));
        assert!(!p.wants_push(), "re-saving the same config is not a change");
    }

    /// Turning "show while paused" off while a paused card is up has to take that card down.
    /// The layout change used to forget what was on screen, which left `plan()` with nothing to
    /// clear and the stale card sitting on the profile until the next song.
    #[test]
    fn dropping_show_paused_while_paused_clears_the_card() {
        let mut p = playing("abc", 30.0);
        p.duration = 185.0;
        p.apply(Msg::Config(Box::new(RpcConfig { show_paused: true, ..Default::default() })));
        p.apply(Msg::Playing(false));
        sent_now(&mut p, 30);
        assert_eq!(p.plan(), Act::Idle, "the paused card is up and current");

        p.apply(Msg::Config(Box::new(RpcConfig::default())));
        assert_eq!(p.plan(), Act::Clear);
    }

    #[test]
    fn field_clamps_to_discords_window() {
        assert_eq!(field("héllo"), "héllo");
        let long = "é".repeat(200);
        assert!(field(&long).chars().count() <= MAX_FIELD);
        assert_eq!(field("V").chars().count(), 2, "1-char fields are padded, not rejected");
    }

    #[test]
    fn thumbnails_are_upscaled_and_bounded() {
        assert_eq!(
            discord_thumb("https://lh3.googleusercontent.com/x=w60-h60-l90-rj").as_deref(),
            Some("https://lh3.googleusercontent.com/x=w512-h512-l90-rj")
        );
        assert_eq!(
            discord_thumb("https://yt3.ggpht.com/y=s176").as_deref(),
            Some("https://yt3.ggpht.com/y=s512")
        );
        let ytimg = "https://i.ytimg.com/vi/abc/maxresdefault.jpg";
        assert_eq!(
            discord_thumb(ytimg).as_deref(),
            Some(ytimg),
            "path-variant thumbs pass through"
        );
        let long = format!("https://example.com/{}", "a".repeat(300));
        assert_eq!(discord_thumb(&long), None, "over-long URLs are dropped, not sent");
    }

    #[test]
    fn window_full_returns_wait() {
        let mut p = playing("abc", 10.0);
        sent_now(&mut p, 10);
        let now = Instant::now();
        p.last_send = Some(now - Duration::from_secs(2));
        p.sends.clear();
        // Discord's documented limit is 5 updates per 20s; fill exactly that many, not MAX_FRAMES,
        // so changing the constant makes this test fail.
        for _ in 0..5 {
            p.sends.push_back(now - Duration::from_secs(5));
        }
        p.apply(Msg::Position { pos: 20.0, at: now });
        let expected_wait = FRAME_WINDOW - Duration::from_secs(5);
        match p.plan() {
            Act::Wait(d) => {
                assert!(
                    d >= expected_wait - Duration::from_secs(1)
                        && d <= expected_wait + Duration::from_secs(1),
                    "expected wait within 1s of {expected_wait:?}, got {d:?}"
                );
            }
            other => panic!("expected Act::Wait, got {other:?}"),
        }
        p.sends.pop_back();
        assert_eq!(p.plan(), Act::Push, "four frames in the window leave room for one more");
    }

    #[test]
    fn quick_track_changes_with_card_up_waits() {
        let mut p = playing("old", 10.0);
        sent_now(&mut p, 10);
        for i in 0..5 {
            p.apply(Msg::Track(track(&format!("new_{i}"))));
        }
        p.apply(Msg::Duration(185.0));
        match p.plan() {
            Act::Wait(d) => {
                assert!(
                    d <= TRACK_SETTLE && d > Duration::ZERO,
                    "expected 0 < d <= {TRACK_SETTLE:?}, got {d:?}"
                );
            }
            other => panic!("expected Act::Wait, got {other:?}"),
        }
    }

    #[test]
    fn card_sent_50s_ago_triggers_push() {
        let mut p = playing("abc", 10.0);
        p.duration = 185.0;
        sent_now(&mut p, 10);
        p.last_send = Some(Instant::now() - Duration::from_secs(50));
        assert_eq!(p.plan(), Act::Push);
    }

    #[test]
    fn card_sent_5s_ago_and_on_timeline_never_pushes() {
        let mut p = playing("abc", 10.0);
        p.duration = 185.0;
        sent_now(&mut p, 10);
        p.last_send = Some(Instant::now() - Duration::from_secs(5));
        p.apply(Msg::Position { pos: 10.0, at: Instant::now() });
        assert!(!p.wants_push());
        assert_eq!(p.plan(), Act::Idle);
    }

    #[test]
    fn a_paused_card_does_not_refresh_after_50s() {
        let cfg = RpcConfig { show_paused: true, ..Default::default() };
        let mut p = Presence::new(true, cfg);
        p.apply(Msg::Track(track("abc")));
        p.apply(Msg::Playing(true));
        p.apply(Msg::Position { pos: 30.0, at: Instant::now() });
        p.apply(Msg::Playing(false));
        p.sent = Some(Sent {
            video_id: "abc".into(),
            playing: false,
            start_ms: now_ms() - 600_000,
            duration: 185.0,
        });
        p.duration = 185.0;
        p.last_send = Some(Instant::now() - Duration::from_secs(50));
        assert!(!p.wants_push());
        assert_eq!(p.plan(), Act::Idle);
    }
}
