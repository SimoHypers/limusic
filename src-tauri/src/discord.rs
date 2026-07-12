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
//! Presence is shown **only while actually playing**. Paused (including the restored-but-not-
//! started queue on app launch) clears the card — a frozen "Listening to…" hours after the user
//! walked away is worse than no card.
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
//! together — and a dropped update is invisible (the socket ACKs it). Two rules keep state from
//! getting stranded behind that: a **trailing-edge floor** (when a push is due but too soon, the
//! loop sleeps until the floor expires and then sends whatever is current — never discards it),
//! and a short **grace** on a brand-new track so its length can land before the first push. Without
//! the grace, a track change pushed a bar-less card and needed a second push milliseconds later,
//! which Discord dropped — leaving the card stuck as an elapsed counter with no progress bar.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::OnceLock;
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
const REPO_URL: &str = "https://github.com/SimoHypers/limusic";

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
/// How long a new track waits for mpv to report its length before we give up and push a card with
/// no progress bar. Collapses the track-change burst (track + length + play state) into one push.
const DURATION_GRACE: Duration = Duration::from_millis(800);
/// Floor for any computed wait — handing `recv_timeout` a zero duration would spin.
const MIN_WAIT: Duration = Duration::from_millis(10);
/// Discord rejects `details`/`state`/`large_text` outside 2–128 characters.
const MAX_FIELD: usize = 128;
/// Discord rejects asset URLs longer than this — better a text-only card than a rejected payload.
const MAX_ASSET_URL: usize = 256;

enum Msg {
    Track(Box<Track>),
    Duration(f64),
    /// A position tick. `at` is when the value was read — the thread ages it before use.
    Position { pos: f64, at: Instant },
    /// A real play/pause transition (mpv's pause flag), never inferred from position ticks.
    Playing(bool),
    Enabled(bool),
}

#[derive(Clone)]
struct Track {
    video_id: String,
    title: String,
    artists: String,
    album: Option<String>,
    thumbnail: Option<String>,
}

/// App-side handle to the presence thread. `None` when the thread couldn't be spawned; every push
/// is then a no-op.
pub struct DiscordHandle {
    tx: Sender<Msg>,
}

impl DiscordHandle {
    pub fn set_track(&self, item: &SongItem) {
        let _ = self.tx.send(Msg::Track(Box::new(Track {
            video_id: item.video_id.clone(),
            title: item.title.clone(),
            artists: item.artists.clone(),
            album: item.album.clone(),
            // Upscale here (stored thumbs are often 60px) and drop over-long URLs outright.
            thumbnail: item.thumbnail.as_deref().and_then(discord_thumb),
        })));
    }

    /// mpv's reported track length — the only source of a real duration (`SongItem::duration` is a
    /// display string like "3:45"). Without it the presence shows elapsed time but no end.
    pub fn set_duration(&self, secs: f64) {
        let _ = self.tx.send(Msg::Duration(secs));
    }

    /// A raw position tick. Carries no play/pause authority — mpv also ticks on paused seeks.
    pub fn set_position(&self, pos: f64) {
        let _ = self.tx.send(Msg::Position { pos, at: Instant::now() });
    }

    pub fn set_playing(&self, playing: bool) {
        let _ = self.tx.send(Msg::Playing(playing));
    }

    pub fn set_enabled(&self, on: bool) {
        let _ = self.tx.send(Msg::Enabled(on));
    }
}

/// Spawn the presence owner thread. `enabled` is the persisted `discord_rpc` setting — when off,
/// the thread parks on the channel and never opens a socket.
pub fn spawn(enabled: bool) -> Option<DiscordHandle> {
    // Without a real app id there is nothing to connect *as*. Say so once, loudly, rather than
    // leaving a settings toggle that silently does nothing.
    if APP_ID.is_empty() || !APP_ID.bytes().all(|b| b.is_ascii_digit()) {
        tracing::warn!(APP_ID, "discord rich presence disabled: APP_ID is not a Discord app id");
        return None;
    }
    let (tx, rx) = channel::<Msg>();
    match std::thread::Builder::new()
        .name("discord-rpc".into())
        .spawn(move || run(rx, Presence::new(enabled)))
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
    /// Last known position + when it was read. `estimate()` ages it while playing.
    pos: f64,
    pos_at: Instant,
    // pushed — `Some` iff a card is currently shown (cards exist only while playing)
    sent: Option<Sent>,
    last_send: Option<Instant>,
}

struct Sent {
    video_id: String,
    /// Epoch millis the pushed timeline started at — `now - start_ms` is where Discord's bar is.
    start_ms: i64,
    /// The length the pushed bar was built from; 0 when the card went out without one.
    duration: f64,
}

impl Presence {
    fn new(enabled: bool) -> Self {
        Presence {
            enabled,
            client: None,
            last_connect_try: None,
            connect_backoff: CONNECT_RETRY_MIN,
            track: None,
            track_at: Instant::now(),
            duration: 0.0,
            playing: false,
            pos: 0.0,
            pos_at: Instant::now(),
            sent: None,
            last_send: None,
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
            Msg::Duration(secs) => self.duration = secs,
            Msg::Position { pos, at } => {
                self.pos = pos;
                self.pos_at = at;
            }
            Msg::Playing(on) => {
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
        let want_card = self.playing && self.track.is_some();
        if want_card {
            if !self.wants_push() {
                return Act::Idle;
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
        if sent.video_id != track.video_id {
            return true;
        }
        // mpv reported the length, or refined it (streams get probed, then corrected) — the bar's
        // end moves. Also the retry path when a first push went out before the length landed.
        if (self.duration - sent.duration).abs() > 1.0 {
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

    /// Time left on the send floor, or `None` when we're clear to send.
    fn floor_remaining(&self) -> Option<Duration> {
        SEND_FLOOR.checked_sub(self.last_send?.elapsed())
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
        let mut act = activity::Activity::new()
            .activity_type(activity::ActivityType::Listening)
            .details(field(&track.title))
            .timestamps(ts)
            .buttons(vec![
                activity::Button::new(
                    "Listen on YouTube Music",
                    format!("{SONG_URL}{}", track.video_id),
                ),
                activity::Button::new("Get Limusic", REPO_URL),
            ]);
        if !track.artists.is_empty() {
            act = act.state(field(&track.artists));
        }
        // Unlike the Gateway, the IPC client accepts a plain https URL here and proxies it itself —
        // no `external-assets` round-trip. Artwork is best-effort: no thumbnail is just a
        // text-only presence.
        if let Some(url) = track.thumbnail.clone() {
            let mut assets = activity::Assets::new().large_image(url);
            match &track.album {
                Some(album) if !album.is_empty() => assets = assets.large_text(field(album)),
                _ => {}
            }
            act = act.assets(assets);
        }

        // The floor is charged for every frame we put on the wire, accepted or not.
        self.last_send = Some(Instant::now());
        if client.set_activity(act).is_ok() && check_response(&mut client, "set_activity") {
            // Recorded even if Discord rejected the payload (warn-logged in check_response):
            // retrying an identical rejected frame in a loop helps nobody; the next real change
            // sends a fresh one.
            self.sent = Some(Sent {
                video_id: track.video_id,
                start_ms,
                duration: self.duration,
            });
            self.client = Some(client);
        } else {
            // Broken socket — Discord quit. Drop it; the reconnect tick picks it back up.
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
            self.last_send = Some(Instant::now()); // a clear is a frame too — it counts
            if client.clear_activity().is_ok() && check_response(&mut client, "clear_activity") {
                self.client = Some(client);
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
        Ok((_, resp)) => {
            if resp.get("evt").and_then(|v| v.as_str()) == Some("ERROR") {
                let msg = resp
                    .pointer("/data/message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
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

/// Ready a thumbnail URL for Discord's card: request a decent resolution (stored thumbs are often
/// row-sized, 60px) and refuse URLs over Discord's length limit. Mirrors `ui/src/lib/thumb.ts` —
/// only googleusercontent-style URLs carry their size in the URL; i.ytimg path-variant thumbs pass
/// through unchanged (other sizes can 404).
fn discord_thumb(url: &str) -> Option<String> {
    static WH: OnceLock<regex::Regex> = OnceLock::new();
    static S: OnceLock<regex::Regex> = OnceLock::new();
    let wh = WH.get_or_init(|| regex::Regex::new(r"=w\d+-h\d+").expect("static regex"));
    let s = S.get_or_init(|| regex::Regex::new(r"=s\d+").expect("static regex"));
    let sized = if wh.is_match(url) {
        wh.replace(url, "=w512-h512").into_owned()
    } else if s.is_match(url) {
        s.replace(url, "=s512").into_owned()
    } else {
        url.to_owned()
    };
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
            album: None,
            thumbnail: None,
        })
    }

    fn playing(id: &str, pos: f64) -> Presence {
        let mut p = Presence::new(true);
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
            start_ms: now_ms() - pos_secs * 1000,
            duration: p.duration,
        });
        p.last_send = Some(Instant::now() - Duration::from_secs(60));
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

        // mpv reports the length (state.rs hands it over the moment mpv transitions).
        p.apply(Msg::Duration(185.0));
        assert_eq!(p.plan(), Act::Push, "with the length known, push immediately");

        // That single push carries the bar; nothing further is pending.
        p.sent = Some(Sent {
            video_id: "new".into(),
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
        let mut p = Presence::new(true);
        assert!(p.connect_backoff_remaining().is_none(), "never tried — try now");

        p.last_connect_try = Some(Instant::now()); // as ensure_connected does on failure
        let rem = p.connect_backoff_remaining().expect("throttled straight after a failure");
        assert!(rem <= CONNECT_RETRY_MIN, "first retry within {CONNECT_RETRY_MIN:?}, got {rem:?}");

        p.connect_backoff = CONNECT_RETRY_MAX; // after repeated failures
        p.last_connect_try = Some(Instant::now() - CONNECT_RETRY_MAX);
        assert!(p.connect_backoff_remaining().is_none(), "the backoff still lets retries through");
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
        assert_eq!(discord_thumb("https://yt3.ggpht.com/y=s176").as_deref(), Some("https://yt3.ggpht.com/y=s512"));
        let ytimg = "https://i.ytimg.com/vi/abc/maxresdefault.jpg";
        assert_eq!(discord_thumb(ytimg).as_deref(), Some(ytimg), "path-variant thumbs pass through");
        let long = format!("https://example.com/{}", "a".repeat(300));
        assert_eq!(discord_thumb(&long), None, "over-long URLs are dropped, not sent");
    }
}
