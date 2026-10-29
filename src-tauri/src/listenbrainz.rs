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
//! The API root is configurable (`listenbrainz_api_root`): any server that speaks the
//! ListenBrainz API works, e.g. a self-hosted [Koito](https://koito.io) at
//! `https://koito.example.com/apis/listenbrainz`. [`normalize_api_root`] makes a pasted host,
//! a full listen URL, or the official root all land on the same canonical base the two calls
//! above append `/1/…` to.
//!
//! Everything is best-effort, like Last.fm: a failed listen is a `debug!` line, never a
//! user-facing error. No offline queue.

use std::sync::Arc;
use std::time::Duration;

use tauri::Emitter;

use crate::db::Db;
use crate::lastfm::Resolved;
use crate::state::AppState;

/// The official service; what an empty/unset setting means.
pub(crate) const DEFAULT_API_ROOT: &str = "https://api.listenbrainz.org";
const API_ROOT_SETTING: &str = "listenbrainz_api_root";
const USER_AGENT: &str =
    concat!("Limusic/", env!("CARGO_PKG_VERSION"), " (https://github.com/SimoHypers/limusic)");

/// A connected listen target. The scrobbler needs both halves together: a token with no root
/// would silently go to the official service, and a root with no token has nowhere to send.
#[derive(Debug, Clone)]
pub(crate) struct Credentials {
    pub token: String,
    pub api_root: String,
}

/// Canonical API base for a user-supplied server URL.
///
/// Accepts what people actually paste: a bare host (`koito.example.com`), the official root,
/// a Koito root (`…/apis/listenbrainz`), or a full endpoint copied from API docs
/// (`…/apis/listenbrainz/1/submit-listens`). The returned value never ends in `/` or `/1`, so
/// callers append `/1/validate-token` and `/1/submit-listens` exactly once.
pub(crate) fn normalize_api_root(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(DEFAULT_API_ROOT.to_string());
    }
    // A bare host is the common case ("koito.example.com/apis/listenbrainz"); default it to
    // https rather than failing with a parse error that does not say what to do.
    let with_scheme =
        if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let mut url = reqwest::Url::parse(&with_scheme)
        .map_err(|_| format!("That server URL is not valid: {input}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("The server URL must start with http:// or https://".into());
    }
    if url.host_str().is_none() {
        return Err("The server URL needs a host, for example https://api.listenbrainz.org".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Put credentials in the token field, not in the server URL".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("The server URL cannot carry a query string or fragment".into());
    }
    // Both the official service and Koito expose their API under `/1`, so a pasted `/1` (or a
    // full endpoint) is the same base as the host + prefix above it.
    let mut path = url.path().trim_end_matches('/').to_string();
    for suffix in ["/1/submit-listens", "/1/validate-token", "/1"] {
        if let Some(stripped) = path.strip_suffix(suffix) {
            path = stripped.trim_end_matches('/').to_string();
            break;
        }
    }
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// The stored API base, normalized. A value that was valid when stored stays valid; anything
/// unreadable falls back to the official service instead of wedging scrobbling.
pub(crate) fn api_root(db: &Db) -> String {
    db.get_setting(API_ROOT_SETTING)
        .and_then(|s| normalize_api_root(&s).ok())
        .unwrap_or_else(|| DEFAULT_API_ROOT.to_string())
}

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

pub(crate) async fn submit(
    api_root: &str,
    token: &str,
    listen_type: &str,
    payload: serde_json::Value,
) -> Result<(), SubmitError> {
    let body = serde_json::json!({ "listen_type": listen_type, "payload": payload });
    let resp = crate::http::client()
        .post(format!("{api_root}/1/submit-listens"))
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

async fn validate(api_root: &str, token: &str) -> Result<String, String> {
    let resp = crate::http::client()
        .get(format!("{api_root}/1/validate-token"))
        .header("Authorization", format!("Token {token}"))
        .header("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    // Self-hosted servers sometimes answer errors as plain text; keep that text rather than
    // surfacing a JSON decode error the user can do nothing with.
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let body: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("{status}: {}", text.chars().take(300).collect::<String>()))?;
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
    server: &str,
    error: Option<&str>,
) {
    let _ = app.emit(
        "listenbrainz-state",
        serde_json::json!({
            "connected": connected,
            "username": username,
            "server": server,
            "error": error,
        }),
    );
}

/// Connect with a user token. `api_root` is the server to send to: empty means the official
/// ListenBrainz, anything else must speak the same API (Koito, Maloja's LB endpoint, …). The
/// token is validated against that server first, so a typo or a wrong URL is an error rather
/// than a stored credential that silently scrobbles nowhere.
pub async fn connect(
    state: Arc<AppState>,
    token: String,
    api_root: Option<String>,
) -> Result<(), String> {
    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err("Paste a ListenBrainz user token first".into());
    }
    let api_root = normalize_api_root(api_root.as_deref().unwrap_or_default())?;
    let username = validate(&api_root, &token).await.map_err(|e| format!("ListenBrainz: {e}"))?;
    state.db.set_setting("listenbrainz_token", &token);
    state.db.set_setting("listenbrainz_username", &username);
    state.db.set_setting(API_ROOT_SETTING, &api_root);
    // Same task, same clock: the Last.fm scrobbler holds this credential next to its session key.
    state
        .lastfm
        .set_listenbrainz(Some(Credentials { token: token.clone(), api_root: api_root.clone() }));
    tracing::info!(user = %username, server = %api_root, "listenbrainz connected");
    emit_state(&state.app, true, Some(&username), &api_root, None);
    Ok(())
}

pub fn disconnect(state: &AppState) {
    state.db.set_setting("listenbrainz_token", "");
    state.db.set_setting("listenbrainz_username", "");
    // The server URL stays: reconnecting to the same self-hosted instance should not mean
    // typing it again. It is not a credential, and the UI prefills it from here.
    state.lastfm.set_listenbrainz(None);
    emit_state(&state.app, false, None, &api_root(&state.db), None);
}

pub fn status(state: &AppState) -> serde_json::Value {
    let token = state.db.get_setting("listenbrainz_token").filter(|s| !s.is_empty());
    let username = state.db.get_setting("listenbrainz_username").filter(|s| !s.is_empty());
    serde_json::json!({
        "connected": token.is_some(),
        "username": username,
        "server": api_root(&state.db),
    })
}

/// The connected user's listen count, for the Scrobbling tab's account card. `None` when nobody
/// is connected. A server that does not implement the public listen-count endpoint (Koito, for
/// one) leaves the count empty; the account still reads as connected.
pub async fn profile(state: &AppState) -> Option<Profile> {
    let username = state.db.get_setting("listenbrainz_username").filter(|s| !s.is_empty())?;
    let root = api_root(&state.db);
    // Listen counts are public: no token needed.
    let count = match crate::http::client()
        .get(format!("{root}/1/user/{username}/listen-count"))
        .header("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(15))
        .send()
        .await
    {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(body) => body.pointer("/payload/count").and_then(|v| v.as_u64()),
            Err(_) => None,
        },
        Err(_) => None,
    };
    // The link only exists on the official site; a self-hosted server has no per-user page.
    let url =
        (root == DEFAULT_API_ROOT).then(|| format!("https://listenbrainz.org/user/{username}/"));
    Some(Profile { username: username.clone(), url, listens: count })
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Profile {
    pub username: String,
    pub url: Option<String>,
    pub listens: Option<u64>,
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

    #[test]
    fn the_official_server_is_what_an_empty_setting_means() {
        assert_eq!(normalize_api_root("").unwrap(), DEFAULT_API_ROOT);
        assert_eq!(normalize_api_root("   ").unwrap(), DEFAULT_API_ROOT);
        assert_eq!(normalize_api_root("https://api.listenbrainz.org/").unwrap(), DEFAULT_API_ROOT);
        assert_eq!(normalize_api_root("https://api.listenbrainz.org/1").unwrap(), DEFAULT_API_ROOT);
    }

    #[test]
    fn a_self_hosted_server_keeps_its_path_without_a_trailing_slash_or_version() {
        // Koito's API lives under `/apis/listenbrainz`, and callers append `/1/...` to it.
        for pasted in [
            "https://koito.example.com/apis/listenbrainz",
            "https://koito.example.com/apis/listenbrainz/",
            "https://koito.example.com/apis/listenbrainz/1",
            "https://koito.example.com/apis/listenbrainz/1/submit-listens",
            "https://koito.example.com/apis/listenbrainz/1/validate-token",
            "  https://koito.example.com/apis/listenbrainz/1  ",
        ] {
            assert_eq!(
                normalize_api_root(pasted).unwrap(),
                "https://koito.example.com/apis/listenbrainz",
                "{pasted}"
            );
        }
    }

    #[test]
    fn a_bare_host_defaults_to_https_and_keeps_an_explicit_http_port() {
        assert_eq!(
            normalize_api_root("koito.example.com/apis/listenbrainz").unwrap(),
            "https://koito.example.com/apis/listenbrainz"
        );
        assert_eq!(
            normalize_api_root("http://localhost:4110/apis/listenbrainz").unwrap(),
            "http://localhost:4110/apis/listenbrainz"
        );
    }

    #[test]
    fn a_server_url_that_could_leak_or_go_somewhere_else_is_refused() {
        assert!(normalize_api_root("ftp://example.com").is_err());
        assert!(normalize_api_root("https://user:pass@example.com").is_err());
        assert!(normalize_api_root("https://example.com/?token=x").is_err());
        assert!(normalize_api_root("https://example.com/#frag").is_err());
        assert!(normalize_api_root("not a url").is_err());
    }
}
