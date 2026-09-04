//! HTTP transport. context/01. Pure — no Tauri/webview/mpv.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use sha1::{Digest, Sha1};

use crate::clients::YouTubeClient;
use crate::models::context::Locale;

pub const BASE_URL: &str = "https://music.youtube.com/youtubei/v1/";
pub const ORIGIN: &str = "https://music.youtube.com";
pub const REFERER: &str = "https://music.youtube.com/";
pub const SW_JS_DATA_URL: &str = "https://music.youtube.com/sw.js_data";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("visitorData not found in sw.js_data")]
    VisitorDataNotFound,
    #[error("Your YouTube Music session expired — open the account menu and sign in again.")]
    SessionExpired,
    #[error("This track is already in the playlist.")]
    AlreadyInPlaylist,
    #[error(
        "YouTube Music only allows custom playlist art on accounts with a verified phone number."
    )]
    CoverRefused,
    #[error("{0}")]
    Other(String),
}

/// Session state, set once at startup / login. context/01 §mutable session state.
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub locale: Locale,
    pub visitor_data: Option<String>,
    pub data_sync_id: Option<String>,
    /// Full cookie string (Phase 3). Present ⇒ authenticated requests possible.
    pub cookie: Option<String>,
}

impl Session {
    /// Pull the `SAPISID` value out of the cookie string, if present.
    fn sapisid(&self) -> Option<String> {
        self.cookie.as_deref().and_then(cookie_sapisid).map(str::to_owned)
    }
}

/// Extract the `SAPISID` (or its modern `__Secure-3PAPISID` alias) value from a Cookie header
/// string. Public so the login flow (context/15) can validate a pasted cookie before setting it.
pub fn cookie_sapisid(cookie: &str) -> Option<&str> {
    cookie.split(';').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        matches!(k.trim(), "SAPISID" | "__Secure-3PAPISID").then(|| v.trim())
    })
}

/// The transport client. One shared `reqwest::Client`; proxy must be set before the
/// first request or reqwest snapshots it as none (context/12, the App.kt gotcha).
///
/// `session` is behind a shared lock: the app clones `InnerTube` into the orchestrator, and a
/// runtime login (context/15) must be visible through every clone. Reads/writes are quick and
/// never held across an `.await`, so a std `RwLock` is right (no async lock needed).
#[derive(Clone)]
pub struct InnerTube {
    http: reqwest::Client,
    session: Arc<RwLock<Session>>,
    /// "Hide music videos" (off by default): drop non-ATV rows from the surfaces YouTube
    /// generates. Shared like `session` so a settings toggle reaches every clone, and an atomic
    /// rather than part of `Session` because the endpoints read it on every parse.
    hide_videos: Arc<AtomicBool>,
}

impl InnerTube {
    pub fn new(session: Session, proxy: Option<&str>) -> Result<Self, Error> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(10);
        if let Some(p) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(p)?);
        }
        Ok(InnerTube {
            http: builder.build()?,
            session: Arc::new(RwLock::new(session)),
            hide_videos: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Turn "hide music videos" on/off (context: the user setting, default off).
    pub fn set_hide_videos(&self, on: bool) {
        self.hide_videos.store(on, Ordering::Relaxed);
    }

    pub(crate) fn hide_videos(&self) -> bool {
        self.hide_videos.load(Ordering::Relaxed)
    }

    // --- session accessors (context/15) -----------------------------------------------------

    /// True when a login cookie is set.
    pub fn is_logged_in(&self) -> bool {
        self.session.read().unwrap().cookie.is_some()
    }

    /// The current visitorData (read fresh per resolve — a login may have refreshed it).
    pub fn visitor_data(&self) -> Option<String> {
        self.session.read().unwrap().visitor_data.clone()
    }

    /// The current cookie header, if logged in (for the stream-validation HEAD request).
    pub fn cookie(&self) -> Option<String> {
        self.session.read().unwrap().cookie.clone()
    }

    pub fn set_cookie(&self, cookie: Option<String>) {
        self.session.write().unwrap().cookie = cookie;
    }

    pub fn set_data_sync_id(&self, id: Option<String>) {
        self.session.write().unwrap().data_sync_id = id;
    }

    pub fn data_sync_id(&self) -> Option<String> {
        self.session.read().unwrap().data_sync_id.clone()
    }

    pub fn set_visitor_data(&self, vd: Option<String>) {
        self.session.write().unwrap().visitor_data = vd;
    }

    /// Apply `Set-Cookie` pairs to the session jar (name → value, `Max-Age=0` deletes). Values
    /// come from authenticated responses only, so every pair belongs to the session that made the
    /// request. `dispatched_sapisid` is that session's identity at dispatch: if the jar was
    /// switched (account switch, sign-out) while the request was in flight, these pairs are stale
    /// and must not be merged into the now-current jar.
    fn merge_set_cookies(&self, set_cookies: &[String], dispatched_sapisid: Option<&str>) {
        let mut s = self.session.write().unwrap();
        let Some(mut jar) = s.cookie.clone() else { return };
        if s.sapisid().as_deref() != dispatched_sapisid {
            return;
        }
        let mut changed = false;
        for raw in set_cookies {
            let mut parts = raw.split(';');
            let Some(pair) = parts.next() else { continue };
            let Some((name, value)) = pair.trim().split_once('=') else { continue };
            let name = name.trim();
            if name.is_empty() || name.contains(' ') {
                continue;
            }
            let deleted = parts.any(|attr| {
                attr.trim()
                    .split_once('=')
                    .map(|(k, v)| k.trim().eq_ignore_ascii_case("max-age") && v.trim() == "0")
                    .unwrap_or(false)
            });
            let mut entries: Vec<(String, String)> = jar
                .split(';')
                .filter_map(|kv| {
                    let (k, v) = kv.trim().split_once('=')?;
                    Some((k.trim().to_owned(), v.trim().to_owned()))
                })
                .collect();
            if deleted {
                let before = entries.len();
                entries.retain(|(k, _)| k != name);
                changed |= entries.len() != before;
            } else if let Some(entry) = entries.iter_mut().find(|(k, _)| k == name) {
                changed |= entry.1 != value;
                entry.1 = value.to_owned();
            } else {
                entries.push((name.to_owned(), value.to_owned()));
                changed = true;
            }
            jar = entries.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
        }
        if changed {
            s.cookie = Some(jar);
        }
    }

    /// Build the request `context` for a client from the current session. Crate-internal — the
    /// endpoints facade calls it. Reads and drops the lock synchronously (no `.await` inside).
    pub(crate) fn context_for(&self, client: &YouTubeClient) -> crate::models::context::Context {
        let s = self.session.read().unwrap();
        // `onBehalfOfUser` makes Google *require* a credential: with no cookie it turns a request
        // that would have worked anonymously into a hard 401. Only send it when we can authenticate.
        let dsid = s.cookie.as_ref().and(s.data_sync_id.as_deref());
        client.to_context(&s.locale, s.visitor_data.as_deref(), dsid)
    }

    /// Build a one-off authenticated context for identity validation without changing the shared
    /// session seen by concurrent browse/playback requests. The caller commits the id only after
    /// the validation response succeeds.
    pub(crate) fn context_for_identity(
        &self,
        client: &YouTubeClient,
        data_sync_id: &str,
    ) -> crate::models::context::Context {
        let s = self.session.read().unwrap();
        let dsid = s.cookie.as_ref().map(|_| data_sync_id);
        client.to_context(&s.locale, s.visitor_data.as_deref(), dsid)
    }

    /// POST a JSON body to an InnerTube endpoint with this client's headers, retrying
    /// transient network errors (3 attempts, 500ms × 2 backoff). context/01 §retry.
    pub async fn post<B: Serialize>(
        &self,
        path: &str,
        client: &YouTubeClient,
        body: &B,
        set_login: bool,
    ) -> Result<serde_json::Value, Error> {
        // `path` may already carry query params (e.g. browse continuations); chain accordingly.
        let sep = if path.contains('?') { '&' } else { '?' };
        let url = format!("{BASE_URL}{path}{sep}prettyPrint=false");
        let (headers, dispatched_sapisid) = self.headers(client, set_login);
        let body = serde_json::to_vec(body)?;

        let mut delay = Duration::from_millis(500);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let res = self
                .http
                .post(&url)
                .headers(headers.clone())
                .body(body.clone())
                .send()
                .await
                .and_then(|r| r.error_for_status());
            match res {
                Ok(resp) => {
                    // Google rotates its short-lived tokens (`__Secure-*SIDTS` and friends) on
                    // authenticated responses; a statically captured cookie eventually stops
                    // authenticating (KI-2). Merge every `Set-Cookie` pair into the session jar
                    // the way a browser's cookie store would, so a session that is actually used
                    // keeps itself alive across switches and restarts. Gated on the same
                    // condition that put the cookie on the request: an anonymous response must
                    // not touch the logged-in jar.
                    if set_login && client.login_supported {
                        let set_cookies = resp
                            .headers()
                            .get_all(reqwest::header::SET_COOKIE)
                            .iter()
                            .filter_map(|v| v.to_str().ok().map(str::to_owned))
                            .collect::<Vec<_>>();
                        self.merge_set_cookies(&set_cookies, dispatched_sapisid.as_deref());
                    }
                    return Ok(resp.json().await?);
                }
                // Retry only on connect/timeout (transient), matching Metrolist's IOException filter.
                Err(e) if attempt < 3 && (e.is_timeout() || e.is_connect() || e.is_request()) => {
                    tracing::warn!(attempt, error = %e, "retrying InnerTube POST {path}");
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                // Signed in and Google says "no credential" (401) or "not for you" (403): the
                // stored cookie has gone stale. Raw reqwest text here reads as a broken app and
                // hands the user a URL instead of the one thing that fixes it.
                Err(e)
                    if self.is_logged_in() && e.status().is_some_and(|s| s == 401 || s == 403) =>
                {
                    tracing::warn!(status = ?e.status(), "InnerTube {path} rejected the session");
                    return Err(Error::SessionExpired);
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// POST raw bytes to a path on the same origin that is *not* under `/youtubei`, with this
    /// client's headers plus `extra`, and hand back the response headers along with the body.
    ///
    /// Google's resumable uploader ("Scotty") lives on its own path and answers the first step in
    /// a header, so neither `post`'s URL shape nor its JSON-only return works here. The
    /// `content-type: application/json` the client headers carry stays put even when the body is
    /// an image: the uploader ignores it, and that is the shape known to work.
    pub(crate) async fn post_upload(
        &self,
        path: &str,
        client: &YouTubeClient,
        extra: &[(&'static str, String)],
        body: Vec<u8>,
    ) -> Result<(HeaderMap, Vec<u8>), Error> {
        let mut headers = self.headers(client, true).0;
        for (name, value) in extra {
            if let Ok(v) = HeaderValue::from_str(value) {
                headers.insert(HeaderName::from_static(name), v);
            }
        }
        // Explicitly, from the body we are about to send: reqwest omits `content-length` entirely
        // when the body is empty, and the uploader answers the empty "start" call with a bare
        // 411 Length Required. Sending it ourselves costs nothing on the calls that carry bytes.
        if let Ok(v) = HeaderValue::from_str(&body.len().to_string()) {
            headers.insert(reqwest::header::CONTENT_LENGTH, v);
        }
        let resp = self
            .http
            .post(format!("{ORIGIN}/{path}"))
            .headers(headers)
            .body(body)
            .send()
            .await?
            .error_for_status()?;
        let headers = resp.headers().clone();
        Ok((headers, resp.bytes().await?.to_vec()))
    }

    /// Per-request headers plus the authenticated identity snapshot they were built from.
    /// context/01 §ytClient. Note `X-YouTube-Client-Name` carries the numeric client **id**, not
    /// the name string — intentional and required.
    ///
    /// Both values come out of one session read: the cookie sent on the wire and the SAPISID that
    /// gates merging the response's `Set-Cookie` pairs must describe the same jar, or an account
    /// switch between two reads could pair one account's cookie with another account's merge gate
    /// (`None` = no authenticated identity on this request).
    fn headers(&self, client: &YouTubeClient, set_login: bool) -> (HeaderMap, Option<String>) {
        let mut h = HeaderMap::new();
        let set = |h: &mut HeaderMap, k: &'static str, v: &str| {
            if let Ok(val) = HeaderValue::from_str(v) {
                h.insert(HeaderName::from_static(k), val);
            }
        };
        set(&mut h, "content-type", "application/json");
        set(&mut h, "accept", "application/json");
        set(&mut h, "accept-language", "en-US,en;q=0.9");
        set(&mut h, "x-goog-api-format-version", "1");
        set(&mut h, "x-youtube-client-name", &client.client_id);
        set(&mut h, "x-youtube-client-version", &client.client_version);
        set(&mut h, "x-origin", ORIGIN);
        set(&mut h, "referer", REFERER);
        set(&mut h, "user-agent", &client.user_agent);

        let s = self.session.read().unwrap();
        if let Some(vd) = &s.visitor_data {
            set(&mut h, "x-goog-visitor-id", vd);
        }

        // SAPISIDHASH cookie auth — only when logged in AND the client supports it (Phase 3).
        let sapisid = s.sapisid();
        if set_login && client.login_supported {
            if let Some(cookie) = &s.cookie {
                set(&mut h, "cookie", cookie);
                if let Some(sapisid) = &sapisid {
                    if let Ok(val) = HeaderValue::from_str(&sapisid_hash(sapisid, ORIGIN)) {
                        h.insert(HeaderName::from_static("authorization"), val);
                    }
                }
            }
        }
        let dispatched = if set_login && client.login_supported { sapisid } else { None };
        (h, dispatched)
    }

    /// Bootstrap `visitorData` anonymously by scraping `sw.js_data`. context/04 §A.
    pub async fn fetch_visitor_data(&self) -> Result<String, Error> {
        let text = self.http.get(SW_JS_DATA_URL).send().await?.error_for_status()?.text().await?;
        parse_visitor_data(&text)
    }

    /// Register a play in watch history: GET the response's
    /// `playbackTracking.videostatsPlaybackUrl.baseUrl` with `c`/`cpn`/`ver` (+ `list`/`referrer`
    /// in a playlist) and the authed client headers. context/01 §registerPlayback. Best-effort —
    /// the caller logs-and-ignores errors.
    pub async fn register_playback(
        &self,
        client: &YouTubeClient,
        base_url: &str,
        cpn: &str,
        playlist_id: Option<&str>,
    ) -> Result<(), Error> {
        let url = build_playback_url(base_url, &client.client_name, cpn, playlist_id);
        let headers = self.headers(client, true).0;
        self.http.get(&url).headers(headers).send().await?.error_for_status()?;
        Ok(())
    }

    #[cfg(any(test, feature = "integration-tests"))]
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

/// Build the playback-tracking GET URL. context/01 §registerPlayback. Pure — unit-tested. The
/// `base_url` already carries YouTube's own query params, so we chain onto it.
fn build_playback_url(
    base_url: &str,
    client_name: &str,
    cpn: &str,
    playlist_id: Option<&str>,
) -> String {
    let sep = if base_url.contains('?') { '&' } else { '?' };
    let mut url = format!(
        "{base_url}{sep}c={}&cpn={}&ver=2",
        urlencoding::encode(client_name),
        urlencoding::encode(cpn),
    );
    if let Some(list) = playlist_id {
        let enc = urlencoding::encode(list);
        url.push_str(&format!("&list={enc}&referrer={enc}"));
    }
    url
}

/// CPN alphabet — 64 URL-safe chars, exactly 6 bits each. context/01.
const CPN_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// A fresh 16-char Content Playback Nonce for one playback. context/01 §registerPlayback.
// ponytail: time+counter-seeded xorshift, not crypto-rand — a CPN only needs to be unique per
// playback, not unpredictable; keeps the `rand` crate out of the tree.
pub fn generate_cpn() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let bump = COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut state = (nanos ^ bump).wrapping_add(0x1234_567);
    if state == 0 {
        state = 0xDEAD_BEEF;
    }
    let mut out = String::with_capacity(16);
    for _ in 0..16 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push(CPN_CHARS[(state & 63) as usize] as char);
    }
    out
}

/// `Authorization: SAPISIDHASH <epoch>_<sha1(epoch SAPISID origin)>`. context/01.
pub fn sapisid_hash(sapisid: &str, origin: &str) -> String {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("SAPISIDHASH {epoch}_{}", sha1_hex(&format!("{epoch} {sapisid} {origin}")))
}

fn sha1_hex(input: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// The `sw.js_data` body starts with a 4–5 char junk prefix (`)]}'`); strip it, parse JSON,
/// and find the element matching `^Cg[ts]` in `[0][2]`. context/04 §A.
fn parse_visitor_data(body: &str) -> Result<String, Error> {
    // Drop everything up to and including the first newline or the `)]}'` guard.
    let json_start = body.find('[').ok_or(Error::VisitorDataNotFound)?;
    let value: serde_json::Value = serde_json::from_str(&body[json_start..])?;
    let arr = value
        .get(0)
        .and_then(|v| v.get(2))
        .and_then(|v| v.as_array())
        .ok_or(Error::VisitorDataNotFound)?;
    arr.iter()
        .filter_map(|v| v.as_str())
        .find(|s| s.starts_with("Cgt") || s.starts_with("Cgs"))
        .map(str::to_owned)
        .ok_or(Error::VisitorDataNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_vector() {
        // SHA1("abc") = a9993e364706816aba3e25717850c26c9cd0d89d
        assert_eq!(sha1_hex("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn sapisid_hash_shape() {
        let h = sapisid_hash("MYSAPISID", ORIGIN);
        assert!(h.starts_with("SAPISIDHASH "));
        let rest = &h["SAPISIDHASH ".len()..];
        let (epoch, hash) = rest.split_once('_').unwrap();
        assert!(epoch.parse::<u64>().is_ok());
        assert_eq!(hash.len(), 40); // sha1 hex
    }

    #[test]
    fn parse_visitor_data_from_blob() {
        // Shape of sw.js_data: outer array; [0][2] holds the visitorData among other strings.
        let blob = r#")]}'
[["wrs","x",["junk","CgtABCDEFG1234567%3D%3D","more"]],null]"#;
        assert_eq!(parse_visitor_data(blob).unwrap(), "CgtABCDEFG1234567%3D%3D");
    }

    #[test]
    fn playback_url_appends_params() {
        // Base URL already has query params → chained with `&`; playlist adds list+referrer.
        let u = build_playback_url(
            "https://s.youtube.com/api/stats/playback?cl=1&docid=abc",
            "WEB_REMIX",
            "CPN1234567890AB",
            Some("RDAMVMxyz"),
        );
        assert!(u.contains("?cl=1&docid=abc&c=WEB_REMIX&cpn=CPN1234567890AB&ver=2"));
        assert!(u.contains("&list=RDAMVMxyz&referrer=RDAMVMxyz"));
        // No existing query → first param uses `?`, no playlist params.
        let u2 = build_playback_url("https://s.youtube.com/x", "IOS", "abc", None);
        assert_eq!(u2, "https://s.youtube.com/x?c=IOS&cpn=abc&ver=2");
    }

    #[test]
    fn cpn_is_16_url_safe_chars() {
        let cpn = generate_cpn();
        assert_eq!(cpn.len(), 16);
        assert!(cpn.bytes().all(|b| CPN_CHARS.contains(&b)));
        // Two calls in quick succession must differ (counter salt).
        assert_ne!(generate_cpn(), generate_cpn());
    }

    #[test]
    fn on_behalf_of_user_needs_a_cookie() {
        let clients = crate::clients::Clients::bundled();
        let web = clients.get(crate::clients::METADATA_CLIENT).unwrap();
        let session = Session { data_sync_id: Some("abc123".into()), ..Default::default() };

        let it = InnerTube::new(session, None).unwrap();
        assert_eq!(it.context_for(web).user.on_behalf_of_user, None, "no cookie ⇒ no obo (401)");

        it.set_cookie(Some("SAPISID=secret".into()));
        assert_eq!(it.context_for(web).user.on_behalf_of_user.as_deref(), Some("abc123"));
    }

    #[test]
    fn identity_validation_context_does_not_mutate_the_committed_session() {
        let clients = crate::clients::Clients::bundled();
        let web = clients.get(crate::clients::METADATA_CLIENT).unwrap();
        let session = Session {
            cookie: Some("SAPISID=secret".into()),
            data_sync_id: Some("committed-id".into()),
            ..Default::default()
        };
        let it = InnerTube::new(session, None).unwrap();

        assert_eq!(
            it.context_for_identity(web, "candidate-id").user.on_behalf_of_user.as_deref(),
            Some("candidate-id")
        );
        assert_eq!(it.context_for(web).user.on_behalf_of_user.as_deref(), Some("committed-id"));
    }

    #[test]
    fn sapisid_extracted_from_cookie() {
        let s = Session {
            cookie: Some("FOO=bar; SAPISID=secret123; OTHER=x".into()),
            ..Default::default()
        };
        assert_eq!(s.sapisid().as_deref(), Some("secret123"));
    }

    /// Google rotates its short-lived tokens on authenticated responses; the transport must
    /// merge them into the jar (add, replace, `Max-Age=0` delete) so a used session stays alive.
    #[test]
    fn set_cookie_pairs_merge_into_the_session_jar() {
        let session = Session { cookie: Some("SAPISID=aaa; __Secure-3PSIDTS=old".into()), ..Default::default() };
        let it = InnerTube::new(session, None).unwrap();

        it.merge_set_cookies(
            &[
                "__Secure-3PSIDTS=new; Path=/; Domain=.youtube.com; HttpOnly; Secure".into(),
                "NEWCOOKIE=fresh; Path=/; Max-Age=3600".into(),
            ],
            Some("aaa"),
        );
        let jar = it.cookie().unwrap();
        assert_eq!(
            jar,
            "SAPISID=aaa; __Secure-3PSIDTS=new; NEWCOOKIE=fresh",
            "rotated tokens replace, new cookies append"
        );

        it.merge_set_cookies(&["__Secure-3PSIDTS=x; Path=/; Max-Age=0".into()], Some("aaa"));
        assert_eq!(it.cookie().unwrap(), "SAPISID=aaa; NEWCOOKIE=fresh", "Max-Age=0 deletes");

        // Not logged in → nothing to merge into, stays None.
        let guest = InnerTube::new(Session::default(), None).unwrap();
        guest.merge_set_cookies(&["SAPISID=ignored".into()], None);
        assert_eq!(guest.cookie(), None);
    }

    /// A response whose request was dispatched under one session must not be merged after the
    /// session switched (or signed out): its `Set-Cookie` pairs belong to the account that made
    /// the request, not the one now in the jar.
    #[test]
    fn set_cookies_from_a_switched_session_are_not_merged() {
        let it = InnerTube::new(
            Session { cookie: Some("SAPISID=old; __Secure-3PSIDTS=stale".into()), ..Default::default() },
            None,
        )
        .unwrap();

        // The request went out as "old"; the user switched accounts before the response landed.
        it.set_cookie(Some("SAPISID=new; __Secure-3PSIDTS=fresh".into()));
        it.merge_set_cookies(&["__Secure-3PSIDTS=rotated; Path=/".into()], Some("old"));
        assert_eq!(
            it.cookie().unwrap(),
            "SAPISID=new; __Secure-3PSIDTS=fresh",
            "the stale response's pairs must not touch the new account's jar"
        );

        // And an anonymous request's response must not seed a jar the user logged into meanwhile.
        let anonymous = InnerTube::new(Session::default(), None).unwrap();
        anonymous.set_cookie(Some("SAPISID=new".into()));
        anonymous.merge_set_cookies(&["SAPISID=anon; Path=/".into()], None);
        assert_eq!(anonymous.cookie().unwrap(), "SAPISID=new");
    }

    /// The cookie sent on the wire and the identity gating the response merge come from one
    /// session snapshot: an account switch between two separate reads (the pre-snapshot shape)
    /// could send one account's cookie while checking another account's identity. The dispatch
    /// snapshot must always report the same SAPISID it sent, and the merge gate must reject any
    /// jar that no longer matches.
    #[test]
    fn dispatch_snapshot_keeps_cookie_and_identity_consistent() {
        let clients = crate::clients::Clients::bundled();
        let web = clients.get(crate::clients::METADATA_CLIENT).unwrap();
        let it = InnerTube::new(
            Session {
                cookie: Some("SAPISID=aaa; __Secure-3PSIDTS=t1".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap();

        let (headers, dispatched) = it.headers(web, true);
        assert_eq!(
            headers.get("cookie").and_then(|v| v.to_str().ok()),
            Some("SAPISID=aaa; __Secure-3PSIDTS=t1"),
            "the cookie on the wire is the snapshot's jar"
        );
        assert_eq!(
            dispatched.as_deref(),
            Some("aaa"),
            "the merge gate carries the same identity the cookie carries"
        );
        assert!(headers.get("authorization").is_some(), "SAPISIDHASH derives from the same snapshot");

        // An account switch between dispatch and response: the dispatched identity no longer
        // matches the jar, so the response's pairs must not be merged into it.
        it.set_cookie(Some("SAPISID=bbb".into()));
        it.merge_set_cookies(&["__Secure-3PSIDTS=rotated; Path=/".into()], dispatched.as_deref());
        assert_eq!(it.cookie().unwrap(), "SAPISID=bbb");

        // Anonymous dispatch: no cookie on the wire and no identity to gate a merge.
        let (headers, dispatched) = it.headers(web, false);
        assert!(headers.get("cookie").is_none());
        assert_eq!(dispatched, None);
    }
}
