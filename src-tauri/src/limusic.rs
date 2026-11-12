//! Limusic Lyrics accounts: the anonymous session, linking Discord or GitHub, renaming, and
//! submitting a lyric.
//!
//! The wire contract is the service's README ("Accounts and community lyrics"). Contributing is
//! anonymous by default: the first submit asks the service for an anonymous account and keeps its
//! session. Linking an identity to it is optional, and is a handoff: the app makes a random verifier, opens the browser on `/start?challenge=<sha256>`,
//! shows the user the challenge's first six characters, and polls `/token` with the verifier
//! until they have confirmed in the browser. The session token is kept the way Last.fm's key is,
//! in settings rows (`limusic_token`, `limusic_handle`, `limusic_anonymous`) the UI cannot read
//! back.
//!
//! The HTTP half takes the server root and a token and knows nothing of `AppState`; the half
//! below it is the glue that stores and emits.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Emitter;

use crate::lyrics::{LyricLine, Lyrics};
use crate::state::AppState;

const TIMEOUT: Duration = Duration::from_secs(8);
const POLL_EVERY: Duration = Duration::from_secs(2);
/// A sign-in lasts ten minutes from `/start`; one more minute so the last answer is the server's.
const POLL_FOR: Duration = Duration::from_secs(11 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// `None` while the account is anonymous: it has no public name.
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub anonymous: bool,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub joined_at: i64,
    /// `/v1/me` only.
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub trusted: bool,
    #[serde(default)]
    pub can_review: bool,
}

// --- HTTP ---------------------------------------------------------------------------------------

fn root() -> String {
    crate::lyrics::limusic_root().trim_end_matches('/').to_owned()
}

/// One request: the status and the JSON body (`Null` when there is none). `None` is a transport
/// failure, which every caller treats as "the service isn't reachable".
async fn send(rb: reqwest::RequestBuilder, token: Option<&str>) -> Option<(u16, Value)> {
    let mut rb = rb.header("User-Agent", crate::lyrics::LRCLIB_UA).timeout(TIMEOUT);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    let resp = rb.send().await.ok()?;
    let status = resp.status().as_u16();
    let body = resp.bytes().await.ok()?;
    Some((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

fn url(path: &str) -> String {
    format!("{}{path}", root())
}

/// The error code of a failed answer, for the UI to put in words. `unavailable` is anything that
/// isn't the service saying something specific (a 5xx, an unreadable body, no connection).
fn failure(answer: Option<(u16, Value)>) -> String {
    let Some((status, body)) = answer else { return "unavailable".into() };
    match (status, body["error"].as_str()) {
        (401, _) => "unauthorized".into(),
        (429, None) => "rate_limited".into(),
        (s, Some(code)) if (400..500).contains(&s) => code.to_owned(),
        _ => "unavailable".into(),
    }
}

/// What the service offers. `anonymous` is the gate for contributing at all: a service that
/// doesn't say it has anonymous accounts (an older one, a 404, no connection) isn't offered.
/// `providers` are the identities that can be linked, and may be empty.
#[derive(Debug, Serialize, PartialEq, Default)]
pub struct Providers {
    pub providers: Vec<String>,
    pub anonymous: bool,
}

fn parse_providers(body: &Value) -> Providers {
    let names = body["providers"].as_array().cloned().unwrap_or_default();
    Providers {
        providers: names
            .iter()
            .filter_map(Value::as_str)
            .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase()))
            .map(str::to_owned)
            .collect(),
        anonymous: body["anonymous"].as_bool() == Some(true),
    }
}

pub async fn providers() -> Providers {
    match send(crate::http::client().get(url("/v1/auth/providers")), None).await {
        Some((200, body)) => parse_providers(&body),
        _ => Providers::default(),
    }
}

/// 32 random bytes, base64url without padding: 43 characters.
fn new_verifier() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// What the user compares with the confirm page: the challenge's first six characters as XXXX-XX.
fn code(challenge: &str) -> String {
    let c = challenge.to_ascii_uppercase();
    format!("{}-{}", &c[..4], &c[4..6])
}

enum Polled {
    Waiting,
    Done(String, Account),
    Expired,
    Banned,
    Failed,
}

async fn poll(verifier: &str) -> Polled {
    let rb =
        crate::http::client().post(url("/v1/auth/token")).json(&json!({ "verifier": verifier }));
    match send(rb, None).await {
        // Not reachable, rate limited or a 5xx: the poll runs again in two seconds.
        None | Some((202 | 429 | 500..=599, _)) => Polled::Waiting,
        Some((200, body)) => {
            let token = body["token"].as_str().map(str::to_owned);
            let account = serde_json::from_value(body["account"].clone()).ok();
            match (token, account) {
                (Some(t), Some(a)) => Polled::Done(t, a),
                _ => Polled::Failed,
            }
        }
        Some((404, _)) => Polled::Expired,
        Some((403, _)) => Polled::Banned,
        Some(_) => Polled::Failed,
    }
}

async fn fetch_me(token: &str) -> Result<Account, String> {
    let answer = send(crate::http::client().get(url("/v1/me")), Some(token)).await;
    match answer {
        Some((200, body)) => serde_json::from_value(body).map_err(|_| "unavailable".into()),
        other => Err(failure(other)),
    }
}

/// The song's live revision on the service, and its lines when it has any. `None` when the song
/// has no live lyric or the service didn't answer: the editor then starts from the provider list.
pub struct Live {
    pub revision: i64,
    pub lyrics: Option<Lyrics>,
}

/// Whether a video id can go into a path as it is.
fn plausible_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() == 11
        && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        && !crate::local::is_local_song(id)
}

pub async fn live(video_id: &str) -> Option<Live> {
    if !plausible_id(video_id) {
        return None;
    }
    // `/state` is never cached, `/lyrics` is, for an hour: so the revision comes from `/state`
    // and the lines are asked for by it (`?r=` is ignored by the service and keys the edge cache
    // per revision), so the two can't disagree.
    let (200, state) =
        send(crate::http::client().get(url(&format!("/v1/lyrics/{video_id}/state"))), None).await?
    else {
        return None;
    };
    let revision = state["revision"].as_i64()?;
    let lyrics = async {
        let (200, body) = send(
            crate::http::client().get(url(&format!("/v1/lyrics/{video_id}?r={revision}"))),
            None,
        )
        .await?
        else {
            return None;
        };
        let answer: crate::lyrics::LimusicAnswer = serde_json::from_value(body).ok()?;
        crate::lyrics::limusic_to_lyrics(answer)
    }
    .await;
    Some(Live { revision, lyrics })
}

/// The song's live revision right now, `None` when it has no live lyric. For a draft that was
/// written on an older revision and is being kept on top of the new one.
pub async fn live_revision(video_id: &str) -> Result<Option<i64>, String> {
    if !plausible_id(video_id) {
        return Err("invalid".into());
    }
    let answer =
        send(crate::http::client().get(url(&format!("/v1/lyrics/{video_id}/state"))), None).await;
    match answer {
        Some((200, state)) => Ok(state["revision"].as_i64()),
        Some((404, _)) => Ok(None),
        other => Err(failure(other)),
    }
}

// --- Session ------------------------------------------------------------------------------------

/// Bumped by every sign-in, cancel and sign-out, so a poll that finds it changed stops.
static GEN: AtomicU64 = AtomicU64::new(0);

fn token(state: &AppState) -> Option<String> {
    state.db.get_setting("limusic_token").filter(|t| !t.is_empty())
}

fn is_anonymous(state: &AppState) -> bool {
    state.db.get_setting("limusic_anonymous").is_some_and(|v| v == "1")
}

fn store(state: &AppState, token: &str, handle: Option<&str>, anonymous: bool) {
    state.db.set_setting("limusic_token", token);
    state.db.set_setting("limusic_handle", handle.unwrap_or(""));
    state.db.set_setting("limusic_anonymous", if anonymous { "1" } else { "" });
}

fn clear(state: &AppState) {
    store(state, "", None, false);
}

/// `limusic-account`: the session changed (made, linked, signed in, or ended). The payload is
/// `status`, plus `error`, a code the UI words (`expired`, `banned`, `failed`, `session_ended`).
fn emit(state: &AppState, error: Option<&str>) {
    let mut v = status(state);
    v["error"] = error.into();
    let _ = state.app.emit("limusic-account", v);
}

/// The stored session, without asking the service. Anonymous accounts have no handle.
pub fn status(state: &AppState) -> Value {
    let handle = state.db.get_setting("limusic_handle").filter(|h| !h.is_empty());
    json!({ "signed_in": token(state).is_some(), "handle": handle, "anonymous": is_anonymous(state) })
}

/// `POST /v1/auth/anonymous`: the session token of a new anonymous account.
async fn post_anonymous() -> Result<String, String> {
    match send(crate::http::client().post(url("/v1/auth/anonymous")), None).await {
        Some((200, body)) => body["token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| "unavailable".into()),
        other => Err(failure(other)),
    }
}

/// A new anonymous account and its session, kept. `Err` is the service's code or `unavailable`.
async fn new_anonymous(state: &AppState) -> Result<String, String> {
    let token = post_anonymous().await?;
    store(state, &token, None, true);
    tracing::info!("limusic lyrics: anonymous account made");
    emit(state, None);
    Ok(token)
}

/// `POST /v1/auth/link`: set the sign-in about to start aside for the anonymous account.
async fn post_link(token: &str, challenge: &str) -> Option<(u16, Value)> {
    let rb =
        crate::http::client().post(url("/v1/auth/link")).json(&json!({ "challenge": challenge }));
    send(rb, Some(token)).await
}

/// Opens the browser and polls in the background; the answer arrives as `limusic-account`. Returns
/// the code the user should see on the confirm page. With an anonymous session the identity is
/// linked to that account (the service is told the challenge first); with none, signing in makes
/// a linked account. A linked session has nothing to link.
pub async fn sign_in(state: std::sync::Arc<AppState>, provider: String) -> Result<String, String> {
    if provider.is_empty() || !provider.chars().all(|c| c.is_ascii_lowercase()) {
        return Err("unknown provider".into());
    }
    let anonymous_token = token(&state).filter(|_| is_anonymous(&state));
    if token(&state).is_some() && anonymous_token.is_none() {
        return Err("already_linked".into());
    }
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let verifier = new_verifier();
    let challenge = challenge(&verifier);
    let shown = code(&challenge);
    if let Some(t) = anonymous_token {
        match post_link(&t, &challenge).await {
            Some((204, _)) => {}
            // The service has forgotten this anonymous account: sign in plainly instead.
            Some((401, _)) => lost(&state),
            other => return Err(failure(other)),
        }
    }
    crate::lastfm::open_browser(&url(&format!("/v1/auth/{provider}/start?challenge={challenge}")))?;

    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        while started.elapsed() < POLL_FOR {
            tokio::time::sleep(POLL_EVERY).await;
            if GEN.load(Ordering::SeqCst) != gen {
                return; // cancelled, signed out, or superseded by a newer sign-in
            }
            let polled = poll(&verifier).await;
            // A cancel or a sign-out can land while the request is in flight.
            if GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            match polled {
                Polled::Waiting => continue,
                Polled::Done(t, account) => {
                    store(&state, &t, account.handle.as_deref(), account.anonymous);
                    tracing::info!("limusic lyrics: signed in");
                    emit(&state, None);
                }
                Polled::Expired => emit(&state, Some("expired")),
                Polled::Banned => emit(&state, Some("banned")),
                Polled::Failed => emit(&state, Some("failed")),
            }
            return;
        }
        if GEN.load(Ordering::SeqCst) == gen {
            emit(&state, Some("expired"));
        }
    });
    Ok(shown)
}

/// Stops a sign-in that is waiting on the browser.
pub fn cancel() {
    GEN.fetch_add(1, Ordering::SeqCst);
}

/// Forgets the session here at once, and tells the service in the background.
pub fn sign_out(state: &AppState) {
    cancel();
    if let Some(t) = token(state) {
        tauri::async_runtime::spawn(async move {
            send(crate::http::client().post(url("/v1/auth/logout")), Some(&t)).await;
        });
    }
    clear(state);
    emit(state, None);
}

/// A 401 means the session is gone (expired, signed out elsewhere, banned): forget it too. An
/// anonymous one is forgotten without a word: the next submit makes another.
fn lost(state: &AppState) {
    let anonymous = is_anonymous(state);
    clear(state);
    emit(state, (!anonymous).then_some("session_ended"));
}

/// The signed-in account from the service. `Ok(None)` when nobody is signed in or the session
/// turned out to be gone; `Err` when the service couldn't be asked.
pub async fn me(state: &AppState) -> Result<Option<Account>, String> {
    let Some(t) = token(state) else { return Ok(None) };
    match fetch_me(&t).await {
        Ok(a) => {
            store(state, &t, a.handle.as_deref(), a.anonymous);
            Ok(Some(a))
        }
        Err(c) if c == "unauthorized" => {
            lost(state);
            Ok(None)
        }
        Err(c) => Err(c),
    }
}

async fn patch_handle(token: &str, handle: &str) -> Result<Account, String> {
    let rb = crate::http::client().patch(url("/v1/me")).json(&json!({ "handle": handle }));
    match send(rb, Some(token)).await {
        Some((200, body)) => serde_json::from_value(body).map_err(|_| "unavailable".into()),
        other => Err(failure(other)),
    }
}

/// `Err` carries the service's code: `taken`, `invalid`, `rate_limited`, `unauthorized`, ...
pub async fn rename(state: &AppState, handle: String) -> Result<Account, String> {
    let t = token(state).filter(|_| !is_anonymous(state)).ok_or("unauthorized")?;
    match patch_handle(&t, &handle).await {
        Ok(account) => {
            state.db.set_setting("limusic_handle", account.handle.as_deref().unwrap_or(""));
            Ok(account)
        }
        Err(code) => {
            if code == "unauthorized" {
                lost(state);
            }
            Err(code)
        }
    }
}

// --- Submitting ---------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct Submission {
    pub video_id: String,
    pub lines: Vec<LyricLine>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    pub base_revision: Option<i64>,
}

/// What the service made of a submission. `Failed` carries its error code (`unauthorized`,
/// `banned`, `invalid` with `detail`, `stale`, `unchanged`, `rate_limited`) or `unavailable`.
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Submitted {
    Published { revision: i64 },
    Pending { revision: i64 },
    Failed { code: String, detail: Option<String> },
}

fn failed(code: &str) -> Submitted {
    Submitted::Failed { code: code.into(), detail: None }
}

fn outcome(answer: Option<(u16, Value)>) -> Submitted {
    if let Some((201, body)) = &answer {
        if let Some(revision) = body["revision"].as_i64() {
            return match body["status"].as_str() {
                Some("published") => Submitted::Published { revision },
                _ => Submitted::Pending { revision },
            };
        }
    }
    let detail = answer.as_ref().and_then(|(_, b)| b["detail"].as_str()).map(str::to_owned);
    Submitted::Failed { code: failure(answer), detail }
}

async fn post_revision(token: &str, s: &Submission) -> Submitted {
    let mut body = json!({
        "lines": s.lines,
        "title": s.title.trim(),
        "artist": s.artist.trim(),
        "base_revision": s.base_revision,
    });
    if let Some(album) = s.album.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        body["album"] = album.into();
    }
    if let Some(d) = s.duration.filter(|d| d.is_finite() && *d > 0.0) {
        body["duration"] = d.round().into();
    }
    let rb = crate::http::client()
        .post(url(&format!("/v1/lyrics/{}/revisions", s.video_id)))
        .json(&body);
    outcome(send(rb, Some(token)).await)
}

pub async fn submit(state: &AppState, s: Submission) -> Submitted {
    // The id goes into a path that carries the bearer token.
    if !plausible_id(&s.video_id) {
        return failed("invalid");
    }
    // No session yet: the first submit makes an anonymous account, silently.
    let (t, fresh) = match token(state) {
        Some(t) => (t, false),
        None => match new_anonymous(state).await {
            Ok(t) => (t, true),
            Err(code) => return failed(&code),
        },
    };
    let mut result = post_revision(&t, &s).await;
    if result == failed("unauthorized") {
        // The service doesn't know the session any more. An anonymous account is replaced once and
        // the submit retried; a linked one is the user's to sign in to again.
        let anonymous = is_anonymous(state);
        lost(state);
        if anonymous && !fresh {
            if let Ok(t) = new_anonymous(state).await {
                result = post_revision(&t, &s).await;
            }
        }
    }
    if matches!(result, Submitted::Published { .. }) {
        // The lyric is live: the next look at this song shows it, not the old cached one.
        let answer = crate::lyrics::LimusicAnswer { origin: "community".into(), lines: s.lines };
        if let Some(mut l) = crate::lyrics::limusic_to_lyrics(answer) {
            l.provider = "limusic".into();
            if let Ok(json) = serde_json::to_string(&l) {
                state.db.put_lyrics(&s.video_id, Some(&json), crate::db::now_secs());
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_the_rfc_7636_one_and_the_code_is_its_first_six() {
        // RFC 7636 appendix B.
        let c = challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(c, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(code(&c), "E9ME-LH");
    }

    #[test]
    fn a_verifier_is_43_url_safe_characters_and_never_repeats() {
        let v = new_verifier();
        assert_eq!(v.len(), 43);
        assert!(v.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)));
        assert_ne!(v, new_verifier());
        assert_eq!(challenge(&v).len(), 43);
    }

    #[test]
    fn errors_become_the_services_code_or_unavailable() {
        let a = |s, b: Value| Some((s, b));
        assert_eq!(failure(a(401, json!({"error": "unauthorized"}))), "unauthorized");
        assert_eq!(failure(a(409, json!({"error": "stale"}))), "stale");
        assert_eq!(failure(a(409, json!({"error": "taken"}))), "taken");
        assert_eq!(failure(a(403, json!({"error": "banned"}))), "banned");
        assert_eq!(failure(a(429, json!({"error": "rate_limited"}))), "rate_limited");
        assert_eq!(failure(a(429, Value::Null)), "rate_limited");
        assert_eq!(failure(a(502, Value::Null)), "unavailable");
        assert_eq!(failure(a(500, json!({"error": "stale"}))), "unavailable");
        assert_eq!(failure(None), "unavailable");
    }

    #[test]
    fn every_documented_submit_answer_has_an_outcome() {
        let published = outcome(Some((201, json!({"revision": 7, "status": "published"}))));
        assert_eq!(published, Submitted::Published { revision: 7 });
        let pending = outcome(Some((201, json!({"revision": 8, "status": "pending"}))));
        assert_eq!(pending, Submitted::Pending { revision: 8 });
        let invalid =
            outcome(Some((400, json!({"error": "invalid", "detail": "line 3 is empty"}))));
        assert_eq!(
            invalid,
            Submitted::Failed { code: "invalid".into(), detail: Some("line 3 is empty".into()) }
        );
        for (status, code) in
            [(409, "stale"), (409, "unchanged"), (403, "banned"), (429, "rate_limited")]
        {
            assert_eq!(outcome(Some((status, json!({ "error": code })))), failed(code));
        }
        assert_eq!(outcome(Some((401, json!({"error": "unauthorized"})))), failed("unauthorized"));
        assert_eq!(outcome(Some((201, json!({"nope": 1})))), failed("unavailable"));
        assert_eq!(outcome(None), failed("unavailable"));
    }

    /// Against a local copy of the service with hand-inserted sessions, never the live one:
    ///   LIMUSIC_LYRICS_URL=http://127.0.0.1:8099 LIMUSIC_TEST_USER=<token of a user>
    ///   LIMUSIC_TEST_ADMIN=<token of an admin> cargo test -p limusic-app --lib \
    ///   local_service -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs a local lyrics server"]
    async fn local_service() {
        let user = std::env::var("LIMUSIC_TEST_USER").unwrap();
        let admin = std::env::var("LIMUSIC_TEST_ADMIN").unwrap();
        let offered = providers().await;
        assert_eq!(
            (offered.providers.as_slice(), offered.anonymous),
            (&["github".to_string()][..], true)
        );
        assert!(matches!(poll(&new_verifier()).await, Polled::Expired));
        assert_eq!(fetch_me(&user).await.unwrap().role, "user");
        assert_eq!(patch_handle(&user, "ab").await.unwrap_err(), "invalid");
        assert_eq!(patch_handle(&user, "tester_admin").await.unwrap_err(), "taken");
        assert_eq!(
            patch_handle(&user, "Renamed_1").await.unwrap().handle.as_deref(),
            Some("Renamed_1")
        );
        assert_eq!(patch_handle("nope", "Renamed_2").await.unwrap_err(), "unauthorized");
        assert_eq!(fetch_me("nope").await.unwrap_err(), "unauthorized");

        let word = |t: &str, a, b| crate::lyrics::LyricWord {
            text: t.into(),
            start_ms: a,
            end_ms: b,
            bg: false,
        };
        let line = |t: u64, w: &[&str]| LyricLine {
            time_ms: Some(t),
            end_time_ms: Some(t + 900),
            text: w.join(" "),
            words: Some(
                w.iter()
                    .enumerate()
                    .map(|(i, x)| word(x, t + i as u64 * 400, t + i as u64 * 400 + 380))
                    .collect(),
            ),
            agent: Some("v1".into()),
            ..Default::default()
        };
        let sub = |lines, base| Submission {
            video_id: "AAAAAAAAAAA".into(),
            lines,
            title: " A song ".into(),
            artist: "An artist".into(),
            album: None,
            duration: Some(200.4),
            base_revision: base,
        };
        let two = || vec![line(1000, &["hello", "there"]), line(3000, &["general", "kenobi"])];

        assert!(live("AAAAAAAAAAA").await.is_none(), "no lyric yet");
        // A user is not trusted yet: pending.
        let first = post_revision(&user, &sub(two(), None)).await;
        assert!(matches!(first, Submitted::Pending { .. }), "{first:?}");
        assert!(live("AAAAAAAAAAA").await.is_none(), "pending is not live");
        let again = post_revision(&user, &sub(two(), None)).await;
        assert!(matches!(again, Submitted::Pending { .. }), "{again:?}");
        // An admin publishes at once.
        let done = post_revision(&admin, &sub(two(), None)).await;
        let Submitted::Published { revision } = done else { panic!("{done:?}") };
        let l = live("AAAAAAAAAAA").await.expect("live now");
        assert_eq!(l.revision, revision);
        let lyr = l.lyrics.unwrap();
        assert_eq!(lyr.lines.len(), 2);
        assert_eq!(lyr.lines[0].agent.as_deref(), Some("v1"));
        // Same lines again, then a change against the old base.
        assert_eq!(post_revision(&admin, &sub(two(), Some(revision))).await, failed("unchanged"));
        let mut changed = two();
        changed[1] = line(3000, &["general", "grievous"]);
        assert_eq!(post_revision(&admin, &sub(changed.clone(), None)).await, failed("stale"));
        assert!(matches!(
            post_revision(&admin, &sub(changed, Some(revision))).await,
            Submitted::Published { .. }
        ));
        // Broken input and a bad session.
        let mut flat = two();
        flat[0].words = Some(vec![]);
        flat[1].words = Some(vec![]);
        let bad = post_revision(&admin, &sub(flat, Some(revision + 1))).await;
        let Submitted::Failed { code, detail } = bad else { panic!() };
        assert_eq!(code, "invalid");
        println!("invalid detail: {detail:?}");
        assert_eq!(post_revision("nope", &sub(two(), None)).await, failed("unauthorized"));
    }

    /// Same local service: the anonymous path, as the app takes it.
    #[tokio::test]
    #[ignore = "needs a local lyrics server"]
    async fn local_service_anonymous() {
        let anon = post_anonymous().await.unwrap();
        let me = fetch_me(&anon).await.unwrap();
        assert!(me.anonymous && me.handle.is_none() && !me.trusted && !me.can_review);
        assert_eq!(patch_handle(&anon, "someone").await.unwrap_err(), "forbidden");
        let line = LyricLine {
            time_ms: Some(1000),
            end_time_ms: Some(1900),
            text: "hello there".into(),
            words: Some(vec![
                crate::lyrics::LyricWord {
                    text: "hello".into(),
                    start_ms: 1000,
                    end_ms: 1400,
                    bg: false,
                },
                crate::lyrics::LyricWord {
                    text: "there".into(),
                    start_ms: 1400,
                    end_ms: 1900,
                    bg: false,
                },
            ]),
            ..Default::default()
        };
        let sub = Submission {
            video_id: "BBBBBBBBBBB".into(),
            lines: vec![line],
            title: "A song".into(),
            artist: "An artist".into(),
            album: None,
            duration: Some(200.0),
            base_revision: None,
        };
        let sent = post_revision(&anon, &sub).await;
        assert!(matches!(sent, Submitted::Pending { .. }), "an anonymous submit waits: {sent:?}");
        assert_eq!(live_revision("BBBBBBBBBBB").await.unwrap(), None, "pending is not live");
        let challenge = challenge(&new_verifier());
        assert_eq!(post_link(&anon, &challenge).await.map(|a| a.0), Some(204));
        assert_eq!(post_link(&anon, &challenge).await.map(|a| a.0), Some(409), "once");
        assert_eq!(post_link("nope", &new_verifier()).await.map(|a| a.0), Some(401));
    }

    #[test]
    fn the_service_is_offered_only_when_it_says_it_has_anonymous_accounts() {
        let both = parse_providers(&json!({"providers": ["discord", "github"], "anonymous": true}));
        assert_eq!(
            both,
            Providers { providers: vec!["discord".into(), "github".into()], anonymous: true }
        );
        let none = parse_providers(&json!({"providers": [], "anonymous": true}));
        assert!(none.anonymous && none.providers.is_empty(), "anonymous works with no provider");
        // A service from before anonymous accounts: providers, but no flag.
        assert!(!parse_providers(&json!({"providers": ["discord"]})).anonymous);
        assert!(!parse_providers(&json!({"providers": ["x"], "anonymous": "yes"})).anonymous);
        assert!(!parse_providers(&Value::Null).anonymous);
        // Odd names are dropped, not shown.
        assert!(parse_providers(&json!({"providers": ["Bad Name", ""], "anonymous": true}))
            .providers
            .is_empty());
    }

    #[test]
    fn an_account_from_the_service_may_have_no_handle() {
        let a: Account = serde_json::from_value(
            json!({"handle": null, "anonymous": true, "score": 0, "joined_at": 1}),
        )
        .unwrap();
        assert!(a.anonymous && a.handle.is_none());
        let b: Account = serde_json::from_value(
            json!({"handle": "carla", "anonymous": false, "avatar_url": null}),
        )
        .unwrap();
        assert_eq!(b.handle.as_deref(), Some("carla"));
        // An answer without the flag (an older service) reads as linked.
        let c: Account = serde_json::from_value(json!({"handle": "x"})).unwrap();
        assert!(!c.anonymous);
    }

    #[test]
    fn only_a_youtube_video_id_goes_into_a_path() {
        assert!(plausible_id("FkxEE7wZ83c"));
        assert!(!plausible_id(""));
        assert!(!plausible_id("a/../b"));
        assert!(!plausible_id("FkxEE7wZ83cX"));
    }
}
