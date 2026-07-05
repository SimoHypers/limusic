//! App state: transport, player, db, and the queue/playback manager. context/11.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use std::sync::Arc;

use innertube::{AudioQuality, Clients, InnerTube, SongItem, MAIN_CLIENT};
use player::Player;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::db::{Db, QueueRow};
use crate::orchestrator::{Orchestrator, PlaybackData, ResolveError};

pub struct AppState {
    pub it: InnerTube,
    pub clients: Clients,
    pub player: Player,
    pub db: Db,
    pub app: AppHandle,
    pub orchestrator: Arc<Orchestrator>,
    queue: Mutex<QueueState>,
    /// Bumped on every explicit `play`/jump so superseded async resolves discard their result
    /// (cancellation without JoinHandle bookkeeping). context/06 §6.
    generation: AtomicU64,
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
}

impl AppState {
    pub fn new(
        it: InnerTube,
        clients: Clients,
        player: Player,
        db: Db,
        app: AppHandle,
        orchestrator: Arc<Orchestrator>,
    ) -> Self {
        AppState {
            it,
            clients,
            player,
            db,
            app,
            orchestrator,
            queue: Mutex::new(QueueState::default()),
            generation: AtomicU64::new(0),
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
                loudness_db: None,
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
            );
        }
        Ok(data)
    }

    /// Start a fresh queue from one track (a search-result click), then hydrate the radio via
    /// `next` and prime the gapless lookahead.
    pub async fn play_song(self: &std::sync::Arc<Self>, seed: SongItem) {
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

    /// Jump to a specific queue index.
    pub async fn play_index(self: &std::sync::Arc<Self>, index: usize) {
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
        }
        if let Some(item) = self.current_item().await {
            self.emit_now_playing(&item, "gapless");
        }
        self.emit_queue().await;
        tracing::info!("advanced to next track (gapless)");
        self.prime_lookahead(gen).await;
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
        let Some(item) = self.current_item().await else { return false };
        let data = match self.resolve(&item.video_id).await {
            Ok(d) => d,
            Err(e) => {
                self.emit_error(&item.video_id, &e.to_string());
                return false;
            }
        };
        if self.generation.load(Ordering::SeqCst) != gen {
            return false; // user moved on
        }
        if let Err(e) = self.player.load(&data.stream_url, &data.headers, data.loudness_db) {
            self.emit_error(&item.video_id, &e.to_string());
            return false;
        }
        let _ = self.player.play();
        {
            let mut q = self.queue.lock().await;
            q.current_client = Some(data.stream_client.clone());
        }
        self.emit_now_playing(&item, &data.stream_client);
        self.emit_queue().await;
        self.persist_queue().await;
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
        // Headers are global in mpv; the direct-URL clients need none beyond UA, which the
        // current track already set. Just append the URL.
        if let Err(e) = self.player.enqueue(&data.stream_url) {
            tracing::warn!(error = %e, "enqueue lookahead failed");
            return;
        }
        let mut q = self.queue.lock().await;
        q.lookahead_loaded = Some(next_index);
        q.lookahead_client = Some(data.stream_client.clone());
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
                "thumbnail": item.thumbnail,
                "duration": item.duration,
                "streamClient": stream_client,
                "liked": item.liked,
            }),
        );
        let _ = self.app.emit("playback-state", "playing");
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

    pub async fn queue_snapshot(&self) -> serde_json::Value {
        let q = self.queue.lock().await;
        serde_json::json!({ "items": &q.items, "currentIndex": q.current })
    }

    async fn persist_queue(&self) {
        let q = self.queue.lock().await;
        let rows: Vec<QueueRow> = q
            .items
            .iter()
            .map(|i| QueueRow {
                video_id: i.video_id.clone(),
                title: i.title.clone().into(),
                artists: i.artists.clone().into(),
                duration: i.duration.clone(),
                thumbnail: i.thumbnail.clone(),
            })
            .collect();
        drop(q);
        self.db.save_queue(&rows);
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
