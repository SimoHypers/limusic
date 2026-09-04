//! App state: transport, player, db, and the queue/playback manager. context/11.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use std::sync::Arc;

use innertube::{
    AccountIdentity, AccountInfo, AudioQuality, Clients, InnerTube, SongItem, MAIN_CLIENT,
};
use listen_protocol::{Playback, PlaybackKind, Track};
use player::Player;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::db::{now_secs, Db};
use crate::discord::DiscordHandle;
use crate::listentogether::{LtSession, SyncCommand};
use crate::media::MediaHandle;
use crate::orchestrator::{Orchestrator, PlaybackData, PlaybackPing, ResolveError};

/// Synthetic browseId for the On Repeat playlist. Not a YouTube id: `get_playlist` intercepts it
/// and builds the page from local play counts, so it must never collide with a real browseId
/// (`VL…` / `MPRE…` / `RD…`).
pub const ON_REPEAT_ID: &str = "LIMUSIC_ON_REPEAT";
/// How far back On Repeat looks. A month is long enough to survive a quiet week and short enough
/// that the list still turns over with what you're actually playing.
pub const ON_REPEAT_WINDOW_SECS: i64 = 30 * 24 * 60 * 60;
/// How many songs it holds.
pub const ON_REPEAT_LIMIT: usize = 20;

pub struct AppState {
    pub it: InnerTube,
    pub clients: Clients,
    pub player: Player,
    pub db: Arc<Db>,
    pub app: AppHandle,
    pub orchestrator: Arc<Orchestrator>,
    /// Listen Together session (context/19). Drives host broadcasts + guest gating.
    pub lt: Arc<LtSession>,
    /// mpv's on-disk audio cache dir (context/14) — wiped by the settings "Clear caches" action.
    cache_dir: std::path::PathBuf,
    /// OS media integration (MPRIS/SMTC/NowPlaying). `None` if it failed to init. context/16.
    media: Option<MediaHandle>,
    /// Discord rich presence. Fed the same track/playback changes as `media`; gated on the
    /// `discord_rpc` setting inside its own thread.
    discord: Option<DiscordHandle>,
    /// Last.fm scrobbler. Same feed again; parks until a session key is set (titlebar button).
    pub lastfm: crate::lastfm::LastfmHandle,
    queue: Mutex<QueueState>,
    /// Bumped on every explicit `play`/jump so superseded async resolves discard their result
    /// (cancellation without JoinHandle bookkeeping). context/06 §6.
    generation: AtomicU64,
    /// Bumped by the `rate` command. [`Self::refresh_rating`] samples it before its round trip
    /// and throws the answer away if it moved: a like landing in that ~400 ms window is newer
    /// than what YouTube was asked, and applying the stale reply would flip the heart back.
    pub rate_epoch: AtomicU64,
    /// A one-shot resume position `(videoId, secs)` set by `restore_queue` and consumed by the
    /// next `start_current` — applied only when that track is the one being started, so jumping to
    /// a different track first doesn't inherit the old position (context/11).
    pending_seek: std::sync::Mutex<Option<(String, f64)>>,
    /// Whether the watch-history ping has already fired for the current play (latched, so the
    /// frequent position events fire it exactly once; cleared wherever a fresh play starts).
    /// An atomic rather than a `QueueState` field because `on_position` runs on every raw mpv
    /// tick, well above the 4 Hz the UI emit is throttled to, and the flag is latched true for all
    /// but the first ~30 seconds of each track: taking the app's most contended lock to read it
    /// stalled the event pump behind `emit_queue`, `persist_queue`, `prime_lookahead` and the
    /// Listen Together broadcasts for nothing.
    history_pinged: AtomicBool,
    /// Mirror of mpv's pause flag (set in `media_set_playing`). Position ticks must consult this
    /// instead of assuming "playing" — mpv fires `time-pos` on seeks while paused too.
    is_playing: AtomicBool,
    /// Latest mpv position (f64 bits) + wall-clock secs of the last DB write, for throttled
    /// resume-position persistence.
    latest_position: AtomicU64,
    last_pos_persist: AtomicU64,
    /// Wall-clock secs of the last position push to the OS media controls (throttled ~1s).
    last_media_push: AtomicU64,
    /// Last `(playing, position)` actually handed to the OS media controls, so the steady-state
    /// scrubber tick can be dropped. souvlaki emits a D-Bus `PropertiesChanged` broadcast on every
    /// `set_playback` even when nothing changed, waking every MPRIS client on the desktop; and
    /// MPRIS `Position` is read-on-demand, so the signal does not even carry the one value that
    /// moved. It cannot be dropped entirely: souvlaki answers a `Position` read from the progress
    /// of the last push, so the value still has to be refreshed periodically.
    last_media_state: std::sync::Mutex<Option<(bool, f64)>>,
    /// videoId → (the googlevideo URL the loopback proxy streams from, unix seconds it stops being
    /// treated as usable). Written by the `video_stream` command, read by `videoproxy::handle` and
    /// by `video_stream` itself: an entry that is still live answers a reopen with no network at
    /// all, where re-resolving costs up to two `/player` round trips the user waits through.
    /// ponytail: cleared wholesale at 8 entries rather than LRU-evicted. The player view only ever
    /// looks at one track at a time, so a real LRU would be book-keeping for nothing.
    video_urls: std::sync::Mutex<std::collections::HashMap<String, (String, i64)>>,
    /// Fingerprint of the item list the UI was last sent (see `queue_fingerprint`). When it is
    /// unchanged, `emit_queue` sends the small `queue-index` event instead of megabytes of JSON:
    /// a Tauri event inlines its payload into a JavaScript source string, so a 5,000-track queue
    /// costs ~26 ms of parse in the webview every time it crosses.
    last_queue_fingerprint: AtomicU64,
    /// Dirty key (`persist_fingerprint`) last written to `queue_json`. Same trick as
    /// `last_queue_fingerprint`, for the persistence side: an advance rewrites 40 bytes rather
    /// than the whole blob.
    last_persisted_fingerprint: AtomicU64,
    /// One authentication transition at a time: [`Self::sign_in`], [`Self::switch_account`],
    /// [`Self::sign_out`]. Each reads the cookie and identity, awaits YouTube, then writes both to
    /// the transport and the database, so two of them overlapping lets the loser land last and
    /// win: a failed heal restoring its stale snapshot over a fresh login, or a heal completing
    /// after the user signed out and putting them back in. Async because it is held across those
    /// awaits, which a std `Mutex` cannot be.
    auth: tokio::sync::Mutex<()>,
}

/// Repeat mode for the queue. Serialized lowercase for the UI + `queue_json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// Which queue a background playlist walk ([`AppState::fill_playlist`]) is filling: the one that's
/// playing, or an "Add to queue" block (whose pages keep their order and carry the block's label).
enum Fill {
    Playing,
    Queued(Option<String>),
}

/// Canonical persisted account selection. `data_sync_id` drives request delegation; every display
/// field beside it belongs to that same identity. `account_json` and the legacy `data_sync_id`
/// setting are written only as atomic projections of this model.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct SelectedIdentity {
    #[serde(default)]
    data_sync_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    has_multiple_identities: bool,
}

pub enum SignInOutcome {
    Complete,
    SelectionRequired,
}

impl SelectedIdentity {
    fn from_account_info(info: AccountInfo, has_multiple_identities: bool) -> Option<Self> {
        Some(Self {
            data_sync_id: info.data_sync_id,
            name: Some(info.name?),
            handle: info.handle,
            email: info.email,
            thumbnail: info.thumbnail,
            channel_id: info.channel_id,
            has_multiple_identities,
        })
    }

    fn from_identity(
        identity: &AccountIdentity,
        refreshed: AccountInfo,
        has_multiple_identities: bool,
    ) -> Self {
        Self {
            data_sync_id: Some(identity.data_sync_id.clone()),
            name: refreshed.name.or_else(|| Some(identity.name.clone())),
            handle: refreshed.handle.or_else(|| identity.handle.clone()),
            email: refreshed.email.or_else(|| identity.email.clone()),
            thumbnail: refreshed.thumbnail.or_else(|| identity.thumbnail.clone()),
            channel_id: refreshed.channel_id.or_else(|| identity.channel_id.clone()),
            has_multiple_identities,
        }
    }

    fn account_json(&self, signed_in: bool) -> serde_json::Value {
        serde_json::json!({
            "signedIn": signed_in,
            "name": self.name,
            "handle": self.handle,
            "email": self.email,
            "thumbnail": self.thumbnail,
            "channelId": self.channel_id,
            "canSwitch": self.has_multiple_identities,
        })
    }

    fn as_account_identity(&self) -> Option<AccountIdentity> {
        Some(AccountIdentity {
            name: self.name.clone().unwrap_or_default(),
            handle: self.handle.clone(),
            email: self.email.clone(),
            thumbnail: self.thumbnail.clone(),
            channel_id: self.channel_id.clone(),
            data_sync_id: self.data_sync_id.clone()?,
            is_selected: false,
        })
    }
}

fn selected_identity_from_db(db: &Db) -> Option<SelectedIdentity> {
    db.get_setting("selected_identity_json")
        .and_then(|json| serde_json::from_str::<SelectedIdentity>(&json).ok())
        .filter(|identity| identity.name.is_some() || identity.data_sync_id.is_some())
        .or_else(|| {
            let account = db
                .get_setting("account_json")
                .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())?;
            let string = |key: &str| {
                account
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            };
            Some(SelectedIdentity {
                data_sync_id: db.get_setting("data_sync_id").filter(|id| !id.is_empty()),
                name: string("name"),
                handle: string("handle"),
                email: string("email"),
                thumbnail: string("thumbnail"),
                channel_id: string("channelId"),
                has_multiple_identities: account
                    .get("canSwitch")
                    .and_then(serde_json::Value::as_bool)
                    // Pre-switcher account_json has no flag. Keep the action discoverable until
                    // the startup identity refresh determines whether this legacy user has one
                    // channel or several.
                    .unwrap_or(true),
            })
        })
}

fn auth_selection_pending(db: &Db) -> bool {
    db.get_setting("account_selection_pending").as_deref() == Some("true")
}

/// Startup compatibility: prefer the canonical model, then fall back to the pre-switcher setting.
pub(crate) fn persisted_data_sync_id(db: &Db) -> Option<String> {
    selected_identity_from_db(db)
        .and_then(|identity| identity.data_sync_id)
        .or_else(|| db.get_setting("data_sync_id").filter(|id| !id.is_empty()))
}

fn identity_selection_key(data_sync_id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    "limusic-account-identity-v1".hash(&mut hasher);
    data_sync_id.hash(&mut hasher);
    format!("identity-{:016x}", hasher.finish())
}

fn identity_snapshot(identity: &AccountIdentity, selected: bool) -> serde_json::Value {
    serde_json::json!({
        "selectionKey": identity_selection_key(&identity.data_sync_id),
        "name": identity.name,
        "handle": identity.handle,
        "email": identity.email,
        "thumbnail": identity.thumbnail,
        "channelId": identity.channel_id,
        "selected": selected,
    })
}

#[derive(Default)]
struct QueueState {
    items: Vec<SongItem>,
    current: usize,
    /// Start of the previously-played run: `items[played_from..current]` is what has actually been
    /// heard (or skipped past) in this queue, and what the panel's "Previously played" section
    /// shows. Not simply `0..current`: starting a playlist at track 7 leaves six untouched tracks
    /// sitting in front of the playing one.
    played_from: usize,
    /// Pre-shuffle order snapshot. `Some(..)` ⇔ shuffle is ON; restored on shuffle-off.
    shuffle_orig: Option<Vec<SongItem>>,
    repeat: RepeatMode,
    /// Radio playlist id for autoplay continuation: `RDAMPL<plId>` when the queue came from a
    /// playlist/album, `None` otherwise (autoplay then seeds `RDAMVM<last video>`, so long
    /// sessions drift with what's actually playing, like YTM's).
    radio_seed: Option<String>,
    /// Human name of what seeded the queue (playlist/album title, "<song> Radio") — the queue
    /// panel's "Next from: …" header. Pure display metadata.
    source_name: Option<String>,
    /// This queue is a radio: YouTube generated every upcoming track, so "Add to queue" replaces
    /// them rather than queueing behind an endless feed the user never asked to finish.
    radio: bool,
    /// The queue index we've already appended to mpv for gapless lookahead (if any).
    lookahead_loaded: Option<usize>,
    /// Which client served the currently-loaded track (for the WEB_REMIX-403 feedback). context/06.
    current_client: Option<String>,
    /// The client that served the primed lookahead track — promoted to `current_client` on a
    /// gapless advance so the failure feedback still knows the client.
    lookahead_client: Option<String>,
    /// Loudness gain (dB) for the primed lookahead. mpv's `af` is global, so this can't ride along
    /// with the appended entry — it's applied when the gapless advance is observed.
    lookahead_gain: Option<Option<f64>>,
    /// Watch-history ping for the current track + the primed lookahead's (promoted on a gapless
    /// advance, mirroring current/lookahead_client). context/01 §registerPlayback.
    playback_ping: Option<PlaybackPing>,
    lookahead_playback_ping: Option<PlaybackPing>,
    /// Content Playback Nonce for the current play. context/01. (The "already pinged" latch that
    /// used to live beside it is `AppState::history_pinged`.)
    cpn: String,
    /// Latest mpv-reported track duration (secs), for the history-ping threshold.
    duration: f64,
    /// Last videoId we re-resolved after a playback failure — guards the one-shot retry in
    /// `on_track_failed` against a retry loop when the retried stream dies too.
    retried: Option<String>,
}

impl QueueState {
    /// Move the play pointer within the same queue. Jumping forward counts everything passed over
    /// as played; going back drags the start of the played run with it, so nothing ever shows as
    /// previously played while it sits ahead of the playing track.
    fn seek_to(&mut self, index: usize) {
        self.current = index;
        self.played_from = self.played_from.min(index);
    }
}

/// How long a resolved googlevideo URL is treated as usable. YouTube's links last about six hours;
/// four is the conservative margin, and the cost of guessing low is one extra resolve.
const VIDEO_URL_TTL_SECS: i64 = 4 * 60 * 60;

/// How many resolved URLs are kept. Small on purpose: the map exists so a reopen and a replay are
/// free, not to be a download cache.
const VIDEO_URL_CAP: usize = 8;

type VideoUrls = std::collections::HashMap<String, (String, i64)>;

fn put_url(map: &mut VideoUrls, id: &str, url: String, now: i64) {
    if map.len() >= VIDEO_URL_CAP {
        map.clear();
    }
    map.insert(id.to_owned(), (url, now + VIDEO_URL_TTL_SECS));
}

fn get_url(map: &VideoUrls, id: &str, now: i64) -> Option<String> {
    let (url, expires_at) = map.get(id)?;
    (*expires_at > now).then(|| url.clone())
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        it: InnerTube,
        clients: Clients,
        player: Player,
        db: Arc<Db>,
        app: AppHandle,
        orchestrator: Arc<Orchestrator>,
        lt: Arc<LtSession>,
        cache_dir: std::path::PathBuf,
        media: Option<MediaHandle>,
        discord: Option<DiscordHandle>,
        lastfm: crate::lastfm::LastfmHandle,
    ) -> Self {
        AppState {
            it,
            clients,
            player,
            db,
            app,
            orchestrator,
            lt,
            cache_dir,
            media,
            discord,
            lastfm,
            queue: Mutex::new(QueueState::default()),
            auth: tokio::sync::Mutex::default(),
            history_pinged: AtomicBool::new(false),
            is_playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            rate_epoch: AtomicU64::new(0),
            pending_seek: std::sync::Mutex::new(None),
            video_urls: std::sync::Mutex::new(std::collections::HashMap::new()),
            latest_position: AtomicU64::new(0),
            last_pos_persist: AtomicU64::new(0),
            last_media_push: AtomicU64::new(0),
            last_media_state: std::sync::Mutex::new(None),
            last_queue_fingerprint: AtomicU64::new(0),
            last_persisted_fingerprint: AtomicU64::new(0),
        }
    }

    /// Remember where the loopback video proxy should fetch `id` from.
    pub fn put_video_url(&self, id: &str, url: String) {
        let Ok(mut map) = self.video_urls.lock() else { return };
        put_url(&mut map, id, url, crate::db::now_secs());
    }

    pub fn video_url(&self, id: &str) -> Option<String> {
        let map = self.video_urls.lock().ok()?;
        get_url(&map, id, crate::db::now_secs())
    }

    /// Drop a remembered URL the `<video>` element could not load. A googlevideo link that has
    /// expired early, or been revoked, looks exactly like this from the webview, and without this
    /// the cache would keep handing back the dead one until its TTL ran out.
    pub fn forget_video_url(&self, id: &str) {
        if let Ok(mut map) = self.video_urls.lock() {
            map.remove(id);
        }
    }

    fn quality(&self) -> AudioQuality {
        match self.db.get_setting("quality").as_deref() {
            Some("LOW") => AudioQuality::Low,
            Some("AUTO") => AudioQuality::Auto,
            _ => AudioQuality::High,
        }
    }

    /// User-disabled stream clients — comma-separated setting. Also the force-fail lever for the
    /// rustypipe-solo acceptance test; `LIMUSIC_DISABLED_CLIENTS` env overrides for quick testing.
    fn disabled_clients(&self) -> HashSet<String> {
        let raw = std::env::var("LIMUSIC_DISABLED_CLIENTS")
            .ok()
            .or_else(|| self.db.get_setting("disabled_stream_clients"))
            .unwrap_or_default();
        raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    }

    // --- auth (context/15) ------------------------------------------------------------------

    /// Sign in with the Cookie header captured from the login webview (context/15 Path A).
    ///
    /// `account_menu` validates the cookie and provides the active header. `accounts_list` then
    /// discovers every server-selectable identity. A persisted matching identity wins over
    /// Google's current default; a new multi-channel login pauses before finalization and asks the
    /// UI to choose.
    pub async fn sign_in(&self, cookie: String) -> Result<SignInOutcome, String> {
        let _turn = self.auth.lock().await;
        let cookie = cookie.trim().to_owned();
        if innertube::cookie_sapisid(&cookie).is_none() {
            return Err("Sign-in didn't complete — try signing in again.".into());
        }
        let previous_cookie = self.it.cookie();
        let previous_data_sync_id = self.it.data_sync_id();
        let persisted_identity = selected_identity_from_db(&self.db);
        let persisted_id = persisted_identity
            .as_ref()
            .and_then(|identity| identity.data_sync_id.clone())
            .or_else(|| self.db.get_setting("data_sync_id").filter(|id| !id.is_empty()));

        self.it.set_cookie(Some(cookie.clone()));
        // Discovery must not inherit a stale/default identity. The persisted id is restored only
        // after the fresh accounts list proves that this cookie can act as it.
        self.it.set_data_sync_id(None);
        let client =
            self.clients.get(innertube::METADATA_CLIENT).ok_or("metadata client missing")?;
        let active = match self.it.account_menu(client).await {
            // A valid, authenticating cookie returns the account header (name). No name means the
            // session didn't actually authenticate — reject it up front so we don't "succeed" into
            // a silently-empty library.
            Ok(i) if i.name.is_some() => i,
            Ok(_) => {
                self.restore_auth_transport(previous_cookie, previous_data_sync_id);
                return Err("That session didn't authenticate — sign in again.".into());
            }
            // Auth didn't take (network) — roll back so we're not half-logged-in.
            Err(e) => {
                self.restore_auth_transport(previous_cookie, previous_data_sync_id);
                return Err(format!("Sign-in failed: {e}"));
            }
        };

        let active_visitor_data = active.visitor_data.clone();

        // Discovery must go out with the login-bound visitorData account_menu just handed us, not
        // the anonymous bootstrap id still sitting in the session: pairing that id with login
        // cookies is what gets accounts_list rejected, and a rejected list reads below as "this
        // Google account has one channel". Every success path persists this same value; hand the
        // old one back until then so a rolled-back sign-in isn't left holding it.
        let previous_visitor_data = self.it.visitor_data();
        if active_visitor_data.is_some() {
            self.it.set_visitor_data(active_visitor_data.clone());
        }
        // Losing the list endpoint must not regress ordinary one-channel sign-in. It only removes
        // the optional switcher for this login attempt; account_menu still supplies a valid active
        // identity exactly as older releases used it.
        let identities = match self.it.account_identities(client).await {
            Ok(identities) => identities,
            Err(error) => {
                tracing::warn!(%error, "could not discover alternate YouTube identities");
                Vec::new()
            }
        };
        self.it.set_visitor_data(previous_visitor_data);

        let chosen = persisted_id
            .as_deref()
            .and_then(|id| identities.iter().find(|identity| identity.data_sync_id == id));
        if let Some(identity) = chosen {
            if identities.len() == 1 {
                let selected = SelectedIdentity::from_identity(identity, active, false);
                self.persist_selected_identity(selected, active_visitor_data.as_deref())
                    .inspect_err(|_| {
                        self.restore_auth_transport(
                            previous_cookie.clone(),
                            previous_data_sync_id.clone(),
                        );
                        self.restore_session_cookie_setting(previous_cookie.as_deref());
                    })?;
                return Ok(SignInOutcome::Complete);
            }
            self.activate_identity(identity, identities.len() > 1, client).await.inspect_err(
                |_| {
                    self.restore_auth_transport(
                        previous_cookie.clone(),
                        previous_data_sync_id.clone(),
                    );
                    self.restore_session_cookie_setting(previous_cookie.as_deref());
                },
            )?;
            return Ok(SignInOutcome::Complete);
        }

        // A missing/unreadable list must not replace a previously selected channel with Google's
        // current default. Revalidate the stored server-issued id directly; account_menu either
        // confirms it and refreshes metadata, or the login fails closed.
        //
        // Only when the list is empty. A list that came back and does *not* contain the persisted
        // id means these cookies belong to a different Google account, so the stored id is theirs
        // to drop: forcing it here would delegate account A's channel onto account B's cookie and
        // fail every sign-in until the user signs out first.
        if identities.is_empty() {
            if let Some(data_sync_id) = persisted_id.as_deref() {
                let identity = persisted_identity
                    .as_ref()
                    .and_then(SelectedIdentity::as_account_identity)
                    .unwrap_or_else(|| AccountIdentity {
                        name: String::new(),
                        handle: None,
                        email: None,
                        thumbnail: None,
                        channel_id: None,
                        data_sync_id: data_sync_id.to_owned(),
                        is_selected: false,
                    });
                self.activate_identity(
                    &identity,
                    persisted_identity.as_ref().is_some_and(|saved| saved.has_multiple_identities),
                    client,
                )
                .await
                .inspect_err(|_| {
                    self.restore_auth_transport(
                        previous_cookie.clone(),
                        previous_data_sync_id.clone(),
                    );
                    self.restore_session_cookie_setting(previous_cookie.as_deref());
                })?;
                return Ok(SignInOutcome::Complete);
            }
        }

        if identities.len() == 1 {
            // One channel stays seamless. Reuse the already-fetched active header instead of
            // introducing another network request or a new delegated id into the historical
            // single-account path.
            let selected = SelectedIdentity::from_account_info(active, false)
                .ok_or("That session didn't return an account identity")?;
            self.persist_selected_identity(selected, active_visitor_data.as_deref()).inspect_err(
                |_| {
                    self.restore_auth_transport(
                        previous_cookie.clone(),
                        previous_data_sync_id.clone(),
                    );
                    self.restore_session_cookie_setting(previous_cookie.as_deref());
                },
            )?;
            return Ok(SignInOutcome::Complete);
        }

        if identities.len() > 1 {
            // No persisted choice for these cookies: atomically keep the authenticated cookie,
            // mark the login unfinished, and remove stale projections. A restart will reopen the
            // required picker instead of silently acting as YouTube's default channel.
            self.it.set_data_sync_id(None);
            self.db.set_pending_auth_selection(&cookie).map_err(|error| {
                self.restore_auth_transport(previous_cookie.clone(), previous_data_sync_id.clone());
                self.restore_session_cookie_setting(previous_cookie.as_deref());
                error.to_string()
            })?;
            self.persist_visitor_data(active_visitor_data.as_deref());
            let _ = self.app.emit("account-selection-required", ());
            return Ok(SignInOutcome::SelectionRequired);
        }

        // No selectable token was exposed (the normal historical single-channel response). Keep
        // the seamless path and persist the active header + response-context dataSyncId together.
        let selected = SelectedIdentity::from_account_info(active, false)
            .ok_or("That session didn't return an account identity")?;
        self.persist_selected_identity(selected, active_visitor_data.as_deref()).inspect_err(
            |_| {
                self.restore_auth_transport(previous_cookie.clone(), previous_data_sync_id.clone());
                self.restore_session_cookie_setting(previous_cookie.as_deref());
            },
        )?;
        Ok(SignInOutcome::Complete)
    }

    /// Fresh switcher rows for the account picker. Raw delegated ids never cross the Tauri
    /// boundary; the UI receives a process-local opaque selector and display metadata only.
    pub async fn account_identities(&self) -> Result<Vec<serde_json::Value>, String> {
        if !self.it.is_logged_in() {
            return Err("Sign in before switching channels.".into());
        }
        let client =
            self.clients.get(innertube::METADATA_CLIENT).ok_or("metadata client missing")?;
        let identities = self.it.account_identities(client).await.map_err(|e| e.to_string())?;
        let selected_id = self.it.data_sync_id();
        // Nothing is selected yet during a forced first pick, so YouTube's own default marker
        // would be a lie: the user hasn't chosen it, and picking it is still a required step.
        let pending = auth_selection_pending(&self.db);
        Ok(identities
            .iter()
            .map(|identity| {
                let selected = !pending
                    && selected_id
                        .as_deref()
                        .map(|id| id == identity.data_sync_id)
                        .unwrap_or(identity.is_selected);
                identity_snapshot(identity, selected)
            })
            .collect())
    }

    /// Switch without a Google re-login. The selector is resolved against a fresh server list and
    /// account_menu must succeed under a one-off context for that exact identity before the shared
    /// transport, persistence, or UI is updated.
    pub async fn switch_account(&self, selection_key: &str) -> Result<serde_json::Value, String> {
        let _turn = self.auth.lock().await;
        if !self.it.is_logged_in() {
            return Err("Sign in before switching channels.".into());
        }
        let client =
            self.clients.get(innertube::METADATA_CLIENT).ok_or("metadata client missing")?;
        let identities = self.it.account_identities(client).await.map_err(|e| e.to_string())?;
        let identity = identities
            .iter()
            .find(|identity| identity_selection_key(&identity.data_sync_id) == selection_key)
            .ok_or(
                "That YouTube channel is no longer available. Refresh the list and try again.",
            )?;
        self.activate_identity(identity, identities.len() > 1, client).await
    }

    async fn activate_identity(
        &self,
        identity: &AccountIdentity,
        has_multiple_identities: bool,
        client: &innertube::YouTubeClient,
    ) -> Result<serde_json::Value, String> {
        let refreshed =
            match self.it.account_menu_for_identity(client, &identity.data_sync_id).await {
                Ok(info) if info.name.is_some() => info,
                Ok(_) => {
                    return Err(
                        "YouTube did not confirm that channel. Try signing in again.".into()
                    );
                }
                Err(error) => {
                    return Err(format!("Couldn't switch YouTube channel: {error}"));
                }
            };
        let visitor_data = refreshed.visitor_data.clone();
        let selected =
            SelectedIdentity::from_identity(identity, refreshed, has_multiple_identities);
        self.persist_selected_identity(selected, visitor_data.as_deref())
    }

    fn persist_selected_identity(
        &self,
        selected: SelectedIdentity,
        visitor_data: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let account = selected.account_json(true);
        let selected_json = serde_json::to_string(&selected).map_err(|e| e.to_string())?;
        let account_json = account.to_string();
        let session_cookie = self
            .it
            .cookie()
            .ok_or("The YouTube session expired before the channel could be saved.")?;
        self.db
            .set_auth_identity(
                &session_cookie,
                &selected_json,
                selected.data_sync_id.as_deref(),
                &account_json,
            )
            .map_err(|e| format!("Couldn't save the selected YouTube channel: {e}"))?;
        self.persist_visitor_data(visitor_data);
        self.forget_playlist_index();
        self.it.set_data_sync_id(selected.data_sync_id.clone());
        let _ = self.app.emit("auth-changed", &account);
        Ok(account)
    }

    /// The playlist membership index belongs to one account, so a sign-out or a channel switch
    /// empties it. Its timestamp goes too, or the next sync would think it was still fresh.
    fn forget_playlist_index(&self) {
        self.db.clear_playlist_index();
        self.db.delete_setting("playlist_index_synced_at");
    }

    fn restore_auth_transport(&self, cookie: Option<String>, data_sync_id: Option<String>) {
        self.it.set_cookie(cookie);
        self.it.set_data_sync_id(data_sync_id);
    }

    fn restore_session_cookie_setting(&self, cookie: Option<&str>) {
        if let Some(cookie) = cookie {
            self.db.set_setting("session_cookie", cookie);
        } else {
            self.db.delete_setting("session_cookie");
        }
    }

    fn persist_visitor_data(&self, visitor_data: Option<&str>) {
        if let Some(visitor_data) = visitor_data {
            self.it.set_visitor_data(Some(visitor_data.to_owned()));
            self.db.set_setting("visitor_data", visitor_data);
        }
    }

    pub async fn sign_out(&self) {
        let _turn = self.auth.lock().await;
        self.it.set_cookie(None);
        self.it.set_data_sync_id(None);
        self.db.delete_setting("session_cookie");
        self.forget_playlist_index();
        let _ = self.db.clear_auth_identity();
        let _ = self.app.emit("auth-changed", serde_json::json!({ "signedIn": false }));
    }

    /// Current account for the UI. New installs derive it from the canonical selected identity;
    /// legacy `account_json` remains a read fallback so existing databases migrate without a wipe.
    pub fn account_snapshot(&self) -> serde_json::Value {
        if auth_selection_pending(&self.db) && self.it.is_logged_in() {
            return serde_json::json!({
                "signedIn": true,
                "selectionRequired": true,
                "canSwitch": true,
            });
        }
        if let Some(selected) = selected_identity_from_db(&self.db) {
            return selected.account_json(self.it.is_logged_in());
        }
        let mut v = self
            .db
            .get_setting("account_json")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        v["signedIn"] = serde_json::json!(self.it.is_logged_in());
        v
    }

    /// `is_upload` comes off the queue row, never off `/player`: the response that has to be
    /// handled here (LOGIN_REQUIRED) carries no `videoDetails` to read it from. Issue #71.
    async fn resolve(&self, video_id: &str, is_upload: bool) -> Result<PlaybackData, ResolveError> {
        // A local file is its own "stream": no network, no cache, no extraction (local.rs).
        if let Some(path) = crate::local::song_path(video_id) {
            return crate::local::playback_data(video_id, path).map_err(|_| {
                // Gone since the last scan. Forget it here rather than at the next scan, and say
                // so — the UI drops the row (and any Shortcuts tile) on the spot instead of
                // leaving something that can only fail again.
                let removed = crate::local::forget_missing(&self.db, path);
                let _ = self.app.emit("local-changed", serde_json::json!({ "removed": removed }));
                ResolveError::LocalMissing(path.to_owned())
            });
        }
        // Latency cache first (context/11) — honor expiry, never a source of truth.
        // 60s safety margin: a URL that expires mid-load/mid-buffer fails as Raw(-13).
        let now = now_secs();
        if let Some(c) = self.db.get_stream(video_id, now + 60) {
            tracing::debug!(video_id, "stream url cache hit");
            // Cached URL carries no fresh metadata; the UI already has it from the queue item.
            return Ok(PlaybackData {
                video_id: video_id.to_owned(),
                stream_url: c.url,
                itag: c.itag,
                headers: Default::default(),
                expires_in_seconds: c.expires_at - now,
                loudness_db: c.loudness_db,
                // Cached alongside the URL: a hit skips `/player`, so without it a replay never
                // registered in watch history — and neither did a first listen whose stream the
                // gapless lookahead had already cached (issue #83). `None` on rows written before
                // the column, and on whatever YouTube declines a stale `ei` for; the ping is
                // best-effort either way.
                playback_ping: c
                    .ping_url
                    .zip(c.ping_client)
                    .map(|(url, client)| PlaybackPing { url, client }),
                title: None,
                artists: None,
                duration: None,
                thumbnail: None,
                // Cached alongside the URL: a hit skips `/player`, so without it a replay inside
                // the cache window (hours, and every track you just listened to) would fall back
                // to the queue row's flag, which is exactly the thing that can't be trusted.
                is_video: c.is_video,
                stream_client: "cache".to_owned(),
            });
        }
        let data = self
            .orchestrator
            .resolve(video_id, is_upload, self.quality(), &self.disabled_clients())
            .await?;
        // Never cache rustypipe URLs: googlevideo serves them only for bounded-Range requests,
        // which mpv doesn't send → LOADING_FAILED(-13). Caching one poisons the videoId for ~6h.
        //
        // Nor upload URLs: they expire in minutes (32 seconds without the MLPT context), and the
        // read above only accepts a row with 60s of life left, so the write is dead on arrival.
        if data.stream_client != "rustypipe" && !is_upload {
            self.db.put_stream(
                video_id,
                &crate::db::CachedStream {
                    url: data.stream_url.clone(),
                    itag: data.itag,
                    expires_at: now + data.expires_in_seconds.max(0),
                    loudness_db: data.loudness_db,
                    is_video: data.is_video,
                    ping_url: data.playback_ping.as_ref().map(|p| p.url.clone()),
                    ping_client: data.playback_ping.as_ref().map(|p| p.client.clone()),
                },
                now,
            );
        }
        Ok(data)
    }

    /// Start a fresh queue from one track (a search-result click), then hydrate the radio via
    /// `next` and prime the gapless lookahead.
    pub async fn play_song(self: &std::sync::Arc<Self>, seed: SongItem) {
        if self.lt.is_guest().await {
            // Guests follow the host; clicking a song adds it to the shared queue instead
            // (Spotify-Jam-style). The host client auto-approves and stamps who added it.
            self.lt.suggest(song_to_track(&seed)).await;
            return;
        }
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let video_id = seed.video_id.clone();
        let sticky = self.sticky_shuffle();

        {
            let mut q = self.queue.lock().await;
            // Unplayed manual adds survive a context switch (Spotify semantics): they follow the
            // new track, ahead of its radio (hydration appends behind them).
            let mut carried = upcoming_queued(&q.items, q.current);
            // A local file has no radio behind it (see below), so don't promise one in the header.
            q.source_name = (!crate::local::is_local_song(&seed.video_id))
                .then(|| format!("{} Radio", seed.title));
            q.items = vec![seed];
            q.items.append(&mut carried);
            q.current = 0;
            q.played_from = 0; // new queue, nothing played in it yet
            q.lookahead_loaded = None;
            q.radio_seed = None; // single-song queue → autoplay re-seeds from the last track
            q.radio = false;
            // Shuffle carries into the new queue only when it's sticky (re-snapshotted after
            // radio hydration); otherwise a new context starts unshuffled.
            q.shuffle_orig = (sticky && q.shuffle_orig.is_some()).then(|| q.items.clone());
        }

        if !self.start_current(gen).await {
            return;
        }

        // A local file isn't a videoId YouTube has ever heard of: asking for its radio is a
        // guaranteed-useless request, and offline (where local music earns its keep) it's a
        // guaranteed-failing one.
        if crate::local::is_local_song(&video_id) {
            self.prime_lookahead(gen).await;
            return;
        }

        // Hydrate up-next radio (context/08) — non-fatal if it fails. Seed the radio playlist
        // directly (`RDAMVM<videoId>`): a bare next(videoId) returns only the seed song + an
        // automixPreviewVideoRenderer, so the queue would never grow past one track.
        let radio_id = format!("RDAMVM{video_id}");
        match self
            .it
            .next(
                self.clients.get(innertube::METADATA_CLIENT).unwrap(),
                Some(&video_id),
                Some(&radio_id),
            )
            .await
        {
            Ok(next) => {
                let mut q = self.queue.lock().await;
                if self.generation.load(Ordering::SeqCst) != gen {
                    return; // superseded
                }
                for item in next.items {
                    if item.video_id != video_id {
                        q.items.push(item);
                    }
                }
                // Shuffle on → the radio hydration is part of the queue: snapshot it as the
                // "original" order, then shuffle the upcoming tracks. (Runs before the lookahead
                // is primed, so nothing stale is in mpv.)
                if q.shuffle_orig.is_some() {
                    q.shuffle_orig = Some(q.items.clone());
                    let cur = q.current;
                    shuffle_upcoming(&mut q.items, cur);
                }
                drop(q);
                self.emit_queue().await;
            }
            Err(e) => tracing::warn!(error = %e, "next() radio hydration failed"),
        }

        self.prime_lookahead(gen).await;
    }

    /// Play a finite list of tracks (a playlist/album). `start` is the clicked track; `None`
    /// means "just play the playlist" (the header Play button) — first track, or any track when
    /// shuffle is on. Unlike `play_song` this seeds NO radio — the given items *are* the queue
    /// (context/08: playlist playback). `source_id` (the page's playlist/album playlist id) makes
    /// autoplay continue with that context's radio when the queue runs out. `source_name` is the
    /// page title for the queue panel's "Next from" header; `shuffle` (the page Shuffle buttons)
    /// turns shuffle ON for this queue — the backend owns the randomization, so un-shuffle can
    /// restore the true order and every re-shuffle is fresh.
    ///
    /// `continuation` is the playlist page's next-page token, if it has one: the queue is seeded
    /// from the tracks the page had loaded and the rest is walked in the background
    /// ([`Self::fill_playlist`]) so a long playlist starts playing now instead of after ~50
    /// chained round trips.
    #[allow(clippy::too_many_arguments)]
    pub async fn play_tracks(
        self: &std::sync::Arc<Self>,
        items: Vec<SongItem>,
        start: Option<usize>,
        source_id: Option<String>,
        source_name: Option<String>,
        shuffle: bool,
        continuation: Option<String>,
    ) {
        if items.is_empty() {
            return;
        }
        if self.lt.is_guest().await {
            self.emit_guest_hint();
            return;
        }
        // A mix has no "rest of the playlist" worth walking (see `is_mix`) — drop the token.
        let continuation = continuation.filter(|_| !is_mix(source_id.as_deref()));
        let sticky = self.sticky_shuffle();
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut q = self.queue.lock().await;
            // Explicitly requested by a page Shuffle button, or already on and set to stick
            // across queues (issue #117: off by default, so an album opened while shuffle is on
            // plays in order).
            let keep_shuffled = shuffle || (sticky && q.shuffle_orig.is_some());
            let start = match start {
                Some(i) => i.min(items.len() - 1),
                None if keep_shuffled => {
                    rand::Rng::gen_range(&mut rand::thread_rng(), 0..items.len())
                }
                None => 0,
            };
            // Unplayed manual adds survive a context switch (Spotify semantics) — spliced back in
            // right after the new current track, ahead of the new context.
            let carried = upcoming_queued(&q.items, q.current);
            q.items = items;
            q.current = start;
            q.lookahead_loaded = None;
            q.radio_seed = radio_seed_for(source_id);
            q.source_name = source_name;
            q.radio = false; // a chosen playlist/album; `start_radio` sets it back on for its own
            if keep_shuffled {
                // Snapshot the real playlist order (for un-shuffle), then play the clicked track
                // first with everything else shuffled behind it. Carried adds are spliced in
                // after — un-shuffle drops them, same as adds made while shuffled (snapshot
                // semantics, see toggle_shuffle).
                q.shuffle_orig = Some(q.items.clone());
                let start = q.current;
                q.current = shuffle_new_queue(&mut q.items, start);
            } else {
                q.shuffle_orig = None; // non-sticky shuffle ends with the queue it was turned on for
            }
            // Whatever the queue starts on is where the played run starts: the tracks in front of
            // a playlist opened at track 7 were never heard.
            q.played_from = q.current;
            let at = q.current + 1;
            for (k, item) in carried.into_iter().enumerate() {
                q.items.insert(at + k, item);
            }
        }
        // start_current emits now-playing + queue + persists; prime the gapless lookahead after.
        if self.start_current(gen).await {
            self.prime_lookahead(gen).await;
        }
        if let Some(token) = continuation {
            let me = self.clone();
            tokio::spawn(async move { me.fill_playlist(gen, token, Fill::Playing).await });
        }
    }

    /// Start a radio (context/08): an endless YouTube-generated queue seeded on a song, artist,
    /// album or playlist. `kind` is one of `song` / `artist` / `album` / `playlist` and `id` the
    /// matching videoId or browseId; `name` is what the queue header calls it.
    ///
    /// A radio *is* a playlist id — the whole feature is a prefix convention plus one `/next`
    /// call. Songs and playlists build theirs client-side (`RDAMVM…` / `RDAMPL…`); artists can't,
    /// so theirs comes off the artist page (`radio_playlist_id`). Once started it continues
    /// through the same autoplay path as any other queue ([`Self::extend_queue_radio`]), which is
    /// why this only has to install a first page and a seed.
    pub async fn start_radio(
        self: &std::sync::Arc<Self>,
        kind: &str,
        id: &str,
        name: Option<String>,
    ) -> Result<(), String> {
        if self.lt.is_guest().await {
            self.emit_guest_hint();
            return Ok(());
        }
        // Guarded here rather than in each menu: everything without a YouTube item behind it
        // (files on disk, the locally-built On Repeat) would otherwise reach `/next` as an id
        // YouTube has never heard of.
        if crate::local::is_local_song(id)
            || id.starts_with(crate::local::ALBUM_PREFIX)
            || id.starts_with(crate::local::ARTIST_PREFIX)
            || id == ON_REPEAT_ID
        {
            return Err("This has no radio behind it.".into());
        }
        let client = self.clients.get(innertube::METADATA_CLIENT).ok_or("no metadata client")?;
        // Resolve the seed to (videoId?, radio playlist id). Album and artist need a page fetch:
        // an album's radio keys off its audio playlist, not its `MPRE…` browseId, and an artist
        // radio id is server-supplied.
        let (video_id, playlist_id) = match kind {
            "song" => (Some(id.to_owned()), format!("RDAMVM{id}")),
            "playlist" => (None, radio_seed_for(Some(id.to_owned())).unwrap()),
            "album" => {
                let page = self.it.album(client, id).await.map_err(|e| e.to_string())?;
                let pl = page.playlist_id.ok_or("This album has no radio.")?;
                (None, radio_seed_for(Some(pl)).unwrap())
            }
            "artist" => {
                let page = self.it.artist(client, id).await.map_err(|e| e.to_string())?;
                match page.radio_playlist_id {
                    Some(pl) => (None, pl),
                    // No radio button on the header: fall back to a radio on this artist's most
                    // played track, which is roughly what that button seeds anyway.
                    None => {
                        let top = page.top_songs.first().ok_or("This artist has no radio.")?;
                        (Some(top.video_id.clone()), format!("RDAMVM{}", top.video_id))
                    }
                }
            }
            other => return Err(format!("unknown radio kind: {other}")),
        };

        let (items, seed) = self.fetch_radio(video_id.as_deref(), &playlist_id).await?;
        let title = name.map(|n| format!("{n} Radio"));

        // Radio on the song that's already playing: splice instead of replacing, so the track
        // keeps playing without a re-buffer (Metrolist's `startRadioSeamlessly`).
        let playing_seed = {
            let q = self.queue.lock().await;
            video_id.is_some() && q.items.get(q.current).map(|i| &i.video_id) == video_id.as_ref()
        };
        if playing_seed && !self.player.is_idle() {
            self.splice_radio(items, seed, title).await;
            return Ok(());
        }
        // The seed song comes back inside the first page (usually first, but the panel decides) —
        // start there so a radio on song X actually opens on X.
        let start = video_id
            .as_ref()
            .and_then(|v| items.iter().position(|i| &i.video_id == v))
            .unwrap_or(0);
        self.play_tracks(items, Some(start), Some(seed), title, false, None).await;
        self.queue.lock().await.radio = true;
        Ok(())
    }

    /// Fetch a radio's first page, escalating when YouTube hands back a dead one. Returns the
    /// tracks plus the playlist id that actually produced them (autoplay's seed, which is not
    /// necessarily the one asked for).
    ///
    /// A `RDAMVM…` radio for an obscure or region-locked track routinely answers with the seed
    /// song and nothing else, and "start radio" then looks like it did nothing. So: ask the song
    /// what mix it belongs to (`automixPreviewVideoRenderer`) and take that instead.
    ///
    /// ponytail: two rungs, not Metrolist's three — the third scrapes the Related tab, which needs
    /// a whole endpoint + an ATV-only filter to salvage the cases these two already miss. Add it
    /// if songs turn up that reach here and still come back empty.
    async fn fetch_radio(
        &self,
        video_id: Option<&str>,
        playlist_id: &str,
    ) -> Result<(Vec<SongItem>, String), String> {
        const NO_RADIO: &str = "YouTube has no radio for this.";
        let client = self.clients.get(innertube::METADATA_CLIENT).ok_or("no metadata client")?;
        let first =
            self.it.next(client, video_id, Some(playlist_id)).await.map_err(|e| e.to_string())?;
        if first.items.len() > 1 {
            return Ok((first.items, playlist_id.to_owned()));
        }
        let Some(video_id) = video_id else { return Err(NO_RADIO.into()) };
        let bare = self.it.next(client, Some(video_id), None).await.map_err(|e| e.to_string())?;
        if let Some(mix) = bare.automix_playlist_id {
            let page = self
                .it
                .next(client, Some(video_id), Some(&mix))
                .await
                .map_err(|e| e.to_string())?;
            if page.items.len() > 1 {
                return Ok((page.items, mix));
            }
        }
        if bare.items.len() > 1 {
            return Ok((bare.items, format!("RDAMVM{video_id}")));
        }
        Err(NO_RADIO.into())
    }

    /// Install a radio behind the track that's already playing: history and the current song stay,
    /// everything after them becomes the radio. Manual "add to queue" items are carried rather
    /// than destroyed (same rule as a context switch in `play_tracks`) — Metrolist silently drops
    /// them, which on a desktop app is just losing the user's work.
    async fn splice_radio(
        self: &std::sync::Arc<Self>,
        items: Vec<SongItem>,
        seed: String,
        title: Option<String>,
    ) {
        {
            let mut q = self.queue.lock().await;
            if !self.sticky_shuffle() {
                q.shuffle_orig = None; // a radio is a new context, so a non-sticky shuffle ends here
            }
            splice_radio_into(&mut q, items, seed, title);
            // Whatever mpv had primed as the gapless next belongs to the old queue.
            if q.lookahead_loaded.take().is_some() {
                let _ = self.player.clear_playlist();
            }
        }
        self.emit_queue().await;
        self.persist_queue().await;
        self.prime_lookahead(self.generation.load(Ordering::SeqCst)).await;
        self.lt_broadcast_queue().await;
    }

    /// Walk the rest of a playlist in the background and append it to the playing queue, page by
    /// page. The alternative (what the UI used to do) is loading every page *before* starting
    /// playback: continuation tokens are chained, so that's ~50 sequential round trips on a
    /// 5000-track playlist, all of them before the first note.
    ///
    /// Shuffle stays a real shuffle. Each arriving page is mixed through the *unplayed* tail
    /// ([`append_page`]), so the only track not drawn from the full playlist is the one already
    /// playing — and the walk is long finished before it ends.
    ///
    /// Guarded by `gen`: if the user starts something else mid-walk, the pages are dropped rather
    /// than appended to a queue they don't belong to. [`Fill`] says which queue the pages join.
    async fn fill_playlist(self: &std::sync::Arc<Self>, gen: u64, mut token: String, fill: Fill) {
        // ponytail: ~5k tracks at 100/page. A bound so a playlist that keeps handing out tokens
        // can't walk forever; raise it if a real playlist ever hits the cap.
        const MAX_PAGES: usize = 50;
        let Some(client) = self.clients.get(innertube::METADATA_CLIENT) else { return };
        let mut pages = 0;
        // The first page goes out at once so the panel visibly starts filling; after that the
        // walk emits at most once a second. Every page is a *full* payload (the rows changed),
        // and a 50-page walk of a 5,000-track playlist sends ~67 MB of JSON through
        // `webview.eval` if each one is announced. The final state is emitted after the loop.
        let mut last_emit: Option<std::time::Instant> = None;
        for _ in 0..MAX_PAGES {
            let page = match self.it.playlist_continuation(client, &token).await {
                Ok(page) => page,
                // A page that fails ends the walk — better a short queue than a stalled one.
                Err(e) => {
                    tracing::warn!(error = %e, "playlist fill stopped early");
                    break;
                }
            };
            if self.generation.load(Ordering::SeqCst) != gen {
                return; // another queue owns the state now — don't touch it, don't persist
            }
            if page.items.is_empty() {
                break; // an empty page is the end, token or not
            }
            let mut items = page.items;
            // The rest of an added playlist belongs to the same block: same markers, same heading,
            // and its order left alone (shuffle is about the playlist that's playing).
            if let Fill::Queued(from) = &fill {
                for item in &mut items {
                    item.queued_end = true;
                    item.queued_from = from.clone();
                }
            }
            {
                let mut q = self.queue.lock().await;
                append_page(&mut q, items, matches!(fill, Fill::Playing));
                // An append can retarget a primed repeat-all wrap (index 0 → the new tail); drop
                // the lookahead when it stops pointing at what plays next, same check as
                // `insert_queued_song`. `append_page` leaves a still-valid slot alone, so the
                // common case re-primes to a no-op instead of re-resolving on every page.
                let expected = next_index(q.items.len(), q.current, q.repeat);
                if q.lookahead_loaded.is_some() && q.lookahead_loaded != expected {
                    q.lookahead_loaded = None;
                    let _ = self.player.clear_playlist();
                }
            }
            pages += 1;
            if last_emit.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1)) {
                last_emit = Some(std::time::Instant::now());
                self.emit_queue().await;
            }
            self.prime_lookahead(gen).await;
            match page.continuation {
                Some(next) => token = next,
                None => break,
            }
        }
        // Persist/broadcast once at the end, not per page: the queue JSON grows with every page and
        // nothing else reads it mid-walk.
        if pages > 0 {
            tracing::info!(pages, "playlist fill appended pages");
            self.emit_queue().await; // whatever the throttle above held back
            self.persist_queue().await;
            self.lt_broadcast_queue().await;
        }
    }

    /// Play/pause toggle that also starts a *restored* (or exhausted) queue: if mpv has nothing
    /// loaded but the queue is non-empty, load the current track (applying any resume position);
    /// otherwise just toggle mpv. Keeps the UI's single play/pause button working after a restart.
    pub async fn resume_or_toggle(self: &std::sync::Arc<Self>) {
        if self.lt.is_guest().await {
            return; // guest playback is host-driven
        }
        if self.player.is_idle() {
            let (idx, has_items) = {
                let q = self.queue.lock().await;
                (q.current, !q.items.is_empty())
            };
            if has_items {
                self.play_index(idx).await;
                return;
            }
        }
        let _ = self.player.toggle();
    }

    /// Jump to a specific queue index.
    pub async fn play_index(self: &std::sync::Arc<Self>, index: usize) {
        if self.lt.is_guest().await {
            self.emit_guest_hint(); // guest playback is host-driven
            return;
        }
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut q = self.queue.lock().await;
            if index >= q.items.len() {
                return;
            }
            q.seek_to(index);
            q.lookahead_loaded = None;
        }
        if self.start_current(gen).await {
            self.prime_lookahead(gen).await;
        }
    }

    /// Advance the queue after a track ended (EOF) or died (load error). Don't assume mpv
    /// gaplessly transitioned — ask it: if a lookahead was primed mpv is already playing the
    /// next entry (just sync pointer + UI); if mpv went idle (lookahead absent/failed, or the
    /// track errored on a single-entry playlist) load the next track explicitly, otherwise
    /// playback silently stalls while the UI shows a phantom "now playing".
    pub async fn on_track_ended(self: &std::sync::Arc<Self>) {
        if self.lt.is_guest().await {
            return; // the host drives track changes for guests; don't auto-advance locally
        }
        let (has_next, primed) = {
            let mut q = self.queue.lock().await;
            match next_index(q.items.len(), q.current, q.repeat) {
                Some(next) => {
                    let primed = q.lookahead_loaded == Some(next);
                    q.seek_to(next); // repeat-all wraps to 0, which starts the played run over
                    (true, primed)
                }
                None => (false, false),
            }
        };
        if !has_next {
            // Try autoplay before declaring the queue dead (the early trigger usually already
            // extended it; this is the fallback when that didn't land in time). Off the pump:
            // spawn, and let the task either continue playback or do the pause bookkeeping —
            // pausing here and un-pausing a second later would flicker every consumer (UI,
            // MPRIS, Discord).
            let me = self.clone();
            tauri::async_runtime::spawn(async move {
                let gen = me.generation.fetch_add(1, Ordering::SeqCst) + 1;
                if me.extend_queue_radio(gen).await > 0 {
                    {
                        let mut q = me.queue.lock().await;
                        q.current += 1; // the first appended track
                        q.lookahead_loaded = None; // start_current's loadfile replaces mpv's playlist
                    }
                    if me.start_current(gen).await {
                        me.prime_lookahead(gen).await;
                    }
                } else {
                    tracing::info!("queue exhausted");
                    let _ = me.app.emit("playback-state", "paused");
                    // mpv goes idle without flipping its pause flag, so no Paused event will fire
                    // — tell the OS widget + Discord ourselves or they show "playing" forever
                    // past the last song.
                    me.media_set_playing(false);
                }
            });
            return;
        }
        if !primed || self.player.is_idle() {
            // No gapless handoff happened. `is_idle` alone is not enough: right at EOF mpv can
            // still report not-idle before it settles, and with nothing primed that would send us
            // into the gapless branch below — a phantom "now playing" over a silent player. If we
            // never primed this index, mpv cannot have advanced into it; load explicitly. Bump the
            // generation so any in-flight lookahead resolve discards itself (double-enqueue).
            let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            tracing::info!("no primed lookahead at track end — loading next explicitly");
            // start_current's loadfile *replaces* mpv's playlist, so whatever was recorded here is
            // gone either way — clear it like every other load path (play_index, on_track_failed,
            // play_tracks) does. Left stale, the re-prime below can no-op as "already primed"
            // against an entry mpv no longer holds, and the next track end finds nothing to
            // advance into.
            self.queue.lock().await.lookahead_loaded = None;
            if self.start_current(gen).await {
                self.prime_lookahead(gen).await;
            }
            return;
        }
        // mpv already advanced into the primed lookahead. Sync pointer + UI, prime the next.
        let gen = self.generation.load(Ordering::SeqCst);
        {
            let mut q = self.queue.lock().await;
            // `af` is global in mpv and the appended entry couldn't carry its own, so the new track
            // is playing at the *previous* one's gain until this lands.
            if let Some(gain) = q.lookahead_gain.take() {
                if let Err(e) = self.player.set_gain(gain) {
                    tracing::warn!(error = %e, "applying lookahead loudness gain failed");
                }
            }
            // The primed entry is what's playing now — nothing is primed beyond it. (Also what
            // lets a single-item repeat-all queue re-prime itself instead of "already primed".)
            q.lookahead_loaded = None;
            q.current_client = q.lookahead_client.take();
            // New track is now playing → fresh history state (mirrors start_current).
            q.playback_ping = q.lookahead_playback_ping.take();
            q.cpn = innertube::generate_cpn();
            self.history_pinged.store(false, Ordering::Relaxed);
            q.duration = 0.0;
        }
        if let Some(item) = self.current_item().await {
            self.emit_now_playing(&item, "gapless");
            // Same as `start_current`: an autoplay-appended track carries no rating of its own.
            self.refresh_rating(&item.video_id, gen);
        }
        self.emit_queue().await;
        self.persist_queue().await; // index advanced without an explicit load → persist it
                                    // Listen Together host: announce the gapless advance to the room.
        self.lt_broadcast_current_track(0, true).await;
        tracing::info!("advanced to next track (gapless)");
        // Prime off the pump, not on it. `prime_lookahead` resolves the next stream over the
        // network, and this fn is awaited by the mpv event pump — blocking here stops mpv's events
        // being drained for the length of a round-trip, which delays the new track's `duration`
        // (its progress bar) and, worse, the *next* track-end. The generation guard inside already
        // makes a superseded resolve discard itself, so it's safe to detach.
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            me.prime_lookahead(gen).await;
            me.extend_queue_radio(gen).await; // autoplay early trigger (no-op unless tail is near)
        });
    }

    /// Which client's URL the playing track was resolved from ("WEB_REMIX", "cache", ...).
    /// Names the culprit in a playback-error toast, which on Windows is the only diagnostic a
    /// reporter can actually produce.
    pub async fn current_stream_client(&self) -> Option<String> {
        self.queue.lock().await.current_client.clone()
    }

    /// A track died at the player layer (dead/403 URL). If WEB_REMIX served it, record the failure
    /// so the next resolve for this id bypasses WEB_REMIX (context/06 §2). Then retry the SAME
    /// track once — the re-resolve now runs the direct-URL fallback clients, which is what makes
    /// PoToken-enforced/niche videos playable — before giving up and advancing the queue.
    /// Returns true when the track was retried (so the caller can skip the error toast).
    pub async fn on_track_failed(self: &std::sync::Arc<Self>) -> bool {
        let (video_id, client, already_retried) = {
            let q = self.queue.lock().await;
            let vid = q.items.get(q.current).map(|i| i.video_id.clone());
            let retried = vid.is_some() && vid == q.retried;
            (vid, q.current_client.clone(), retried)
        };
        if let (Some(vid), Some(c)) = (video_id, client) {
            // Whatever served it, the URL is dead — evict so the retry can't replay it from cache.
            self.db.evict_stream(&vid);
            if c == MAIN_CLIENT {
                tracing::warn!(video_id = %vid, "WEB_REMIX stream failed on GET — marking + evicting");
                self.orchestrator.mark_web_remix_failed(&vid).await;
            }
            // Retry once for WEB_REMIX-served and cache-served URLs. A failure from a fallback
            // client, or a second failure of the same id, advances as before.
            if (c == MAIN_CLIENT || c == "cache") && !already_retried {
                {
                    let mut q = self.queue.lock().await;
                    q.retried = Some(vid.clone());
                    q.lookahead_loaded = None; // start_current's loadfile replaces mpv's playlist
                }
                tracing::info!(video_id = %vid, "retrying failed track via fallback clients");
                let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
                if self.start_current(gen).await {
                    self.prime_lookahead(gen).await;
                }
                return true;
            }
        }
        self.on_track_ended().await;
        false
    }

    /// Resolve + load the current track into mpv (replace). Returns false if resolve failed or the
    /// request was superseded.
    async fn start_current(self: &std::sync::Arc<Self>, gen: u64) -> bool {
        // Resolve the current track, auto-skipping any that no client can play (dead / region-locked
        // videos — context/06 "no client could resolve") instead of stalling the queue on them.
        // Bounded: each failure advances current by one, so the loop terminates at the queue tail.
        let (mut item, data) = loop {
            if self.generation.load(Ordering::SeqCst) != gen {
                return false; // user moved on
            }
            let Some(item) = self.current_item().await else { return false };
            let resolved = self.resolve(&item.video_id, item.is_upload).await;
            // A resolve takes seconds; a skip during it bumps the generation. Re-check before
            // acting on the result: an abandoned failure would otherwise move `current` under the
            // track that's already playing and leave a stale error banner (nothing clears it, the
            // new track's now-playing fired before the banner appeared).
            if self.generation.load(Ordering::SeqCst) != gen {
                return false;
            }
            match resolved {
                Ok(d) => break (item, d),
                // A deleted local file is gone for good, so it leaves the queue outright rather
                // than being skipped over — a row that can only ever fail again is noise. Every
                // other failure is transient in principle (network, region, a cipher self-heal),
                // so those keep their place.
                Err(ResolveError::LocalMissing(_)) => {
                    let exhausted = {
                        let mut q = self.queue.lock().await;
                        let cur = q.current;
                        if cur < q.items.len() {
                            q.items.remove(cur);
                        }
                        q.lookahead_loaded = None;
                        // `current` now points at what followed it. When it was the tail, step
                        // back onto the last surviving track: an index past the end leaves the
                        // transport pointing at nothing, and Play does nothing at all.
                        let exhausted = q.current >= q.items.len();
                        if exhausted {
                            let last = q.items.len().saturating_sub(1);
                            q.seek_to(last);
                        }
                        exhausted
                    };
                    self.emit_queue().await;
                    self.persist_queue().await;
                    self.lt_broadcast_queue().await;
                    self.emit_notice(&format!("{} is no longer on your disk", item.title));
                    if exhausted {
                        return false;
                    }
                }
                Err(e) => {
                    let mut q = self.queue.lock().await;
                    // Deliberately ignores repeat-all: wrapping the unplayable-skip would spin
                    // forever on a queue where nothing resolves. Skips stop at the tail.
                    if q.current + 1 >= q.items.len() {
                        drop(q);
                        self.emit_error(&item.video_id, &e.to_string()); // nothing left to skip to
                        return false;
                    }
                    q.current += 1;
                    q.lookahead_loaded = None;
                    drop(q);
                    self.emit_skip(&item.title, skip_reason(&e));
                    self.emit_queue().await;
                }
            }
        };
        if self.generation.load(Ordering::SeqCst) != gen {
            return false; // user moved on
        }
        if let Err(e) =
            self.player.load(&data.stream_url, &data.headers, loudness_gain(data.loudness_db))
        {
            self.emit_error(&item.video_id, &e.to_string());
            return false;
        }
        let _ = self.player.play();
        // Resume a restored position, but only for the exact track it was saved against (any first
        // play consumes it, so jumping elsewhere doesn't inherit it). mpv queues an absolute seek
        // issued right after loadfile and applies it when the file loads.
        // ponytail: if resume-position proves flaky on some mpv build, switch to the loadfile
        // `start=` option instead of a post-load seek.
        let seek = self
            .pending_seek
            .lock()
            .unwrap()
            .take()
            .filter(|(vid, _)| *vid == item.video_id)
            .map(|(_, pos)| pos);
        if let Some(pos) = seek {
            let _ = self.player.seek(pos);
        }
        // Items played from cards/radio can arrive without a duration; the player response knows
        // the exact length of the cut we stream. Backfill before emitting — lyrics matching keys
        // on it (a wrong-cut LRCLIB match plays lyrics seconds off the audio).
        backfill_metadata(
            &mut item,
            data.duration.as_deref(),
            data.artists.as_deref(),
            data.is_video,
        );
        {
            let mut q = self.queue.lock().await;
            q.current_client = Some(data.stream_client.clone());
            // Fresh play → fresh history state (context/01 §registerPlayback).
            q.playback_ping = data.playback_ping.clone();
            q.cpn = innertube::generate_cpn();
            self.history_pinged.store(false, Ordering::Relaxed);
            q.duration = 0.0;
            let cur = q.current;
            if let Some(qi) = q.items.get_mut(cur) {
                // Same repair on the queue's own copy: it's what the queue panel renders, what
                // `persist_queue` saves, and what `record_play` writes into On Repeat.
                if qi.video_id == item.video_id {
                    backfill_metadata(
                        qi,
                        data.duration.as_deref(),
                        data.artists.as_deref(),
                        data.is_video,
                    );
                }
            }
        }
        self.emit_now_playing(&item, &data.stream_client);
        // The rating just emitted is whatever the row was parsed with, which for a search result
        // or a radio track is nothing at all (issue #93). Ask.
        self.refresh_rating(&item.video_id, gen);
        // We just told mpv to play, but its `pause` flag was already `false`, so no property event
        // will announce it (see `Player::is_playing`). Say so ourselves — otherwise MPRIS and
        // Discord never learn the track started. After `emit_now_playing`, so the new track is the
        // current one before anything renders it as playing.
        self.media_set_playing(true);
        self.emit_queue().await;
        self.persist_queue().await;
        // Listen Together host: announce the new track (fresh play → position 0, playing).
        self.lt_broadcast_current_track(0, true).await;
        // Autoplay early trigger: extend the queue while the tail still plays, so the gapless
        // lookahead can prime into the continuation. The near-tail guard inside makes this a
        // no-op for almost every track start. Detached — never on the caller's path.
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.extend_queue_radio(gen).await });
        true
    }

    /// Resolve the next queue item and append it to mpv for a gapless transition. context/14.
    ///
    /// An upcoming track no client can resolve is skipped *here*, while the current track is
    /// still playing — deferring it to the track boundary pauses playback while the whole client
    /// waterfall fails again on the event pump (and used to wedge until a manual skip).
    /// ponytail: at most 3 removals per prime so a network outage can't eat the whole queue.
    async fn prime_lookahead(self: &std::sync::Arc<Self>, gen: u64) {
        for _ in 0..3 {
            let next_idx = {
                let q = self.queue.lock().await;
                // Repeat-one primes nothing: mpv loops the file itself (next_index → None).
                let Some(next) = next_index(q.items.len(), q.current, q.repeat) else { return };
                if q.lookahead_loaded == Some(next) {
                    return; // already primed
                }
                next
            };
            let (next_video, next_title, next_upload) = {
                let q = self.queue.lock().await;
                match q.items.get(next_idx) {
                    Some(item) => (item.video_id.clone(), item.title.clone(), item.is_upload),
                    None => return,
                }
            };
            match self.resolve(&next_video, next_upload).await {
                Ok(d) => {
                    self.enqueue_lookahead(gen, next_idx, &next_video, d).await;
                    return;
                }
                Err(e) => {
                    tracing::warn!(video_id = %next_video, error = %e, "lookahead resolve failed — dropping from queue");
                    if self.generation.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    // The queue can shift under a resolve — only remove the slot if it still
                    // holds the track that failed. (Also refuses next_idx == current, i.e. the
                    // single-item repeat-all case, via remove_from_queue's own guard.)
                    let same = {
                        let q = self.queue.lock().await;
                        q.items.get(next_idx).map(|i| i.video_id == next_video).unwrap_or(false)
                    };
                    if !same {
                        return;
                    }
                    // Box::pin: remove_from_queue can itself re-prime (async recursion).
                    Box::pin(self.remove_from_queue(next_idx)).await;
                    self.emit_skip(&next_title, skip_reason(&e));
                }
            }
        }
    }

    /// Second half of [`Self::prime_lookahead`]: hand the resolved stream to mpv and record it.
    async fn enqueue_lookahead(
        self: &std::sync::Arc<Self>,
        gen: u64,
        next_idx: usize,
        next_video: &str,
        data: PlaybackData,
    ) {
        if self.generation.load(Ordering::SeqCst) != gen {
            return;
        }
        let mut q = self.queue.lock().await;
        // The queue can change under a resolve (a guest add inserts at current+1) — enqueueing
        // then would gaplessly play the wrong song. Verify the slot still holds the same track.
        if q.items.get(next_idx).map(|i| i.video_id != next_video).unwrap_or(true) {
            tracing::debug!(index = next_idx, "queue changed during lookahead resolve — dropped");
            return;
        }
        // Another prime already claimed this slot while we resolved. Two run concurrently at the
        // autoplay trigger point — `start_current` spawns `extend_queue_radio` (which primes after
        // extending) and its caller primes right after, both on the same generation, so the
        // pre-resolve "already primed" check in `prime_lookahead` can't see the other one. Appending
        // again leaves a *duplicate* entry in mpv's playlist, and since the queue advances one index
        // per end-file, that offsets mpv from `current` for the rest of the session: the dup replays
        // while the UI shows the track after it.
        if q.lookahead_loaded == Some(next_idx) {
            tracing::debug!(
                index = next_idx,
                "lookahead already primed by a concurrent resolve — dropped"
            );
            return;
        }
        // Headers are global in mpv; the direct-URL clients need none beyond UA, which the
        // current track already set. Just append the URL.
        if let Err(e) = self.player.enqueue(&data.stream_url) {
            tracing::warn!(error = %e, "enqueue lookahead failed");
            return;
        }
        q.lookahead_loaded = Some(next_idx);
        q.lookahead_client = Some(data.stream_client.clone());
        q.lookahead_gain = Some(loudness_gain(data.loudness_db));
        q.lookahead_playback_ping = data.playback_ping.clone();
        // Same backfill as start_current: a gapless advance emits this item straight from the
        // queue, so the repair has to land before it becomes the current track.
        if let Some(qi) = q.items.get_mut(next_idx) {
            backfill_metadata(qi, data.duration.as_deref(), data.artists.as_deref(), data.is_video);
        }
        tracing::debug!(index = next_idx, "gapless lookahead primed");
    }

    async fn current_item(&self) -> Option<SongItem> {
        let q = self.queue.lock().await;
        q.items.get(q.current).cloned()
    }

    // --- events (context/11 UI contract) ----------------------------------------------------

    /// Everything the `now-playing` event carries. Shared with [`Self::playback_snapshot`] so a
    /// window that asks for the current track can't be told a different shape than one that
    /// listened for it.
    fn now_playing_json(item: &SongItem, stream_client: &str) -> serde_json::Value {
        serde_json::json!({
            "videoId": item.video_id,
            "title": item.title,
            "artists": item.artists,
            "artistId": item.artist_id,
            // Per-artist links, so a collab in the player bar navigates like it does in a row.
            "artistRuns": item.artist_runs,
            "thumbnail": item.thumbnail,
            "duration": item.duration,
            "streamClient": stream_client,
            "rating": item.rating,
            // YouTube's own `musicVideoType` says this track is a video upload, not the generated
            // audio track, which is what the player view's music-video mode gates on (plan 031).
            "isVideo": item.is_video,
        })
    }

    /// What a window that opened mid-playback missed. Events are fire-and-forget, so the mini
    /// player (a second webview, created long after the track started) and the main window on a
    /// cold start both have to ask once instead of guessing.
    pub async fn playback_snapshot(&self) -> serde_json::Value {
        let (duration, item) = {
            let q = self.queue.lock().await;
            (q.duration, q.items.get(q.current).cloned())
        };
        serde_json::json!({
            "now": item.as_ref().map(|i| Self::now_playing_json(i, "current")),
            "paused": !self.is_playing.load(Ordering::Relaxed),
            "position": self.current_position(),
            "duration": duration,
            "volume": saved_volume(&self.db),
        })
    }

    /// Ask YouTube what the current track's rating actually is, and tell every window if it
    /// differs from the snapshot they were just handed. Issue #93.
    ///
    /// A `SongItem`'s `rating` is only ever a snapshot of the response that produced the row, and
    /// most rows state nothing at all: search rows carry no `likeStatus` (0 of 29) and `/next`
    /// panel rows carry none either (0 of 50), both live-checked 2026-08-24, so a liked song
    /// played from search or reached by autoplay renders exactly like an unrated one. A restored
    /// queue's rating is worse than absent: it is however old the database is, so a like made on
    /// the phone since never shows up. The authority is a bare `next(videoId)`'s
    /// `playerOverlays…likeButtonRenderer` (~4 KB gzipped), which is what YouTube Music's own
    /// player bar draws from.
    ///
    /// Detached and doubly guarded: nothing waits on it, a skip during the round trip discards the
    /// answer (`generation`), and so does a rating the user set meanwhile (`rate_epoch`).
    fn refresh_rating(self: &std::sync::Arc<Self>, video_id: &str, gen: u64) {
        // Signed out there is no rating to be wrong about, and a local file is not a videoId
        // YouTube has ever heard of.
        if !self.it.is_logged_in() || crate::local::is_local_song(video_id) {
            return;
        }
        let me = self.clone();
        let video_id = video_id.to_owned();
        tauri::async_runtime::spawn(async move {
            let Some(client) = me.clients.get(innertube::METADATA_CLIENT) else { return };
            let epoch = me.rate_epoch.load(Ordering::SeqCst);
            // No playlist id: the overlay is all this call is for, and the bare form is ~25x
            // smaller than the radio one (4 KB vs 100 KB gzipped).
            let rating = match me.it.next(client, Some(&video_id), None).await {
                Ok(next) => next.rating,
                Err(e) => {
                    tracing::debug!(error = %e, "rating refresh failed; keeping the snapshot");
                    return;
                }
            };
            let Some(rating) = rating else { return };
            if me.generation.load(Ordering::SeqCst) != gen
                || me.rate_epoch.load(Ordering::SeqCst) != epoch
            {
                return;
            }
            // Write it back to every copy in the queue as well: that is what the queue panel
            // renders, what `persist_queue` saves, and what the next `now-playing` emits.
            let changed = {
                let mut q = me.queue.lock().await;
                let mut changed = false;
                for item in q.items.iter_mut().filter(|i| i.video_id == video_id) {
                    changed |= item.rating != Some(rating);
                    item.rating = Some(rating);
                }
                changed
            };
            if !changed {
                return;
            }
            // Not persisted on purpose: `persist_fingerprint` excludes `rating`, and this runs on
            // every track start anyway, so a stored copy would only ever be the stale one again.
            let _ =
                me.app.emit("rating", serde_json::json!({ "videoId": video_id, "rating": rating }));
        });
    }

    fn emit_now_playing(&self, item: &SongItem, stream_client: &str) {
        let _ = self.app.emit("now-playing", Self::now_playing_json(item, stream_client));
        let _ = self.app.emit("playback-state", "playing");
        // Push the same metadata to the OS media widget (context/16) and Discord.
        if let Some(m) = &self.media {
            // MPRIS/SMTC want a URL; a local track's artwork is a path, so hand it a file:// one.
            // Scheme, not leading slash: a Windows cover path is `C:\...`, and SMTC drops the whole
            // metadata update (cover included) when the thumbnail URI won't parse.
            let cover = item.thumbnail.as_ref().map(|t| {
                if t.contains("://") {
                    t.clone()
                } else {
                    format!("file://{t}")
                }
            });
            m.set_metadata(&item.title, &item.artists, item.album.as_deref(), cover.as_deref());
        }
        if let Some(d) = &self.discord {
            d.set_track(item);
        }
        self.lastfm.set_track(item);
        // New track ⇒ let the next position tick through immediately instead of waiting out the
        // ~1s throttle, so a restored seek position (and the play-state self-heal) lands at once.
        self.last_media_push.store(0, Ordering::Relaxed);
    }

    /// Push play/pause state + the current position to the OS media controls (context/16) and
    /// Discord. The single choke point for play/pause, so both stay in step with mpv. Discord gets
    /// the flag only — its position flows exclusively through the ticks, so a stale
    /// `current_position()` here (the last tick can predate a track change) can't poison its
    /// timeline.
    pub fn media_set_playing(&self, playing: bool) {
        self.is_playing.store(playing, Ordering::Relaxed);
        if let Some(m) = &self.media {
            let pos = self.current_position();
            // Record what the controls now hold, or the next tick would repeat this very push.
            *self.last_media_state.lock().unwrap() = Some((playing, pos));
            m.set_playback(playing, pos);
        }
        #[cfg(target_os = "windows")]
        crate::taskbar::set_playing(&self.app, playing);
        if let Some(d) = &self.discord {
            d.set_playing(playing);
        }
    }

    /// Toggle Discord presence at runtime (the `discord_rpc` setting). Turning it off clears the
    /// presence and closes the socket; turning it on re-pushes the current track.
    pub fn set_discord_enabled(&self, on: bool) {
        if let Some(d) = &self.discord {
            d.set_enabled(on);
        }
    }

    /// Latest mpv position (secs) — for OS scrubber updates + relative media-key seeks.
    pub fn current_position(&self) -> f64 {
        f64::from_bits(self.latest_position.load(Ordering::SeqCst))
    }

    /// Advance/rewind the queue (OS "next"/"previous" keys + the UI's skip buttons). `play_index`
    /// itself no-ops for guests.
    pub async fn next_in_queue(self: &std::sync::Arc<Self>) {
        let i = {
            let q = self.queue.lock().await;
            // Manual next escapes a repeat-one loop (the next track then loops too), so treat One
            // as All here: with any repeat engaged the queue wraps instead of dead-ending.
            let repeat = if q.repeat == RepeatMode::One { RepeatMode::All } else { q.repeat };
            next_index(q.items.len(), q.current, repeat)
        };
        match i {
            Some(i) => self.play_index(i).await,
            // Repeat off at the tail. Autoplay tops the queue up asynchronously (on track start /
            // track end), so the continuation may simply not have landed yet — fetch it now rather
            // than leaving Skip a dead button. Still a no-op when autoplay is off, when the user is
            // a guest, or when the radio returns nothing: there genuinely is no next track then.
            None => {
                let gen = self.generation.load(Ordering::SeqCst);
                if self.extend_queue_radio(gen).await > 0 {
                    let next = self.queue.lock().await.current + 1;
                    self.play_index(next).await;
                }
            }
        }
    }

    pub async fn prev_in_queue(self: &std::sync::Arc<Self>) {
        let i = self.queue.lock().await.current.saturating_sub(1);
        self.play_index(i).await;
    }

    /// Tell the UI the queue changed.
    ///
    /// A Tauri event is delivered by inlining its JSON into a JavaScript source string and
    /// `eval`ing it in every listening webview, so the payload is *parsed as source*: 2.6 MB and
    /// ~26 ms for a 5,000-track queue, twice over with the mini player open. The play pointer
    /// moving is by far the most common reason this is called and it does not need the rows at
    /// all, so when the track list is unchanged this sends a few hundred bytes instead. The UI
    /// keeps the array it already holds (`player.svelte.ts`, `onQueueIndex`).
    async fn emit_queue(&self) {
        let q = self.queue.lock().await;
        let fingerprint = queue_fingerprint(&q.items);
        let unchanged =
            self.last_queue_fingerprint.swap(fingerprint, Ordering::Relaxed) == fingerprint;
        let payload = if unchanged {
            serde_json::json!({
                "currentIndex": q.current,
                "playedFrom": q.played_from,
                "shuffle": q.shuffle_orig.is_some(),
                "repeat": q.repeat,
                "sourceName": &q.source_name,
                // The playing row, so `start_current`'s duration/artists backfill still reaches
                // the panel without shipping the other 4,999 rows to carry it.
                "current": q.items.get(q.current),
            })
        } else {
            serde_json::json!({
                "items": &q.items,
                "currentIndex": q.current,
                "playedFrom": q.played_from,
                "shuffle": q.shuffle_orig.is_some(),
                "repeat": q.repeat,
                "sourceName": &q.source_name,
            })
        };
        let _ = self.app.emit(if unchanged { "queue-index" } else { "queue-changed" }, payload);
    }

    /// A pure tail append: ship the added rows, not the whole list.
    ///
    /// `emit_queue` sends every row whenever the fingerprint changed, and an autoplay top-up
    /// changes it every time. Since the list only grew at the end, the panel can push the new rows
    /// onto the array it already holds. `len` is what the queue is now, so a UI that missed an
    /// event can tell (its own length will not match) and re-sync with `get_queue` instead of
    /// drifting silently.
    async fn emit_queue_appended(&self, added: usize) {
        let q = self.queue.lock().await;
        let start = q.items.len().saturating_sub(added);
        let payload = serde_json::json!({
            "items": &q.items[start..],
            "len": q.items.len(),
            "currentIndex": q.current,
            "playedFrom": q.played_from,
        });
        // Load-bearing: without it the next `emit_queue` thinks the rows are unchanged and sends a
        // `queue-index` for a list that grew, leaving the panel stale.
        self.last_queue_fingerprint.store(queue_fingerprint(&q.items), Ordering::Relaxed);
        let _ = self.app.emit("queue-appended", payload);
    }

    fn emit_error(&self, video_id: &str, message: &str) {
        tracing::error!(video_id, message, "playback error");
        let _ = self
            .app
            .emit("playback-error", serde_json::json!({ "videoId": video_id, "message": message }));
    }

    /// Guest tried a host-only playback action — explain instead of silently ignoring.
    fn emit_guest_hint(&self) {
        let _ = self.app.emit(
            "playback-notice",
            serde_json::json!({ "message": "The host controls playback — click a song to add it to the session queue" }),
        );
    }

    /// A transient toast. Same channel as [`Self::emit_skip`], for messages that phrase themselves.
    fn emit_notice(&self, message: &str) {
        tracing::info!(message, "playback notice");
        let _ = self.app.emit("playback-notice", serde_json::json!({ "message": message }));
    }

    /// A track was auto-skipped because no client could resolve it — a transient toast, not the
    /// persistent error banner: the queue keeps playing, so this shouldn't read as a failure.
    /// `reason` is the parenthetical the toast shows. "unavailable" for anything YouTube simply
    /// would not serve; an upload gets its own, because the song is not gone, the session is.
    fn emit_skip(&self, title: &str, reason: &str) {
        tracing::warn!(title, reason, "skipping unplayable track");
        let _ = self.app.emit(
            "playback-notice",
            serde_json::json!({ "message": format!("Skipped ({reason}): {title}") }),
        );
    }

    pub async fn queue_snapshot(&self) -> serde_json::Value {
        let q = self.queue.lock().await;
        serde_json::json!({
            "items": &q.items,
            "currentIndex": q.current,
            "playedFrom": q.played_from,
            "shuffle": q.shuffle_orig.is_some(),
            "repeat": q.repeat,
            "sourceName": &q.source_name,
        })
    }

    /// A position tick from mpv. Once the current track passes the play threshold (context/01
    /// §registerPlayback) this counts the play locally (the On Repeat playlist) and fires the
    /// watch-history ping, latched to happen exactly once per play. The ping is additionally
    /// gated on the `enable_history` setting + being logged in. Best-effort (errors logged).
    pub async fn on_position(&self, pos: f64) {
        self.record_position(pos);
        // Latched: nothing below can fire again for this play, so don't queue up behind the
        // queue mutex on every tick just to be told so. See `AppState::history_pinged`.
        if self.history_pinged.load(Ordering::Relaxed) {
            return;
        }
        let crossed = {
            let q = self.queue.lock().await;
            // `swap` and not a store: the early-return above reads the flag outside the lock,
            // so the latch has to be the thing that decides, not the read. Latches even if the
            // ping below is missing or fails, so it is never retried.
            if pos >= history_threshold(q.duration)
                && !self.history_pinged.swap(true, Ordering::Relaxed)
            {
                Some((q.playback_ping.clone(), q.cpn.clone(), q.items.get(q.current).cloned()))
            } else {
                None
            }
        };
        let Some((ping, cpn, played)) = crossed else { return };

        // Local play count, on the same threshold. Deliberately not gated on `enable_history` or
        // sign-in: that setting is about registering plays with YouTube, while this never leaves
        // the machine and has to work signed out.
        // ponytail: resuming a restored track from past the threshold counts it a second time (the
        // watch-history ping has always done the same). One extra count per launch, for one song,
        // against a month of plays. If it ever skews the list, pass the `pending_seek` offset into
        // the latch and skip the record when the play didn't start near zero.
        // Local files don't count: On Repeat is the only thing built from this table, and it's a
        // YouTube Music playlist — a row pointing at a path on this disk doesn't belong in it.
        if let Some(item) = played.filter(|i| !crate::local::is_local_song(&i.video_id)) {
            if let Ok(json) = serde_json::to_string(&item) {
                self.db.record_play(&item.video_id, &json, now_secs(), ON_REPEAT_WINDOW_SECS);
            }
        }

        if !self.history_enabled() || !self.it.is_logged_in() {
            return;
        }
        // Say so when there's nothing to ping. This used to be a silent `return`, which is why
        // issue #83 could only be diagnosed by reading the source: the play was dropped with no
        // trace anywhere. Anything above `debug` is noise on a working install, but the line
        // exists now.
        let Some(ping) = ping else {
            tracing::info!(
                "no watch-history ping for this play (no client returned a tracking URL)"
            );
            return;
        };
        // Ping as the client that was issued the URL, not always the metadata one.
        let Some(client) = self.clients.get(&ping.client).cloned() else {
            tracing::warn!(client = %ping.client, "watch-history ping: unknown client");
            return;
        };
        let it = self.it.clone();
        tauri::async_runtime::spawn(async move {
            match it.register_playback(&client, &ping.url, &cpn, None).await {
                Ok(()) => tracing::debug!(client = %ping.client, "watch-history ping sent"),
                Err(e) => tracing::warn!(error = %e, "watch-history ping failed"),
            }
        });
    }

    /// Latest mpv-reported track duration (secs), feeding the history-ping threshold + OS scrubber.
    pub async fn on_duration(&self, secs: f64) {
        if secs.is_finite() && secs > 0.0 {
            self.queue.lock().await.duration = secs;
            if let Some(m) = &self.media {
                m.set_duration(secs);
            }
            if let Some(d) = &self.discord {
                d.set_duration(secs);
            }
            self.lastfm.set_duration(secs);
        }
    }

    /// Watch-history ping enabled? Default on; only an explicit `"false"` disables it.
    fn history_enabled(&self) -> bool {
        self.db.get_setting("enable_history").map(|v| v != "false").unwrap_or(true)
    }

    /// Shuffle sticky across queues? Default off: turning shuffle on applies to the queue that's
    /// playing, and opening an album/playlist afterwards plays it in order (issue #117).
    fn sticky_shuffle(&self) -> bool {
        self.db.get_setting("sticky_shuffle").as_deref() == Some("true")
    }

    /// Autoplay enabled? Default on; only an explicit `"false"` disables it (mirrors
    /// `history_enabled`).
    fn autoplay_enabled(&self) -> bool {
        self.db.get_setting("autoplay").map(|v| v != "false").unwrap_or(true)
    }

    /// Extend the queue with radio continuation when it's nearly out (autoplay). Returns how many
    /// tracks were appended. Guards: setting on, repeat Off, not a guest, tail near (last two
    /// tracks), generation unchanged across the network call. Continuation matches where the queue
    /// came from: playlist/album radio (`radio_seed`) or song radio seeded from the last track.
    /// Dedupes against the entire current queue; caps at `AUTOPLAY_BATCH` per hop. When the radio
    /// returns nothing new, playback later stops exactly as pre-autoplay (no retry loop).
    async fn extend_queue_radio(self: &std::sync::Arc<Self>, gen: u64) -> usize {
        const AUTOPLAY_BATCH: usize = 20;
        if !self.autoplay_enabled() || self.lt.is_guest().await {
            return 0;
        }
        let (last_video, seed, existing) = {
            let q = self.queue.lock().await;
            if q.repeat != RepeatMode::Off {
                return 0; // the queue never exhausts under repeat
            }
            if q.items.len().saturating_sub(q.current) > 2 {
                return 0; // tail not near yet
            }
            let Some(last) = q.items.last() else { return 0 };
            // Nothing to continue from when the queue ends on a local file: its path is not a
            // videoId, and a queue of local music is exactly the case that has to work offline.
            if q.radio_seed.is_none() && crate::local::is_local_song(&last.video_id) {
                return 0;
            }
            let seed = q.radio_seed.clone().unwrap_or_else(|| format!("RDAMVM{}", last.video_id));
            let existing: HashSet<String> = q.items.iter().map(|i| i.video_id.clone()).collect();
            (last.video_id.clone(), seed, existing)
        };
        let Some(client) = self.clients.get(innertube::METADATA_CLIENT) else { return 0 };
        // Snapshot → network → re-lock, same discipline as `prime_lookahead`; the generation
        // check between them is what makes it safe. A track added *during* the fetch could
        // theoretically duplicate — accepted (YTM's own radio repeats occasionally too).
        let fresh = match self.it.next(client, Some(&last_video), Some(&seed)).await {
            Ok(next) => next.items,
            Err(e) => {
                tracing::warn!(error = %e, "autoplay radio fetch failed");
                return 0;
            }
        };
        if self.generation.load(Ordering::SeqCst) != gen {
            return 0; // user moved on while we fetched
        }
        let (added, trimmed) = {
            let mut q = self.queue.lock().await;
            let added = merge_radio(&mut q.items, fresh, existing, AUTOPLAY_BATCH);
            // Not while shuffle is on: `shuffle_orig` is a parallel clone of the list and rebasing
            // both consistently is a bigger change than this one. Deliberate, see plan 039.
            let trimmed = if added > 0 && q.shuffle_orig.is_none() {
                let cur = q.current;
                let dropped = trim_played(&mut q.items, cur, KEEP_PLAYED);
                if dropped > 0 {
                    q.current -= dropped;
                    q.played_from = q.played_from.saturating_sub(dropped);
                    // Only ever set to `current` or `current + 1`, so this never underflows.
                    q.lookahead_loaded = q.lookahead_loaded.map(|i| i.saturating_sub(dropped));
                }
                dropped
            } else {
                0
            };
            (added, trimmed)
        };
        if added > 0 {
            tracing::info!(added, trimmed, seed = %seed, "autoplay extended the queue");
            if trimmed == 0 {
                self.emit_queue_appended(added).await;
            } else {
                self.emit_queue().await; // the front moved, so indices shifted: send the truth
            }
            self.persist_queue().await;
            self.lt_broadcast_queue().await;
            // Appending at the tail never touches a primed lookahead slot; this covers the
            // "current was last, nothing was primed" case.
            self.prime_lookahead(gen).await;
        }
        added
    }

    /// Persist the queue so a restart can restore it losslessly (context/11 §state). Called
    /// whenever the queue changes or advances.
    ///
    /// The blob is 526 bytes per track (measured), so a 5,000-track queue is a ~2.6 MB
    /// synchronous write, and the advance path calls this on every single track. On an advance
    /// the rows are identical and only the pointer moved, so that case writes ~40 bytes to
    /// `queue_index` instead. `queue_index` is written every time, blob or not, and carries the
    /// fingerprint of the rows it was true for, so `restore_queue` can tell a fresher pointer from
    /// one left over from a different item list.
    async fn persist_queue(&self) {
        let (index_json, blob) = {
            let q = self.queue.lock().await;
            let fingerprint = queue_fingerprint(&q.items);
            let index_json = serde_json::json!({
                "current": q.current,
                "playedFrom": q.played_from,
                "repeat": q.repeat,
                // Which item list these numbers were true for. `queue_json` and `queue_index` are
                // two separate writes, so a kill between them, or two `persist_queue` calls
                // landing out of order, can pair a pointer with rows it was never valid for.
                // `restore_queue` checks this before trusting any of it.
                "fp": fingerprint.to_string(),
            })
            .to_string();
            let persist_fp = persist_fingerprint(&q);
            let changed =
                self.last_persisted_fingerprint.swap(persist_fp, Ordering::Relaxed) != persist_fp;
            let blob = changed.then(|| {
                serde_json::json!({
                    "items": &q.items,
                    "current": q.current,
                    "playedFrom": q.played_from,
                    "repeat": q.repeat,
                    "shuffleOrig": &q.shuffle_orig,
                    "radioSeed": &q.radio_seed,
                    "sourceName": &q.source_name,
                    "radio": q.radio,
                })
                .to_string()
            });
            (index_json, blob)
        };
        if let Some(blob) = blob {
            self.db.set_setting("queue_json", &blob);
        }
        self.db.set_setting("queue_index", &index_json);
    }

    /// Restore the last session's queue on startup — paused, not autoplaying (context/11). The
    /// saved position is applied when the user first hits play (see `start_current`). Emits
    /// `queue-changed` + `now-playing` so the UI shows the restored track.
    pub async fn restore_queue(self: &std::sync::Arc<Self>) {
        let Some(json) = self.db.get_setting("queue_json") else { return };
        let Ok(saved) = serde_json::from_str::<serde_json::Value>(&json) else { return };
        let items: Vec<SongItem> = saved
            .get("items")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        if items.is_empty() {
            return;
        }
        let mut current = (saved.get("current").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
            .min(items.len() - 1);
        // Absent in blobs written before "Previously played" existed: those restore with an empty
        // played run rather than claiming the whole prefix was heard.
        let mut played_from =
            (saved.get("playedFrom").and_then(|v| v.as_u64()).unwrap_or(current as u64) as usize)
                .min(current);
        // Shuffle/repeat ride the same blob; read tolerantly — old blobs lack them.
        let mut repeat: RepeatMode = saved
            .get("repeat")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let shuffle_orig: Option<Vec<SongItem>> =
            saved.get("shuffleOrig").and_then(|v| serde_json::from_value(v.clone()).ok()).flatten();
        let radio_seed: Option<String> =
            saved.get("radioSeed").and_then(|v| v.as_str()).map(str::to_owned);
        let source_name: Option<String> =
            saved.get("sourceName").and_then(|v| v.as_str()).map(str::to_owned);
        let radio = saved.get("radio").and_then(|v| v.as_bool()).unwrap_or(false);
        // `queue_index` is rewritten on every persist while `queue_json` is only rewritten when
        // the rows change, so on an advance it is the fresher of the two. But the two are separate
        // writes: a kill between them, or two `persist_queue` calls landing out of order (a
        // background playlist fill racing a track advance), can leave a `queue_index` on disk that
        // was computed against a different item list than the one in `queue_json` now. Trusting it
        // blindly would pair a pointer with rows it was never valid for and `.min(items.len() - 1)`
        // would turn that into a plausible-looking wrong track instead of anything visible. The
        // `fp` stamp is what the index was true for; only apply it when it still matches what we
        // just parsed. A mismatch (or a database written before the stamp existed) falls back to
        // the blob's own current/playedFrom/repeat, which is exactly today's behaviour.
        let items_fp = queue_fingerprint(&items).to_string();
        if let Some(idx) = self
            .db
            .get_setting("queue_index")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .filter(|idx| idx.get("fp").and_then(|v| v.as_str()) == Some(items_fp.as_str()))
        {
            if let Some(c) = idx.get("current").and_then(|v| v.as_u64()) {
                current = (c as usize).min(items.len() - 1);
            }
            if let Some(p) = idx.get("playedFrom").and_then(|v| v.as_u64()) {
                played_from = (p as usize).min(current);
            }
            if let Some(r) = idx.get("repeat").and_then(|v| serde_json::from_value(v.clone()).ok())
            {
                repeat = r;
            }
        }
        let pos = self.db.get_setting("queue_position").and_then(|s| s.parse::<f64>().ok());
        if let Some(p) = pos.filter(|p| *p > 0.0) {
            *self.pending_seek.lock().unwrap() = Some((items[current].video_id.clone(), p));
        }
        {
            let mut q = self.queue.lock().await;
            q.current = current;
            q.played_from = played_from;
            q.items = items;
            q.repeat = repeat;
            q.shuffle_orig = shuffle_orig;
            q.radio_seed = radio_seed;
            q.source_name = source_name;
            q.radio = radio;
        }
        if repeat == RepeatMode::One {
            let _ = self.player.set_loop_file(true);
        }
        if let Some(item) = self.current_item().await {
            // Restored, not playing — announce the track but leave playback paused. Declare the
            // paused state *first*: mpv reports `pause: false` while idle at boot, so a track
            // announced before this would briefly look like it was playing (and put a presence card
            // up for a song nobody started).
            self.media_set_playing(false);
            self.emit_now_playing(&item, "restored");
            // The stored rating is as old as the database: a like made on another device since
            // has never been seen here (issue #93).
            self.refresh_rating(&item.video_id, self.generation.load(Ordering::SeqCst));
            let _ = self.app.emit("playback-state", "paused");
        }
        self.emit_queue().await;
    }

    /// Throttled position persistence for resume-on-restart. Records the latest position always
    /// (for a precise flush on pause) and writes it to the DB at most every 5s.
    fn record_position(&self, pos: f64) {
        self.latest_position.store(pos.to_bits(), Ordering::SeqCst);
        let now = now_secs() as u64;
        if now.saturating_sub(self.last_pos_persist.load(Ordering::Relaxed)) >= 5 {
            self.last_pos_persist.store(now, Ordering::Relaxed);
            self.db.set_setting("queue_position", &pos.to_string());
        }
        // Update the OS scrubber (~1s), throttled separately from the DB write. Discord rides the
        // same tick — not to redraw its bar (it runs its own clock off the timestamps we pushed)
        // but so it can notice a seek and re-push. A tick is NOT proof of playback (mpv also fires
        // `time-pos` on seeks while paused), so ask mpv for the play state rather than assuming it.
        if now.saturating_sub(self.last_media_push.load(Ordering::Relaxed)) >= 1 {
            self.last_media_push.store(now, Ordering::Relaxed);
            // Never ask mpv anything here — this runs on the event pump, and `mpv_get_property` is
            // synchronous on mpv's core lock. `is_playing` is kept current by `PlayerEvent::Playing`,
            // which mpv now pushes (it derives from `idle-active`, not just `pause`).
            let playing = self.is_playing.load(Ordering::Relaxed);
            if let Some(m) = &self.media {
                // Push when the play state flipped, or when the position moved further than one
                // tick of ordinary playback explains (a seek, a track change). Not every second:
                // see `last_media_state`. At a 1 s gate this settles into a push every 2-3
                // seconds, which keeps a polled scrubber within a couple of seconds of the truth.
                let mut last = self.last_media_state.lock().unwrap();
                let send = match *last {
                    None => true,
                    Some((p, at)) => p != playing || (pos - at).abs() > 2.0,
                };
                if send {
                    *last = Some((playing, pos));
                    drop(last);
                    m.set_playback(playing, pos);
                }
            }
            if let Some(d) = &self.discord {
                d.set_position(pos);
            }
            self.lastfm.set_position(pos);
        }
    }

    /// Flush the latest known position to the DB immediately (e.g. on pause).
    pub fn flush_position(&self) {
        let pos = f64::from_bits(self.latest_position.load(Ordering::SeqCst));
        self.db.set_setting("queue_position", &pos.to_string());
    }

    /// Clear both cache tiers (settings "Clear caches"): the SQLite URL cache + mpv's on-disk
    /// audio bytes. Best-effort on the files — the current track may re-buffer. context/14.
    pub fn clear_caches(&self) {
        self.db.clear_stream_cache();
        // The stored PoToken is a cache too, and "clear caches" is where someone goes when
        // playback has started behaving oddly. Dropping it costs one BotGuard bootstrap.
        self.db.delete_setting("potoken_session");
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for e in entries.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    // --- Listen Together (context/19) --------------------------------------------------------

    /// Apply one sync command from the connection (the bridge task drives this). Guest playback +
    /// host seeding. See `crate::listentogether`.
    pub async fn apply_sync(self: &std::sync::Arc<Self>, cmd: SyncCommand) {
        match cmd {
            SyncCommand::HostSeed => self.lt_host_seed().await,
            SyncCommand::Release => {} // role already flipped; nothing to undo
            SyncCommand::ApplyState(state) => self.lt_apply_state(state).await,
            SyncCommand::ChangeTrack { track, position_ms, playing, queue } => {
                self.lt_apply_change_track(track, position_ms, playing, queue).await
            }
            SyncCommand::Play { position_ms, server_time_ms } => {
                self.lt_apply_play(position_ms, server_time_ms).await
            }
            SyncCommand::Pause { position_ms } => self.lt_apply_pause(position_ms).await,
            SyncCommand::Seek { position_ms } => {
                let _ = self.player.seek(position_ms as f64 / 1000.0);
            }
            SyncCommand::SyncQueue { queue } => self.lt_mirror_queue(queue).await,
            SyncCommand::GuestAdd { track } => self.lt_enqueue_track(track).await,
        }
    }

    /// Guest: apply a full room-state snapshot (join / reconnect / re-sync). If the current track is
    /// already loaded, just correct the position + play state (no reload blip); otherwise load it.
    async fn lt_apply_state(&self, state: listen_protocol::RoomState) {
        let Some(track) = state.current_track else { return };
        let already_loaded = {
            let q = self.queue.lock().await;
            q.items.get(q.current).map(|i| i.video_id == track.id).unwrap_or(false)
        };
        if already_loaded && !self.player.is_idle() {
            let target = state.position_ms as f64 / 1000.0;
            if state.is_playing {
                // Only correct meaningful drift — avoid a re-buffer glitch when we're already synced
                // (e.g. the post-join auto re-sync after the initial compensation nailed it).
                if (self.current_position() - target).abs() > 0.35 {
                    let _ = self.player.seek(target);
                }
                let _ = self.player.play();
            } else {
                if target > 0.5 {
                    let _ = self.player.seek(target);
                }
                let _ = self.player.pause();
            }
            // A re-sync/reconnect snapshot also carries the queue — mirror it so guest adds that
            // happened while we were away aren't missing until the next track change.
            self.lt_mirror_queue(state.queue).await;
        } else {
            self.lt_apply_change_track(track, state.position_ms, state.is_playing, state.queue)
                .await;
        }
    }

    /// Guest: load a host-chosen track, seek to its live position, set play/pause, mirror the queue.
    async fn lt_apply_change_track(
        &self,
        track: Track,
        position_ms: i64,
        playing: bool,
        upcoming: Vec<Track>,
    ) {
        // Timestamp entry: resolving + loading the stream takes ~1–2s, during which the host keeps
        // playing. We add that elapsed wall-time to the seek target so the guest lands on the host's
        // *live* position, not the stale one captured at join. context/19 §6.5.
        let t0 = std::time::Instant::now();
        // Bump the generation so any in-flight local resolve discards itself.
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut q = self.queue.lock().await;
            let mut items = vec![track_to_song(&track)];
            items.extend(upcoming.iter().map(track_to_song));
            q.items = items;
            q.current = 0;
            q.played_from = 0; // the host's queue starts at the track it sent; no local history
            q.lookahead_loaded = None;
            q.shuffle_orig = None; // host rebuilt the queue — local shuffle snapshot is stale
            q.radio_seed = None; // guests never autoplay — the host drives
            q.source_name = None; // the host's context isn't known — header falls back
        }
        // A Listen Together track is the host's; `Track` carries no upload flag and a guest could
        // not stream someone else's upload anyway.
        let data = match self.resolve(&track.id, false).await {
            Ok(d) => d,
            Err(e) => {
                self.emit_error(&track.id, &e.to_string());
                return;
            }
        };
        if self.generation.load(Ordering::SeqCst) != gen {
            return; // superseded by a newer sync
        }
        if let Err(e) =
            self.player.load(&data.stream_url, &data.headers, loudness_gain(data.loudness_db))
        {
            self.emit_error(&track.id, &e.to_string());
            return;
        }
        // Seek first (mpv queues it until the file loads), then set play/pause — avoids a blip of
        // audio at 0 before the seek lands.
        let target_ms =
            if playing { position_ms + t0.elapsed().as_millis() as i64 } else { position_ms };
        let pos = target_ms as f64 / 1000.0;
        if pos > 0.5 {
            let _ = self.player.seek(pos);
        }
        let _ = if playing { self.player.play() } else { self.player.pause() };
        if let Some(item) = self.current_item().await {
            self.emit_now_playing(&item, "listen-together");
        }
        if !playing {
            let _ = self.app.emit("playback-state", "paused");
        }
        self.emit_queue().await;
        // The elapsed-compensation above still can't see mpv's own decode/buffer startup. Fire one
        // delayed re-sync so the guest snaps to the host's live position once audio is actually
        // flowing. Re-sync is seek-only for the loaded track, so there's no reload blip.
        if playing {
            let lt = self.lt.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                lt.request_sync().await;
            });
        }
    }

    /// Guest: apply a play, offsetting the target position by transit latency (context/19 §6.5).
    async fn lt_apply_play(&self, position_ms: i64, server_time_ms: i64) {
        let target = if server_time_ms > 0 {
            position_ms + (now_ms() - server_time_ms).max(0)
        } else {
            position_ms
        };
        let cur_ms = (self.current_position() * 1000.0) as i64;
        if (cur_ms - target).abs() > 2000 {
            let _ = self.player.seek(target as f64 / 1000.0);
        }
        let _ = self.player.play();
    }

    /// Guest: apply a pause, correcting position if it drifted past tolerance.
    async fn lt_apply_pause(&self, position_ms: i64) {
        let cur_ms = (self.current_position() * 1000.0) as i64;
        if (cur_ms - position_ms).abs() > 2000 {
            let _ = self.player.seek(position_ms as f64 / 1000.0);
        }
        let _ = self.player.pause();
    }

    /// Host: broadcast the current track + upcoming queue as a ChangeTrack. No-op unless host.
    async fn lt_broadcast_current_track(&self, position_ms: i64, playing: bool) {
        if !self.lt.is_host().await {
            return;
        }
        let (track, queue) = {
            let q = self.queue.lock().await;
            let Some(cur) = q.items.get(q.current) else { return };
            let track = song_to_track(cur);
            let queue: Vec<Track> =
                q.items.iter().skip(q.current + 1).take(50).map(song_to_track).collect();
            (track, queue)
        };
        let mut p = Playback::new(PlaybackKind::ChangeTrack);
        p.track = Some(track);
        p.position_ms = position_ms;
        p.playing = playing;
        p.queue = Some(queue);
        self.lt.broadcast_playback(p).await;
    }

    /// Host: seed a freshly-created room with whatever we're currently playing.
    async fn lt_host_seed(&self) {
        let position_ms = (self.current_position() * 1000.0) as i64;
        // `is_idle` only says whether a file is loaded — it's still false while paused, so seeding
        // from it would tell guests to play a song the host has paused.
        let playing = self.is_playing.load(Ordering::Relaxed);
        self.lt_broadcast_current_track(position_ms, playing).await;
    }

    /// Host: broadcast play/pause with the live position (called from the event pump). No-op unless
    /// host.
    pub async fn lt_on_play_state(&self, playing: bool) {
        if !self.lt.is_host().await {
            return;
        }
        let pos_ms = (self.current_position() * 1000.0) as i64;
        let p = if playing {
            let mut p = Playback::at(PlaybackKind::Play, pos_ms);
            p.playing = true;
            p
        } else {
            Playback::at(PlaybackKind::Pause, pos_ms)
        };
        self.lt.broadcast_playback(p).await;
    }

    /// User seek from the UI. Blocked for guests; broadcast for host.
    pub async fn user_seek(&self, position: f64) -> Result<(), String> {
        if self.lt.is_guest().await {
            return Ok(()); // guests can't scrub — the host controls the timeline
        }
        self.player.seek(position).map_err(|e| e.to_string())?;
        if self.lt.is_host().await {
            self.lt
                .broadcast_playback(Playback::at(PlaybackKind::Seek, (position * 1000.0) as i64))
                .await;
        }
        Ok(())
    }

    /// Toggle shuffle on the current queue. ON: snapshot the order, then Fisher–Yates only the
    /// *upcoming* items. OFF: restore the snapshot, keeping the playing track, minus anything the
    /// shuffle already played (see [`unshuffled`]). Guests: host-only hint.
    /// ponytail: OFF restores from the snapshot — tracks added while shuffled are dropped from the
    /// restored order (still playing/played fine; snapshot semantics).
    pub async fn toggle_shuffle(self: &std::sync::Arc<Self>) {
        if self.lt.is_guest().await {
            self.emit_guest_hint();
            return;
        }
        {
            let mut q = self.queue.lock().await;
            if q.items.is_empty() {
                return;
            }
            if let Some(orig) = q.shuffle_orig.take() {
                let playing = q.items[q.current].video_id.clone();
                let fallback = q.current;
                // The shuffled prefix (through the playing track) is what's already been played —
                // the restored order must not offer any of it again.
                let heard: HashSet<String> =
                    q.items[..=q.current].iter().map(|i| i.video_id.clone()).collect();
                let (items, idx) = unshuffled(orig, &heard, &playing, fallback);
                q.items = items;
                q.current = idx;
                // The restored prefix is the playlist's own order, not the order things were heard
                // in, so the played run no longer describes anything real. Start it over.
                q.played_from = idx;
            } else {
                q.shuffle_orig = Some(q.items.clone());
                let current = q.current;
                shuffle_upcoming(&mut q.items, current);
            }
            // The primed lookahead almost certainly points at the wrong song now — drop it
            // unconditionally (cheap; re-primed below).
            if q.lookahead_loaded.take().is_some() {
                let _ = self.player.clear_playlist();
            }
        }
        self.emit_queue().await;
        self.persist_queue().await;
        let gen = self.generation.load(Ordering::SeqCst);
        self.prime_lookahead(gen).await;
        // Un-shuffling can leave nothing upcoming — a full shuffled pass through an album drops
        // every heard track from the restored order. Autoplay's own triggers only fire on track
        // start / track end, so without this the queue panel shows no continuation (and the gapless
        // lookahead has nothing to prime) until the current song runs out. No-op unless the tail is
        // actually near.
        self.extend_queue_radio(gen).await;
        self.lt_broadcast_queue().await;
    }

    /// Set the repeat mode. Repeat-one is enforced by mpv's `loop-file` (seamless, no end-file
    /// event); all/off live in the queue-advance logic (`next_index`). Guests: host-only hint.
    pub async fn set_repeat(self: &std::sync::Arc<Self>, mode: RepeatMode) {
        if self.lt.is_guest().await {
            self.emit_guest_hint();
            return;
        }
        {
            let mut q = self.queue.lock().await;
            q.repeat = mode;
        }
        let _ = self.player.set_loop_file(mode == RepeatMode::One);
        self.emit_queue().await; // carries the new repeat state to the UI
        self.persist_queue().await;
        // Repeat-all newly on while playing the last track: the wrap target needs priming.
        self.prime_lookahead(self.generation.load(Ordering::SeqCst)).await;
    }

    /// "Play next" from a ⋯ menu: the tracks land at the "up next" boundary — right after the
    /// current song, behind any earlier manual adds (FIFO) — never buried at the end. `from` is
    /// the album/playlist they came from, for the queue panel's block heading.
    pub async fn play_next(
        self: &std::sync::Arc<Self>,
        items: Vec<SongItem>,
        from: Option<String>,
    ) {
        self.enqueue(items, true, from, None).await;
    }

    /// "Add to queue": the tracks go at the back of the manual block, ahead of the playing context
    /// and anything the app generated (see [`enqueue_at`]). `continuation` is the next-page token —
    /// the rest of a long playlist is walked in the background instead of adding only page one.
    pub async fn add_to_queue(
        self: &std::sync::Arc<Self>,
        items: Vec<SongItem>,
        from: Option<String>,
        continuation: Option<String>,
    ) {
        self.enqueue(items, false, from, continuation).await;
    }

    /// Shared body of "Play next" / "Add to queue". In a session, guests route through the host
    /// (suggest → auto-approve) and the host tags the add with their own name so the room sees who
    /// added it.
    async fn enqueue(
        self: &std::sync::Arc<Self>,
        items: Vec<SongItem>,
        next: bool,
        from: Option<String>,
        continuation: Option<String>,
    ) {
        if items.is_empty() {
            return;
        }
        if self.lt.is_guest().await {
            // A guest owns no queue: every track goes to the host as its own suggestion, in order.
            for item in items {
                self.lt.suggest(song_to_track(&item)).await;
            }
            return;
        }
        let items = match self.lt.is_host().await {
            false => items,
            true => {
                let by = self.lt.my_username().await.unwrap_or_else(|| "Host".into());
                items
                    .into_iter()
                    .map(|mut i| {
                        i.queued_by = Some(by.clone());
                        i
                    })
                    .collect()
            }
        };
        self.insert_queued(items, next, from.clone()).await;
        if let Some(token) = continuation {
            // After `insert_queued`: an add to an empty queue starts playback, which bumps the
            // generation the walk has to match.
            let gen = self.generation.load(Ordering::SeqCst);
            let me = self.clone();
            tokio::spawn(async move { me.fill_playlist(gen, token, Fill::Queued(from)).await });
        }
    }

    /// Host: add a session track to the real queue at the session boundary. Thin wrapper over
    /// `insert_queued` (the `Track` wire shape drops the nav ids solo adds keep).
    pub async fn lt_enqueue_track(self: &std::sync::Arc<Self>, track: Track) {
        self.insert_queued(vec![track_to_song(&track)], true, None).await;
    }

    /// Splice a block of manually-queued tracks into the queue, then emit/persist/re-prime and (as
    /// host) broadcast. Shared by "Play next", "Add to queue" and approved guest suggestions.
    ///
    /// Both land in the same place — the "Next in queue" block right behind the playing track,
    /// ahead of the context and of anything the app generated. `next` ("Play next") marks them
    /// `queued` and puts them at the front of that block; "Add to queue" marks them `queued_end`
    /// and puts them at its back, so a play-next always plays before an add-to-queue. `from` names
    /// the album/playlist they came from, for the block's heading.
    async fn insert_queued(
        self: &std::sync::Arc<Self>,
        mut items: Vec<SongItem>,
        next: bool,
        from: Option<String>,
    ) {
        if items.is_empty() {
            return;
        }
        for item in &mut items {
            item.queued = next;
            item.queued_end = !next;
            item.queued_from = from.clone();
        }
        let dedupe = self.db.get_setting("prevent_duplicates").as_deref() == Some("true");
        let was_empty = {
            let mut q = self.queue.lock().await;
            let was_empty = q.items.is_empty();
            let before = q.items.get(q.current + 1).map(|i| i.video_id.clone());
            // "Prevent duplicates" is a *move*, not a reject: every existing copy goes, then the
            // tracks are spliced in at the target. `at` is computed after, so a copy removed from
            // before the playing track doesn't push the insert one slot too far.
            let ids: HashSet<&str> = items.iter().map(|i| i.video_id.as_str()).collect();
            let qm = &mut *q; // the guard hands out one borrow; a struct ref splits per field
            let mut removed = dedupe
                && drop_duplicates(&mut qm.items, &mut qm.current, &mut qm.played_from, &ids);
            // Setting or not: a copy already waiting in the manual block is *moved*, never doubled.
            // "Play next" on a row you can see in the queue means move it up, and a second identical
            // row is no answer to that. Context/playlist rows are left alone — queueing one of those
            // is a deliberate copy.
            let cur = qm.current;
            let n = qm.items.len();
            let mut i = 0;
            qm.items.retain(|it| {
                let keep = i <= cur
                    || !((it.queued || it.queued_end) && ids.contains(it.video_id.as_str()));
                i += 1;
                keep
            });
            removed |= qm.items.len() != n;
            if removed {
                if let Some(orig) = qm.shuffle_orig.as_mut() {
                    // Off the snapshot too, or turning shuffle off rebuilds the queue with them.
                    let kept: HashSet<String> =
                        qm.items.iter().map(|i| i.video_id.clone()).collect();
                    orig.retain(|i| kept.contains(&i.video_id));
                }
            }
            let at = if next { guest_insert_index(&q.items, q.current) } else { enqueue_at(&q) };
            // The snapshot takes the new tracks too, or turning shuffle off would delete them. It
            // takes them in their real order: that's what un-shuffle restores.
            if let Some(orig) = q.shuffle_orig.as_mut() {
                orig.extend(items.iter().cloned());
                // Shuffle is on, so a block added from an album/playlist joins it right away
                // rather than waiting for the next toggle (same rule as `shuffle_upcoming`: the
                // block keeps its place, its own tracks get randomized).
                if from.is_some() {
                    use rand::seq::SliceRandom;
                    items.shuffle(&mut rand::thread_rng());
                }
            }
            q.items.splice(at..at, items);
            // Drop the primed lookahead when what plays next moved (an append past the tail
            // retargets a primed repeat-all wrap from index 0 to the new item) or when a different
            // song now sits in the primed slot — otherwise the gapless advance plays the wrong one.
            let expected = next_index(q.items.len(), q.current, q.repeat);
            let after = q.items.get(q.current + 1).map(|i| i.video_id.clone());
            // `removed`: the removals renumbered the queue, so the recorded index no longer means
            // what it did. Cheaper to drop it and let the re-prime below sort it out.
            if q.lookahead_loaded.is_some()
                && (removed || q.lookahead_loaded != expected || before != after)
            {
                q.lookahead_loaded = None;
                let _ = self.player.clear_playlist();
            }
            was_empty
        };
        // Nothing was playing and nothing was queued: an add is the whole queue, so start it —
        // otherwise the tracks sit there paused and the click looks like it did nothing.
        if was_empty {
            self.play_index(0).await; // emits + persists + primes on its way
            self.lt_broadcast_queue().await;
            return;
        }
        self.emit_queue().await;
        self.persist_queue().await;
        // Re-prime: replaces a dropped stale lookahead, and covers the insert-after-last case
        // (no lookahead existed because nothing was next). No-op when still primed correctly.
        self.prime_lookahead(self.generation.load(Ordering::SeqCst)).await;
        self.lt_broadcast_queue().await;
    }

    /// Remove an upcoming track from the queue (host's ✕ on guest adds; also plain local queue
    /// editing outside a session). The currently playing index can't be removed; guests can't
    /// remove anything (add-only).
    pub async fn remove_from_queue(self: &std::sync::Arc<Self>, index: usize) {
        if self.lt.is_guest().await {
            return;
        }
        let stale_lookahead = {
            let mut q = self.queue.lock().await;
            if index >= q.items.len() || index == q.current {
                return;
            }
            q.items.remove(index);
            if index < q.current {
                q.current -= 1;
                // Removing from in front of the played run shifts it; removing from inside it just
                // makes it one shorter, which the decremented `current` already does.
                if index < q.played_from {
                    q.played_from -= 1;
                }
            }
            match q.lookahead_loaded {
                // mpv holds the removed song as the gapless next — drop it. (Compared against the
                // recorded index, not `current + 1`, so a primed repeat-all wrap target is caught.)
                Some(i) if i == index => {
                    q.lookahead_loaded = None;
                    let _ = self.player.clear_playlist();
                    true
                }
                // The primed entry is the same song at a shifted index.
                Some(i) if i > index => {
                    q.lookahead_loaded = Some(i - 1);
                    false
                }
                _ => false,
            }
        };
        self.emit_queue().await;
        self.persist_queue().await;
        if stale_lookahead {
            self.prime_lookahead(self.generation.load(Ordering::SeqCst)).await;
        }
        self.lt_broadcast_queue().await;
    }

    /// Drag-to-reorder in the queue panel: take the track at `from` and drop it at `to`. Only
    /// upcoming tracks move, and only among themselves — the playing track and the history stay
    /// where they are (both indices are clamped past `current`). Guests own no queue.
    ///
    /// ponytail: a pure move, markers untouched. A playlist track dragged into the manual block is
    /// still a playlist track, so the panel re-splits its headings around where it landed, which is
    /// the truth. Adopt-the-block-you-land-in only if that reads wrong in practice.
    pub async fn move_in_queue(self: &std::sync::Arc<Self>, from: usize, to: usize) {
        if self.lt.is_guest().await {
            return;
        }
        let stale_lookahead = {
            let mut q = self.queue.lock().await;
            let first = q.current + 1;
            if from < first || from >= q.items.len() || to < first || to >= q.items.len() {
                return;
            }
            if from == to {
                return;
            }
            let before = q.items.get(first).map(|i| i.video_id.clone());
            let item = q.items.remove(from);
            q.items.insert(to, item);
            // Only what plays *next* can invalidate the primed gapless slot; a move deeper in the
            // queue leaves it alone rather than paying for a re-resolve on every drag.
            let expected = next_index(q.items.len(), q.current, q.repeat);
            let stale = q.lookahead_loaded.is_some()
                && (q.lookahead_loaded != expected
                    || before != q.items.get(first).map(|i| i.video_id.clone()));
            if stale {
                q.lookahead_loaded = None;
                let _ = self.player.clear_playlist();
            }
            stale
        };
        self.emit_queue().await;
        self.persist_queue().await;
        if stale_lookahead {
            self.prime_lookahead(self.generation.load(Ordering::SeqCst)).await;
        }
        self.lt_broadcast_queue().await;
    }

    /// Remove every upcoming track the user queued by hand — both blocks, "Play next" and
    /// "Add to queue" (the panel's Clear queue). Played/playing items and the playlist context
    /// stay. Guests: add-only, no clearing.
    pub async fn clear_queued(self: &std::sync::Arc<Self>) {
        if self.lt.is_guest().await {
            return;
        }
        {
            let mut q = self.queue.lock().await;
            let cur = q.current;
            let before = q.items.len();
            let mut i = 0;
            q.items.retain(|item| {
                let keep = i <= cur || !(item.queued || item.queued_end);
                i += 1;
                keep
            });
            if q.items.len() == before {
                return; // nothing was queued — don't touch the lookahead
            }
            // Indices shifted — a primed lookahead may point at the wrong slot. Drop it
            // unconditionally (cheap; re-primed below), same as toggle_shuffle.
            if q.lookahead_loaded.take().is_some() {
                let _ = self.player.clear_playlist();
            }
        }
        self.emit_queue().await;
        self.persist_queue().await;
        self.prime_lookahead(self.generation.load(Ordering::SeqCst)).await;
        self.lt_broadcast_queue().await;
    }

    /// Host: broadcast the upcoming queue (everything after current) to the room. No-op for
    /// non-hosts (`broadcast_playback` gates on role).
    async fn lt_broadcast_queue(&self) {
        if !self.lt.is_host().await {
            return;
        }
        let queue: Vec<Track> = {
            let q = self.queue.lock().await;
            q.items.iter().skip(q.current + 1).take(50).map(song_to_track).collect()
        };
        let mut p = Playback::new(PlaybackKind::SyncQueue);
        p.queue = Some(queue);
        self.lt.broadcast_playback(p).await;
    }

    /// Guest: mirror the host's upcoming queue into the local one (everything after current), so
    /// the up-next panel reflects adds/removes the moment they happen.
    async fn lt_mirror_queue(&self, upcoming: Vec<Track>) {
        {
            let mut q = self.queue.lock().await;
            let keep = q.current + 1;
            q.items.truncate(keep);
            q.items.extend(upcoming.iter().map(track_to_song));
        }
        self.emit_queue().await;
    }
}

/// Current wall-clock in ms (for guest latency compensation).
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn song_to_track(s: &SongItem) -> Track {
    Track {
        id: s.video_id.clone(),
        title: s.title.clone(),
        artist: s.artists.clone(),
        thumbnail: s.thumbnail.clone(),
        duration_ms: parse_duration_ms(s.duration.as_deref()),
        queued_by: s.queued_by.clone(),
    }
}

fn track_to_song(t: &Track) -> SongItem {
    SongItem {
        video_id: t.id.clone(),
        title: t.title.clone(),
        artists: t.artist.clone(),
        artist_id: None,
        artist_runs: Vec::new(),
        album: None,
        album_id: None,
        duration: if t.duration_ms > 0 { Some(format_duration(t.duration_ms)) } else { None },
        play_count: None,
        thumbnail: t.thumbnail.clone(),
        set_video_id: None,
        added_by: None,
        added_by_avatar: None,
        rating: None,
        queued_by: t.queued_by.clone(),
        queued: false,
        queued_end: false,
        queued_from: None,
        autoplay: false,
        is_video: false,
        // The Listen Together wire shape carries no upload flag either; a guest could not stream
        // the host's upload anyway, so false is the right answer, not a lossy one.
        is_upload: false,
        // The Listen Together wire shape carries no badge, so a mirrored guest queue shows none.
        explicit: false,
        // Same: no menu came with a mirrored row, so there is no library token to offer.
        library: None,
    }
}

/// Where an "Add to queue" lands: at the back of the manual block, so it plays after everything
/// already queued by hand and before the playing context, its radio, and autoplay's filler. The
/// tail of a playlist is not where "add to queue" belongs — a radio has no end at all, so a track
/// queued behind one is never heard, and a 50-track playlist buries it just as effectively.
fn enqueue_at(q: &QueueState) -> usize {
    let mut at = (q.current + 1).min(q.items.len());
    while q.items.get(at).map(|i| i.queued || i.queued_end).unwrap_or(false) {
        at += 1;
    }
    at
}

/// Drop every copy of `ids` already in the queue, so a manual add moves the track instead of
/// duplicating it (the "prevent duplicates" setting). The playing track is never dropped, and
/// `current` follows its own track down. Returns whether anything went.
fn drop_duplicates(
    items: &mut Vec<SongItem>,
    current: &mut usize,
    played_from: &mut usize,
    ids: &HashSet<&str>,
) -> bool {
    let mut removed = false;
    for i in (0..items.len()).rev() {
        if i != *current && ids.contains(items[i].video_id.as_str()) {
            items.remove(i);
            if i < *current {
                *current -= 1;
                // A copy taken from in front of the played run shifts the run; one taken from
                // inside it just makes it shorter, which the moved `current` already does.
                if i < *played_from {
                    *played_from -= 1;
                }
            }
            removed = true;
        }
    }
    removed
}

/// Where a "Play next" track goes: right after the current song, behind any earlier "Play next"
/// adds (`queued`, FIFO — includes solo adds, guest suggestions, and host adds), ahead of anything
/// added with "Add to queue" (`queued_end`) and of the upcoming playlist.
fn guest_insert_index(items: &[SongItem], current: usize) -> usize {
    let mut at = (current + 1).min(items.len());
    while items.get(at).map(|i| i.queued).unwrap_or(false) {
        at += 1;
    }
    at
}

/// The autoplay radio seed for a queue source: playlist/album pages pass their playlist id
/// (`VL…` browseId or bare `OLAK5uy_…`/`PL…`) → `RDAMPL<id>` playlist radio. `None` (single
/// song / artist top-songs) → no pinned seed; autoplay seeds `RDAMVM<last video>` at extension
/// time instead.
fn radio_seed_for(source_id: Option<String>) -> Option<String> {
    source_id.map(|id| {
        let id = id.strip_prefix("VL").unwrap_or(&id);
        // A mix id already *is* a radio playlist; wrapping it in another `RDAMPL` asks for a
        // playlist YouTube has never heard of. This is the seed that continues a mix past the page
        // the UI had loaded, since `fill_playlist` deliberately doesn't walk one.
        if id.starts_with("RD") {
            id.to_owned()
        } else {
            format!("RDAMPL{id}")
        }
    })
}

/// Replace everything after the playing track with a radio, keeping the history, the current song
/// and any unplayed manual adds. Deduped against what survives, so the seed can't come round again
/// two tracks later. The tracks aren't marked `autoplay`: this queue is what the user asked for,
/// not filler appended behind one.
fn splice_radio_into(
    q: &mut QueueState,
    items: Vec<SongItem>,
    seed: String,
    title: Option<String>,
) {
    let carried = upcoming_queued(&q.items, q.current);
    q.items.truncate(q.current + 1);
    q.items.extend(carried);
    let mut seen: HashSet<String> = q.items.iter().map(|i| i.video_id.clone()).collect();
    for item in items {
        if seen.insert(item.video_id.clone()) {
            q.items.push(item);
        }
    }
    q.radio_seed = Some(seed);
    q.source_name = title;
    q.radio = true;
    // Shuffle is sticky across queues: re-snapshot the new order as the "original", then shuffle
    // what's upcoming (same handling as a radio hydration in `play_song`).
    if q.shuffle_orig.is_some() {
        q.shuffle_orig = Some(q.items.clone());
        let cur = q.current;
        shuffle_upcoming(&mut q.items, cur);
    }
}

/// How many already-played rows an endlessly-extending queue keeps behind the play pointer.
///
/// Autoplay appends forever and nothing used to drop anything, so a long session grew the item list
/// without bound (measured 336,776 bytes of `queue_json` on a real install). Every append re-emits
/// the whole list as a JavaScript source string and rewrites the whole blob, so the per-track cost
/// rose with how long you had been listening. 200 is well past what the panel's "Previously played"
/// section shows and past any plausible scroll-back.
const KEEP_PLAYED: usize = 200;

/// Drop played rows beyond `keep` from the front, returning how many were removed so the caller can
/// rebase every index that pointed into `items`.
fn trim_played(items: &mut Vec<innertube::SongItem>, current: usize, keep: usize) -> usize {
    let drop = current.saturating_sub(keep);
    if drop == 0 {
        return 0;
    }
    items.drain(..drop);
    drop
}

/// Append radio-continuation tracks to the queue: dedupe against `existing` (the whole current
/// queue + everything appended this hop), cap at `cap`, and mark each as `autoplay` so the UI can
/// show where the chosen queue ends and autoplay begins. Returns how many were appended.
fn merge_radio(
    items: &mut Vec<SongItem>,
    fresh: Vec<SongItem>,
    mut existing: HashSet<String>,
    cap: usize,
) -> usize {
    let mut added = 0;
    for mut item in fresh {
        if added >= cap {
            break;
        }
        if !existing.insert(item.video_id.clone()) {
            continue; // already in the queue (or appended this hop)
        }
        item.autoplay = true;
        items.push(item);
        added += 1;
    }
    added
}

/// What the skip toast blames. "unavailable" is right for a video YouTube would not serve, and
/// wrong for an upload: the file is still there, the session is what failed. Issue #71.
fn skip_reason(e: &ResolveError) -> &'static str {
    match e {
        ResolveError::UploadUnavailable(_) => "sign-in needed",
        _ => "unavailable",
    }
}

/// The queue index that plays after `current`, honoring repeat-all wrap. `None` at the tail
/// when repeat is off (exhausted) or one (mpv loops the file itself — the queue never advances).
fn next_index(len: usize, current: usize, repeat: RepeatMode) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let next = current + 1;
    if next < len {
        Some(next)
    } else if repeat == RepeatMode::All {
        Some(0)
    } else {
        None
    }
}

/// Un-shuffle: restore the snapshot, re-locate the playing track in it, then drop from what's
/// still *upcoming* anything the shuffle already played (`heard`). Restoring verbatim re-queues
/// those: shuffle through a whole album and the tracks that happen to sit after the last shuffled
/// one in album order come back as "up next" the moment shuffle goes off, even though they just
/// played. Heard tracks *behind* the playing one stay — they're the hidden history, and removing
/// them would only move the playing track's index for no visible gain.
/// ponytail: dupes match by videoId — the same song twice in one queue keeps only its first copy
/// past the playing track; harmless (identical items).
fn unshuffled(
    orig: Vec<SongItem>,
    heard: &HashSet<String>,
    playing_id: &str,
    fallback: usize,
) -> (Vec<SongItem>, usize) {
    let idx = orig
        .iter()
        .position(|i| i.video_id == playing_id)
        .unwrap_or_else(|| fallback.min(orig.len().saturating_sub(1)));
    let mut items = orig;
    let mut i = 0;
    items.retain(|it| {
        let keep = i <= idx || !heard.contains(&it.video_id);
        i += 1;
        keep
    });
    // "Play next" adds go back to the boundary rather than wherever the snapshot happens to hold
    // them (they're appended to it as they're made): they're queue state, not playlist order, and
    // un-shuffling must not demote them behind the whole playlist. Order among them is kept.
    let tail = items.split_off((idx + 1).min(items.len()));
    let (queued, rest): (Vec<_>, Vec<_>) =
        tail.into_iter().partition(|it| it.queued || it.queued_end);
    items.extend(queued);
    items.extend(rest);
    (items, idx)
}

/// The unplayed manual adds (after `current`, either marker) — carried into a new queue on a
/// context switch so what the user lined up survives them playing something else.
fn upcoming_queued(items: &[SongItem], current: usize) -> Vec<SongItem> {
    items.iter().skip(current + 1).filter(|i| i.queued || i.queued_end).cloned().collect()
}

/// Fisher–Yates over the *upcoming* items only — the playing track and the already-played prefix
/// stay put, and the leading run of manually-queued tracks keeps its spot: "Next in queue" plays
/// next regardless of shuffle (Spotify semantics). Autoplay tracks stay *behind* the remaining
/// queue tracks (each section shuffled within itself) — shuffle never promotes radio filler ahead
/// of the playlist.
fn shuffle_upcoming(items: &mut [SongItem], current: usize) {
    use rand::seq::SliceRandom;
    let mut start = current + 1;
    while items.get(start).map(|i| i.queued || i.queued_end).unwrap_or(false) {
        start += 1;
    }
    // Inside that pinned block: a whole album/playlist put there is a set of tracks like any other,
    // so shuffle each such run in place (un-shuffle restores the real order from the snapshot).
    // Songs queued one at a time carry no `queued_from` and keep their FIFO order — "play this
    // next, then that" is an order the user stated, not one to randomize.
    let mut i = current + 1;
    while i < start {
        let Some(from) = items[i].queued_from.clone() else {
            i += 1;
            continue;
        };
        let mut end = i + 1;
        while end < start && items[end].queued_from.as_deref() == Some(from.as_str()) {
            end += 1;
        }
        items[i..end].shuffle(&mut rand::thread_rng());
        i = end;
    }
    if start < items.len() {
        items[start..].shuffle(&mut rand::thread_rng());
        items[start..].sort_by_key(|i| i.autoplay); // stable: both sections stay shuffled
    }
}

/// Append a background-walked playlist page ([`AppState::fill_playlist`]) to a queue that's
/// already playing. The page goes onto `shuffle_orig` in its true order too, so un-shuffle still
/// restores the *whole* playlist and not just the part that had loaded when playback started.
///
/// With shuffle on and `playing`, the unplayed tail is re-shuffled so the new page is mixed through
/// it instead of sitting at the end. The pivot is the primed gapless slot when there is one: that
/// track is already loaded into mpv, so moving it would desync what mpv plays next from what the
/// queue says is next. It was drawn from the same random tail anyway. A page walked in for
/// "Add to queue" passes `playing: false` — those tracks join the manual block their first page is
/// in (not the tail of the queue) and keep their own order, since shuffle is about the playlist
/// that's playing, not about what the user lined up behind it.
fn append_page(q: &mut QueueState, page: Vec<SongItem>, playing: bool) {
    if let Some(orig) = q.shuffle_orig.as_mut() {
        orig.extend(page.iter().cloned());
    }
    if playing {
        q.items.extend(page);
        if q.shuffle_orig.is_some() {
            let pivot = q.lookahead_loaded.filter(|&i| i > q.current).unwrap_or(q.current);
            shuffle_upcoming(&mut q.items, pivot);
        }
    } else {
        let at = enqueue_at(q);
        q.items.splice(at..at, page);
    }
}

/// A mix (`RD…`, reached as the `VLRD…` browseId) is a generated, effectively endless feed rather
/// than a playlist with an end, and `extend_queue_radio` already tops the queue up from its tail.
/// Walking its continuations would fetch page after page for no gain.
fn is_mix(source_id: Option<&str>) -> bool {
    source_id.is_some_and(|id| id.strip_prefix("VL").unwrap_or(id).starts_with("RD"))
}

/// Shuffle a *fresh* queue around the clicked track (shuffle was already on when it started,
/// Spotify semantics): the clicked track plays first, everything else follows in random order.
/// Returns the new current index (always 0). The swap is fine — everything past 0 is shuffled.
fn shuffle_new_queue(items: &mut [SongItem], start: usize) -> usize {
    if !items.is_empty() {
        items.swap(0, start.min(items.len() - 1));
        shuffle_upcoming(items, 0);
    }
    0
}

/// Parse a `"m:ss"` / `"h:mm:ss"` duration string to ms (0 if absent/unparseable).
fn parse_duration_ms(s: Option<&str>) -> i64 {
    let Some(s) = s else { return 0 };
    let parts: Vec<i64> = s.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    let secs = match parts.as_slice() {
        [s] => *s,
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => 0,
    };
    secs * 1000
}

fn format_duration(ms: i64) -> String {
    let total = ms / 1000;
    match total / 3600 {
        0 => format!("{}:{:02}", total / 60, total % 60),
        h => format!("{}:{:02}:{:02}", h, (total % 3600) / 60, total % 60),
    }
}

/// Fill a missing item duration from the player response's `lengthSeconds` (e.g. "167") — the
/// exact length of the cut we stream. Never overwrites an existing duration.
/// Repair a queue item from the `/player` response, which is the one thing every entry path (search,
/// album, card, radio, a restored queue, a row replayed out of On Repeat) goes through.
///
/// `length_seconds` fills a duration items from cards/radio arrive without. `author` is
/// `videoDetails.author` from the MAIN client, i.e. YouTube's own artist for the track: it repairs
/// an artist string that is missing (album rows ship the artist column empty) or that is a whole
/// display subtitle rather than a name ("Miley Cyrus • Plastic Hearts • 2020"). A "•" never appears
/// in a real artist line, collabs use "&" and ",". Both shapes reach the player bar, the OS widget
/// and Last.fm, and a wrong artist there is worse than a missing one: it scrobbles as another
/// artist entirely. Rows persisted before this existed are healed the next time they play, because
/// the caller writes the repaired item back into the queue.
fn backfill_metadata(
    item: &mut SongItem,
    length_seconds: Option<&str>,
    author: Option<&str>,
    is_video: Option<bool>,
) {
    if item.duration.is_none() {
        if let Some(secs) = length_seconds.and_then(|s| s.trim().parse::<i64>().ok()) {
            item.duration = Some(format_duration(secs * 1000));
        }
    }
    // YouTube's own `musicVideoType` outranks whatever the row was parsed with: a card played
    // from a shelf never carried the flag, an album row keeps it after being swapped to its audio
    // id, and a mirrored or restored row has no flag at all. Overwrite, don't OR: a stale `true`
    // on an audio track would put the still album-art "video" on screen (plan 031).
    if let Some(is_video) = is_video {
        item.is_video = is_video;
    }
    if let Some(author) = author.map(str::trim).filter(|a| !a.is_empty()) {
        if item.artists.trim().is_empty() || item.artists.contains('•') {
            item.artists = author.to_owned();
            // The per-artist links belong to the string we just replaced; the UI renders them in
            // place of `artists` when they're there, so a stale set would put the bad line back.
            item.artist_runs.clear();
        }
    }
}

/// The level to come up at: what the user left the slider on last run. Written by the UI on
/// commit rather than by `set_volume`, which a drag calls every frame (and every settings write
/// is an fsync). mpv would start at 100 otherwise.
pub fn saved_volume(db: &Db) -> i64 {
    let v = db.get_setting("volume").and_then(|s| s.parse().ok());
    v.filter(|v| (0..=100).contains(v)).unwrap_or(100)
}

/// How far into a track a play counts (context/01 §registerPlayback): halfway, capped at 30s.
/// `duration` is mpv's, which is 0.0 until it reports one, so an unknown length means the full 30s.
fn history_threshold(duration: f64) -> f64 {
    if duration > 1.0 {
        (duration / 2.0).min(30.0)
    } else {
        30.0
    }
}

/// Per-track loudness gain (dB) from YouTube's `loudnessDb` (context/03, context/14). Attenuate
/// only toward reference loudness: loud masters get pulled down, quieter tracks aren't boosted,
/// so there's no clipping and no limiter to add.
///
/// `loudnessDb` is measured against **-14 LUFS** (YouTube's own response proves it:
/// `perceptualLoudnessDb == loudnessDb - 14`, i.e. the track's absolute LUFS). Applying it raw
/// normalizes to -14 like the *video* site, but YouTube **Music** only pulls down what exceeds
/// **-7 LUFS**, so raw made us ~6 dB quieter than YTM web on a typical modern master and left
/// most tracks attenuated that YTM never touches at all. Normalize to the same -7 target.
///
/// `None` means "no filter", and that's most tracks now: below the target there is nothing to
/// attenuate, so mpv gets its `af` cleared rather than a `volume=0dB` no-op in the chain.
// ponytail: attenuate-only, clamped to -24 dB. If quiet tracks feel too soft, allow positive gain
// plus an `alimiter` af to catch the resulting peaks.
fn loudness_gain(loudness_db: Option<f64>) -> Option<f64> {
    let gain = TARGET_LUFS - (loudness_db? - 14.0);
    (gain < -0.05).then(|| gain.max(-24.0))
}

/// Loudness target, matching YouTube Music's own player rather than the video site's -14.
const TARGET_LUFS: f64 = -7.0;

/// Fingerprint of a queue's track list: what `emit_queue` compares to decide whether the UI
/// already has these rows. Hashes length, `video_id`, and every field the panel renders
/// differently per row (`set_video_id`, `queued`, `queued_end`, `queued_from`, `queued_by`,
/// `autoplay`): two rows identical in all of those are interchangeable in the panel, so staying
/// quiet about a swap between them is correct. Deliberately excludes `duration`, `artists`,
/// `title`, `thumbnail` and `rating`: those are exactly the fields `backfill_metadata` repairs on
/// the playing row, and that repair rides along on the `queue-index` event's `current` field on
/// purpose. Hashing them would send the whole queue on every track start and undo the point of
/// that split.
fn queue_fingerprint(items: &[SongItem]) -> u64 {
    let mut hasher = DefaultHasher::new();
    items.len().hash(&mut hasher);
    for item in items {
        item.video_id.hash(&mut hasher);
        item.set_video_id.hash(&mut hasher);
        item.queued.hash(&mut hasher);
        item.queued_end.hash(&mut hasher);
        item.queued_from.hash(&mut hasher);
        item.queued_by.hash(&mut hasher);
        item.autoplay.hash(&mut hasher);
    }
    hasher.finish()
}

/// Dirty key for `queue_json`, which stores more than the rows. `queue_index` carries
/// current/playedFrom/repeat, so anything else the blob holds has to force a blob rewrite by
/// itself. `shuffle_upcoming` only touches rows after `current`, so toggling shuffle while the
/// last track plays leaves `items` byte-identical and the row fingerprint alone would skip the
/// write, losing the shuffle state across a restart.
fn persist_fingerprint(q: &QueueState) -> u64 {
    let mut hasher = DefaultHasher::new();
    queue_fingerprint(&q.items).hash(&mut hasher);
    q.shuffle_orig.is_some().hash(&mut hasher);
    q.radio_seed.hash(&mut hasher);
    q.source_name.hash(&mut hasher);
    q.radio.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::{
        append_page, backfill_metadata, drop_duplicates, enqueue_at, format_duration, get_url,
        guest_insert_index, history_threshold, is_mix, loudness_gain, merge_radio, next_index,
        parse_duration_ms, persist_fingerprint, put_url, queue_fingerprint, radio_seed_for,
        shuffle_new_queue, shuffle_upcoming, splice_radio_into, trim_played, unshuffled,
        upcoming_queued, QueueState, RepeatMode, VideoUrls, KEEP_PLAYED,
    };

    /// The whole point of the video-URL map is answering a reopen without a round trip, so a live
    /// entry has to hit and a stale one has to miss. Getting the second one wrong hands the proxy a
    /// dead googlevideo link for hours.
    #[test]
    fn a_cached_video_url_lives_then_expires() {
        let mut map = VideoUrls::new();
        let t0 = 1_700_000_000;
        put_url(&mut map, "abc", "https://example.invalid/v".into(), t0);
        assert_eq!(get_url(&map, "abc", t0 + 60).as_deref(), Some("https://example.invalid/v"));
        assert_eq!(get_url(&map, "abc", t0 + 5 * 60 * 60), None, "past the TTL, must miss");
        assert_eq!(get_url(&map, "nope", t0).as_deref(), None);
    }

    /// The play threshold decides whether a play ever reaches YouTube's history or On Repeat, and
    /// the two ends are the ones that bite: a track whose duration mpv hasn't reported yet must not
    /// count instantly, and a long track must not need half an hour.
    #[test]
    fn a_play_counts_halfway_in_or_at_thirty_seconds() {
        assert_eq!(history_threshold(0.0), 30.0, "unknown duration falls back to the full 30s");
        assert_eq!(history_threshold(40.0), 20.0, "short track: halfway");
        assert_eq!(history_threshold(600.0), 30.0, "long track: capped, not five minutes");
    }

    /// The cap is a clear, not an eviction, so the ninth insert leaves exactly one entry.
    #[test]
    fn the_video_url_map_clears_at_its_cap() {
        let mut map = VideoUrls::new();
        for i in 0..9 {
            put_url(&mut map, &format!("id{i}"), "u".into(), 0);
        }
        assert_eq!(map.len(), 1);
        assert!(get_url(&map, "id8", 0).is_some(), "the newest insert survives the clear");
    }

    #[test]
    fn queue_fingerprint_tracks_the_rows_not_their_metadata() {
        let row = |id: &str| innertube::SongItem { video_id: id.into(), ..Default::default() };
        let a = vec![row("a"), row("b"), row("c")];
        // Same rows in the same order: the UI already has them.
        assert_eq!(queue_fingerprint(&a), queue_fingerprint(&[row("a"), row("b"), row("c")]));
        // A reorder, an append and a removal all have to be visible.
        assert_ne!(queue_fingerprint(&a), queue_fingerprint(&[row("a"), row("c"), row("b")]));
        assert_ne!(queue_fingerprint(&a), queue_fingerprint(&[row("a"), row("b")]));
        assert_ne!(
            queue_fingerprint(&a),
            queue_fingerprint(&[row("a"), row("b"), row("c"), row("d")])
        );
        // A metadata repair on an existing row does not: it rides on the index event instead.
        let mut patched = a.clone();
        patched[0].duration = Some("3:11".into());
        assert_eq!(queue_fingerprint(&a), queue_fingerprint(&patched));
        // `restore_queue` compares these as strings off disk, not as numbers, so the string form
        // has to discriminate as well as the hash does: a stamp that matched everything would
        // silently re-open the wrong-track gap the stamp exists to close.
        assert_ne!(
            queue_fingerprint(&a).to_string(),
            queue_fingerprint(&[row("a"), row("b")]).to_string()
        );
    }

    // `move_in_queue` can drag one occurrence of a repeated videoId past the other (a track queued
    // twice from two different playlists, say). The panel tells the two rows apart by the fields
    // that differ between them, not by videoId alone, so the fingerprint has to as well: otherwise
    // the swap is invisible, `emit_queue` stays on the small `queue-index` event, and the drag
    // looks like it did nothing.
    #[test]
    fn queue_fingerprint_distinguishes_same_video_id_rows_by_queue_metadata() {
        let row = |queued_from: Option<&str>| innertube::SongItem {
            video_id: "x".into(),
            queued_from: queued_from.map(Into::into),
            ..Default::default()
        };
        let a = vec![row(Some("Chill Mix")), row(None)];
        let b = vec![row(None), row(Some("Chill Mix"))];
        assert_ne!(queue_fingerprint(&a), queue_fingerprint(&b));

        // Same story for `set_video_id`, the other field that tells apart two occurrences of the
        // same videoId (one came from a playlist page, so it carries a playlistSetVideoId; a
        // second copy queued manually doesn't).
        let row2 = |set_video_id: Option<&str>| innertube::SongItem {
            video_id: "x".into(),
            set_video_id: set_video_id.map(Into::into),
            ..Default::default()
        };
        let c = vec![row2(Some("1")), row2(Some("2"))];
        let d = vec![row2(Some("2")), row2(Some("1"))];
        assert_ne!(queue_fingerprint(&c), queue_fingerprint(&d));
    }

    // `shuffle_upcoming` only reorders rows after `current`, so toggling shuffle while the last
    // track is playing leaves `items` byte-identical: the rows-only `queue_fingerprint` can't see
    // it, and `persist_queue`'s dirty check has to cover the rest of what `queue_json` stores or
    // the toggle is lost across a restart.
    #[test]
    fn persist_fingerprint_changes_when_shuffle_toggles_with_identical_items() {
        let mut q = QueueState {
            items: vec![innertube::SongItem { video_id: "a".into(), ..Default::default() }],
            ..QueueState::default()
        };
        let off = persist_fingerprint(&q);
        q.shuffle_orig = Some(q.items.clone());
        let on = persist_fingerprint(&q);
        assert_ne!(off, on);
    }

    #[test]
    fn durations_round_trip_past_an_hour() {
        assert_eq!(format_duration(191_000), "3:11");
        assert_eq!(format_duration(5_468_000), "1:31:08");
        // The parser is the inverse for both shapes (queue rows persist the string).
        assert_eq!(parse_duration_ms(Some("1:31:08")), 5_468_000);
        assert_eq!(parse_duration_ms(Some("3:11")), 191_000);
    }
    use std::collections::HashSet;

    // `by.is_some()` (a named guest/host add) and a nameless solo add are both manual adds:
    // the `queued` marker is what forms the FIFO block, not the name.
    fn song(id: &str, by: Option<&str>) -> innertube::SongItem {
        innertube::SongItem {
            video_id: id.into(),
            title: id.into(),
            queued: by.is_some(),
            queued_by: by.map(Into::into),
            ..Default::default()
        }
    }

    /// The repair every entry path goes through. Covers the two shapes that reached Last.fm as an
    /// artist name: nothing at all (album rows), and a whole display subtitle (song cards, and rows
    /// replayed out of On Repeat that were recorded back when the card parser leaked one).
    #[test]
    fn player_response_repairs_a_missing_or_bogus_artist() {
        let with = |artists: &str, runs: Vec<&str>| innertube::SongItem {
            artists: artists.into(),
            artist_runs: runs
                .into_iter()
                .map(|t| innertube::models::metadata::ArtistRun {
                    text: t.into(),
                    id: Some("UCstale".into()),
                })
                .collect(),
            ..song("v", None)
        };

        // No artist (an album track) → YouTube's own author.
        let mut it = with("", vec![]);
        backfill_metadata(&mut it, None, Some("Delara"), None);
        assert_eq!(it.artists, "Delara");

        // A display subtitle that was parsed as the artist → replaced, and the links that pointed
        // at the old text go with it (the UI renders runs instead of `artists` when they exist).
        for bogus in [
            "Miley Cyrus • Plastic Hearts • 2020",
            "late night slow • 29M views",
            "Song • Dua Lipa",
        ] {
            let mut it = with(bogus, vec![bogus]);
            backfill_metadata(&mut it, None, Some("Dua Lipa"), None);
            assert_eq!(it.artists, "Dua Lipa", "{bogus} should have been replaced");
            assert!(it.artist_runs.is_empty(), "{bogus}: stale links must not survive the swap");
        }

        // A real artist line is never second-guessed, links included. Collabs use "&" and ",".
        let mut it = with("Nicki Minaj, Ice Spice & Aqua", vec!["Nicki Minaj", " & ", "Aqua"]);
        backfill_metadata(&mut it, None, Some("Nicki Minaj"), None);
        assert_eq!(it.artists, "Nicki Minaj, Ice Spice & Aqua");
        assert_eq!(it.artist_runs.len(), 3);

        // Nothing to repair with (rustypipe served the stream, or /player had no author) → as-is.
        for author in [None, Some(""), Some("   ")] {
            let mut it = with("", vec![]);
            backfill_metadata(&mut it, None, author, None);
            assert_eq!(it.artists, "");
        }
    }

    #[test]
    fn player_response_fills_only_a_missing_duration() {
        let mut it = song("v", None);
        backfill_metadata(&mut it, Some("191"), None, None);
        assert_eq!(it.duration.as_deref(), Some("3:11"));
        // A duration the row already carried wins: it's the length of the cut YouTube listed.
        let mut it = innertube::SongItem { duration: Some("3:02".into()), ..song("v", None) };
        backfill_metadata(&mut it, Some("191"), None, None);
        assert_eq!(it.duration.as_deref(), Some("3:02"));
        // Junk from the player response can't blank an existing one.
        let mut it = song("v", None);
        backfill_metadata(&mut it, Some("not-a-number"), None, None);
        assert_eq!(it.duration, None);
    }

    /// The player view's music-video mode reads this flag, and the rows that reach it are only as
    /// good as the parse that built them: a card played from a shelf carries no flag at all, and an
    /// album row keeps a `true` after being swapped to its audio id. `/player` settles both ways.
    #[test]
    fn player_response_settles_whether_the_track_is_a_video() {
        let mut it = song("v", None);
        backfill_metadata(&mut it, None, None, Some(true));
        assert!(it.is_video);
        // A stale `true` on what YouTube calls an audio track is cleared, not kept: video mode on
        // an art track shows a still image, which is worse than the artwork it replaced.
        backfill_metadata(&mut it, None, None, Some(false));
        assert!(!it.is_video);
        // Nothing said (rustypipe, or a cached URL) → whatever the row already claimed stands.
        it.is_video = true;
        backfill_metadata(&mut it, None, None, None);
        assert!(it.is_video);
    }

    /// The trim is what keeps an endless radio session from growing the queue (and every emit and
    /// every persisted blob with it) forever. Indices are what it can break, so assert on video ids.
    #[test]
    fn trim_played_keeps_the_history_bounded() {
        let mut items: Vec<_> = (0..300).map(|i| song(&format!("s{i}"), None)).collect();

        // Nothing to drop while the played run is still within the cap.
        let mut short = items[..150].to_vec();
        assert_eq!(trim_played(&mut short, 150, KEEP_PLAYED), 0);
        assert_eq!(short.len(), 150);
        assert_eq!(trim_played(&mut Vec::new(), 0, KEEP_PLAYED), 0);

        // Past the cap: exactly the excess goes, and the playing row is still the playing row.
        let current = 250;
        let playing = items[current].video_id.clone();
        let dropped = trim_played(&mut items, current, KEEP_PLAYED);
        assert_eq!(dropped, 50);
        assert_eq!(items.len(), 250);
        assert_eq!(items[current - dropped].video_id, playing);

        // An endless radio: append a batch, advance through it, trim. Bounded, not 600-and-rising.
        let mut items = vec![song("seed", None)];
        let mut current = 0;
        for hop in 0..30 {
            items.extend((0..20).map(|i| song(&format!("h{hop}-{i}"), None)));
            current += 20;
            let playing = items[current].video_id.clone();
            current -= trim_played(&mut items, current, KEEP_PLAYED);
            assert_eq!(items[current].video_id, playing);
            assert!(items.len() <= KEEP_PLAYED + 21, "grew to {}", items.len());
        }
    }

    #[test]
    fn guest_adds_stack_fifo_after_current() {
        let solo = |id: &str| innertube::SongItem { queued: true, ..song(id, None) };
        // Host playlist [A*, B, C] (playing A): manual add goes right after current, not the end.
        let items = vec![song("a", None), song("b", None), song("c", None)];
        assert_eq!(guest_insert_index(&items, 0), 1);
        // A guest track already up next → the new one queues behind it (FIFO), before B.
        let items = vec![song("a", None), song("g1", Some("kim")), song("b", None)];
        assert_eq!(guest_insert_index(&items, 0), 2);
        // Solo adds (no name, `queued`) form the same FIFO block: the next add lands behind them.
        let items = vec![song("a", None), solo("s1"), song("b", None)];
        assert_eq!(guest_insert_index(&items, 0), 2);
        // Current is the last item → append.
        let items = vec![song("a", None)];
        assert_eq!(guest_insert_index(&items, 0), 1);
        // Empty queue (nothing playing yet) → index 0… clamped, no panic.
        assert_eq!(guest_insert_index(&[], 0), 0);
    }

    // "Start radio" on the song that's already playing: the track keeps playing, the tail is
    // replaced, and the manual adds the user made are not collateral damage (Metrolist drops them).
    #[test]
    fn radio_replaces_the_tail_but_keeps_history_and_manual_adds() {
        let ids = |items: &[innertube::SongItem]| {
            items.iter().map(|i| i.video_id.clone()).collect::<Vec<_>>()
        };
        let mut q = QueueState {
            // played, playing, a manual add, then two tracks of the old context
            items: vec![
                song("done", None),
                song("now", None),
                song("mine", Some("me")),
                song("old1", None),
                song("old2", None),
            ],
            current: 1,
            ..QueueState::default()
        };
        // The radio's first page leads with the seed song — it must not be queued a second time.
        // Nor may it re-offer what's still in the queue (history, or the user's own adds). A track
        // from the replaced tail (`old1`) is fair game: it isn't in the queue any more.
        let page = vec![
            song("now", None),
            song("r1", None),
            song("done", None),
            song("mine", None),
            song("old1", None),
        ];
        splice_radio_into(&mut q, page, "RDAMVMnow".into(), Some("Now Radio".into()));

        assert_eq!(ids(&q.items), ["done", "now", "mine", "r1", "old1"]);
        assert_eq!(q.current, 1); // nothing moved under the playing track
        assert_eq!(q.radio_seed.as_deref(), Some("RDAMVMnow"));
        assert_eq!(q.source_name.as_deref(), Some("Now Radio"));
        // Radio tracks are the queue, not autoplay filler — no "Autoplay" divider under them.
        assert!(q.items.iter().all(|i| !i.autoplay));
    }

    #[test]
    fn radio_started_under_shuffle_stays_shuffled_and_restorable() {
        let mut items = vec![song("now", None)];
        let mut q = QueueState {
            shuffle_orig: Some(items.clone()),
            items: std::mem::take(&mut items),
            current: 0,
            ..QueueState::default()
        };
        let page: Vec<_> = (0..50).map(|i| song(&format!("r{i}"), None)).collect();
        splice_radio_into(&mut q, page, "RDAMVMnow".into(), None);

        assert_eq!(q.items[0].video_id, "now"); // the playing track never moves
                                                // Un-shuffle has the whole radio to restore, not just what was there before it started.
        let orig = q.shuffle_orig.as_ref().unwrap();
        assert_eq!(orig.len(), 51);
        let mut a: Vec<_> = orig.iter().map(|i| i.video_id.clone()).collect();
        let mut b: Vec<_> = q.items.iter().map(|i| i.video_id.clone()).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b); // nothing lost, nothing duplicated
    }

    #[test]
    fn next_index_wraps_only_on_repeat_all() {
        assert_eq!(next_index(3, 2, RepeatMode::Off), None);
        assert_eq!(next_index(3, 2, RepeatMode::All), Some(0));
        assert_eq!(next_index(3, 2, RepeatMode::One), None); // mpv loops the file itself
        assert_eq!(next_index(3, 0, RepeatMode::Off), Some(1));
        assert_eq!(next_index(0, 0, RepeatMode::All), None);
        assert_eq!(next_index(1, 0, RepeatMode::All), Some(0)); // single-item wrap
    }

    // The background playlist walk: a page that lands mid-playback has to reach the *unplayed*
    // tail (otherwise "shuffle" only ever shuffles page 1), without moving the track already
    // handed to mpv for the gapless advance.
    #[test]
    fn appended_page_mixes_into_the_tail_but_leaves_the_primed_slot() {
        let ids = |items: &[innertube::SongItem]| {
            items.iter().map(|i| i.video_id.clone()).collect::<Vec<_>>()
        };
        let page = || (0..100).map(|i| song(&format!("p{i}"), None)).collect::<Vec<_>>();
        let mut tail_positions = Vec::new();

        for _ in 0..20 {
            let items = vec![song("a", None), song("b", None), song("c", None)];
            let mut q = QueueState {
                shuffle_orig: Some(items.clone()),
                items,
                current: 0,
                lookahead_loaded: Some(1), // "b" is already loaded into mpv
                ..QueueState::default()
            };
            append_page(&mut q, page(), true);

            assert_eq!(q.items.len(), 103);
            assert_eq!(q.items[0].video_id, "a"); // playing
            assert_eq!(q.items[1].video_id, "b"); // primed gapless slot, pinned
                                                  // Un-shuffle restores the whole playlist, including pages that arrived after playback
                                                  // started.
            assert_eq!(ids(q.shuffle_orig.as_ref().unwrap()).len(), 103);
            assert_eq!(ids(q.shuffle_orig.as_ref().unwrap())[..3], ["a", "b", "c"]);
            let mut sorted = ids(&q.items);
            sorted.sort();
            let mut expected = ids(q.shuffle_orig.as_ref().unwrap());
            expected.sort();
            assert_eq!(sorted, expected); // nothing lost, nothing duplicated

            tail_positions.push(q.items.iter().position(|i| i.video_id == "c").unwrap());
        }
        // Without the re-shuffle the page would sit behind "c" and it would land at index 2 every
        // time. (Staying at 2 in all 20 runs by chance is (1/101)^20.)
        assert!(tail_positions.iter().any(|&p| p != 2), "page was appended, not mixed in");
    }

    #[test]
    fn appended_page_keeps_order_when_shuffle_is_off() {
        let mut q = QueueState {
            items: vec![song("a", None), song("b", None)],
            current: 0,
            ..QueueState::default()
        };
        append_page(&mut q, vec![song("c", None), song("d", None)], true);
        let ids: Vec<_> = q.items.iter().map(|i| i.video_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c", "d"]);
        assert!(q.shuffle_orig.is_none());
    }

    // The rest of an added playlist joins the block its first page went into — not the tail of the
    // queue, which is a different place entirely now that "Add to queue" inserts up front. Order
    // kept even under shuffle: the user queued that album, shuffle belongs to what's playing.
    #[test]
    fn appended_page_joins_the_add_to_queue_block_it_belongs_to() {
        let added = |id: &str| innertube::SongItem { queued_end: true, ..song(id, None) };
        let items = vec![song("a", None), added("x1"), song("b", None)];
        let mut q = QueueState {
            shuffle_orig: Some(items.clone()),
            items,
            current: 0,
            ..QueueState::default()
        };
        append_page(&mut q, vec![added("x2"), added("x3")], false);
        let ids: Vec<_> = q.items.iter().map(|i| i.video_id.as_str()).collect();
        assert_eq!(ids, ["a", "x1", "x2", "x3", "b"]);
        // Still on the snapshot, so un-shuffle keeps them.
        assert_eq!(q.shuffle_orig.as_ref().unwrap().len(), 5);
    }

    // The played run ("Previously played") is `played_from..current`, and only moving the pointer
    // backwards may extend it: forward jumps mean those tracks really were passed over.
    #[test]
    fn the_played_run_follows_the_pointer_backwards_only() {
        let mut q = QueueState {
            items: vec![song("a", None), song("b", None), song("c", None), song("d", None)],
            current: 1,
            played_from: 1,
            ..QueueState::default()
        };
        q.seek_to(3); // jumped over "c", so it counts: heard or not, it's behind the playing track
        assert_eq!((q.played_from, q.current), (1, 3));
        q.seek_to(0); // back to the top: nothing is behind it any more
        assert_eq!((q.played_from, q.current), (0, 0));
    }

    // The one that silently breaks: a duplicate sitting *before* the playing track. Removing it
    // renumbers the queue, so `current` has to follow its own song or the insert lands one slot off.
    #[test]
    fn dropping_a_duplicate_before_the_current_track_moves_the_current_index() {
        let mut items =
            vec![song("dup", None), song("a", None), song("b", None), song("dup", None)];
        let mut current = 1; // playing "a"
        let mut played_from = 0; // "dup" was heard, then "a" started
        let ids = HashSet::from(["dup"]);
        assert!(drop_duplicates(&mut items, &mut current, &mut played_from, &ids));

        let left: Vec<_> = items.iter().map(|i| i.video_id.as_str()).collect();
        assert_eq!(left, ["a", "b"]);
        assert_eq!(current, 0); // still playing "a"
        assert_eq!(played_from, 0); // its one played track went with it; the run is empty, not stale
        assert_eq!(guest_insert_index(&items, current), 1); // the add lands right after it

        // The playing track is exempt: "play next" on the current song is a repeat gesture.
        let mut items = vec![song("a", None), song("b", None)];
        let mut current = 0;
        let mut played_from = 0;
        assert!(!drop_duplicates(
            &mut items,
            &mut current,
            &mut played_from,
            &HashSet::from(["a"])
        ));
        assert_eq!(items.len(), 2);
    }

    // "Add to queue" lands at the back of the manual block, ahead of the context — the bug in #26
    // was it landing at the very end, where a radio or a long playlist buries it forever.
    #[test]
    fn add_to_queue_goes_behind_the_manual_block_but_ahead_of_the_context() {
        let queued = |id: &str| innertube::SongItem { queued: true, ..song(id, None) };
        let added = |id: &str| innertube::SongItem { queued_end: true, ..song(id, None) };

        // Plain playlist queue → straight behind the playing track.
        let q = QueueState {
            items: vec![song("a", None), song("b", None), song("c", None)],
            current: 0,
            ..QueueState::default()
        };
        assert_eq!(enqueue_at(&q), 1);

        // Behind a waiting "Play next" block and behind earlier adds, never inside either.
        let q = QueueState {
            items: vec![song("a", None), queued("mine"), added("x1"), song("b", None)],
            current: 0,
            ..QueueState::default()
        };
        assert_eq!(enqueue_at(&q), 3);

        // A radio is no different: the add is heard next instead of after an endless feed.
        let q = QueueState {
            items: vec![song("r1", None), song("r2", None), song("r3", None)],
            current: 0,
            radio: true,
            ..QueueState::default()
        };
        assert_eq!(enqueue_at(&q), 1);

        // Nothing after the playing track: appended, not out of bounds.
        let q = QueueState { items: vec![song("a", None)], current: 0, ..QueueState::default() };
        assert_eq!(enqueue_at(&q), 1);
    }

    #[test]
    fn mixes_are_not_walked() {
        assert!(is_mix(Some("VLRDCLAK5uy_abc"))); // a mix as the playlist page sees it
        assert!(is_mix(Some("RDAMVMxyz")));
        assert!(!is_mix(Some("VLPL123")));
        assert!(!is_mix(Some("VLOLAK5uy_abc")));
        assert!(!is_mix(None));
    }

    #[test]
    fn unshuffle_restores_order_and_current() {
        let orig = vec![song("a", None), song("b", None), song("c", None), song("d", None)];
        let heard = |ids: &[&str]| ids.iter().map(|s| (*s).to_owned()).collect::<HashSet<String>>();
        let ids = |items: &[innertube::SongItem]| {
            items.iter().map(|i| i.video_id.clone()).collect::<Vec<_>>()
        };
        // Shuffle played "c" first → nothing upcoming has been heard, plain restore.
        let (items, idx) = unshuffled(orig.clone(), &heard(&["c"]), "c", 9);
        assert_eq!(ids(&items), ["a", "b", "c", "d"]);
        assert_eq!(idx, 2);
        // A full shuffled pass ending on "c": "d" already played, so it must not come back as
        // up-next; "a"/"b" stay behind the playing track as hidden history.
        let (items, idx) = unshuffled(orig.clone(), &heard(&["a", "b", "c", "d"]), "c", 9);
        assert_eq!(ids(&items), ["a", "b", "c"]);
        assert_eq!(idx, 2);
        // Partial pass: only the heard ones are dropped from upcoming, order otherwise intact.
        let (items, idx) = unshuffled(orig.clone(), &heard(&["a", "c"]), "a", 9);
        assert_eq!(ids(&items), ["a", "b", "d"]);
        assert_eq!(idx, 0);
        // Playing id absent from the snapshot → fallback index, clamped to len-1.
        let (_, idx) = unshuffled(orig, &HashSet::new(), "zz", 9);
        assert_eq!(idx, 3);

        // "Play next" adds are appended to the snapshot as they're made, so a plain restore would
        // drop them behind the whole playlist. They belong right after the playing track.
        let with_adds = vec![
            song("a", None),
            song("b", None),
            song("c", None),
            innertube::SongItem { queued: true, ..song("mine1", None) },
            innertube::SongItem { queued: true, ..song("mine2", None) },
        ];
        let (items, idx) = unshuffled(with_adds, &heard(&["a"]), "a", 9);
        assert_eq!(ids(&items), ["a", "mine1", "mine2", "b", "c"]);
        assert_eq!(idx, 0);
    }

    #[test]
    fn shuffle_preserves_prefix_and_multiset() {
        let ids: Vec<String> = (0..10).map(|i| format!("t{i}")).collect();
        let mut items: Vec<_> = ids.iter().map(|id| song(id, None)).collect();
        shuffle_upcoming(&mut items, 2);
        // The playing track and the already-played prefix stay put…
        for (i, id) in ids.iter().take(3).enumerate() {
            assert_eq!(&items[i].video_id, id);
        }
        // …and the whole queue is a permutation (nothing lost, nothing duplicated).
        let mut got: Vec<_> = items.iter().map(|i| i.video_id.clone()).collect();
        got.sort();
        let mut want = ids.clone();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn new_queue_shuffle_plays_clicked_track_first() {
        let ids: Vec<String> = (0..10).map(|i| format!("t{i}")).collect();
        let mut items: Vec<_> = ids.iter().map(|id| song(id, None)).collect();
        let cur = shuffle_new_queue(&mut items, 7);
        assert_eq!(cur, 0);
        assert_eq!(items[0].video_id, "t7"); // the clicked track plays first…
        let mut got: Vec<_> = items.iter().map(|i| i.video_id.clone()).collect();
        got.sort();
        let mut want = ids.clone();
        want.sort();
        assert_eq!(got, want); // …and the rest is a permutation of everything else
                               // Degenerate cases: empty and single-item queues don't panic.
        let mut empty: Vec<innertube::SongItem> = vec![];
        assert_eq!(shuffle_new_queue(&mut empty, 3), 0);
        let mut one = vec![song("only", None)];
        assert_eq!(shuffle_new_queue(&mut one, 5), 0);
        assert_eq!(one[0].video_id, "only");
    }

    #[test]
    fn manual_adds_survive_context_switch() {
        let solo = |id: &str| innertube::SongItem { queued: true, ..song(id, None) };
        // Playing B (index 1); Q1/Q2 are unplayed manual adds, Q0 already played — only the
        // unplayed ones carry into a new queue.
        let items = vec![solo("q0"), song("b", None), solo("q1"), song("c", None), solo("q2")];
        let carried = upcoming_queued(&items, 1);
        let ids: Vec<_> = carried.iter().map(|i| i.video_id.as_str()).collect();
        assert_eq!(ids, ["q1", "q2"]);
        // Nothing queued upcoming → nothing carried.
        assert!(upcoming_queued(&[song("a", None)], 0).is_empty());
    }

    #[test]
    fn shuffle_leaves_manual_queue_block_in_place() {
        let solo = |id: &str| innertube::SongItem { queued: true, ..song(id, None) };
        let mut items = vec![song("now", None), solo("q1"), solo("q2")];
        items.extend((0..8).map(|i| song(&format!("t{i}"), None)));
        shuffle_upcoming(&mut items, 0);
        // The manual "Next in queue" block still plays next, in order.
        assert_eq!(items[1].video_id, "q1");
        assert_eq!(items[2].video_id, "q2");
        // Everything is still a permutation (nothing lost).
        assert_eq!(items.len(), 11);
    }

    // A whole album queued with "Play next" is a set of tracks the user shuffles like any other:
    // the block keeps its place at the front, its own tracks get randomized. Songs queued one at a
    // time keep the order they were queued in.
    #[test]
    fn shuffle_randomizes_a_play_next_album_but_not_loose_adds() {
        let solo = |id: &str| innertube::SongItem { queued: true, ..song(id, None) };
        let from = |id: &str, name: &str| innertube::SongItem {
            queued: true,
            queued_from: Some(name.into()),
            ..song(id, None)
        };
        let mut moved = false;
        for _ in 0..20 {
            let mut items = vec![song("now", None), solo("q1"), solo("q2")];
            items.extend((0..12).map(|i| from(&format!("a{i}"), "Album")));
            items.extend((0..4).map(|i| song(&format!("t{i}"), None)));
            shuffle_upcoming(&mut items, 0);
            // The loose adds stay where they were, in order, ahead of the album block.
            assert_eq!(items[1].video_id, "q1");
            assert_eq!(items[2].video_id, "q2");
            // The album block stays a block — it just plays in a different order inside it.
            let block: Vec<_> = items[3..15].iter().map(|i| i.video_id.clone()).collect();
            assert!(block.iter().all(|id| id.starts_with('a')), "the block kept its place");
            moved |= block != (0..12).map(|i| format!("a{i}")).collect::<Vec<_>>();
            let mut sorted = block.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), 12, "nothing lost or duplicated");
        }
        assert!(moved, "the album block was never shuffled");
    }

    #[test]
    fn shuffle_keeps_autoplay_after_queue_tracks() {
        let auto = |id: &str| innertube::SongItem { autoplay: true, ..song(id, None) };
        // Playing index 0; upcoming = 4 playlist tracks + 4 autoplay tracks.
        let mut items = vec![song("now", None)];
        items.extend((0..4).map(|i| song(&format!("p{i}"), None)));
        items.extend((0..4).map(|i| auto(&format!("a{i}"))));
        shuffle_upcoming(&mut items, 0);
        // Every playlist track still comes before every autoplay track.
        let flags: Vec<bool> = items[1..].iter().map(|i| i.autoplay).collect();
        assert_eq!(flags, [false, false, false, false, true, true, true, true]);
        assert_eq!(items.len(), 9);
    }

    #[test]
    fn radio_seed_from_source() {
        // Playlist browseIds are VL-prefixed — stripped before building the radio id.
        // A mix is already a radio playlist — it seeds autoplay as itself, not wrapped again.
        assert_eq!(radio_seed_for(Some("VLRDCLAK5uy_x".into())).as_deref(), Some("RDCLAK5uy_x"));
        assert_eq!(radio_seed_for(Some("VLPL123".into())).as_deref(), Some("RDAMPLPL123"));
        // Album audio playlist ids come bare.
        assert_eq!(radio_seed_for(Some("OLAK5uy_x".into())).as_deref(), Some("RDAMPLOLAK5uy_x"));
        // No source (single song / artist top-songs) → no pinned seed.
        assert_eq!(radio_seed_for(None), None);
    }

    #[test]
    fn autoplay_merge_dedupes_and_caps() {
        let mut items = vec![song("a", None), song("b", None)];
        let existing: std::collections::HashSet<String> =
            items.iter().map(|i| i.video_id.clone()).collect();
        // "a" is already queued, "c" appears twice in the radio result — one copy survives.
        let fresh = vec![
            song("a", None),
            song("c", None),
            song("c", None),
            song("d", None),
            song("e", None),
        ];
        let added = merge_radio(&mut items, fresh, existing.clone(), 2);
        assert_eq!(added, 2); // cap honored: c + d, e cut off
        let ids: Vec<_> = items.iter().map(|i| i.video_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c", "d"]);
        // Appended tracks are marked autoplay (the UI's divider/badge key); originals aren't.
        assert!(items[2].autoplay && items[3].autoplay);
        assert!(!items[0].autoplay && !items[1].autoplay);
        // Nothing new in the radio result → 0, queue untouched (playback then stops as before).
        assert_eq!(merge_radio(&mut items.clone(), vec![song("a", None)], existing, 20), 0);
    }

    #[test]
    fn loudness_gain_attenuates_only_above_the_target() {
        // "As It Was" measures loudnessDb 7.77 ⇒ -6.23 LUFS, 0.77 dB over the -7 target.
        assert_eq!(loudness_gain(Some(7.77)).map(|g| (g * 100.0).round()), Some(-77.0));
        // Exactly on target, and everything below -7 LUFS, gets no filter at all — YTM doesn't
        // touch these either. (Real values: "Levitating" 6.85, "Shape of You" 6.35, "bad guy" 0.11.)
        for l in [7.0, 6.85, 6.35, 0.11, -5.0] {
            assert_eq!(loudness_gain(Some(l)), None, "loudnessDb {l} should not attenuate");
        }
        // Extreme loudness clamps at −24 dB.
        assert_eq!(loudness_gain(Some(40.0)), Some(-24.0));
        // No metadata → no filter.
        assert_eq!(loudness_gain(None), None);
    }
}
