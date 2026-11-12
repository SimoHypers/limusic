//! Limusic Lyrics accounts: the anonymous session, linking Discord or GitHub, renaming,
//! submitting a lyric, and voting on or reporting the one that is live.
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
            Some((401, _)) => lost(&state, &t),
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
/// anonymous one is forgotten without a word: the next submit makes another. `was` is the token
/// the 401 answered: a session signed out or replaced while the request was away is left alone.
fn lost(state: &AppState, was: &str) {
    if token(state).as_deref() != Some(was) {
        return;
    }
    let anonymous = is_anonymous(state);
    clear(state);
    emit(state, (!anonymous).then_some("session_ended"));
}

/// The signed-in account from the service. `Ok(None)` when nobody is signed in or the session
/// turned out to be gone; `Err` when the service couldn't be asked.
pub async fn me(state: &AppState) -> Result<Option<Account>, String> {
    let Some(t) = token(state) else { return Ok(None) };
    match fetch_me(&t).await {
        // Signed out while the answer was on its way: storing it would bring the session back.
        Ok(_) if token(state).as_deref() != Some(t.as_str()) => Ok(None),
        Ok(a) => {
            store(state, &t, a.handle.as_deref(), a.anonymous);
            Ok(Some(a))
        }
        Err(c) if c == "unauthorized" => {
            lost(state, &t);
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
                lost(state, &t);
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
        lost(state, &t);
        if anonymous && !fresh {
            if let Ok(t) = new_anonymous(state).await {
                result = post_revision(&t, &s).await;
            }
        }
    }
    if let Submitted::Published { revision } = &result {
        // The lyric is live: the next look at this song shows it, not the old cached one.
        let answer = crate::lyrics::LimusicAnswer {
            origin: "community".into(),
            revision: Some(*revision),
            lines: s.lines,
            ..Default::default()
        };
        if let Some(mut l) = crate::lyrics::limusic_to_lyrics(answer) {
            l.provider = "limusic".into();
            if let Ok(json) = serde_json::to_string(&l) {
                state.db.put_lyrics(&s.video_id, Some(&json), crate::db::now_secs());
            }
        }
    }
    result
}

// --- Votes and reports --------------------------------------------------------------------------
// On the song's live revision, mirrored ones included. Like a submit, the first one makes an
// anonymous account silently.

/// What the source picker shows for the lyric Limusic has live.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LyricState {
    pub revision: i64,
    pub up: i64,
    pub down: i64,
    /// -1, 0 or 1.
    pub my_vote: i64,
    /// The live revision is this account's work: the service refuses a vote on it, so none is
    /// offered.
    pub own: bool,
}

/// `/state`'s answer. `None` when the song has no live lyric (only something of ours pending).
fn parse_state(body: &Value) -> Option<LyricState> {
    Some(LyricState {
        revision: body["revision"].as_i64()?,
        up: body["votes"]["up"].as_i64().unwrap_or(0).max(0),
        down: body["votes"]["down"].as_i64().unwrap_or(0).max(0),
        my_vote: body["my_vote"].as_i64().unwrap_or(0).clamp(-1, 1),
        own: false,
    })
}

/// Whether the live revision is the caller's, from `GET /v1/revisions/{id}` with their session.
/// The service shows a revision in full to its author and to reviewers only. An anonymous
/// account can never review, so being shown it at all means having written it; a linked one may
/// be a reviewer, so it is theirs when it carries their name.
fn is_own(anonymous: bool, handle: Option<&str>, answer: &Option<(u16, Value)>) -> bool {
    let Some((200, body)) = answer else { return false };
    anonymous
        || matches!((handle, body["author"].as_str()), (Some(h), Some(a)) if h.eq_ignore_ascii_case(a))
}

/// The live lyric's votes, this account's own vote, and whether the lyric is its own work.
/// `Ok(None)` when the song has no live lyric. Works with no session: the counts need none.
pub async fn lyric_state(state: &AppState, video_id: &str) -> Result<Option<LyricState>, String> {
    if !plausible_id(video_id) {
        return Err("invalid".into());
    }
    let ask = |t: Option<String>| async move {
        let rb = crate::http::client().get(url(&format!("/v1/lyrics/{video_id}/state")));
        send(rb, t.as_deref()).await
    };
    let mut session = token(state);
    let mut answer = ask(session.clone()).await;
    if let (Some((401, _)), Some(t)) = (&answer, &session) {
        // The service refuses a dead token even where one is optional: ask again as nobody.
        lost(state, t);
        session = None;
        answer = ask(None).await;
    }
    let mut found = match answer {
        Some((200, body)) => match parse_state(&body) {
            Some(s) => s,
            None => return Ok(None),
        },
        Some((404, _)) => return Ok(None),
        other => return Err(failure(other)),
    };
    if let Some(t) = session {
        let rb = crate::http::client().get(url(&format!("/v1/revisions/{}", found.revision)));
        let handle = state.db.get_setting("limusic_handle").filter(|h| !h.is_empty());
        found.own = is_own(is_anonymous(state), handle.as_deref(), &send(rb, Some(&t)).await);
    }
    Ok(Some(found))
}

/// A write as a contributor: with the session there is, else with an anonymous account made for
/// it. When the service no longer knows an anonymous session, another is made and the write is
/// tried once more; a linked one that is gone is the user's to sign in to again.
async fn as_contributor<T, F, Fut>(state: &AppState, call: F) -> Result<T, String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let (t, fresh) = match token(state) {
        Some(t) => (t, false),
        None => (new_anonymous(state).await?, true),
    };
    let result = call(t.clone()).await;
    if !matches!(&result, Err(c) if c == "unauthorized") {
        return result;
    }
    let anonymous = is_anonymous(state);
    lost(state, &t);
    if anonymous && !fresh {
        return call(new_anonymous(state).await?).await;
    }
    result
}

/// What a vote left behind, for the two counts and the pressed thumb.
#[derive(Debug, Serialize, PartialEq)]
pub struct Votes {
    pub up: i64,
    pub down: i64,
    pub my_vote: i64,
}

async fn put_vote(token: &str, video_id: &str, value: i64) -> Result<Votes, String> {
    let rb = crate::http::client()
        .put(url(&format!("/v1/lyrics/{video_id}/vote")))
        .json(&json!({ "value": value }));
    let body = expect(send(rb, Some(token)).await, 200)?;
    Ok(Votes {
        up: body["up"].as_i64().unwrap_or(0).max(0),
        down: body["down"].as_i64().unwrap_or(0).max(0),
        my_vote: body["my_vote"].as_i64().unwrap_or(value).clamp(-1, 1),
    })
}

/// 1 up, -1 down, 0 takes the vote back. `Err` is the service's code: `own`, `not_found` (no
/// live lyric), `rate_limited`, `banned`, or `unavailable`.
pub async fn vote(state: &AppState, video_id: &str, value: i64) -> Result<Votes, String> {
    if !(-1..=1).contains(&value) || !plausible_id(video_id) {
        return Err("invalid".into());
    }
    as_contributor(state, |t| async move { put_vote(&t, video_id, value).await }).await
}

/// The reasons the service takes.
const REPORT_REASONS: [&str; 4] = ["wrong_song", "timing", "text", "abuse"];

async fn post_report(
    token: &str,
    video_id: &str,
    reason: &str,
    note: Option<&str>,
) -> Result<(), String> {
    let mut body = json!({ "reason": reason });
    if let Some(n) = clip_reason(note) {
        body["note"] = n.into();
    }
    let rb = crate::http::client().post(url(&format!("/v1/lyrics/{video_id}/reports"))).json(&body);
    expect(send(rb, Some(token)).await, 202).map(|_| ())
}

/// Report the live lyric. One per account and revision: a repeat is accepted and changes
/// nothing. `Err` is the service's code, as for a vote.
pub async fn report(
    state: &AppState,
    video_id: &str,
    reason: &str,
    note: Option<String>,
) -> Result<(), String> {
    if !REPORT_REASONS.contains(&reason) || !plausible_id(video_id) {
        return Err("invalid".into());
    }
    let note = note.as_deref();
    as_contributor(state, |t| async move { post_report(&t, video_id, reason, note).await }).await
}

// --- Review -------------------------------------------------------------------------------------
// For an account `/v1/me` says `can_review`. Every call carries the session; a 401 forgets it.

/// One waiting revision, as the queue lists it. `kind` is `new`, `edit` or `reported` (a live
/// lyric with enough open reports). `author` is absent for an anonymous sender.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct QueueItem {
    pub revision: i64,
    pub video_id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub author: Option<String>,
    pub kind: String,
    pub created_at: i64,
    #[serde(default)]
    pub reports: i64,
}

/// Oldest first. `next` is the `after` of the following page, `None` on the last.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Queue {
    pub items: Vec<QueueItem>,
    #[serde(default)]
    pub next: Option<i64>,
}

/// One revision with its lines, ready for the lyrics view.
#[derive(Debug, Serialize)]
pub struct Revision {
    pub revision: i64,
    pub video_id: String,
    pub status: String,
    pub kind: String,
    pub author: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Option<f64>,
    pub base_revision: Option<i64>,
    pub note: Option<String>,
    pub created_at: i64,
    pub lyrics: Option<Lyrics>,
}

#[derive(Deserialize)]
struct RawRevision {
    revision: i64,
    video_id: String,
    status: String,
    kind: String,
    #[serde(default)]
    author: Option<String>,
    lines: Vec<LyricLine>,
    title: String,
    artist: String,
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    base_revision: Option<i64>,
    #[serde(default)]
    note: Option<String>,
    created_at: i64,
}

fn parse_revision(body: Value) -> Option<Revision> {
    let r: RawRevision = serde_json::from_value(body).ok()?;
    let answer = crate::lyrics::LimusicAnswer {
        origin: "community".into(),
        revision: Some(r.revision),
        lines: r.lines,
        ..Default::default()
    };
    Some(Revision {
        revision: r.revision,
        video_id: r.video_id,
        status: r.status,
        kind: r.kind,
        author: r.author,
        title: r.title,
        artist: r.artist,
        album: r.album,
        duration: r.duration,
        base_revision: r.base_revision,
        note: r.note,
        created_at: r.created_at,
        lyrics: crate::lyrics::limusic_to_lyrics(answer),
    })
}

/// The service's reason, at most 300 bytes (its limit), cut on a character boundary. `None` when
/// there is nothing but space.
fn clip_reason(reason: Option<&str>) -> Option<String> {
    let r = reason?.trim();
    let mut end = r.len().min(300);
    while !r.is_char_boundary(end) {
        end -= 1;
    }
    (end > 0).then(|| r[..end].trim_end().to_owned())
}

fn session(state: &AppState) -> Result<String, String> {
    token(state).ok_or_else(|| "unauthorized".into())
}

/// The body of an answer that was the `ok` status, else its code.
fn expect(answer: Option<(u16, Value)>, ok: u16) -> Result<Value, String> {
    match answer {
        Some((status, body)) if status == ok => Ok(body),
        other => Err(failure(other)),
    }
}

/// A 401 means the session `token` is gone: forget it, and pass the result on.
fn checked<T>(state: &AppState, token: &str, r: Result<T, String>) -> Result<T, String> {
    if matches!(&r, Err(c) if c == "unauthorized") {
        lost(state, token);
    }
    r
}

async fn get_queue(token: &str, after: Option<i64>) -> Result<Queue, String> {
    let mut path = "/v1/review/queue?limit=50".to_owned();
    if let Some(a) = after.filter(|a| *a > 0) {
        path.push_str(&format!("&after={a}"));
    }
    let body = expect(send(crate::http::client().get(url(&path)), Some(token)).await, 200)?;
    serde_json::from_value(body).map_err(|_| "unavailable".into())
}

async fn get_revision(token: &str, revision: i64) -> Result<Revision, String> {
    let rb = crate::http::client().get(url(&format!("/v1/revisions/{revision}")));
    let body = expect(send(rb, Some(token)).await, 200)?;
    parse_revision(body).ok_or_else(|| "unavailable".into())
}

/// `POST /v1/revisions/{id}/review`: the status the service reports.
async fn post_verdict(
    token: &str,
    revision: i64,
    verdict: &str,
    reason: Option<&str>,
) -> Result<String, String> {
    let mut body = json!({ "verdict": verdict });
    if let Some(r) = clip_reason(reason) {
        body["reason"] = r.into();
    }
    let rb =
        crate::http::client().post(url(&format!("/v1/revisions/{revision}/review"))).json(&body);
    let answer = expect(send(rb, Some(token)).await, 200)?;
    Ok(answer["status"].as_str().unwrap_or_default().to_owned())
}

/// `POST /v1/lyrics/{id}/revert`: the live revision afterwards. The key is always sent: a missing
/// `to_revision` is a 400, never a silent takedown.
async fn post_revert(
    token: &str,
    video_id: &str,
    to_revision: Option<i64>,
    reason: Option<&str>,
) -> Result<Option<i64>, String> {
    let mut body = json!({ "to_revision": to_revision });
    if let Some(r) = clip_reason(reason) {
        body["reason"] = r.into();
    }
    let rb = crate::http::client().post(url(&format!("/v1/lyrics/{video_id}/revert"))).json(&body);
    let answer = expect(send(rb, Some(token)).await, 200)?;
    Ok(answer["revision"].as_i64())
}

/// A page of the review queue, oldest first, 50 at a time from `after`.
pub async fn review_queue(state: &AppState, after: Option<i64>) -> Result<Queue, String> {
    let t = session(state)?;
    checked(state, &t, get_queue(&t, after).await)
}

pub async fn review_revision(state: &AppState, revision: i64) -> Result<Revision, String> {
    let t = session(state)?;
    checked(state, &t, get_revision(&t, revision).await)
}

/// The song's live lyric on the service, for the "current" side of an edit.
pub async fn live_lyrics(video_id: &str) -> Option<Lyrics> {
    live(video_id).await?.lyrics
}

/// Approve or reject. `Ok` is the status the service reports: `published` for an approval,
/// `rejected`, or `reverted` for a live lyric that was taken down. An approval puts the lyric in
/// this device's cache in place of the old one, and a takedown forgets the cached one.
pub async fn review_verdict(
    state: &AppState,
    revision: i64,
    video_id: &str,
    verdict: &str,
    reason: Option<String>,
) -> Result<String, String> {
    if !matches!(verdict, "approve" | "reject") || !plausible_id(video_id) {
        return Err("invalid".into());
    }
    let t = session(state)?;
    let status = checked(state, &t, post_verdict(&t, revision, verdict, reason.as_deref()).await)?;
    if verdict == "approve" {
        // The lyric just went live: the next look at this song shows it, not the cached one. The
        // old one goes first, so a failed read of the revision leaves nothing stale behind.
        state.db.delete_lyrics(video_id);
        if let Some(mut l) = get_revision(&t, revision).await.ok().and_then(|r| r.lyrics) {
            l.provider = "limusic".into();
            if let Ok(json) = serde_json::to_string(&l) {
                state.db.put_lyrics(video_id, Some(&json), crate::db::now_secs());
            }
        }
    } else if status == "reverted" {
        state.db.delete_lyrics(video_id);
    }
    Ok(status)
}

/// Make an earlier revision of the song live again, or `None` to leave it with no lyric. `Ok` is
/// the live revision afterwards.
pub async fn revert(
    state: &AppState,
    video_id: &str,
    to_revision: Option<i64>,
    reason: Option<String>,
) -> Result<Option<i64>, String> {
    if !plausible_id(video_id) {
        return Err("invalid".into());
    }
    let t = session(state)?;
    let live = checked(state, &t, post_revert(&t, video_id, to_revision, reason.as_deref()).await)?;
    state.db.delete_lyrics(video_id);
    Ok(live)
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

    /// Same local service: the reviewer's side. Needs LIMUSIC_TEST_USER (not a reviewer) and
    /// LIMUSIC_TEST_ADMIN (can review), and the anonymous test to have run first on a fresh DB.
    #[tokio::test]
    #[ignore = "needs a local lyrics server"]
    async fn local_service_review() {
        let user = std::env::var("LIMUSIC_TEST_USER").unwrap();
        let admin = std::env::var("LIMUSIC_TEST_ADMIN").unwrap();
        assert_eq!(get_queue(&user, None).await.unwrap_err(), "forbidden");
        assert_eq!(get_queue("nope", None).await.unwrap_err(), "unauthorized");
        let anon = post_anonymous().await.unwrap();
        let line = |t: u64, w: &[&str]| LyricLine {
            time_ms: Some(t),
            end_time_ms: Some(t + 900),
            text: w.join(" "),
            words: Some(
                w.iter()
                    .enumerate()
                    .map(|(i, x)| crate::lyrics::LyricWord {
                        text: (*x).into(),
                        start_ms: t + i as u64 * 400,
                        end_ms: t + i as u64 * 400 + 380,
                        bg: false,
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        let sub = |lines, base| Submission {
            video_id: "CCCCCCCCCCC".into(),
            lines,
            title: "Review me".into(),
            artist: "An artist".into(),
            album: None,
            duration: Some(120.0),
            base_revision: base,
        };
        let first = vec![line(1000, &["one", "two"]), line(3000, &["three", "four"])];
        let Submitted::Pending { revision } = post_revision(&anon, &sub(first.clone(), None)).await
        else {
            panic!("an anonymous submit waits");
        };
        let q = get_queue(&admin, None).await.unwrap();
        let item = q.items.iter().find(|i| i.revision == revision).expect("it is waiting");
        assert_eq!((item.kind.as_str(), item.author.as_deref()), ("new", None));
        let r = get_revision(&admin, revision).await.unwrap();
        assert_eq!(r.lyrics.unwrap().lines.len(), 2);
        assert_eq!(get_revision(&user, revision).await.unwrap_err(), "forbidden");
        assert_eq!(post_verdict(&admin, revision, "approve", None).await.unwrap(), "published");
        assert!(get_queue(&admin, None)
            .await
            .unwrap()
            .items
            .iter()
            .all(|i| i.revision != revision));
        // Approving the live revision again keeps it (it is how a reported one is cleared).
        assert_eq!(post_verdict(&admin, revision, "approve", None).await.unwrap(), "published");
        assert_eq!(post_verdict(&admin, 99999, "approve", None).await.unwrap_err(), "not_found");
        // An edit from the same anonymous account waits, and a reject carries a reason.
        let mut edited = first.clone();
        edited[1] = line(3000, &["three", "five"]);
        let Submitted::Pending { revision: edit } =
            post_revision(&anon, &sub(edited, Some(revision))).await
        else {
            panic!();
        };
        let q = get_queue(&admin, None).await.unwrap();
        assert_eq!(q.items.iter().find(|i| i.revision == edit).unwrap().kind, "edit");
        let said = post_verdict(&admin, edit, "reject", Some("  the timing is wrong ")).await;
        assert_eq!(said.unwrap(), "rejected");
        // Revert to no lyric, then the live revision is gone.
        assert_eq!(post_revert(&admin, "CCCCCCCCCCC", None, Some("test")).await.unwrap(), None);
        assert_eq!(live_revision("CCCCCCCCCCC").await.unwrap(), None);
        assert_eq!(post_revert(&user, "CCCCCCCCCCC", None, None).await.unwrap_err(), "forbidden");
    }

    #[test]
    fn the_state_reads_as_the_service_writes_it() {
        let s = parse_state(&json!({"revision": 12, "origin": "community", "authors": ["carla"],
            "votes": {"up": 14, "down": 1}, "my_vote": -1, "my_pending": null}))
        .unwrap();
        assert_eq!((s.revision, s.up, s.down, s.my_vote, s.own), (12, 14, 1, -1, false));
        // Nothing live, only something of ours waiting: no lyric to vote on.
        assert!(parse_state(&json!({"revision": null, "origin": null, "authors": [],
            "votes": {"up": 0, "down": 0}, "my_vote": 0, "my_pending": 9}))
        .is_none());
        // Missing or odd numbers never reach the UI as such.
        let odd = parse_state(&json!({"revision": 3, "votes": {"up": -2}, "my_vote": 7})).unwrap();
        assert_eq!((odd.up, odd.down, odd.my_vote), (0, 0, 1));
    }

    #[test]
    fn a_lyric_is_ones_own_when_the_service_shows_it_as_such() {
        let shown = |author: Value| Some((200, json!({"revision": 12, "author": author})));
        // Anonymous: only the author is shown a revision in full.
        assert!(is_own(true, None, &shown(Value::Null)));
        assert!(!is_own(true, None, &Some((403, json!({"error": "forbidden"})))));
        // Linked: a reviewer is shown every revision, so the name decides.
        assert!(is_own(false, Some("Carla"), &shown("carla".into())));
        assert!(!is_own(false, Some("carla"), &shown("moss_fern".into())));
        assert!(!is_own(false, Some("carla"), &shown(Value::Null)), "a mirrored lyric");
        assert!(!is_own(false, None, &shown("carla".into())));
        assert!(!is_own(false, Some("carla"), &None), "no answer: let the service refuse");
    }

    /// Same local service, after `local_service` has published AAAAAAAAAAA as the admin.
    #[tokio::test]
    #[ignore = "needs a local lyrics server"]
    async fn local_service_votes() {
        let user = std::env::var("LIMUSIC_TEST_USER").unwrap();
        let admin = std::env::var("LIMUSIC_TEST_ADMIN").unwrap();
        let id = "AAAAAAAAAAA";
        let anon = post_anonymous().await.unwrap();
        // A vote shows in the counts, changes sides, and is taken back.
        let before = put_vote(&anon, id, 0).await.unwrap();
        let up = put_vote(&anon, id, 1).await.unwrap();
        assert_eq!((up.up, up.down, up.my_vote), (before.up + 1, before.down, 1));
        let down = put_vote(&user, id, -1).await.unwrap();
        assert_eq!((down.up, down.down, down.my_vote), (before.up + 1, before.down + 1, -1));
        let flipped = put_vote(&anon, id, -1).await.unwrap();
        assert_eq!((flipped.up, flipped.down), (before.up, before.down + 2));
        let cleared = put_vote(&anon, id, 0).await.unwrap();
        assert_eq!((cleared.down, cleared.my_vote), (before.down + 1, 0));
        // Not on one's own work, not without a lyric, not without a session.
        assert_eq!(put_vote(&admin, id, 1).await.unwrap_err(), "own");
        assert_eq!(put_vote(&anon, "ZZZZZZZZZZZ", 1).await.unwrap_err(), "not_found");
        assert_eq!(put_vote("nope", id, 1).await.unwrap_err(), "unauthorized");
        // The state says the same, and whose lyric it is.
        let rb = crate::http::client().get(url(&format!("/v1/lyrics/{id}/state")));
        let seen = parse_state(&send(rb, Some(&user)).await.unwrap().1).unwrap();
        assert_eq!((seen.my_vote, seen.down), (-1, before.down + 1));
        let shown = |t: &str| {
            let rb = crate::http::client().get(url(&format!("/v1/revisions/{}", seen.revision)));
            let t = t.to_owned();
            async move { send(rb, Some(&t)).await }
        };
        assert!(is_own(false, Some("tester_admin"), &shown(&admin).await));
        assert!(!is_own(false, Some("Renamed_1"), &shown(&user).await));
        assert!(!is_own(true, None, &shown(&anon).await));
        // Reports: accepted once, a repeat changes nothing, a note is optional.
        assert_eq!(post_report(&anon, id, "timing", Some("  the chorus is late ")).await, Ok(()));
        assert_eq!(post_report(&anon, id, "timing", None).await, Ok(()));
        assert_eq!(post_report(&user, id, "wrong_song", None).await, Ok(()));
        assert_eq!(post_report(&anon, "ZZZZZZZZZZZ", "text", None).await.unwrap_err(), "not_found");
        assert_eq!(post_report("nope", id, "abuse", None).await.unwrap_err(), "unauthorized");
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
    fn the_queue_reads_as_the_service_writes_it() {
        let q: Queue = serde_json::from_value(json!({
            "items": [
                {"revision": 4, "video_id": "AAAAAAAAAAA", "title": "T", "artist": "A",
                 "duration": 200.0, "kind": "new", "created_at": 10, "reports": 0},
                {"revision": 9, "video_id": "BBBBBBBBBBB", "title": "U", "artist": "B",
                 "duration": null, "author": "carla", "kind": "reported", "created_at": 20,
                 "reports": 3}
            ],
            "next": 9
        }))
        .unwrap();
        assert_eq!(q.items.len(), 2);
        assert_eq!(q.items[0].author, None, "an anonymous sender has no author");
        assert_eq!(q.items[1].author.as_deref(), Some("carla"));
        assert_eq!((q.items[1].reports, q.next), (3, Some(9)));
        let last: Queue = serde_json::from_value(json!({"items": [], "next": null})).unwrap();
        assert_eq!((last.items.len(), last.next), (0, None));
    }

    #[test]
    fn a_revision_comes_with_its_lyric_ready_to_show() {
        let body = json!({
            "revision": 7, "video_id": "AAAAAAAAAAA", "status": "pending", "origin": "community",
            "kind": "edit", "lines": [{"time_ms": 1000, "text": "hi there", "words": [
                {"text": "hi", "start_ms": 1000, "end_ms": 1300},
                {"text": "there", "start_ms": 1300, "end_ms": 1900}], "agent": "v1"}],
            "title": "T", "artist": "A", "album": null, "duration": 200.0, "base_revision": 3,
            "note": "fixed the second verse", "created_at": 55, "review": null
        });
        let r = parse_revision(body).unwrap();
        assert_eq!((r.revision, r.base_revision, r.author.as_deref()), (7, Some(3), None));
        assert_eq!(r.note.as_deref(), Some("fixed the second verse"));
        let l = r.lyrics.unwrap();
        assert!(l.synced && l.lines[0].words.as_ref().unwrap().len() == 2);
        assert_eq!(l.source, "Limusic");
        assert!(parse_revision(json!({"revision": 1})).is_none());
        // A revision whose lines are empty has no lyric to show, but still reads.
        let empty = json!({"revision": 1, "video_id": "AAAAAAAAAAA", "status": "pending",
            "kind": "new", "lines": [], "title": "T", "artist": "A", "created_at": 1});
        assert!(parse_revision(empty).unwrap().lyrics.is_none());
    }

    #[test]
    fn a_reason_is_trimmed_and_cut_to_the_services_limit_on_a_character() {
        assert_eq!(clip_reason(None), None);
        assert_eq!(clip_reason(Some("   ")), None);
        assert_eq!(clip_reason(Some("  timing is off ")).as_deref(), Some("timing is off"));
        let long = "é".repeat(200); // 400 bytes
        let cut = clip_reason(Some(&long)).unwrap();
        assert!(cut.len() <= 300 && cut.chars().all(|c| c == 'é'), "{}", cut.len());
        assert_eq!(clip_reason(Some(&"a".repeat(500))).unwrap().len(), 300);
    }

    #[test]
    fn only_a_youtube_video_id_goes_into_a_path() {
        assert!(plausible_id("FkxEE7wZ83c"));
        assert!(!plausible_id(""));
        assert!(!plausible_id("a/../b"));
        assert!(!plausible_id("FkxEE7wZ83cX"));
    }
}
