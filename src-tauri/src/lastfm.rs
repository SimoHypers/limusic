//! Last.fm scrobbling. A second consumer of the same track/duration/position stream that feeds
//! `discord.rs` — but simpler: Last.fm doesn't care about live position, only two moments per
//! track. `track.updateNowPlaying` when a track starts, and one `track.scrobble` once the track
//! has played half its length or 4 minutes, whichever comes first (Last.fm's official rule;
//! tracks under 30s never scrobble).
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

enum Msg {
    Track(Box<Track>),
    Duration(f64),
    Position(f64),
    /// Session key set (connect) or cleared (disconnect).
    Session(Option<String>),
}

struct Track {
    title: String,
    artists: String,
    album: Option<String>,
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
        let _ = self.tx.send(Msg::Track(Box::new(Track {
            title: item.title.clone(),
            artists: item.artists.clone(),
            album: item.album.clone(),
        })));
    }

    pub fn set_duration(&self, secs: f64) {
        let _ = self.tx.send(Msg::Duration(secs));
    }

    pub fn set_position(&self, pos: f64) {
        let _ = self.tx.send(Msg::Position(pos));
    }

    fn set_session(&self, key: Option<String>) {
        let _ = self.tx.send(Msg::Session(key));
    }

    fn bump_gen(&self) -> u64 {
        self.auth_gen.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn gen(&self) -> u64 {
        self.auth_gen.load(Ordering::SeqCst)
    }
}

/// Spawn the scrobbler task. `session_key` is the persisted login; `None` parks the task until
/// the user connects.
pub fn spawn(session_key: Option<String>) -> LastfmHandle {
    let (tx, mut rx) = unbounded_channel::<Msg>();
    tauri::async_runtime::spawn(async move {
        let mut s = Scrobbler::new(session_key);
        while let Some(msg) = rx.recv().await {
            s.apply(msg).await;
        }
    });
    LastfmHandle { tx, auth_gen: AtomicU64::new(0) }
}

struct Scrobbler {
    http: reqwest::Client,
    session: Option<String>,
    track: Option<Track>,
    /// Epoch secs when the current track started — the scrobble's `timestamp`.
    started_at: u64,
    duration: f64,
    scrobbled: bool,
}

impl Scrobbler {
    fn new(session: Option<String>) -> Self {
        Scrobbler {
            http: reqwest::Client::new(),
            session,
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
            Msg::Duration(secs) => self.duration = secs,
            Msg::Position(pos) => {
                if !self.scrobbled && crosses_threshold(pos, self.duration) {
                    self.scrobbled = true; // latch even on failure — never re-fire per tick
                    self.scrobble().await;
                }
            }
            Msg::Session(key) => self.session = key,
        }
    }

    async fn now_playing(&self) {
        let (Some(sk), Some(t)) = (&self.session, &self.track) else { return };
        let mut params = vec![
            ("artist".to_string(), t.artists.clone()),
            ("track".to_string(), t.title.clone()),
            ("sk".to_string(), sk.clone()),
        ];
        if let Some(album) = t.album.as_ref().filter(|a| !a.is_empty()) {
            params.push(("album".to_string(), album.clone()));
        }
        match call(&self.http, "track.updateNowPlaying", params, true).await {
            Ok(_) => tracing::debug!(track = %t.title, "last.fm now playing sent"),
            Err(e) => tracing::debug!(error = %e.message, "last.fm now playing failed"),
        }
    }

    async fn scrobble(&self) {
        let (Some(sk), Some(t)) = (&self.session, &self.track) else { return };
        let mut params = vec![
            ("artist".to_string(), t.artists.clone()),
            ("track".to_string(), t.title.clone()),
            ("timestamp".to_string(), self.started_at.to_string()),
            ("sk".to_string(), sk.clone()),
        ];
        if let Some(album) = t.album.as_ref().filter(|a| !a.is_empty()) {
            params.push(("album".to_string(), album.clone()));
        }
        if self.duration > 0.0 {
            params.push(("duration".to_string(), (self.duration as i64).to_string()));
        }
        match call(&self.http, "track.scrobble", params, true).await {
            Ok(_) => tracing::info!(track = %t.title, "scrobbled to last.fm"),
            Err(e) => tracing::warn!(error = %e.message, "last.fm scrobble failed"),
        }
    }
}

/// Last.fm's scrobble rule: half the track or 4 minutes, whichever comes first; tracks under 30s
/// never scrobble. Unknown duration (mpv hasn't reported yet) leaves only the 4-minute rule.
fn crosses_threshold(pos: f64, duration: f64) -> bool {
    if duration > 0.0 && duration < 30.0 {
        return false;
    }
    let half = if duration > 0.0 { duration / 2.0 } else { f64::INFINITY };
    pos >= half.min(240.0)
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
    http: &reqwest::Client,
    method: &str,
    mut params: Vec<(String, String)>,
    post: bool,
) -> Result<serde_json::Value, ApiError> {
    params.push(("api_key".to_string(), API_KEY.to_string()));
    params.push(("method".to_string(), method.to_string()));
    params.push(("api_sig".to_string(), sign(&params)));
    params.push(("format".to_string(), "json".to_string()));

    let req = if post {
        http.post(API_ROOT).form(&params)
    } else {
        http.get(API_ROOT).query(&params)
    };
    let resp = req
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(ApiError::transport)?;
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

fn emit_state(app: &tauri::AppHandle, connected: bool, username: Option<&str>, error: Option<&str>) {
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
    let http = reqwest::Client::new();
    let token = call(&http, "auth.getToken", vec![], false)
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
            match call(&http, "auth.getSession", params, false).await {
                Ok(body) => {
                    let name = body.pointer("/session/name").and_then(|v| v.as_str());
                    let key = body.pointer("/session/key").and_then(|v| v.as_str());
                    let (Some(name), Some(key)) = (name, key) else {
                        emit_state(&state.app, false, None, Some("Last.fm sent a malformed session"));
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

/// Open a URL in the user's default browser. No opener plugin in the app; three lines cover the
/// three platforms.
fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    cmd.map(|_| ()).map_err(|e| format!("Couldn't open the browser: {e}"))
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

    #[test]
    fn scrobble_threshold_follows_lastfm_rules() {
        // Half the track wins for short tracks…
        assert!(!crosses_threshold(89.0, 180.0));
        assert!(crosses_threshold(90.0, 180.0));
        // …4 minutes wins for long ones.
        assert!(!crosses_threshold(239.0, 1200.0));
        assert!(crosses_threshold(240.0, 1200.0));
        // Under 30s never scrobbles.
        assert!(!crosses_threshold(29.0, 20.0));
        // Unknown duration: only the 4-minute rule applies.
        assert!(!crosses_threshold(120.0, 0.0));
        assert!(crosses_threshold(240.0, 0.0));
    }
}
