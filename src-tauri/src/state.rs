//! App state: transport, player, db, and the queue/playback manager. context/11.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use std::sync::Arc;

use innertube::{AudioQuality, Clients, InnerTube, SongItem, MAIN_CLIENT};
use listen_protocol::{Playback, PlaybackKind, Track};
use player::Player;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::db::Db;
use crate::discord::DiscordHandle;
use crate::listentogether::{LtSession, SyncCommand};
use crate::media::MediaHandle;
use crate::orchestrator::{Orchestrator, PlaybackData, ResolveError};

pub struct AppState {
    pub it: InnerTube,
    pub clients: Clients,
    pub player: Player,
    pub db: Db,
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
    queue: Mutex<QueueState>,
    /// Bumped on every explicit `play`/jump so superseded async resolves discard their result
    /// (cancellation without JoinHandle bookkeeping). context/06 §6.
    generation: AtomicU64,
    /// A one-shot resume position `(videoId, secs)` set by `restore_queue` and consumed by the
    /// next `start_current` — applied only when that track is the one being started, so jumping to
    /// a different track first doesn't inherit the old position (context/11).
    pending_seek: std::sync::Mutex<Option<(String, f64)>>,
    /// Mirror of mpv's pause flag (set in `media_set_playing`). Position ticks must consult this
    /// instead of assuming "playing" — mpv fires `time-pos` on seeks while paused too.
    is_playing: AtomicBool,
    /// Latest mpv position (f64 bits) + wall-clock secs of the last DB write, for throttled
    /// resume-position persistence.
    latest_position: AtomicU64,
    last_pos_persist: AtomicU64,
    /// Wall-clock secs of the last position push to the OS media controls (throttled ~1s).
    last_media_push: AtomicU64,
}

#[derive(Default)]
struct QueueState {
    items: Vec<SongItem>,
    current: usize,
    /// The queue index we've already appended to mpv for gapless lookahead (if any).
    lookahead_loaded: Option<usize>,
    /// Which client served the currently-loaded track (for the WEB_REMIX-403 feedback). context/06.
    current_client: Option<String>,
    /// The client that served the primed lookahead track — promoted to `current_client` on a
    /// gapless advance so the failure feedback still knows the client.
    lookahead_client: Option<String>,
    /// Watch-history tracking URL for the current track + the primed lookahead's (promoted on a
    /// gapless advance, mirroring current/lookahead_client). context/01 §registerPlayback.
    playback_url: Option<String>,
    lookahead_playback_url: Option<String>,
    /// Content Playback Nonce for the current play + whether we've already fired the history ping
    /// for it (latched so the frequent position events fire it exactly once). context/01.
    cpn: String,
    history_pinged: bool,
    /// Latest mpv-reported track duration (secs), for the history-ping threshold.
    duration: f64,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        it: InnerTube,
        clients: Clients,
        player: Player,
        db: Db,
        app: AppHandle,
        orchestrator: Arc<Orchestrator>,
        lt: Arc<LtSession>,
        cache_dir: std::path::PathBuf,
        media: Option<MediaHandle>,
        discord: Option<DiscordHandle>,
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
            queue: Mutex::new(QueueState::default()),
            is_playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            pending_seek: std::sync::Mutex::new(None),
            latest_position: AtomicU64::new(0),
            last_pos_persist: AtomicU64::new(0),
            last_media_push: AtomicU64::new(0),
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
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    // --- auth (context/15) ------------------------------------------------------------------

    /// Sign in with a pasted Cookie header (context/15 Path B). Validates SAPISID presence, sets
    /// the cookie on the transport, fetches `account_menu` (→ dataSyncId + display info + a fresh
    /// visitorData), and persists it all to `settings`. Returns the account JSON for the UI.
    pub async fn sign_in(&self, cookie: String) -> Result<serde_json::Value, String> {
        let cookie = cookie.trim().to_owned();
        if innertube::cookie_sapisid(&cookie).is_none() {
            return Err("That cookie has no SAPISID — copy the full Cookie header from a \
                        logged-in music.youtube.com tab."
                .into());
        }
        self.it.set_cookie(Some(cookie.clone()));
        let client =
            self.clients.get(innertube::METADATA_CLIENT).ok_or("metadata client missing")?;
        let info = match self.it.account_menu(client).await {
            // A valid, authenticating cookie returns the account header (name). No name means the
            // cookie didn't actually authenticate (stale/incomplete paste) — reject it up front so
            // we don't "succeed" into a silently-empty library.
            Ok(i) if i.name.is_some() => i,
            Ok(_) => {
                self.it.set_cookie(None);
                return Err("That cookie didn't authenticate — copy a fresh Cookie header from a \
                            logged-in music.youtube.com tab (its session cookies rotate, so grab a \
                            current one) and try again."
                    .into());
            }
            // Auth didn't take (network) — roll back so we're not half-logged-in.
            Err(e) => {
                self.it.set_cookie(None);
                return Err(format!("Sign-in failed: {e}"));
            }
        };
        // Persist. Plaintext SQLite — acceptable for a single-user personal tool (context/15).
        self.db.set_setting("session_cookie", &cookie);
        if let Some(id) = &info.data_sync_id {
            self.it.set_data_sync_id(Some(id.clone()));
            self.db.set_setting("data_sync_id", id);
        }
        if let Some(vd) = &info.visitor_data {
            self.it.set_visitor_data(Some(vd.clone()));
            self.db.set_setting("visitor_data", vd);
        }
        let account = serde_json::json!({
            "signedIn": true,
            "name": info.name,
            "handle": info.handle,
            "thumbnail": info.thumbnail,
        });
        self.db.set_setting("account_json", &account.to_string());
        let _ = self.app.emit("auth-changed", &account);
        Ok(account)
    }

    pub async fn sign_out(&self) {
        self.it.set_cookie(None);
        self.it.set_data_sync_id(None);
        self.db.delete_setting("session_cookie");
        self.db.delete_setting("data_sync_id");
        self.db.delete_setting("account_json");
        let _ = self.app.emit("auth-changed", serde_json::json!({ "signedIn": false }));
    }

    /// Current account for the UI. `signedIn` reflects the live cookie; the rest is the last
    /// persisted display info.
    pub fn account_snapshot(&self) -> serde_json::Value {
        let mut v = self
            .db
            .get_setting("account_json")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        v["signedIn"] = serde_json::json!(self.it.is_logged_in());
        v
    }

    async fn resolve(&self, video_id: &str) -> Result<PlaybackData, ResolveError> {
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
                // Not cached — a replay from cache doesn't re-register watch history (best-effort).
                playback_url: None,
                title: None,
                artists: None,
                duration: None,
                thumbnail: None,
                stream_client: "cache".to_owned(),
            });
        }
        let data = self
            .orchestrator
            .resolve(video_id, self.quality(), &self.disabled_clients())
            .await?;
        // Never cache rustypipe URLs: googlevideo serves them only for bounded-Range requests,
        // which mpv doesn't send → LOADING_FAILED(-13). Caching one poisons the videoId for ~6h.
        if data.stream_client != "rustypipe" {
            self.db.put_stream(
                video_id,
                &data.stream_url,
                data.itag,
                now + data.expires_in_seconds.max(0),
                data.loudness_db,
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

        {
            let mut q = self.queue.lock().await;
            q.items = vec![seed];
            q.current = 0;
            q.lookahead_loaded = None;
        }

        if !self.start_current(gen).await {
            return;
        }

        // Hydrate up-next radio (context/08) — non-fatal if it fails. Seed the radio playlist
        // directly (`RDAMVM<videoId>`): a bare next(videoId) returns only the seed song + an
        // automixPreviewVideoRenderer, so the queue would never grow past one track.
        let radio_id = format!("RDAMVM{video_id}");
        match self.it.next(self.clients.get(innertube::METADATA_CLIENT).unwrap(), &video_id, Some(&radio_id)).await {
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
                drop(q);
                self.emit_queue().await;
            }
            Err(e) => tracing::warn!(error = %e, "next() radio hydration failed"),
        }

        self.prime_lookahead(gen).await;
    }

    /// Play a finite list of tracks (a playlist/album) starting at `start`. Unlike `play_song`
    /// this seeds NO radio — the given items *are* the queue (context/08: playlist playback).
    pub async fn play_tracks(self: &std::sync::Arc<Self>, items: Vec<SongItem>, start: usize) {
        if items.is_empty() {
            return;
        }
        if self.lt.is_guest().await {
            self.emit_guest_hint();
            return;
        }
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let start = start.min(items.len() - 1);
        {
            let mut q = self.queue.lock().await;
            q.items = items;
            q.current = start;
            q.lookahead_loaded = None;
        }
        // start_current emits now-playing + queue + persists; prime the gapless lookahead after.
        if self.start_current(gen).await {
            self.prime_lookahead(gen).await;
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
            q.current = index;
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
        let has_next = {
            let mut q = self.queue.lock().await;
            let next = q.current + 1;
            if next >= q.items.len() {
                false
            } else {
                q.current = next;
                true
            }
        };
        if !has_next {
            tracing::info!("queue exhausted");
            let _ = self.app.emit("playback-state", "paused");
            // mpv goes idle without flipping its pause flag, so no Paused event will fire — tell
            // the OS widget + Discord ourselves or they show "playing" forever past the last song.
            self.media_set_playing(false);
            return;
        }
        if self.player.is_idle() {
            // No gapless handoff happened. Bump the generation so any in-flight lookahead
            // resolve for this index discards itself (it would double-enqueue), then load.
            let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            tracing::info!("no primed lookahead at track end — loading next explicitly");
            if self.start_current(gen).await {
                self.prime_lookahead(gen).await;
            }
            return;
        }
        // mpv already advanced into the primed lookahead. Sync pointer + UI, prime the next.
        let gen = self.generation.load(Ordering::SeqCst);
        {
            let mut q = self.queue.lock().await;
            q.current_client = q.lookahead_client.take();
            // New track is now playing → fresh history state (mirrors start_current).
            q.playback_url = q.lookahead_playback_url.take();
            q.cpn = innertube::generate_cpn();
            q.history_pinged = false;
            q.duration = 0.0;
        }
        if let Some(item) = self.current_item().await {
            self.emit_now_playing(&item, "gapless");
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
        tauri::async_runtime::spawn(async move { me.prime_lookahead(gen).await });
    }

    /// A track died at the player layer (dead/403 URL). If WEB_REMIX served it, record the failure
    /// so the next resolve for this id bypasses WEB_REMIX, and evict its poisoned cache entry
    /// (context/06 §2). Then advance the queue like a normal end.
    pub async fn on_track_failed(self: &std::sync::Arc<Self>) {
        let (video_id, client) = {
            let q = self.queue.lock().await;
            (q.items.get(q.current).map(|i| i.video_id.clone()), q.current_client.clone())
        };
        if let (Some(vid), Some(c)) = (video_id, client) {
            if c == MAIN_CLIENT {
                tracing::warn!(video_id = %vid, "WEB_REMIX stream failed on GET — marking + evicting");
                self.orchestrator.mark_web_remix_failed(&vid).await;
                self.db.evict_stream(&vid);
            }
        }
        self.on_track_ended().await;
    }

    /// Resolve + load the current track into mpv (replace). Returns false if resolve failed or the
    /// request was superseded.
    async fn start_current(self: &std::sync::Arc<Self>, gen: u64) -> bool {
        // Resolve the current track, auto-skipping any that no client can play (dead / region-locked
        // videos — context/06 "no client could resolve") instead of stalling the queue on them.
        // Bounded: each failure advances current by one, so the loop terminates at the queue tail.
        let (item, data) = loop {
            if self.generation.load(Ordering::SeqCst) != gen {
                return false; // user moved on
            }
            let Some(item) = self.current_item().await else { return false };
            match self.resolve(&item.video_id).await {
                Ok(d) => break (item, d),
                Err(e) => {
                    let mut q = self.queue.lock().await;
                    if q.current + 1 >= q.items.len() {
                        drop(q);
                        self.emit_error(&item.video_id, &e.to_string()); // nothing left to skip to
                        return false;
                    }
                    q.current += 1;
                    q.lookahead_loaded = None;
                    drop(q);
                    self.emit_skip(&item.title);
                    self.emit_queue().await;
                }
            }
        };
        if self.generation.load(Ordering::SeqCst) != gen {
            return false; // user moved on
        }
        if let Err(e) = self.player.load(&data.stream_url, &data.headers, loudness_gain(data.loudness_db)) {
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
        {
            let mut q = self.queue.lock().await;
            q.current_client = Some(data.stream_client.clone());
            // Fresh play → fresh history state (context/01 §registerPlayback).
            q.playback_url = data.playback_url.clone();
            q.cpn = innertube::generate_cpn();
            q.history_pinged = false;
            q.duration = 0.0;
        }
        self.emit_now_playing(&item, &data.stream_client);
        // We just told mpv to play, but its `pause` flag was already `false`, so no property event
        // will announce it (see `Player::is_playing`). Say so ourselves — otherwise MPRIS and
        // Discord never learn the track started. After `emit_now_playing`, so the new track is the
        // current one before anything renders it as playing.
        self.media_set_playing(true);
        self.emit_queue().await;
        self.persist_queue().await;
        // Listen Together host: announce the new track (fresh play → position 0, playing).
        self.lt_broadcast_current_track(0, true).await;
        true
    }

    /// Resolve the next queue item and append it to mpv for a gapless transition. context/14.
    async fn prime_lookahead(self: &std::sync::Arc<Self>, gen: u64) {
        let next_index = {
            let q = self.queue.lock().await;
            if q.lookahead_loaded == Some(q.current + 1) {
                return; // already primed
            }
            q.current + 1
        };
        let next_video = {
            let q = self.queue.lock().await;
            match q.items.get(next_index) {
                Some(item) => item.video_id.clone(),
                None => return,
            }
        };
        let data = match self.resolve(&next_video).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(video_id = %next_video, error = %e, "lookahead resolve failed");
                return;
            }
        };
        if self.generation.load(Ordering::SeqCst) != gen {
            return;
        }
        let mut q = self.queue.lock().await;
        // The queue can change under a resolve (a guest add inserts at current+1) — enqueueing
        // then would gaplessly play the wrong song. Verify the slot still holds the same track.
        if q.items.get(next_index).map(|i| i.video_id != next_video).unwrap_or(true) {
            tracing::debug!(index = next_index, "queue changed during lookahead resolve — dropped");
            return;
        }
        // Headers are global in mpv; the direct-URL clients need none beyond UA, which the
        // current track already set. Just append the URL.
        if let Err(e) = self.player.enqueue(&data.stream_url) {
            tracing::warn!(error = %e, "enqueue lookahead failed");
            return;
        }
        q.lookahead_loaded = Some(next_index);
        q.lookahead_client = Some(data.stream_client.clone());
        q.lookahead_playback_url = data.playback_url.clone();
        tracing::debug!(index = next_index, "gapless lookahead primed");
    }

    async fn current_item(&self) -> Option<SongItem> {
        let q = self.queue.lock().await;
        q.items.get(q.current).cloned()
    }

    // --- events (context/11 UI contract) ----------------------------------------------------

    fn emit_now_playing(&self, item: &SongItem, stream_client: &str) {
        let _ = self.app.emit(
            "now-playing",
            serde_json::json!({
                "videoId": item.video_id,
                "title": item.title,
                "artists": item.artists,
                "artistId": item.artist_id,
                "thumbnail": item.thumbnail,
                "duration": item.duration,
                "streamClient": stream_client,
                "liked": item.liked,
            }),
        );
        let _ = self.app.emit("playback-state", "playing");
        // Push the same metadata to the OS media widget (context/16) and Discord.
        if let Some(m) = &self.media {
            m.set_metadata(&item.title, &item.artists, item.album.as_deref(), item.thumbnail.as_deref());
        }
        if let Some(d) = &self.discord {
            d.set_track(item);
        }
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
            m.set_playback(playing, self.current_position());
        }
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
        let i = self.queue.lock().await.current + 1;
        self.play_index(i).await;
    }

    pub async fn prev_in_queue(self: &std::sync::Arc<Self>) {
        let i = self.queue.lock().await.current.saturating_sub(1);
        self.play_index(i).await;
    }

    async fn emit_queue(&self) {
        let q = self.queue.lock().await;
        let _ = self.app.emit(
            "queue-changed",
            serde_json::json!({ "items": &q.items, "currentIndex": q.current }),
        );
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

    /// A track was auto-skipped because no client could resolve it — a transient toast, not the
    /// persistent error banner: the queue keeps playing, so this shouldn't read as a failure.
    fn emit_skip(&self, title: &str) {
        tracing::warn!(title, "skipping unplayable track");
        let _ = self
            .app
            .emit("playback-notice", serde_json::json!({ "message": format!("Skipped (unavailable): {title}") }));
    }

    pub async fn queue_snapshot(&self) -> serde_json::Value {
        let q = self.queue.lock().await;
        serde_json::json!({ "items": &q.items, "currentIndex": q.current })
    }

    /// A position tick from mpv. Fires the watch-history ping once the current track passes the
    /// play threshold (context/01 §registerPlayback) — latched to fire exactly once per play,
    /// gated on the `enable_history` setting + being logged in. Best-effort (errors logged).
    pub async fn on_position(&self, pos: f64) {
        self.record_position(pos);
        let ping = {
            let mut q = self.queue.lock().await;
            if q.history_pinged {
                None
            } else {
                // Threshold: halfway, capped at 30s (default 30s until mpv reports duration).
                let threshold = if q.duration > 1.0 { (q.duration / 2.0).min(30.0) } else { 30.0 };
                if pos >= threshold {
                    q.history_pinged = true; // latch even if the URL is missing — never retry
                    q.playback_url.clone().map(|url| (url, q.cpn.clone()))
                } else {
                    None
                }
            }
        };
        let Some((url, cpn)) = ping else { return };
        if !self.history_enabled() || !self.it.is_logged_in() {
            return;
        }
        let Some(client) = self.clients.get(innertube::METADATA_CLIENT).cloned() else { return };
        let it = self.it.clone();
        tauri::async_runtime::spawn(async move {
            match it.register_playback(&client, &url, &cpn, None).await {
                Ok(()) => tracing::debug!("watch-history ping sent"),
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
        }
    }

    /// Watch-history ping enabled? Default on; only an explicit `"false"` disables it.
    fn history_enabled(&self) -> bool {
        self.db.get_setting("enable_history").map(|v| v != "false").unwrap_or(true)
    }

    /// Persist the queue (items + current index) as a JSON blob so a restart can restore it
    /// losslessly (context/11 §state). Called whenever the queue changes or advances.
    async fn persist_queue(&self) {
        let json = {
            let q = self.queue.lock().await;
            serde_json::json!({ "items": &q.items, "current": q.current }).to_string()
        };
        self.db.set_setting("queue_json", &json);
    }

    /// Restore the last session's queue on startup — paused, not autoplaying (context/11). The
    /// saved position is applied when the user first hits play (see `start_current`). Emits
    /// `queue-changed` + `now-playing` so the UI shows the restored track.
    pub async fn restore_queue(&self) {
        let Some(json) = self.db.get_setting("queue_json") else { return };
        let Ok(saved) = serde_json::from_str::<serde_json::Value>(&json) else { return };
        let items: Vec<SongItem> = saved
            .get("items")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        if items.is_empty() {
            return;
        }
        let current = (saved.get("current").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
            .min(items.len() - 1);
        let pos = self.db.get_setting("queue_position").and_then(|s| s.parse::<f64>().ok());
        if let Some(p) = pos.filter(|p| *p > 0.0) {
            *self.pending_seek.lock().unwrap() = Some((items[current].video_id.clone(), p));
        }
        {
            let mut q = self.queue.lock().await;
            q.current = current;
            q.items = items;
        }
        if let Some(item) = self.current_item().await {
            // Restored, not playing — announce the track but leave playback paused. Declare the
            // paused state *first*: mpv reports `pause: false` while idle at boot, so a track
            // announced before this would briefly look like it was playing (and put a presence card
            // up for a song nobody started).
            self.media_set_playing(false);
            self.emit_now_playing(&item, "restored");
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
                m.set_playback(playing, pos);
            }
            if let Some(d) = &self.discord {
                d.set_position(pos);
            }
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
            self.lt_apply_change_track(track, state.position_ms, state.is_playing, state.queue).await;
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
            q.lookahead_loaded = None;
        }
        let data = match self.resolve(&track.id).await {
            Ok(d) => d,
            Err(e) => {
                self.emit_error(&track.id, &e.to_string());
                return;
            }
        };
        if self.generation.load(Ordering::SeqCst) != gen {
            return; // superseded by a newer sync
        }
        if let Err(e) = self.player.load(&data.stream_url, &data.headers, loudness_gain(data.loudness_db)) {
            self.emit_error(&track.id, &e.to_string());
            return;
        }
        // Seek first (mpv queues it until the file loads), then set play/pause — avoids a blip of
        // audio at 0 before the seek lands.
        let target_ms = if playing { position_ms + t0.elapsed().as_millis() as i64 } else { position_ms };
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
        let playing = !self.player.is_idle();
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
            self.lt.broadcast_playback(Playback::at(PlaybackKind::Seek, (position * 1000.0) as i64))
                .await;
        }
        Ok(())
    }

    /// "Add to queue" from the UI's track menu. Solo: appended to the end. In a session everyone's
    /// adds land at the session boundary ("up next", FIFO): guests route through the host
    /// (suggest → auto-approve), the host inserts directly, tagged with their own name so the
    /// room sees who added it.
    pub async fn add_to_queue(self: &std::sync::Arc<Self>, item: SongItem) {
        if self.lt.is_guest().await {
            self.lt.suggest(song_to_track(&item)).await;
            return;
        }
        if self.lt.is_host().await {
            let mut track = song_to_track(&item);
            track.queued_by = Some(self.lt.my_username().await.unwrap_or_else(|| "Host".into()));
            self.lt_enqueue_track(track).await;
            return;
        }
        // Solo: end of the queue.
        {
            let mut q = self.queue.lock().await;
            q.items.push(item);
        }
        self.emit_queue().await;
        self.persist_queue().await;
        // If the playing track was the last one, there's a next now — prime it for gapless.
        self.prime_lookahead(self.generation.load(Ordering::SeqCst)).await;
    }

    /// Host: add a session track to the real queue at the *session boundary* — right after the
    /// current song, behind any earlier session adds (FIFO), ahead of the host's own playlist —
    /// then broadcast the updated queue. "Up next", never buried at the end.
    pub async fn lt_enqueue_track(self: &std::sync::Arc<Self>, track: Track) {
        {
            let mut q = self.queue.lock().await;
            let at = guest_insert_index(&q.items, q.current);
            let song = track_to_song(&track);
            q.items.insert(at, song);
            // If mpv already holds a primed lookahead for the old `current + 1`, it now points at
            // the wrong song — drop it so the gapless advance plays the added track.
            if at == q.current + 1 && q.lookahead_loaded.take().is_some() {
                let _ = self.player.clear_playlist();
            }
        };
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
                // The primed entry is the same song at a shifted index.
                q.lookahead_loaded = q.lookahead_loaded.map(|i| i - 1);
                false
            } else if index == q.current + 1 && q.lookahead_loaded.take().is_some() {
                // mpv holds the removed song as the gapless next — drop it.
                let _ = self.player.clear_playlist();
                true
            } else {
                false
            }
        };
        self.emit_queue().await;
        self.persist_queue().await;
        if stale_lookahead {
            self.prime_lookahead(self.generation.load(Ordering::SeqCst)).await;
        }
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
        album: None,
        album_id: None,
        duration: if t.duration_ms > 0 { Some(format_duration(t.duration_ms)) } else { None },
        thumbnail: t.thumbnail.clone(),
        set_video_id: None,
        liked: None,
        queued_by: t.queued_by.clone(),
    }
}

/// Where a guest-added track goes: right after the current song, behind any earlier guest adds
/// (FIFO), ahead of the host's own upcoming playlist.
fn guest_insert_index(items: &[SongItem], current: usize) -> usize {
    let mut at = (current + 1).min(items.len());
    while items.get(at).map(|i| i.queued_by.is_some()).unwrap_or(false) {
        at += 1;
    }
    at
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
    format!("{}:{:02}", total / 60, total % 60)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Per-track loudness gain (dB) from YouTube's `loudnessDb` (context/03, context/14). Attenuate
/// only toward reference loudness: loud masters (`loudnessDb > 0`) get `-loudnessDb`; quieter
/// tracks aren't boosted, so there's no clipping and no limiter to add.
// ponytail: attenuate-only, clamped to -24 dB. If quiet tracks feel too soft, allow positive gain
// plus an `alimiter` af to catch the resulting peaks.
fn loudness_gain(loudness_db: Option<f64>) -> Option<f64> {
    loudness_db.map(|l| (-l).clamp(-24.0, 0.0))
}

#[cfg(test)]
mod tests {
    use super::{guest_insert_index, loudness_gain};

    #[test]
    fn guest_adds_stack_fifo_after_current() {
        let song = |id: &str, by: Option<&str>| innertube::SongItem {
            video_id: id.into(),
            title: id.into(),
            artists: String::new(),
            artist_id: None,
            album: None,
            album_id: None,
            duration: None,
            thumbnail: None,
            set_video_id: None,
            liked: None,
            queued_by: by.map(Into::into),
        };
        // Host playlist [A*, B, C] (playing A): guest add goes right after current, not the end.
        let items = vec![song("a", None), song("b", None), song("c", None)];
        assert_eq!(guest_insert_index(&items, 0), 1);
        // A guest track already up next → the new one queues behind it (FIFO), before B.
        let items = vec![song("a", None), song("g1", Some("kim")), song("b", None)];
        assert_eq!(guest_insert_index(&items, 0), 2);
        // Current is the last item → append.
        let items = vec![song("a", None)];
        assert_eq!(guest_insert_index(&items, 0), 1);
        // Empty queue (nothing playing yet) → index 0… clamped, no panic.
        assert_eq!(guest_insert_index(&[], 0), 0);
    }

    #[test]
    fn loudness_gain_attenuates_loud_only() {
        // Loud master (+7 dB over reference) → attenuate 7 dB.
        assert_eq!(loudness_gain(Some(7.0)), Some(-7.0));
        // Quiet track (−5 dB) → no boost (clamped to 0).
        assert_eq!(loudness_gain(Some(-5.0)), Some(0.0));
        // Extreme loudness clamps at −24 dB.
        assert_eq!(loudness_gain(Some(40.0)), Some(-24.0));
        // No metadata → no filter.
        assert_eq!(loudness_gain(None), None);
    }
}
