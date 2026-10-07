//! ListenBrainz scrobbling. Same shape as [`crate::lastfm`]: a second consumer of the same
//! track/duration/position stream, with the same threshold and the same metadata cleanup
//! ([`crate::lastfm::resolve`]). Only the transport differs.
//!
//! Auth is a user token, pasted from <https://listenbrainz.org/settings/> (the "User Token"
//! without which nothing validates). It is stored as `listenbrainz_token` with the validated
//! `listenbrainz_username` beside it, and sent as `Authorization: Token <token>` on every call.
//! Validation is `GET /1/validate-token`; listens go to `POST /1/submit-listens` as
//! `playing_now` on track start and one `single` once the track crosses the threshold.
//!
//! Everything is best-effort, like Last.fm: a failed listen is a `debug!` line, never a
//! user-facing error. No offline queue.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use innertube::SongItem;
use tauri::Emitter;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::lastfm::{crosses_threshold, resolve, Resolved, ScrobbleConfig, Track};
use crate::state::AppState;

const API_ROOT: &str = "https://api.listenbrainz.org";
const USER_AGENT: &str = concat!("Limusic/", env!("CARGO_PKG_VERSION"), " (https://github.com/SimoHypers/limusic)");

enum Msg {
    Track(Box<Track>),
    /// Same late-album path as Last.fm (#309): a search card starts with no album, the radio
    /// behind it supplies one. Keyed by video id so a stale album can't land on the next track.
    Album { video_id: String, album: String },
    Duration(f64),
    Position(f64),
    /// User token set (connect) or cleared (disconnect).
    Token(Option<String>),
    Config(Box<ScrobbleConfig>),
}

/// App-side handle to the ListenBrainz task.
pub struct ListenBrainzHandle {
    tx: UnboundedSender<Msg>,
}

impl ListenBrainzHandle {
    pub fn set_track(&self, item: &SongItem) {
        let _ = self.tx.send(Msg::Track(Box::new(Track::from(item))));
    }

    pub fn set_album(&self, video_id: &str, album: &str) {
        let _ = self.tx.send(Msg::Album { video_id: video_id.to_owned(), album: album.to_owned() });
    }

    pub fn set_duration(&self, secs: f64) {
        let _ = self.tx.send(Msg::Duration(secs));
    }

    pub fn set_position(&self, pos: f64) {
        let _ = self.tx.send(Msg::Position(pos));
    }

    /// The shared scrobbling settings changed (the same `lastfm_config` blob Last.fm reads).
    /// Applies to the track already playing, from its listen on.
    pub fn set_config(&self, cfg: ScrobbleConfig) {
        let _ = self.tx.send(Msg::Config(Box::new(cfg)));
    }

    fn set_token(&self, token: Option<String>) {
        let _ = self.tx.send(Msg::Token(token));
    }
}

/// Spawn the task. `token` is the persisted user token; `None` parks it until the user connects.
pub fn spawn(token: Option<String>, cfg: ScrobbleConfig) -> ListenBrainzHandle {
    let (tx, mut rx) = unbounded_channel::<Msg>();
    tauri::async_runtime::spawn(async move {
        let mut s = Scrobbler::new(token, cfg);
        while let Some(msg) = rx.recv().await {
            s.apply(msg).await;
        }
    });
    ListenBrainzHandle { tx }
}

struct Scrobbler {
    token: Option<String>,
    cfg: ScrobbleConfig,
    track: Option<Track>,
    /// Epoch secs when the current track started — the listen's `listened_at`.
    started_at: u64,
    duration: f64,
    scrobbled: bool,
}

impl Scrobbler {
    fn new(token: Option<String>, cfg: ScrobbleConfig) -> Self {
        Scrobbler { token, cfg, track: None, started_at: 0, duration: 0.0, scrobbled: false }
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
                self.now_playing().await;
            }
            Msg::Duration(secs) => self.duration = secs,
            Msg::Position(pos) => {
                if !self.scrobbled && crosses_threshold(pos, self.duration, &self.cfg) {
                    self.scrobbled = true; // latch even on failure — never re-fire per tick
                    self.scrobble().await;
                }
            }
            Msg::Token(token) => self.token = token,
            Msg::Config(cfg) => self.cfg = *cfg,
        }
    }

    /// The token and what to send, or `None` when this track isn't going to ListenBrainz.
    fn outgoing(&self) -> Option<(&str, Resolved)> {
        let (Some(token), Some(t)) = (self.token.as_deref(), &self.track) else { return None };
        if !self.cfg.enabled() {
            return None;
        }
        let r = resolve(t, &self.cfg);
        r.skip.is_none().then_some((token, r))
    }

    async fn now_playing(&self) {
        if !self.cfg.now_playing_enabled() {
            return;
        }
        let Some((token, r)) = self.outgoing() else { return };
        let video_id = self.track.as_ref().map(|t| t.video_id.as_str()).unwrap_or("");
        let payload = serde_json::json!([{
            "track_metadata": track_metadata(&r, video_id, self.duration),
        }]);
        match submit(token, "playing_now", payload).await {
            Ok(_) => tracing::debug!(track = %r.title, "listenbrainz now playing sent"),
            Err(e) => tracing::debug!(error = %e, "listenbrainz now playing failed"),
        }
    }

    async fn scrobble(&self) {
        let Some((token, r)) = self.outgoing() else { return };
        let video_id = self.track.as_ref().map(|t| t.video_id.as_str()).unwrap_or("");
        let payload = serde_json::json!([{
            "listened_at": self.started_at,
            "track_metadata": track_metadata(&r, video_id, self.duration),
        }]);
        match submit(token, "single", payload).await {
            Ok(_) => tracing::info!(track = %r.title, "submitted listen to listenbrainz"),
            Err(e) => tracing::warn!(error = %e, "listenbrainz listen failed"),
        }
    }
}

/// The `track_metadata` object for one listen. `video_id` feeds `origin_url` for YouTube tracks
/// (and picks the music service); local files carry no URL.
fn track_metadata(r: &Resolved, video_id: &str, duration: f64) -> serde_json::Value {
    let mut meta = serde_json::json!({
        "artist_name": r.artist,
        "track_name": r.title,
    });
    if !r.album.is_empty() {
        meta["release_name"] = serde_json::Value::String(r.album.clone());
    }
    let mut info = serde_json::Map::new();
    info.insert("media_player".into(), serde_json::Value::String("Limusic".into()));
    info.insert("submission_client".into(), serde_json::Value::String("Limusic".into()));
    info.insert(
        "submission_client_version".into(),
        serde_json::Value::String(env!("CARGO_PKG_VERSION").into()),
    );
    if duration > 0.0 {
        info.insert("duration_ms".into(), serde_json::json!((duration * 1000.0) as i64));
    }
    if !crate::local::is_local_song(video_id) && !video_id.is_empty() {
        info.insert("music_service".into(), serde_json::Value::String("music.youtube.com".into()));
        info.insert(
            "origin_url".into(),
            serde_json::Value::String(format!("https://music.youtube.com/watch?v={video_id}")),
        );
    }
    meta["additional_info"] = serde_json::Value::Object(info);
    meta
}

async fn submit(token: &str, listen_type: &str, payload: serde_json::Value) -> Result<(), String> {
    let body = serde_json::json!({ "listen_type": listen_type, "payload": payload });
    let resp = crate::http::client()
        .post(format!("{API_ROOT}/1/submit-listens"))
        .header("Authorization", format!("Token {token}"))
        .header("User-Agent", USER_AGENT)
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let short: String = text.chars().take(300).collect();
        return Err(format!("{status}: {short}"));
    }
    Ok(())
}

async fn validate(token: &str) -> Result<String, String> {
    let resp = crate::http::client()
        .get(format!("{API_ROOT}/1/validate-token"))
        .header("Authorization", format!("Token {token}"))
        .header("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if body.get("valid").and_then(|v| v.as_bool()) != Some(true) {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("ListenBrainz rejected that token");
        return Err(msg.into());
    }
    body
        .get("user_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "ListenBrainz validated the token but named no user".to_string())
}

fn emit_state(app: &tauri::AppHandle, connected: bool, username: Option<&str>, error: Option<&str>) {
    let _ = app.emit(
        "listenbrainz-state",
        serde_json::json!({ "connected": connected, "username": username, "error": error }),
    );
}

/// Connect with a user token pasted from ListenBrainz settings. Validates it first, so a typo
/// is an error rather than a stored token that silently scrobbles nothing.
pub async fn connect(state: Arc<AppState>, token: String) -> Result<(), String> {
    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err("Paste your ListenBrainz user token first — find it under Settings on listenbrainz.org.".into());
    }
    let username = validate(&token).await.map_err(|e| format!("ListenBrainz: {e}"))?;
    state.db.set_setting("listenbrainz_token", &token);
    state.db.set_setting("listenbrainz_username", &username);
    state.listenbrainz.set_token(Some(token));
    tracing::info!(user = %username, "listenbrainz connected");
    emit_state(&state.app, true, Some(&username), None);
    Ok(())
}

pub fn disconnect(state: &AppState) {
    state.db.set_setting("listenbrainz_token", "");
    state.db.set_setting("listenbrainz_username", "");
    state.listenbrainz.set_token(None);
    emit_state(&state.app, false, None, None);
}

pub fn status(state: &AppState) -> serde_json::Value {
    let token = state.db.get_setting("listenbrainz_token").filter(|s| !s.is_empty());
    let username = state.db.get_setting("listenbrainz_username").filter(|s| !s.is_empty());
    serde_json::json!({ "connected": token.is_some(), "username": username })
}

/// The connected user's listen count, for the Scrobbling tab's account card. `None` when nobody
/// is connected or ListenBrainz didn't answer.
pub async fn profile(state: &AppState) -> Option<Profile> {
    let username = state.db.get_setting("listenbrainz_username").filter(|s| !s.is_empty())?;
    let token = state.db.get_setting("listenbrainz_token").filter(|s| !s.is_empty())?;
    let mut req = crate::http::client()
        .get(format!("{API_ROOT}/1/user/{username}/listen-count"))
        .header("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(15));
    if !token.is_empty() {
        req = req.header("Authorization", format!("Token {token}"));
    }
    let body: serde_json::Value = req.send().await.ok()?.json().await.ok()?;
    let count = body.pointer("/payload/count").and_then(|v| v.as_u64()).unwrap_or(0);
    Some(Profile {
        username: username.clone(),
        url: format!("https://listenbrainz.org/user/{username}/"),
        listens: count,
    })
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Profile {
    pub username: String,
    pub url: String,
    pub listens: u64,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_metadata_carries_resolved_fields_and_client() {
        let r = Resolved {
            artist: "Ruelle".into(),
            title: "Fire".into(),
            album: "Album".into(),
            skip: None,
            edit: None,
            split: false,
            rules: vec![],
            errors: vec![],
        };
        let m = track_metadata(&r, "dQw4w9WgXcQ", 180.0);
        assert_eq!(m["artist_name"], "Ruelle");
        assert_eq!(m["track_name"], "Fire");
        assert_eq!(m["release_name"], "Album");
        assert_eq!(m["additional_info"]["duration_ms"], 180_000);
        assert_eq!(m["additional_info"]["submission_client"], "Limusic");
        assert_eq!(
            m["additional_info"]["origin_url"],
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        // No album: no release_name key at all.
        let r = Resolved { album: String::new(), ..r };
        let m = track_metadata(&r, "dQw4w9WgXcQ", 0.0);
        assert!(m.get("release_name").is_none());
        assert!(m["additional_info"].get("duration_ms").is_none());
        // Local files carry no URL.
        let m = track_metadata(&r, "LOCAL:/music/song.mp3", 10.0);
        assert!(m["additional_info"].get("origin_url").is_none());
    }
}
