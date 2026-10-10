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

const API_ROOT: &str = "https://api.listenbrainz.org";
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

pub(crate) async fn submit(
    token: &str,
    listen_type: &str,
    payload: serde_json::Value,
) -> Result<(), String> {
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
pub async fn connect(state: Arc<AppState>, token: String) -> Result<(), String> {
    let token = token.trim().to_owned();
    let username = validate(&token).await.map_err(|e| format!("ListenBrainz: {e}"))?;
    state.db.set_setting("listenbrainz_token", &token);
    state.db.set_setting("listenbrainz_username", &username);
    // Same task, same clock: the Last.fm scrobbler holds this token next to its session key.
    state.lastfm.set_token(Some(token));
    tracing::info!(user = %username, "listenbrainz connected");
    emit_state(&state.app, true, Some(&username), None);
    Ok(())
}

pub fn disconnect(state: &AppState) {
    state.db.set_setting("listenbrainz_token", "");
    state.db.set_setting("listenbrainz_username", "");
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
    // Listen counts are public: no token needed.
    let body: serde_json::Value = crate::http::client()
        .get(format!("{API_ROOT}/1/user/{username}/listen-count"))
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
