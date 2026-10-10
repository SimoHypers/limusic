//! Last.fm + ListenBrainz scrobbling. One consumer of the same track/duration/position stream
//! that feeds `discord.rs` — but simpler: neither service cares about live position, only two
//! moments per track. A now-playing ping when a track starts, and one scrobble once the track
//! has played half its length or 4 minutes, whichever comes first (Last.fm's official rule;
//! tracks under 30s never scrobble). Both numbers are the user's to change in the Scrobbling
//! settings tab (#327), along with what gets sent: see [`ScrobbleConfig`] and [`resolve`].
//!
//! One task, one clock: the [`Scrobbler`] holds the Last.fm session key next to the ListenBrainz
//! user token and sends to whichever service is connected from the same `now_playing` and
//! `scrobble`. Only the transports differ — Last.fm's signed calls live here, ListenBrainz's
//! `submit-listens` calls in [`crate::listenbrainz`]. A fix to when a play counts (repeat-one,
//! real played time) lands once, not in two tasks.
//!
//! Everything is best-effort (context/16 fail-soft, same as Discord/media): a failed scrobble is
//! a `debug!` line, never a user-facing error. No offline queue — Last.fm accepts scrobbles up to
//! two weeks late, but a queue is complexity we add only if dropped scrobbles ever show up.
//! ponytail: no offline queue; add one if scrobbles visibly go missing.
//!
//! Auth is the desktop flow: `auth.getToken` → open the user's browser on the authorize page →
//! poll `auth.getSession` until they approve. Session keys never expire, so the key + username in
//! settings (`lastfm_session_key` / `lastfm_username`) are the whole persistent state.
//!
//! The scrobble threshold runs off mpv's *position*, not accumulated play time — seeking forward
//! can technically trigger it early, which is how most desktop scrobblers behave anyway.
//! ponytail: position-based threshold; track real played-time if cheating ever matters.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use innertube::SongItem;
use md5::{Digest, Md5};
use tauri::Emitter;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::state::AppState;

/// Last.fm API credentials. Baked in at compile time from the gitignored `src-tauri/lastfm.keys`
/// (via build.rs) — never from source, the repo is public. Registered at
/// <https://www.last.fm/api/account/create>. Without them the titlebar button errors out with a
/// clear message instead of silently doing nothing.
const API_KEY: &str = match option_env!("LIMUSIC_LASTFM_API_KEY") {
    Some(v) => v,
    None => "",
};
const API_SECRET: &str = match option_env!("LIMUSIC_LASTFM_API_SECRET") {
    Some(v) => v,
    None => "",
};

const API_ROOT: &str = "https://ws.audioscrobbler.com/2.0/";
const AUTH_URL: &str = "https://www.last.fm/api/auth/";

/// How long the user gets to approve the app in their browser: 60 polls × 5s = 5 minutes.
/// (The token itself is valid for 60 minutes — the UI spinner is the real constraint.)
const AUTH_POLL_EVERY: Duration = Duration::from_secs(5);
const AUTH_POLL_TRIES: u32 = 60;

/// Last.fm error 14: "token has not been authorized" — the user hasn't clicked Allow yet.
const ERR_TOKEN_PENDING: i64 = 14;
/// Last.fm error 16: service temporarily unavailable — retryable, same as pending.
const ERR_TEMP_UNAVAILABLE: i64 = 16;

/// How scrobbling behaves, from the Scrobbling settings tab (#327, #404). One JSON blob in the
/// `lastfm_config` setting, the same pattern as `discord_rpc_config`: a new knob is a field here,
/// a field in `ui/src/lib/scrobble.ts`, and a control in the tab. Missing or unparseable fields
/// fall back to [`Default`], which is exactly how Limusic scrobbled before the tab existed.
#[derive(Clone, serde::Deserialize)]
#[serde(default)]
pub struct ScrobbleConfig {
    /// Scrobble at all. Off pauses scrobbling without signing out of Last.fm.
    enabled: bool,
    /// Send `track.updateNowPlaying` when a track starts.
    now_playing: bool,
    /// Scrobble once this share of the track has played, 1 to 100. Last.fm's rule is 50.
    percent: u32,
    /// ...or once this many minutes have played, whichever comes first. 0 turns the cap off.
    /// Last.fm's rule is 4.
    minutes: u32,
    /// A video upload credited to a channel ("alexias788") usually names the real artist in its
    /// title ("Ruelle - Fire Meets Fate"). Off by default: an official video can carry a dash in
    /// the song title itself ("Blinding Lights - Chromatics Remix"), and only the user knows
    /// which kind they listen to.
    split_video_titles: bool,
    rules: Vec<Rule>,
    edits: Vec<Edit>,
    /// `lastfm_primary_artist` / `lastfm_primary_strict` (#231). They predate the tab and keep
    /// their own settings rows, so [`Self::load`] copies them in; the preview passes them inline.
    primary_artist: bool,
    primary_strict: bool,
}

impl Default for ScrobbleConfig {
    fn default() -> Self {
        ScrobbleConfig {
            enabled: true,
            now_playing: true,
            percent: 50,
            minutes: 4,
            split_video_titles: false,
            rules: Vec::new(),
            edits: Vec::new(),
            primary_artist: false,
            primary_strict: false,
        }
    }
}

impl ScrobbleConfig {
    /// Anything unparseable is the default, never an error: a corrupt blob must not stop scrobbling.
    pub fn parse(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    /// The stored config, with the two settings that live in rows of their own.
    pub fn load(db: &crate::db::Db) -> Self {
        let mut cfg = Self::parse(&db.get_setting("lastfm_config").unwrap_or_default());
        cfg.primary_artist = db.get_setting("lastfm_primary_artist").as_deref() == Some("true");
        cfg.primary_strict = db.get_setting("lastfm_primary_strict").as_deref() == Some("true");
        cfg
    }
}

/// Find and replace on one field, applied to every track in list order. The pattern is a regular
/// expression and ignores case (`(?-i)` turns that off). Named groups `artist`, `title` and
/// `album` switch the rule to extraction: a match moves each group into its field instead, which
/// is how a title like "Artist - Song" gets split without a dedicated option per site.
#[derive(Clone, serde::Deserialize)]
#[serde(default)]
struct Rule {
    enabled: bool,
    /// `title` | `artist` | `album`.
    field: String,
    find: String,
    replace: String,
}

impl Default for Rule {
    fn default() -> Self {
        Rule { enabled: true, field: "title".into(), find: String::new(), replace: String::new() }
    }
}

/// A fix for one track, applied every time it plays (#404). Last.fm has no API to change a
/// scrobble already sent, so this is how a wrong one stops recurring.
#[derive(Clone, Default, serde::Deserialize)]
#[serde(default)]
struct Edit {
    /// The video id this edit is for. Empty on edits imported from Pano Scrobbler, which match on
    /// the `from_` fields instead.
    key: String,
    /// What YouTube called the track. With a `key` this is only for display; without one, every
    /// non-empty field has to equal the track's (ignoring case), so `from_artist` alone matches
    /// every track by that artist.
    from_artist: String,
    from_title: String,
    from_album: String,
    /// What Last.fm gets. Empty keeps YouTube's value. Rules don't touch an edited track: the edit
    /// is the user's final answer for it.
    artist: String,
    title: String,
    album: String,
    /// Don't scrobble this track at all.
    skip: bool,
}

impl Edit {
    fn matches(&self, t: &Track) -> bool {
        if !self.key.is_empty() {
            return self.key == t.video_id;
        }
        let same = |want: &str, have: &str| {
            want.trim().is_empty() || want.trim().to_lowercase() == have.trim().to_lowercase()
        };
        let any = [&self.from_artist, &self.from_title, &self.from_album]
            .iter()
            .any(|f| !f.trim().is_empty());
        any && same(&self.from_artist, &t.artists)
            && same(&self.from_title, &t.title)
            && same(&self.from_album, t.album.as_deref().unwrap_or(""))
    }
}

enum Msg {
    Track(Box<Track>),
    /// The playing track's album, learned after it started. Unlike `Track`, keeps the clock.
    /// Keyed by video id: nothing serializes a stale `start_current`'s `Track` against the
    /// current one, so an album for a track that is no longer playing has to be dropped.
    Album {
        video_id: String,
        album: String,
    },
    Duration(f64),
    Position(f64),
    /// Session key set (connect) or cleared (disconnect).
    Session(Option<String>),
    /// ListenBrainz user token set (connect) or cleared (disconnect).
    Token(Option<String>),
    Config(Box<ScrobbleConfig>),
}

/// The track as YouTube describes it. What gets sent is [`resolve`]d from this at send time, so
/// a rule or edit saved mid-track still applies to that track's scrobble.
#[derive(Default, serde::Deserialize)]
#[serde(default)]
pub struct Track {
    video_id: String,
    title: String,
    artists: String,
    album: Option<String>,
    is_video: bool,
}

impl From<&SongItem> for Track {
    fn from(item: &SongItem) -> Self {
        Track {
            video_id: item.video_id.clone(),
            title: item.title.clone(),
            artists: item.artists.clone(),
            album: item.album.clone(),
            is_video: item.is_video,
        }
    }
}

/// App-side handle to the scrobbler task.
pub struct LastfmHandle {
    tx: UnboundedSender<Msg>,
    /// Bumped by every connect/disconnect; in-flight auth polls compare against it and bail when
    /// superseded, so a stale poll can never overwrite a newer session (or a disconnect).
    auth_gen: AtomicU64,
}

impl LastfmHandle {
    pub fn set_track(&self, item: &SongItem) {
        let _ = self.tx.send(Msg::Track(Box::new(Track::from(item))));
    }

    /// A search or home card starts playing with no album name; the radio fetched behind it
    /// supplies one a moment later (#309). A second `set_track` would restart the scrobble clock.
    pub fn set_album(&self, video_id: &str, album: &str) {
        let _ = self.tx.send(Msg::Album { video_id: video_id.to_owned(), album: album.to_owned() });
    }

    pub fn set_duration(&self, secs: f64) {
        let _ = self.tx.send(Msg::Duration(secs));
    }

    pub fn set_position(&self, pos: f64) {
        let _ = self.tx.send(Msg::Position(pos));
    }

    /// Any of the scrobbling settings changed. Applies to the track already playing, from its
    /// scrobble on.
    pub fn set_config(&self, cfg: ScrobbleConfig) {
        let _ = self.tx.send(Msg::Config(Box::new(cfg)));
    }

    fn set_session(&self, key: Option<String>) {
        let _ = self.tx.send(Msg::Session(key));
    }

    /// ListenBrainz connect/disconnect writes here (see `crate::listenbrainz`): same task, same
    /// clock, second credential.
    pub(crate) fn set_token(&self, token: Option<String>) {
        let _ = self.tx.send(Msg::Token(token));
    }

    fn bump_gen(&self) -> u64 {
        self.auth_gen.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn gen(&self) -> u64 {
        self.auth_gen.load(Ordering::SeqCst)
    }
}

/// Spawn the scrobbler task. `session_key` is the persisted Last.fm login and `lb_token` the
/// persisted ListenBrainz user token; `None` parks that half until the user connects.
pub fn spawn(
    session_key: Option<String>,
    lb_token: Option<String>,
    cfg: ScrobbleConfig,
) -> LastfmHandle {
    let (tx, mut rx) = unbounded_channel::<Msg>();
    tauri::async_runtime::spawn(async move {
        let mut s = Scrobbler::new(session_key, lb_token, cfg);
        while let Some(msg) = rx.recv().await {
            s.apply(msg).await;
        }
    });
    LastfmHandle { tx, auth_gen: AtomicU64::new(0) }
}

struct Scrobbler {
    session: Option<String>,
    lb_token: Option<String>,
    cfg: ScrobbleConfig,
    track: Option<Track>,
    /// Epoch secs when the current track started — the scrobble's `timestamp` / `listened_at`.
    started_at: u64,
    duration: f64,
    scrobbled: bool,
}

impl Scrobbler {
    fn new(session: Option<String>, lb_token: Option<String>, cfg: ScrobbleConfig) -> Self {
        Scrobbler {
            session,
            lb_token,
            cfg,
            track: None,
            started_at: 0,
            duration: 0.0,
            scrobbled: false,
        }
    }

    async fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Track(t) => {
                self.track = Some(*t);
                self.started_at = now_secs();
                self.duration = 0.0;
                self.scrobbled = false;
                self.now_playing().await;
            }
            Msg::Album { video_id, album } => {
                let Some(t) = self.track.as_mut().filter(|t| t.video_id == video_id) else {
                    return;
                };
                t.album = Some(album);
                // Re-send it: `updateNowPlaying` already went out without the album, and the
                // clock fields stay untouched so the scrobble still times from the real start.
                self.now_playing().await;
            }
            Msg::Duration(secs) => self.duration = secs,
            Msg::Position(pos) => {
                if !self.scrobbled && crosses_threshold(pos, self.duration, &self.cfg) {
                    self.scrobbled = true; // latch even on failure — never re-fire per tick
                    self.scrobble().await;
                }
            }
            Msg::Session(key) => self.session = key,
            Msg::Token(token) => self.lb_token = token,
            // ponytail: "now playing" isn't re-sent on a config change. The tab saves as the user
            // types, and each service would get one call per pause in typing a rule.
            Msg::Config(cfg) => self.cfg = *cfg,
        }
    }

    /// What the current track resolves to, or `None` when nothing is sent anywhere: scrobbling
    /// off, no track yet, or this track skips. Which services it goes to is decided at send
    /// time, from whichever credential is set.
    fn outgoing(&self) -> Option<Resolved> {
        let t = self.track.as_ref()?;
        if !self.cfg.enabled {
            return None;
        }
        let r = resolve(t, &self.cfg);
        r.skip.is_none().then_some(r)
    }

    async fn now_playing(&self) {
        if !self.cfg.now_playing {
            return;
        }
        let Some(r) = self.outgoing() else { return };
        if let Some(sk) = self.session.as_deref() {
            self.now_playing_lastfm(sk, &r).await;
        }
        if let Some(token) = self.lb_token.as_deref() {
            self.now_playing_listenbrainz(token, &r).await;
        }
    }

    async fn now_playing_lastfm(&self, sk: &str, r: &Resolved) {
        let mut params = vec![
            ("artist".to_string(), r.artist.clone()),
            ("track".to_string(), r.title.clone()),
            ("sk".to_string(), sk.to_string()),
        ];
        if !r.album.is_empty() {
            params.push(("album".to_string(), r.album.clone()));
        }
        match call("track.updateNowPlaying", params, true).await {
            Ok(_) => tracing::debug!(track = %r.title, "last.fm now playing sent"),
            Err(e) => tracing::debug!(error = %e.message, "last.fm now playing failed"),
        }
    }

    async fn now_playing_listenbrainz(&self, token: &str, r: &Resolved) {
        let video_id = self.track.as_ref().map(|t| t.video_id.as_str()).unwrap_or("");
        let payload = serde_json::json!([{
            "track_metadata": crate::listenbrainz::track_metadata(r, video_id, self.duration),
        }]);
        match crate::listenbrainz::submit(token, "playing_now", payload).await {
            Ok(_) => tracing::debug!(track = %r.title, "listenbrainz now playing sent"),
            Err(e) => tracing::debug!(error = %e, "listenbrainz now playing failed"),
        }
    }

    async fn scrobble(&self) {
        let Some(r) = self.outgoing() else { return };
        if let Some(sk) = self.session.as_deref() {
            self.scrobble_lastfm(sk, &r).await;
        }
        if let Some(token) = self.lb_token.as_deref() {
            self.scrobble_listenbrainz(token, &r).await;
        }
    }

    async fn scrobble_lastfm(&self, sk: &str, r: &Resolved) {
        let mut params = vec![
            ("artist".to_string(), r.artist.clone()),
            ("track".to_string(), r.title.clone()),
            ("timestamp".to_string(), self.started_at.to_string()),
            ("sk".to_string(), sk.to_string()),
        ];
        if !r.album.is_empty() {
            params.push(("album".to_string(), r.album.clone()));
        }
        if self.duration > 0.0 {
            params.push(("duration".to_string(), (self.duration as i64).to_string()));
        }
        match call("track.scrobble", params, true).await {
            Ok(_) => tracing::info!(track = %r.title, "scrobbled to last.fm"),
            Err(e) => tracing::warn!(error = %e.message, "last.fm scrobble failed"),
        }
    }

    async fn scrobble_listenbrainz(&self, token: &str, r: &Resolved) {
        let video_id = self.track.as_ref().map(|t| t.video_id.as_str()).unwrap_or("");
        let payload = serde_json::json!([{
            "listened_at": self.started_at,
            "track_metadata": crate::listenbrainz::track_metadata(r, video_id, self.duration),
        }]);
        match crate::listenbrainz::submit(token, "single", payload).await {
            Ok(_) => tracing::info!(track = %r.title, "submitted listen to listenbrainz"),
            Err(e) => tracing::warn!(error = %e, "listenbrainz listen failed"),
        }
    }
}

/// What a track scrobbles as, and why. The scrobbler sends `artist`/`title`/`album` unless `skip`
/// is set; the settings tab renders all of it as its preview, so the two can never disagree.
#[derive(serde::Serialize)]
pub struct Resolved {
    pub artist: String,
    pub title: String,
    pub album: String,
    /// Why nothing is sent: `edit` (the user's edit says skip) or `incomplete` (no title or artist
    /// left, or an untagged local file).
    pub skip: Option<&'static str>,
    /// Index into `edits` of the edit that decided this track.
    pub edit: Option<usize>,
    /// The video title was split into artist and song.
    pub split: bool,
    /// Indices of the rules that changed something, in order.
    pub rules: Vec<usize>,
    /// Rules whose pattern doesn't compile, by index, with the reason. They are skipped.
    pub errors: Vec<(usize, String)>,
}

impl Resolved {
    fn field(&mut self, name: &str) -> Option<&mut String> {
        match name {
            "artist" => Some(&mut self.artist),
            "title" => Some(&mut self.title),
            "album" => Some(&mut self.album),
            _ => None,
        }
    }
}

/// Apply the user's scrobbling settings to a track, in this order: a matching edit (which ends it),
/// the video-title split, the primary-artist cut, then each rule in list order.
pub fn resolve(t: &Track, cfg: &ScrobbleConfig) -> Resolved {
    let mut r = Resolved {
        artist: t.artists.clone(),
        title: t.title.clone(),
        album: t.album.clone().unwrap_or_default(),
        skip: None,
        edit: None,
        split: false,
        rules: Vec::new(),
        errors: Vec::new(),
    };
    let compiled: Vec<Option<regex::Regex>> = cfg
        .rules
        .iter()
        .enumerate()
        .map(|(i, rule)| {
            if !rule.enabled || rule.find.is_empty() {
                return None;
            }
            regex::RegexBuilder::new(&rule.find)
                .case_insensitive(true)
                .build()
                .map_err(|e| r.errors.push((i, e.to_string())))
                .ok()
        })
        .collect();

    if let Some((i, e)) = cfg.edits.iter().enumerate().find(|(_, e)| e.matches(t)) {
        r.edit = Some(i);
        if e.skip {
            r.skip = Some("edit");
            return r;
        }
        for (have, want) in
            [(&mut r.artist, &e.artist), (&mut r.title, &e.title), (&mut r.album, &e.album)]
        {
            if !want.trim().is_empty() {
                *have = want.trim().to_owned();
            }
        }
        return finish(r);
    }

    if cfg.split_video_titles && t.is_video {
        if let Some((artist, title)) = split_title(&t.title) {
            r.artist = artist;
            r.title = title;
            r.split = true;
        }
    }
    if cfg.primary_artist {
        r.artist = primary_artist(&r.artist, cfg.primary_strict);
    }
    for (i, (rule, re)) in cfg.rules.iter().zip(&compiled).enumerate() {
        if let Some(re) = re {
            if apply_rule(re, &rule.field, &rule.replace, &mut r) {
                r.rules.push(i);
            }
        }
    }
    finish(r)
}

fn finish(mut r: Resolved) -> Resolved {
    for f in [&mut r.artist, &mut r.title, &mut r.album] {
        *f = f.trim().to_owned();
    }
    // A rule that wipes the title or artist leaves nothing Last.fm would take. An untagged local
    // file has a filename for a title and no artist behind it; submitting that writes "Unknown
    // artist" into the user's public profile, which is worse than not scrobbling it.
    if r.title.is_empty() || r.artist.is_empty() || r.artist == crate::local::UNKNOWN_ARTIST {
        r.skip = Some("incomplete");
    }
    r
}

const GROUPS: [&str; 3] = ["artist", "title", "album"];

/// One rule against `r`. True when it changed something.
fn apply_rule(re: &regex::Regex, field: &str, replace: &str, r: &mut Resolved) -> bool {
    let Some(value) = r.field(field) else { return false };
    let named: Vec<&str> = re.capture_names().flatten().filter(|n| GROUPS.contains(n)).collect();
    if named.is_empty() {
        let out = re.replace_all(value, replace);
        if out == value.as_str() {
            return false;
        }
        *value = out.into_owned();
        return true;
    }
    let Some(caps) = re.captures(value) else { return false };
    let found: Vec<(&str, String)> = named
        .iter()
        .filter_map(|n| caps.name(n).map(|m| (*n, m.as_str().trim().to_owned())))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let mut changed = false;
    for (name, v) in found {
        let f = r.field(name).expect("GROUPS are all fields");
        changed |= *f != v;
        *f = v;
    }
    changed
}

/// "Artist - Song" out of a video title: the first dash with spaces around it (hyphen, en or em
/// dash) and text on both sides.
fn split_title(title: &str) -> Option<(String, String)> {
    let (at, sep) =
        [" - ", " – ", " — "].iter().filter_map(|s| title.find(s).map(|i| (i, *s))).min()?;
    let (artist, song) = (title[..at].trim(), title[at + sep.len()..].trim());
    (!artist.is_empty() && !song.is_empty()).then(|| (artist.to_owned(), song.to_owned()))
}

/// The first credit of a multi-artist byline ("Artist A, Artist B" -> "Artist A"), for the
/// `lastfm_primary_artist` setting (issue #231). Last.fm has no notion of a joined credit, so the
/// full byline creates one artist literally named "Artist A, Artist B": dead page, split stats,
/// and no match against the real album.
///
/// Cuts at "," always. "&" only when `strict`, the sub-setting: a joint act ("Future & Metro
/// Boomin") is usually a real Last.fm artist with its own page, so cutting it misattributes every
/// one of its tracks, and the same goes for a duo whose name simply contains "&" (Simon &
/// Garfunkel). Users who want every play on one artist opt into that trade. "feat." needs no
/// handling either way, YouTube Music puts features in the track title, which is also where
/// Last.fm wants them.
///
/// This runs on the string, not on `artist_runs`, because the runs are gone exactly where this is
/// needed most: `backfill_metadata` clears them whenever it repairs the byline from
/// `videoDetails.author`, and a Listen Together guest's queue never had them.
// ponytail: separator scan. A real credit parser only if users report bylines it gets wrong.
fn primary_artist(artists: &str, strict: bool) -> String {
    let end = artists.find(|c: char| c == ',' || (strict && c == '&')).unwrap_or(artists.len());
    let first = artists[..end].trim();
    if first.is_empty() {
        artists.to_owned()
    } else {
        first.to_owned()
    }
}

/// When the scrobble fires, in seconds into the track: `percent` of it or `minutes` in, whichever
/// comes first. `None` means never: tracks under 30s don't scrobble (Last.fm's floor), and with the
/// minutes cap off a track of unknown length has nothing to time from yet.
fn scrobble_at(duration: f64, cfg: &ScrobbleConfig) -> Option<f64> {
    if duration > 0.0 && duration < 30.0 {
        return None;
    }
    let share = if duration > 0.0 {
        duration * f64::from(cfg.percent.clamp(1, 100)) / 100.0
    } else {
        f64::INFINITY
    };
    let cap = if cfg.minutes > 0 { f64::from(cfg.minutes) * 60.0 } else { f64::INFINITY };
    Some(share.min(cap)).filter(|at| at.is_finite())
}

fn crosses_threshold(pos: f64, duration: f64, cfg: &ScrobbleConfig) -> bool {
    scrobble_at(duration, cfg).is_some_and(|at| pos >= at)
}

// --- API plumbing ---------------------------------------------------------------------------

struct ApiError {
    /// Last.fm's numeric error code, when the response carried one (vs a transport failure).
    code: Option<i64>,
    message: String,
}

impl ApiError {
    fn transport(e: impl std::fmt::Display) -> Self {
        ApiError { code: None, message: e.to_string() }
    }
    fn retryable(&self) -> bool {
        matches!(self.code, Some(ERR_TOKEN_PENDING) | Some(ERR_TEMP_UNAVAILABLE) | None)
    }
}

/// One signed API call. `params` are method-specific; `api_key`, `method`, `api_sig`, and
/// `format=json` are added here (`format` is excluded from the signature, per the docs). Write
/// methods POST; auth reads GET.
async fn call(
    method: &str,
    mut params: Vec<(String, String)>,
    post: bool,
) -> Result<serde_json::Value, ApiError> {
    params.push(("api_key".to_string(), API_KEY.to_string()));
    params.push(("method".to_string(), method.to_string()));
    params.push(("api_sig".to_string(), sign(&params)));
    params.push(("format".to_string(), "json".to_string()));

    let http = crate::http::client();
    let req =
        if post { http.post(API_ROOT).form(&params) } else { http.get(API_ROOT).query(&params) };
    let resp = req.timeout(Duration::from_secs(15)).send().await.map_err(ApiError::transport)?;
    let body: serde_json::Value = resp.json().await.map_err(ApiError::transport)?;
    if let Some(code) = body.get("error").and_then(|v| v.as_i64()) {
        let message = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown last.fm error")
            .to_string();
        return Err(ApiError { code: Some(code), message });
    }
    Ok(body)
}

/// The `api_sig`: md5 over the params sorted by name, concatenated as `namevalue`, + the secret.
fn sign(params: &[(String, String)]) -> String {
    let mut sorted: Vec<_> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut s = String::new();
    for (k, v) in sorted {
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(API_SECRET);
    format!("{:x}", Md5::digest(s.as_bytes()))
}

// --- auth flow (connect / disconnect / status) ----------------------------------------------

fn emit_state(
    app: &tauri::AppHandle,
    connected: bool,
    username: Option<&str>,
    error: Option<&str>,
) {
    let _ = app.emit(
        "lastfm-state",
        serde_json::json!({ "connected": connected, "username": username, "error": error }),
    );
}

/// Start the connect flow: fetch a request token, open the authorize page in the user's browser,
/// and poll `auth.getSession` in the background until they approve (or the poll times out /
/// is superseded). Resolution arrives via the `lastfm-state` event, not this command.
pub async fn connect(state: Arc<AppState>) -> Result<(), String> {
    if API_KEY.is_empty() || API_SECRET.is_empty() {
        return Err("Last.fm isn't configured in this build — paste an API key into lastfm.rs \
                    (see https://www.last.fm/api/account/create)."
            .into());
    }
    let gen = state.lastfm.bump_gen();
    let token = call("auth.getToken", vec![], false)
        .await
        .map_err(|e| format!("Last.fm: {}", e.message))?
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or("Last.fm returned no token")?
        .to_string();

    open_browser(&format!("{AUTH_URL}?api_key={API_KEY}&token={token}"))?;

    tauri::async_runtime::spawn(async move {
        for _ in 0..AUTH_POLL_TRIES {
            tokio::time::sleep(AUTH_POLL_EVERY).await;
            if state.lastfm.gen() != gen {
                return; // superseded by a newer connect, or a disconnect
            }
            let params = vec![("token".to_string(), token.clone())];
            match call("auth.getSession", params, false).await {
                Ok(body) => {
                    let name = body.pointer("/session/name").and_then(|v| v.as_str());
                    let key = body.pointer("/session/key").and_then(|v| v.as_str());
                    let (Some(name), Some(key)) = (name, key) else {
                        emit_state(
                            &state.app,
                            false,
                            None,
                            Some("Last.fm sent a malformed session"),
                        );
                        return;
                    };
                    state.db.set_setting("lastfm_session_key", key);
                    state.db.set_setting("lastfm_username", name);
                    state.lastfm.set_session(Some(key.to_string()));
                    tracing::info!(user = name, "last.fm connected");
                    emit_state(&state.app, true, Some(name), None);
                    return;
                }
                Err(e) if e.retryable() => continue, // not approved yet (or transient) — keep polling
                Err(e) => {
                    emit_state(&state.app, false, None, Some(&format!("Last.fm: {}", e.message)));
                    return;
                }
            }
        }
        emit_state(&state.app, false, None, Some("Last.fm authorization timed out — try again"));
    });
    Ok(())
}

pub fn disconnect(state: &AppState) {
    state.lastfm.bump_gen(); // cancels any in-flight auth poll
    state.db.set_setting("lastfm_session_key", "");
    state.db.set_setting("lastfm_username", "");
    state.lastfm.set_session(None);
    emit_state(&state.app, false, None, None);
}

pub fn status(state: &AppState) -> serde_json::Value {
    let key = state.db.get_setting("lastfm_session_key").filter(|s| !s.is_empty());
    let username = state.db.get_setting("lastfm_username").filter(|s| !s.is_empty());
    serde_json::json!({ "connected": key.is_some(), "username": username })
}

/// The connected user's public profile, for the Scrobbling tab's account card. `None` when nobody
/// is connected or Last.fm didn't answer: the card falls back to the name it already has.
pub async fn profile(state: &AppState) -> Option<Profile> {
    let name = state.db.get_setting("lastfm_username").filter(|s| !s.is_empty())?;
    let body = call("user.getInfo", vec![("user".to_string(), name)], false).await.ok()?;
    parse_profile(&body)
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Profile {
    /// The avatar, or `None` for an account that never set one.
    image: Option<String>,
    url: Option<String>,
    scrobbles: u64,
    artists: u64,
    tracks: u64,
    /// Epoch seconds the account was created.
    since: u64,
}

/// `user.getInfo` sends every number as a string, and an unset avatar as `""` in each size.
fn parse_profile(body: &serde_json::Value) -> Option<Profile> {
    let u = body.get("user")?;
    let num = |v: Option<&serde_json::Value>| {
        v.and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0)
    };
    let images = u.get("image").and_then(|v| v.as_array());
    // 174px: the card draws it at 44, so twice that and change for a HiDPI screen.
    let image = ["large", "extralarge", "medium"].iter().find_map(|size| {
        images?
            .iter()
            .find(|i| i.get("size").and_then(|s| s.as_str()) == Some(size))?
            .get("#text")?
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    });
    Some(Profile {
        image,
        url: u.get("url").and_then(|v| v.as_str()).map(str::to_owned),
        scrobbles: num(u.get("playcount")),
        artists: num(u.get("artist_count")),
        tracks: num(u.get("track_count")),
        since: num(u.pointer("/registered/unixtime")),
    })
}

/// Open a URL in the user's default browser. No opener plugin in the app; three lines cover the
/// three platforms.
pub(crate) fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let cmd = {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(url);
        unappimage(&mut cmd);
        cmd.spawn()
    };
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    // cmd.exe re-parses its own command line, and `Command::arg` only quotes args containing
    // spaces — so an unquoted `&` in the URL split it into a second command and the browser got
    // `…/auth?api_key=X` with the token chopped off ("Invalid API key" once the user clicks Allow).
    // raw_arg passes the quoted URL through verbatim.
    #[cfg(target_os = "windows")]
    let cmd = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd").raw_arg(format!("/C start \"\" \"{url}\"")).spawn()
    };
    cmd.map(|_| ()).map_err(|e| format!("Couldn't open the browser: {e}"))
}

/// Undo the AppImage's environment for a child process.
///
/// Inside an AppImage the AppDir is first on `LD_LIBRARY_PATH` (plus `GTK_PATH`, `GIO_MODULE_DIR`,
/// `XDG_DATA_DIRS`, …) and every child inherits it. `xdg-open` immediately hands off to a *host*
/// binary (`kde-open`, `gio`), which then links the host's libcurl against our bundled libssl,
/// built on Ubuntu 24.04 (OpenSSL 3.0) and missing every symbol version added since:
///
/// ```text
/// kde-open: /tmp/.mount_limusiXXXXXX/usr/lib/libssl.so.3: version `OPENSSL_3.5.0' not found
///           (required by /lib64/libcurl.so.4)
/// ```
///
/// The opener dies before the browser ever starts, and since `spawn` itself succeeded, Last.fm's
/// connect flow just sat there polling forever (issue #50, Fedora 44 / KDE).
///
/// Same bug class as the bundled-libwayland breakage: our libs shadowing the host's for code we
/// don't control. Fixed the same way, by scope: strip AppDir paths from the child, keep the user's.
#[cfg(target_os = "linux")]
fn unappimage(cmd: &mut std::process::Command) {
    let Some(appdir) = std::env::var("APPDIR").ok().filter(|d| !d.is_empty()) else { return };
    for (key, value) in std::env::vars_os() {
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else { continue };
        if !value.contains(&appdir) {
            continue;
        }
        match strip_appdir(value, &appdir) {
            Some(kept) => cmd.env(key, kept),
            None => cmd.env_remove(key),
        };
    }
}

/// Drop AppDir-rooted entries from a colon-separated env value. `None` means nothing survived, so
/// the variable should be unset rather than set to an empty string (an empty `LD_LIBRARY_PATH`
/// entry means "current directory").
#[cfg(target_os = "linux")]
fn strip_appdir(value: &str, appdir: &str) -> Option<String> {
    let kept: Vec<&str> =
        value.split(':').filter(|p| !p.starts_with(appdir) && !p.is_empty()).collect();
    (!kept.is_empty()).then(|| kept.join(":"))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The signature is the auth-critical path: params sorted by name, `namevalue` concat, secret
    /// appended, md5 hex. Verified against a hand-computed digest (secret is "" in test builds).
    #[test]
    fn api_sig_is_sorted_concat_md5() {
        let params = vec![
            ("method".to_string(), "auth.getSession".to_string()),
            ("api_key".to_string(), "abc".to_string()),
            ("token".to_string(), "xyz".to_string()),
        ];
        // Sorted: api_key abc, method auth.getSession, token xyz
        let expected = format!(
            "{:x}",
            Md5::digest(format!("api_keyabcmethodauth.getSessiontokenxyz{API_SECRET}").as_bytes())
        );
        assert_eq!(sign(&params), expected);
    }

    /// Issue #50: what the child of `xdg-open` must not inherit.
    #[cfg(target_os = "linux")]
    #[test]
    fn strip_appdir_keeps_host_paths_and_drops_the_var_when_nothing_is_left() {
        let dir = "/tmp/.mount_limusiNnkaBP";
        // LD_LIBRARY_PATH as AppRun.wrapped writes it: AppDir dirs first, user's value appended.
        assert_eq!(
            strip_appdir(&format!("{dir}/usr/lib/:{dir}/usr/lib64/:/opt/mine/lib"), dir),
            Some("/opt/mine/lib".to_string())
        );
        // Nothing but AppDir dirs (the usual case): unset it, don't leave "" behind.
        assert_eq!(strip_appdir(&format!("{dir}/usr/lib:{dir}/usr/lib64"), dir), None);
        // A lone AppDir value, e.g. GSETTINGS_SCHEMA_DIR.
        assert_eq!(strip_appdir(dir, dir), None);
        // XDG_DATA_DIRS keeps the host's dirs, which is how xdg-open finds the .desktop handlers.
        assert_eq!(
            strip_appdir(&format!("{dir}/usr/share:/usr/share:/usr/local/share"), dir),
            Some("/usr/share:/usr/local/share".to_string())
        );
    }

    /// Trimmed from a real `user.getInfo` answer (2026-10-06).
    #[test]
    fn profile_reads_user_info() {
        let body = serde_json::json!({ "user": {
            "name": "someone", "playcount": "7805", "artist_count": "1363", "track_count": "2309",
            "url": "https://www.last.fm/user/someone",
            "image": [
                { "size": "small", "#text": "https://lastfm-img.freetls.fastly.net/i/u/34s/a.png" },
                { "size": "large", "#text": "https://lastfm-img.freetls.fastly.net/i/u/174s/a.png" },
                { "size": "extralarge", "#text": "https://lastfm-img.freetls.fastly.net/i/u/300x300/a.png" }
            ],
            "registered": { "unixtime": "1784282707", "#text": 1784282707 }
        }});
        let p = parse_profile(&body).unwrap();
        assert_eq!(
            p.image.as_deref(),
            Some("https://lastfm-img.freetls.fastly.net/i/u/174s/a.png")
        );
        assert_eq!((p.scrobbles, p.artists, p.tracks, p.since), (7805, 1363, 2309, 1784282707));
        // No avatar set: every size is "", which is no image rather than a broken one.
        let bare = serde_json::json!({ "user": { "image": [{ "size": "large", "#text": "" }] } });
        assert_eq!(parse_profile(&bare).unwrap().image, None);
        assert_eq!(parse_profile(&serde_json::json!({ "error": 6 })), None);
    }

    #[test]
    fn primary_artist_cuts_at_commas_and_only_at_ampersands_when_strict() {
        assert_eq!(primary_artist("Artist A, Artist B", false), "Artist A");
        assert_eq!(primary_artist("Kendrick Lamar,SZA", false), "Kendrick Lamar");
        // A joint act stays whole until the user opts into the strict cut.
        assert_eq!(primary_artist("Future & Metro Boomin", false), "Future & Metro Boomin");
        assert_eq!(primary_artist("Future & Metro Boomin", true), "Future");
        assert_eq!(primary_artist("Simon&Garfunkel", true), "Simon");
        // Whichever separator comes first wins.
        assert_eq!(primary_artist("A & B, C", true), "A");
        assert_eq!(primary_artist("A, B & C", true), "A");
        // A lone artist, and a byline that starts with the separator, come back untouched.
        assert_eq!(primary_artist("Delara", true), "Delara");
        assert_eq!(primary_artist(", Artist B", true), ", Artist B");
        assert_eq!(primary_artist("& Juliet", true), "& Juliet");
    }

    #[test]
    fn scrobble_threshold_follows_lastfm_rules_by_default() {
        let cfg = ScrobbleConfig::default();
        // Half the track wins for short tracks…
        assert!(!crosses_threshold(89.0, 180.0, &cfg));
        assert!(crosses_threshold(90.0, 180.0, &cfg));
        // …4 minutes wins for long ones.
        assert!(!crosses_threshold(239.0, 1200.0, &cfg));
        assert!(crosses_threshold(240.0, 1200.0, &cfg));
        // Under 30s never scrobbles.
        assert!(!crosses_threshold(29.0, 20.0, &cfg));
        // Unknown duration: only the 4-minute rule applies.
        assert!(!crosses_threshold(120.0, 0.0, &cfg));
        assert!(crosses_threshold(240.0, 0.0, &cfg));
    }

    #[test]
    fn scrobble_threshold_follows_the_users_numbers() {
        let cfg = ScrobbleConfig { percent: 90, minutes: 0, ..Default::default() };
        // No cap: a 20-minute mix scrobbles at 18 minutes, not at 4.
        assert!(!crosses_threshold(1000.0, 1200.0, &cfg));
        assert!(crosses_threshold(1080.0, 1200.0, &cfg));
        // Nothing to time from until mpv reports the length.
        assert_eq!(scrobble_at(0.0, &cfg), None);
        let cfg = ScrobbleConfig { percent: 100, minutes: 2, ..Default::default() };
        assert_eq!(scrobble_at(200.0, &cfg), Some(120.0));
        // Out-of-range percentages clamp instead of never firing.
        let cfg = ScrobbleConfig { percent: 0, minutes: 0, ..Default::default() };
        assert_eq!(scrobble_at(200.0, &cfg), Some(2.0));
    }

    fn track(title: &str, artists: &str, is_video: bool) -> Track {
        Track {
            video_id: "T1Vkc5w79-M".into(),
            title: title.into(),
            artists: artists.into(),
            album: None,
            is_video,
        }
    }

    fn rule(field: &str, find: &str, replace: &str) -> Rule {
        Rule {
            field: field.into(),
            find: find.into(),
            replace: replace.into(),
            ..Default::default()
        }
    }

    /// The default config sends YouTube's metadata untouched, which is what every install did
    /// before the tab existed.
    #[test]
    fn default_config_changes_nothing() {
        let t = Track { album: Some("Album".into()), ..track("Song - Live", "A, B", true) };
        let r = resolve(&t, &ScrobbleConfig::default());
        assert_eq!(
            (r.artist.as_str(), r.title.as_str(), r.album.as_str()),
            ("A, B", "Song - Live", "Album")
        );
        assert!(r.skip.is_none() && r.edit.is_none() && !r.split && r.rules.is_empty());
        // The unparseable blob is the same default.
        assert!(ScrobbleConfig::parse("{oops").enabled);
    }

    /// Issue #404's example: a soundtrack upload credited to the uploader's channel.
    #[test]
    fn video_titles_split_into_artist_and_song_when_asked() {
        let t = track(
            "Ruelle - Fire Meets Fate - Shadowhunters 3x10 Music (Mid Season Finale)",
            "alexias788",
            true,
        );
        let cfg = ScrobbleConfig { split_video_titles: true, ..Default::default() };
        let r = resolve(&t, &cfg);
        assert!(r.split);
        assert_eq!(r.artist, "Ruelle");
        assert_eq!(r.title, "Fire Meets Fate - Shadowhunters 3x10 Music (Mid Season Finale)");
        // Songs are never split, whatever their title says.
        let r = resolve(&track("Song - Remastered 2011", "Band", false), &cfg);
        assert!(!r.split && r.artist == "Band");
        // An en dash counts; a dash with no artist before it doesn't.
        assert_eq!(
            split_title("Ruelle – War of Hearts"),
            Some(("Ruelle".into(), "War of Hearts".into()))
        );
        assert_eq!(split_title(" - Intro"), None);
        assert_eq!(split_title("Re-Run"), None);
    }

    #[test]
    fn rules_replace_in_order_and_ignore_case() {
        let cfg = ScrobbleConfig {
            rules: vec![
                rule("title", r"\s*[(\[](?:official\s+)?(?:music\s+)?video[)\]]", ""),
                rule("artist", "VEVO$", ""),
                rule("album", "x", "y"), // no album: nothing to do, nothing reported
            ],
            ..Default::default()
        };
        let r = resolve(&track("Higher Ground [OFFICIAL Music Video]", "NovaSkyVEVO", true), &cfg);
        assert_eq!((r.title.as_str(), r.artist.as_str()), ("Higher Ground", "NovaSky"));
        assert_eq!(r.rules, vec![0, 1]);
        // (?-i) makes a rule case-sensitive again.
        let cfg =
            ScrobbleConfig { rules: vec![rule("artist", "(?-i)vevo$", "")], ..Default::default() };
        assert!(resolve(&track("x", "NovaSkyVEVO", false), &cfg).rules.is_empty());
    }

    #[test]
    fn named_groups_move_text_between_fields() {
        let cfg = ScrobbleConfig {
            rules: vec![rule("title", r"^(?<artist>.+?) - (?<title>.+)$", "ignored")],
            ..Default::default()
        };
        let r = resolve(&track("Ruelle - Fire Meets Fate", "alexias788", false), &cfg);
        assert_eq!((r.artist.as_str(), r.title.as_str()), ("Ruelle", "Fire Meets Fate"));
        // No match, no change.
        let r = resolve(&track("Fire Meets Fate", "Ruelle", false), &cfg);
        assert!(r.rules.is_empty() && r.title == "Fire Meets Fate");
    }

    #[test]
    fn a_broken_rule_is_reported_and_skipped() {
        let cfg = ScrobbleConfig {
            rules: vec![rule("title", "(unclosed", ""), rule("title", "Song", "Track")],
            ..Default::default()
        };
        let r = resolve(&track("Song", "A", false), &cfg);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].0, 0);
        assert_eq!(r.title, "Track");
        // A rule that empties the artist leaves nothing to send.
        let cfg = ScrobbleConfig { rules: vec![rule("artist", ".*", "")], ..Default::default() };
        assert_eq!(resolve(&track("Song", "A", false), &cfg).skip, Some("incomplete"));
    }

    #[test]
    fn an_edit_is_final_and_can_skip() {
        let edit = Edit {
            key: "T1Vkc5w79-M".into(),
            artist: "Ruelle".into(),
            title: "Fire Meets Fate".into(),
            ..Default::default()
        };
        let cfg = ScrobbleConfig {
            edits: vec![edit.clone()],
            rules: vec![rule("artist", "Ruelle", "Nope")],
            split_video_titles: true,
            ..Default::default()
        };
        let t =
            Track { album: Some("Shadowhunters".into()), ..track("whatever", "alexias788", true) };
        let r = resolve(&t, &cfg);
        assert_eq!(r.edit, Some(0));
        // Rules and the split don't touch an edited track; an empty edit field keeps YouTube's.
        assert_eq!(
            (r.artist.as_str(), r.title.as_str(), r.album.as_str()),
            ("Ruelle", "Fire Meets Fate", "Shadowhunters")
        );
        assert!(!r.split && r.rules.is_empty());
        // Another video is not this edit's business.
        let other = Track { video_id: "dQw4w9WgXcQ".into(), ..track("x", "y", false) };
        assert!(resolve(&other, &cfg).edit.is_none());
        let cfg = ScrobbleConfig { edits: vec![Edit { skip: true, ..edit }], ..Default::default() };
        assert_eq!(resolve(&t, &cfg).skip, Some("edit"));
    }

    /// Pano Scrobbler's edits and blocklist carry no video id: they match on the metadata, and an
    /// empty field matches anything.
    #[test]
    fn keyless_edits_match_on_metadata() {
        let block = Edit { from_artist: "white noise co".into(), skip: true, ..Default::default() };
        let cfg = ScrobbleConfig { edits: vec![block], ..Default::default() };
        assert_eq!(resolve(&track("Rain 10h", "White Noise Co", false), &cfg).skip, Some("edit"));
        assert!(resolve(&track("Rain 10h", "Someone", false), &cfg).skip.is_none());
        // An edit with nothing to match on matches nothing, not everything.
        let cfg = ScrobbleConfig {
            edits: vec![Edit { skip: true, ..Default::default() }],
            ..Default::default()
        };
        assert!(resolve(&track("a", "b", false), &cfg).skip.is_none());
    }

    #[test]
    fn untagged_local_files_never_scrobble() {
        let r = resolve(
            &track("01 - track", crate::local::UNKNOWN_ARTIST, false),
            &ScrobbleConfig::default(),
        );
        assert_eq!(r.skip, Some("incomplete"));
    }
}
