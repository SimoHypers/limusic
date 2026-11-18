//! ListenBrainz transport + account calls. Timing lives in [`crate::lastfm`]: the single
//! scrobbler task holds the user token next to the Last.fm session key and sends to whichever
//! service is connected from the same now-playing and scrobble, so this module keeps no task,
//! no clock and no config of its own.
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
use std::time::Duration;

use tauri::Emitter;

use crate::lastfm::Resolved;
use crate::state::AppState;

const DEFAULT_ROOT: &str = "https://api.listenbrainz.org";

/// Trimmed, no trailing slash, http(s) with a host and no query or fragment (endpoints are
/// appended as `{root}/1/...`). Empty = official server.
fn clean_root(url: &str) -> Result<String, String> {
    // Empty before trimming slashes: a lone `/` is a typo, not a request for the official server.
    let url = url.trim();
    if url.is_empty() {
        return Ok(String::new());
    }
    let url = url.trim_end_matches('/');
    let ok = reqwest::Url::parse(url).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some()
            && u.query().is_none()
            && u.fragment().is_none()
    });
    if !ok {
        return Err("ListenBrainz: server URL must be http(s)://host[/path], without ? or #".into());
    }
    Ok(url.to_owned())
}

fn root_or_default(root: &str) -> String {
    if root.is_empty() { DEFAULT_ROOT } else { root }.to_owned()
}

const USER_AGENT: &str =
    concat!("Limusic/", env!("CARGO_PKG_VERSION"), " (https://github.com/SimoHypers/limusic)");

/// The `track_metadata` object for one listen. `video_id` feeds `origin_url` for YouTube tracks
/// (and picks the music service); local files carry no URL.
pub(crate) fn track_metadata(r: &Resolved, video_id: &str, duration: f64) -> serde_json::Value {
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

/// A failed submit. `problem` is set when ListenBrainz refused the listen for a reason only the
/// user can fix (see [`problem`]); anything else is a hiccup that stays a log line.
pub(crate) struct SubmitError {
    pub problem: Option<&'static str>,
    message: String,
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `root` is the `listenbrainz_url` the token was validated against (empty = official server).
pub(crate) async fn submit(
    root: &str,
    token: &str,
    listen_type: &str,
    payload: serde_json::Value,
) -> Result<(), SubmitError> {
    let body = serde_json::json!({ "listen_type": listen_type, "payload": payload });
    let resp = crate::http::client()
        .post(format!("{}/1/submit-listens", root_or_default(root)))
        .header("Authorization", format!("Token {token}"))
        .header("User-Agent", USER_AGENT)
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| SubmitError { problem: None, message: e.to_string() })?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let short: String = text.chars().take(300).collect();
        return Err(SubmitError {
            problem: problem(status, &text),
            message: format!("{status}: {short}"),
        });
    }
    Ok(())
}

/// Why ListenBrainz refuses this token's listens, as the code the UI words: `email` while the
/// MetaBrainz account's email is unverified, `rejected` for any other 401 (a token reset on the
/// website). It answers 401 for both, and `/1/validate-token` accepts both.
// ponytail: matched on the English body; if ListenBrainz rewords it, the email case reads as
// `rejected`, which still tells the user something is wrong.
fn problem(status: reqwest::StatusCode, body: &str) -> Option<&'static str> {
    (status == reqwest::StatusCode::UNAUTHORIZED).then(|| {
        if body.contains("verified email") {
            "email"
        } else {
            "rejected"
        }
    })
}

/// `listenbrainz-problem`: a [`problem`] code when ListenBrainz starts refusing listens, `null`
/// once one goes through again.
pub(crate) fn emit_problem(app: &tauri::AppHandle, problem: Option<&str>) {
    let _ = app.emit("listenbrainz-problem", problem);
}

async fn validate(token: &str, root: &str) -> Result<String, String> {
    let resp = crate::http::client()
        .get(format!("{}/1/validate-token", root_or_default(root)))
        .header("Authorization", format!("Token {token}"))
        .header("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    // A bad token is a 200 with `valid:false` (some proxies answer 401); anything else is not
    // this API talking.
    if !status.is_success() && status != reqwest::StatusCode::UNAUTHORIZED {
        return Err(format!(
            "{status}: ListenBrainz at {} did not accept the request",
            root_or_default(root)
        ));
    }
    let body: serde_json::Value = resp.json().await.map_err(|_| {
        format!(
            "{status}: no ListenBrainz API answered at {} (is this the API address?)",
            root_or_default(root)
        )
    })?;
    if body.get("valid").and_then(|v| v.as_bool()) != Some(true) {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("ListenBrainz rejected that token");
        return Err(msg.into());
    }
    body.get("user_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "ListenBrainz validated the token but named no user".to_string())
}

fn emit_state(
    app: &tauri::AppHandle,
    connected: bool,
    username: Option<&str>,
    error: Option<&str>,
) {
    let _ = app.emit(
        "listenbrainz-state",
        serde_json::json!({ "connected": connected, "username": username, "error": error }),
    );
}

/// Connect with a user token pasted from ListenBrainz settings. Validates it first, so a typo
/// is an error rather than a stored token that silently scrobbles nothing.
pub async fn connect(state: Arc<AppState>, token: String, url: String) -> Result<(), String> {
    let token = token.trim().to_owned();
    let url = clean_root(&url)?;
    let username = validate(&token, &url).await.map_err(|e| format!("ListenBrainz: {e}"))?;
    state.db.set_setting("listenbrainz_url", &url);
    state.db.set_setting("listenbrainz_token", &token);
    state.db.set_setting("listenbrainz_username", &username);
    // Same task, same clock: the Last.fm scrobbler holds this token next to its session key.
    state.lastfm.set_token(Some((url, token)));
    tracing::info!(user = %username, "listenbrainz connected");
    emit_state(&state.app, true, Some(&username), None);
    Ok(())
}

pub fn disconnect(state: &AppState) {
    state.db.set_setting("listenbrainz_token", "");
    state.db.set_setting("listenbrainz_username", "");
    state.db.set_setting("listenbrainz_url", "");
    state.lastfm.set_token(None);
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
    let root = root_or_default(&state.db.get_setting("listenbrainz_url").unwrap_or_default());
    // Listen counts are public: no token needed.
    let body: serde_json::Value = crate::http::client()
        .get(format!("{root}/1/user/{username}/listen-count"))
        .header("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let count = body.pointer("/payload/count").and_then(|v| v.as_u64()).unwrap_or(0);
    Some(Profile {
        username: username.clone(),
        // Custom instances serve the web UI from the same root as the API.
        url: if root == DEFAULT_ROOT {
            format!("https://listenbrainz.org/user/{username}/")
        } else {
            format!("{root}/user/{username}/")
        },
        listens: count,
    })
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Profile {
    pub username: String,
    pub url: String,
    pub listens: u64,
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

    /// One-shot stand-in server: answers `body` to the first request and returns that request's
    /// head (request line + headers).
    async fn stand_in(body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}/lb", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let n = sock.read(&mut buf).await.unwrap();
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            head
        });
        (root, task)
    }

    #[tokio::test]
    async fn calls_go_to_the_root_they_are_given() {
        let (root, req) = stand_in(r#"{"status":"ok"}"#).await;
        assert!(submit(&root, "tok-custom", "single", serde_json::json!([])).await.is_ok());
        let head = req.await.unwrap().to_lowercase();
        assert!(head.starts_with("post /lb/1/submit-listens "), "{head}");
        assert!(head.contains("authorization: token tok-custom"), "{head}");

        let (root, req) = stand_in(r#"{"valid":true,"user_name":"eve"}"#).await;
        assert_eq!(validate("tok-custom", &root).await.unwrap(), "eve");
        assert!(req.await.unwrap().starts_with("GET /lb/1/validate-token "));
    }

    #[test]
    fn custom_root_is_normalized_and_validated() {
        assert_eq!(clean_root(" https://lb.example.com/ ").unwrap(), "https://lb.example.com");
        assert_eq!(clean_root("").unwrap(), "");
        assert_eq!(clean_root("http://10.0.0.2:8080/lb/").unwrap(), "http://10.0.0.2:8080/lb");
        for bad in
            ["lb.example.com", "https://", "https://h/p?x=1", "https://h/#f", "ftp://h", "/", "//"]
        {
            assert!(clean_root(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_refused_listen_names_what_the_user_has_to_fix() {
        use reqwest::StatusCode;
        // Verbatim start of what ListenBrainz answered an unverified account (2026-10-10).
        let unverified = r#"{"code":401,"error":"The listens were rejected because your MetaBrainz account does not have a verified email address. Please check your inbox"#;
        assert_eq!(problem(StatusCode::UNAUTHORIZED, unverified), Some("email"));
        assert_eq!(problem(StatusCode::UNAUTHORIZED, r#"{"code":401}"#), Some("rejected"));
        // Not the user's to fix: stays a log line.
        assert_eq!(problem(StatusCode::BAD_REQUEST, unverified), None);
        assert_eq!(problem(StatusCode::SERVICE_UNAVAILABLE, ""), None);
    }
}
