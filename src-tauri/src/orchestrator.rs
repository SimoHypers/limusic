//! The brain: videoId → a playable stream. Full context/06 algorithm.
//!
//! Phase 2: WEB_REMIX is the primary client (STS + PoToken + cipher/n-transform), with the
//! direct-URL clients (VISIONOS → ANDROID_VR → IOS) as graceful fallback and rustypipe as the
//! last-ditch net. The context/06 critical behaviors are preserved: metadata from MAIN, the
//! per-videoId WEB_REMIX failure memory, the HIGH two-pass, off-hot-path self-heal, and graceful
//! PoToken/cipher degradation. Every client is HEAD-validated (see the note in `resolve`); for an
//! upload a failed HEAD demotes the URL instead of rejecting it, because there is no anonymous
//! chain behind an upload to fall through to.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use innertube::{
    find_format, find_video_format, rustypipe_fallback, AudioQuality, Clients, Format, InnerTube,
    PlayerResponse, MAIN_CLIENT, STREAM_FALLBACK_ORDER, UPLOAD_FALLBACK_ORDER,
};
use tokio::sync::Mutex;

use crate::cipher::CipherDeobfuscator;
use crate::potoken::PoTokenGenerator;

/// Everything the player + UI + media layer need for one track. context/06 PlaybackData.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaybackData {
    pub video_id: String,
    pub stream_url: String,
    pub itag: i64,
    /// HTTP headers mpv must send (User-Agent; Phase 3 adds Cookie).
    #[serde(skip)]
    pub headers: std::collections::HashMap<String, String>,
    pub expires_in_seconds: i64,
    pub loudness_db: Option<f64>,
    /// Where to register this play in watch history (context/01). `None` when no client that
    /// answered carried the tracking block.
    pub playback_ping: Option<PlaybackPing>,
    pub title: Option<String>,
    pub artists: Option<String>,
    pub duration: Option<String>,
    pub thumbnail: Option<String>,
    /// YouTube's own `musicVideoType` for this videoId: `Some(true)` = a video upload, `Some(false)`
    /// = the generated audio track, `None` = the metadata client never answered. The player view's
    /// music-video mode believes this over the queue row's flag, which several rows arrive without
    /// (a card played from a shelf, a Listen Together mirror, an album row swapped to its audio id).
    pub is_video: Option<bool>,
    /// Which client produced the stream (diagnostics). context/06.
    pub stream_client: String,
}

/// The watch-history ping for one play: `playbackTracking.videostatsPlaybackUrl.baseUrl` plus the
/// registry key of the client whose `/player` response carried it (context/01 §registerPlayback).
///
/// The two travel together because the ping's `c=` param, and its headers, have to be the client
/// that was issued the URL. Reading the URL off one client's response and sending it as another's
/// is what YouTube sees as a mismatch.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaybackPing {
    pub url: String,
    pub client: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("no client could resolve a playable stream for {0}")]
    AllClientsFailed(String),
    /// One of the user's own uploads that no authenticated client would stream. Distinct from
    /// `AllClientsFailed` because "unavailable" reads as "the song is gone", and the likely cause
    /// here is a session that needs signing in again. Issue #71.
    #[error("this upload could not be played. Try signing in to YouTube Music again ({0})")]
    UploadUnavailable(String),
    /// A local file that was in the library but is no longer on disk (context: local.rs).
    #[error("this file is no longer on your disk: {0}")]
    LocalMissing(String),
}

/// Client keys that need the `n`-transform applied to their stream URLs. context/06.
const NEEDS_N_TRANSFORM: [&str; 4] = ["WEB", "WEB_REMIX", "WEB_CREATOR", "TVHTML5"];

// WEB_REMIX is validated with a HEAD like every other client — see `validate_head`.

/// A remembered best-but-not-ideal stream, for the HIGH two-pass (context/06 §4).
struct Candidate {
    format: Format,
    url: String,
    expires: i64,
    client: String,
    ping: Option<PlaybackPing>,
}

pub struct Orchestrator {
    it: InnerTube,
    clients: Clients,
    cipher: Arc<CipherDeobfuscator>,
    potoken: Arc<PoTokenGenerator>,
    /// videoId → when its WEB_REMIX stream last 403'd on the real GET, so the next resolve skips
    /// WEB_REMIX for it (context/06 §2). Cleared when the cipher self-heals. `Arc` so the
    /// off-hot-path self-heal task can clear it. Entries expire: the bar only has to survive the
    /// retry that follows the failure, and a permanent one meant a single bad minute cost that
    /// track its best client for the rest of the session.
    web_remix_failed: Arc<Mutex<HashMap<String, Instant>>>,
}

const WEB_REMIX_BLACKLIST_TTL: Duration = Duration::from_secs(30 * 60);

/// Record a failure, dropping expired entries on the way so the map cannot grow.
fn blacklist_insert(map: &mut HashMap<String, Instant>, video_id: &str, now: Instant) {
    map.retain(|_, at| now.duration_since(*at) < WEB_REMIX_BLACKLIST_TTL);
    map.insert(video_id.to_owned(), now);
}

/// Is WEB_REMIX still barred for this id? An entry past the TTL counts as absent.
fn blacklist_blocks(map: &HashMap<String, Instant>, video_id: &str, now: Instant) -> bool {
    map.get(video_id).is_some_and(|at| now.duration_since(*at) < WEB_REMIX_BLACKLIST_TTL)
}

impl Orchestrator {
    pub fn new(
        it: InnerTube,
        clients: Clients,
        cipher: Arc<CipherDeobfuscator>,
        potoken: Arc<PoTokenGenerator>,
    ) -> Self {
        Orchestrator {
            it,
            clients,
            cipher,
            potoken,
            web_remix_failed: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record that a WEB_REMIX stream for `video_id` failed on the real GET (called by the player
    /// layer on a playback 403). The next resolve for this id bypasses WEB_REMIX. context/06 §2.
    pub async fn mark_web_remix_failed(&self, video_id: &str) {
        blacklist_insert(&mut *self.web_remix_failed.lock().await, video_id, Instant::now());
    }

    /// Resolve a videoId to a playable stream. context/06 full algorithm.
    pub async fn resolve(
        &self,
        video_id: &str,
        is_upload: bool,
        quality: AudioQuality,
        disabled: &HashSet<String>,
    ) -> Result<PlaybackData, ResolveError> {
        let prefer_high = matches!(quality, AudioQuality::High | AudioQuality::Auto);
        let logged_in = self.it.is_logged_in();
        let visitor = self.it.visitor_data();
        // An upload only streams to an authenticated client, so it gets its own chain and never
        // falls through to the anonymous ones (context: clients::UPLOAD_FALLBACK_ORDER, issue #71).
        let order: &[&str] =
            if is_upload { &UPLOAD_FALLBACK_ORDER } else { &STREAM_FALLBACK_ORDER };
        // Without the uploads-playlist context YouTube hands back upload URLs that expire in about
        // 32 seconds (Metrolist PR #3857). Harmless for ordinary tracks, so scoped to uploads.
        let playlist_id = is_upload.then_some("MLPT");

        // 1. Signature timestamp from the deciphering player.js (context/05).
        let sts = self.cipher.signature_timestamp().await;

        // 2. Session PoToken for the main web client's /player body (context/04). Cached in Rust
        // with its TTL, so this is usually free; may be None (timeout / broken webview) —
        // degrade gracefully.
        let main_client = self.clients.get(MAIN_CLIENT);
        let session_pot_owned = match (main_client, &visitor) {
            (Some(c), Some(vd)) if c.use_web_po_tokens && !disabled.contains(MAIN_CLIENT) => {
                self.potoken.get_session_po_token(vd).await
            }
            _ => None,
        };
        let session_pot = session_pot_owned.as_deref();

        // 3. Main request as WEB_REMIX (metadata source even when a fallback wins the stream).
        let mut main_resp = match main_client {
            Some(c) if !disabled.contains(MAIN_CLIENT) => {
                self.it.player(c, video_id, playlist_id, sts, session_pot).await.ok()
            }
            _ => None,
        };

        // Which client `main_resp` actually came from — WEB_CREATOR can replace it just below, and
        // a tracking URL has to be pinged as the client that was issued it.
        let mut main_key = MAIN_CLIENT;

        // Age/login gate on WEB_REMIX → retry with WEB_CREATOR (login-only). context/06 §4, seam #7.
        // ponytail: WEB_CREATOR streams are ciphered, so this depends on the whole web path working
        // (decipher, then a PoToken googlevideo accepts). Both do since 2026-08-25 (KI-1), so an
        // age-gated track now has a real chance here; when the path fails it still falls through to
        // the direct clients / rustypipe exactly as before.
        if logged_in && main_resp.as_ref().is_some_and(|r| r.playability_status.is_age_gated()) {
            if let Some(cc) = self.clients.get("WEB_CREATOR") {
                let cc_pot = if cc.use_web_po_tokens { session_pot } else { None };
                let cc_sts = if cc.use_signature_timestamp { sts } else { None };
                tracing::info!(video_id, "WEB_REMIX age/login-gated → retrying WEB_CREATOR");
                if let Ok(r) = self.it.player(cc, video_id, playlist_id, cc_sts, cc_pot).await {
                    main_resp = Some(r);
                    main_key = "WEB_CREATOR";
                }
            }
        }

        let main_ok = main_resp.as_ref().is_some_and(|r| r.playability_status.is_ok());
        let has_high = main_resp
            .as_ref()
            .and_then(|r| r.streaming_data.as_ref())
            .is_some_and(|s| s.adaptive_formats.iter().any(is_high));
        let mut audio_config_loudness = main_resp.as_ref().and_then(main_loudness);
        // Prefer main's tracking block: a ping sent as WEB_REMIX is what registers the play as a
        // YouTube *Music* one. But `playbackTracking` is only present on an OK response, so when
        // main degraded (no PoToken, age gate, a stale cipher) it isn't there at all and the play
        // would go unregistered even though a fallback client streamed it fine. Take that client's
        // block instead — it carries the same `docid`/`ei`/`of` for this videoId. Issue #83.
        let main_ping = main_resp.as_ref().and_then(|r| playback_ping(r, main_key));

        // 4. Fallback loop. idx == -1 reuses the main response; 0.. are the fallback clients.
        let mut best: Option<Candidate> = None;
        // A login client's upload URL that failed HEAD. Used only if nothing validates.
        let mut upload_fallback: Option<Candidate> = None;
        let last_idx = order.len() as isize - 1;

        for idx in -1..=last_idx {
            let (key, resp): (String, PlayerResponse) = if idx == -1 {
                // A WEB_REMIX stream that already died in the player is not retried for this
                // video: it passed HEAD and failed anyway, so validation has nothing left to say.
                // Uploads included. This used to exempt them, on the belief that skipping this
                // slot left the retry with nothing, but the rest of `UPLOAD_FALLBACK_ORDER`
                // (TVHTML5, then WEB_CREATOR) is exactly what the retry is for. Exempting them
                // meant the second attempt re-resolved the same dead WEB_REMIX URL and failed
                // identically, which is the loop issue #71 has been stuck in.
                if !main_ok
                    || disabled.contains(MAIN_CLIENT)
                    || blacklist_blocks(
                        &*self.web_remix_failed.lock().await,
                        video_id,
                        Instant::now(),
                    )
                {
                    continue;
                }
                (MAIN_CLIENT.to_owned(), main_resp.clone().unwrap())
            } else {
                let key = order[idx as usize];
                if disabled.contains(key) {
                    continue;
                }
                let Some(client) = self.clients.get(key) else { continue };
                if client.login_required && !logged_in {
                    continue;
                }
                let client_pot = if client.use_web_po_tokens { session_pot } else { None };
                let client_sts = if client.use_signature_timestamp { sts } else { None };
                match self.it.player(client, video_id, playlist_id, client_sts, client_pot).await {
                    Ok(r) if r.playability_status.is_ok() => (key.to_owned(), r),
                    Ok(r) => {
                        tracing::debug!(client = key, status = %r.playability_status.status, "not OK");
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(client = key, error = %e, "player call failed");
                        continue;
                    }
                }
            };

            let Some(streaming) = resp.streaming_data.as_ref() else { continue };
            let Some(expires) = streaming.expires_in_seconds else { continue };
            let Some(format) = find_format(streaming, quality) else { continue };
            if audio_config_loudness.is_none() {
                audio_config_loudness = main_loudness(&resp);
            }

            // Resolve the URL: direct, else decipher (context/05). A ciphered format with no
            // working cipher webview lands here, and for an upload that is fatal: every client on
            // its chain is a web client, so the whole chain produces nothing and the user sees
            // "sign-in needed" for what is really a broken extraction runtime. Issues #71/#128.
            let Some(mut url) = self.find_url(format, video_id).await else {
                tracing::warn!(video_id, client = %key, itag = format.itag, "no stream URL (deciphering unavailable?)");
                continue;
            };

            // n-transform + &pot= for web clients (context/05, 06).
            let client = self.clients.get(&key);
            let needs_n = client.is_some_and(|c| c.use_web_po_tokens)
                || NEEDS_N_TRANSFORM.contains(&key.as_str());
            if needs_n {
                url = self.cipher.transform_n_param_in_url(&url).await;
                if client.is_some_and(|c| c.use_web_po_tokens) {
                    if let Some(vd) = &visitor {
                        if let Some(pot) = self.potoken.get_streaming_po_token(video_id, vd).await {
                            let sep = if url.contains('?') { '&' } else { '?' };
                            url = format!("{url}{sep}pot={}", urlencoding::encode(&pot));
                        }
                    }
                }
            }

            // HIGH two-pass: remember the best non-HIGH and keep looking if a HIGH exists elsewhere.
            if prefer_high && !is_high(format) && has_high {
                if better(format, best.as_ref().map(|c| &c.format)) {
                    let ping = main_ping.clone().or_else(|| playback_ping(&resp, &key));
                    best =
                        Some(Candidate { format: format.clone(), url, expires, client: key, ping });
                }
                continue;
            }

            // EVERY client is validated, including WEB_REMIX and the last one in the chain. Both
            // used to be accepted blind and both were wrong for an mpv-backed player:
            //
            // - The last client had rustypipe behind it, so there was never nothing to fall
            //   through to; skipping the check only hid a dead URL until playback.
            // - WEB_REMIX skipped it on Metrolist's note that its authed URLs 403 on HEAD but
            //   stream on GET. That holds for ExoPlayer, which fetches in bounded ranges. mpv opens
            //   with `Range: bytes=0-`, and for the videos where googlevideo caps a WEB_REMIX URL
            //   (only the first ~768 KiB is served, in <=256 KiB pieces) that open-ended request
            //   gets the same 403 the HEAD does.
            //
            // Measured on fresh URLs, HEAD agrees with what mpv gets every time: 200/206 for
            // dQw4w9WgXcQ, 403/403 for XqZsoesa55w and D07O_cbJ_Rw. So the check costs one
            // round trip and turns a guaranteed failed load, an error toast, a retry and a round
            // of cipher/PoToken self-heal churn into a silent fall-through at resolve time.
            //
            // It also stays correct if a valid PoToken lifts the cap on those videos: then HEAD
            // passes and WEB_REMIX is used. Nothing here has to know which way that goes.
            //
            // The probe sends exactly the headers `build` will hand mpv (same UA, cookie only
            // where mpv gets one), because a HEAD that carries something the real GET does not
            // is not a prediction of anything. Issue #71.
            let headers =
                stream_headers(client.map(|c| c.user_agent.clone()), self.it.cookie(), is_upload);
            if self.validate_head(&url, &headers).await {
                let ping = main_ping.clone().or_else(|| playback_ping(&resp, &key));
                return Ok(self.build(
                    video_id,
                    format,
                    url,
                    expires,
                    &key,
                    audio_config_loudness,
                    &main_resp,
                    ping,
                    headers,
                ));
            }

            // An upload's failed HEAD is a demotion, never a rejection. Metrolist stopped
            // validating privately-owned tracks outright (PR #3517) because a HEAD against one
            // does not reliably predict its GET, and this app then went further and returned the
            // very first URL unvalidated. That made WEB_REMIX the only client an upload ever
            // used: TVHTML5 and WEB_CREATOR sat behind an unconditional `return` and could never
            // run, so an account whose WEB_REMIX URLs 403 (no accepted PoToken on that machine, a
            // stale cipher) had no second chance and no way to recover. Issue #71.
            //
            // So: keep the first URL as a last resort, let the rest of the chain have its turn,
            // and hand the unvalidated one back only if nothing better turns up. Worst case this
            // is what the old code did, one or two round trips later.
            if is_upload {
                if upload_fallback.is_none() {
                    tracing::info!(video_id, client = %key, "upload stream failed HEAD, trying the next login client");
                    let ping = main_ping.clone().or_else(|| playback_ping(&resp, &key));
                    upload_fallback =
                        Some(Candidate { format: format.clone(), url, expires, client: key, ping });
                }
                continue;
            }

            if needs_n {
                self.self_heal();
            }
        }

        // 6. HIGH wanted but only a non-HIGH found → use the remembered best.
        if let Some(c) = best {
            let headers = self.headers_for(&c.client, is_upload);
            return Ok(self.build(
                video_id,
                &c.format,
                c.url,
                c.expires,
                &c.client,
                audio_config_loudness,
                &main_resp,
                c.ping,
                headers,
            ));
        }

        // 6b. An upload nothing validated: hand back the first URL a login client produced
        // rather than skip the track. See the demotion note in the loop.
        if let Some(c) = upload_fallback {
            tracing::warn!(video_id, client = %c.client, "no upload stream passed HEAD, using the first anyway");
            // Every login client's URL was refused, which is the one upload failure that does say
            // something about the session rather than about the track. Heal off the hot path so a
            // machine stuck on a rejected PoToken or a stale cipher can get itself out; without
            // this an upload-only failure had no route back at all.
            self.self_heal();
            let headers = self.headers_for(&c.client, is_upload);
            return Ok(self.build(
                video_id,
                &c.format,
                c.url,
                c.expires,
                &c.client,
                audio_config_loudness,
                &main_resp,
                c.ping,
                headers,
            ));
        }

        // 7. Net: rustypipe whole-videoId resolution (last-ditch). context/06, seam #11.
        // rustypipe is anonymous, so it can never see a privately-owned track: skip the round trip
        // and say what actually went wrong instead of "unavailable". Issue #71.
        if is_upload {
            tracing::warn!(video_id, "no authenticated client could stream this upload");
            return Err(ResolveError::UploadUnavailable(video_id.to_owned()));
        }
        tracing::info!(video_id, "all InnerTube clients exhausted → rustypipe fallback");
        match rustypipe_fallback::resolve(video_id, prefer_high).await {
            Ok(c) => Ok(PlaybackData {
                video_id: video_id.to_owned(),
                stream_url: c.url,
                itag: c.itag as i64,
                headers: std::collections::HashMap::new(),
                expires_in_seconds: c.expires_in_seconds as i64,
                loudness_db: c.loudness_db.map(|f| f as f64),
                playback_ping: None,
                title: c.title,
                artists: None,
                duration: c.duration_secs.map(|s| s.to_string()),
                thumbnail: None,
                // rustypipe answers without a `musicVideoType`, so the queue row's flag stands.
                is_video: None,
                stream_client: "rustypipe".to_owned(),
            }),
            Err(e) => {
                tracing::error!(video_id, error = %e, "rustypipe fallback failed");
                Err(ResolveError::AllClientsFailed(video_id.to_owned()))
            }
        }
    }

    /// A video-only stream URL for `video_id`, for the player view's music-video mode (plan 031).
    ///
    /// Deliberately not part of [`resolve`](Self::resolve): this runs only while someone is looking
    /// at the player view with video on, it needs no cipher, no PoToken and no HEAD two-pass, and it
    /// must never be able to make audio slower or less reliable. A `None` here just means the view
    /// keeps the artwork.
    pub async fn resolve_video(&self, video_id: &str, max_height: i32) -> Option<String> {
        for key in ["VISIONOS", "ANDROID_VR_1_65_10"] {
            let Some(client) = self.clients.get(key) else { continue };
            let resp = match self.it.player(client, video_id, None, None, None).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!(video_id, client = key, error = %e, "video: /player failed");
                    continue;
                }
            };
            if !resp.playability_status.is_ok() {
                continue;
            }
            let Some(sd) = resp.streaming_data.as_ref() else { continue };
            // Only ever a direct URL: these clients don't cipher, and a ciphered video is not worth
            // waking the cipher webview for.
            if let Some(url) = find_video_format(sd, max_height).and_then(|f| f.direct_url()) {
                tracing::debug!(video_id, client = key, "video: resolved");
                return Some(url.to_owned());
            }
        }
        tracing::debug!(video_id, "video: no usable format");
        None
    }

    /// A format's playable URL: direct, else deciphered from its `signatureCipher`. context/05.
    async fn find_url(&self, format: &Format, video_id: &str) -> Option<String> {
        if let Some(u) = format.direct_url() {
            return Some(u.to_owned());
        }
        let cipher = format.cipher_string()?;
        self.cipher.deobfuscate_stream_url(cipher, video_id).await
    }

    /// HEAD validation (context/06 §validateStatus). Success = 2xx. False on any error.
    ///
    /// `headers` is what mpv will send for this stream, so the probe is the same request the
    /// player will make. It used to always attach the cookie while `build` attached it only for
    /// uploads, which let an ordinary track pass validation and then 403 on the real open.
    async fn validate_head(&self, url: &str, headers: &HashMap<String, String>) -> bool {
        // The 10s budget used to live on a client of its own; it is a property of this one
        // probe, not of the app's HTTP.
        let mut req = crate::http::client().head(url).timeout(Duration::from_secs(10));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        matches!(req.send().await, Ok(r) if r.status().is_success())
    }

    /// A cipher client's stream was refused → its config may be stale. Heal off the hot path so
    /// it never blocks falling through (context/06 §7). If the heal changes the config table,
    /// clear the WEB_REMIX failure memory (context/06 §2).
    fn self_heal(&self) {
        let cipher = self.cipher.clone();
        let potoken = self.potoken.clone();
        let failed = self.web_remix_failed.clone();
        tauri::async_runtime::spawn(async move {
            // The session PoToken now outlives the process, so a rejected web stream is the only
            // signal left that Google stopped honouring it early. Drop it here rather than replay
            // it for the rest of its nominal 12 hours.
            potoken.invalidate_session_token().await;
            if cipher.on_stream_rejected().await {
                failed.lock().await.clear();
            }
        });
    }

    /// [`stream_headers`] for a client registry key.
    fn headers_for(&self, client: &str, is_upload: bool) -> HashMap<String, String> {
        stream_headers(
            self.clients.get(client).map(|c| c.user_agent.clone()),
            self.it.cookie(),
            is_upload,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        &self,
        video_id: &str,
        format: &Format,
        url: String,
        expires: i64,
        client: &str,
        loudness: Option<f64>,
        main_resp: &Option<PlayerResponse>,
        ping: Option<PlaybackPing>,
        headers: HashMap<String, String>,
    ) -> PlaybackData {
        let vd = main_resp.as_ref().and_then(|r| r.video_details.as_ref());
        tracing::info!(video_id, client, itag = format.itag, "resolved stream");
        PlaybackData {
            video_id: video_id.to_owned(),
            stream_url: url,
            itag: format.itag as i64,
            headers,
            expires_in_seconds: expires,
            loudness_db: format.loudness_db.or(loudness),
            playback_ping: ping,
            title: vd.and_then(|v| v.title.clone()),
            artists: vd.and_then(|v| v.author.clone()),
            duration: vd.and_then(|v| v.length_seconds.clone()),
            thumbnail: main_resp.as_ref().and_then(best_thumbnail),
            is_video: vd.and_then(|v| v.is_music_video()),
            stream_client: client.to_owned(),
        }
    }
}

/// The headers mpv (and the validating HEAD) must send for one stream.
///
/// A privately-owned track's googlevideo URL is only served to the session that owns it, so an
/// upload's GET has to carry the cookie. Uploads only: this is the hot path and there is no
/// evidence an ordinary stream wants one. Issue #71.
///
/// mpv's header properties are global (crates/player: `http-header-fields`), so a track appended
/// for gapless playback inherits whatever the current one set. Same host either way, so it is
/// harmless, but it means the cookie can outlive the upload that needed it.
fn stream_headers(
    ua: Option<String>,
    cookie: Option<String>,
    is_upload: bool,
) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    if let Some(ua) = ua {
        headers.insert("User-Agent".to_owned(), ua);
    }
    if is_upload {
        if let Some(cookie) = cookie {
            headers.insert("Cookie".to_owned(), cookie);
        }
    }
    headers
}

fn is_high(f: &Format) -> bool {
    f.audio_quality.as_deref() == Some("AUDIO_QUALITY_HIGH")
}

/// Better-than comparison for the HIGH two-pass (context/06 §isBetter): quality rank, then audio
/// channels, then codec (opus > mp4a), then bitrate.
fn better(a: &Format, b: Option<&Format>) -> bool {
    let Some(b) = b else { return true };
    let rank = |f: &Format| match f.audio_quality.as_deref() {
        Some("AUDIO_QUALITY_HIGH") => 3,
        Some("AUDIO_QUALITY_MEDIUM") => 2,
        Some("AUDIO_QUALITY_LOW") => 1,
        _ => 0u8,
    };
    let codec = |f: &Format| {
        if f.mime_type.contains("opus") {
            2
        } else if f.mime_type.contains("mp4a") {
            1
        } else {
            0u8
        }
    };
    (rank(a), a.audio_channels.unwrap_or(2), codec(a), a.bitrate)
        > (rank(b), b.audio_channels.unwrap_or(2), codec(b), b.bitrate)
}

fn main_loudness(resp: &PlayerResponse) -> Option<f64> {
    resp.player_config.as_ref().and_then(|c| c.audio_config.as_ref()).and_then(|a| a.loudness_db)
}

fn playback_ping(resp: &PlayerResponse, client: &str) -> Option<PlaybackPing> {
    let url = resp
        .playback_tracking
        .as_ref()
        .and_then(|t| t.videostats_playback_url.as_ref())
        .and_then(|b| b.base_url.clone())?;
    Some(PlaybackPing { url, client: client.to_owned() })
}

fn best_thumbnail(resp: &PlayerResponse) -> Option<String> {
    resp.video_details
        .as_ref()
        .and_then(|v| v.thumbnail.as_ref())
        .and_then(|t| t.thumbnails.last())
        .map(|t| t.url.clone())
}

#[cfg(test)]
mod tests {
    use super::{blacklist_blocks, blacklist_insert, stream_headers, WEB_REMIX_BLACKLIST_TTL};
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    #[test]
    fn the_web_remix_bar_expires_and_stays_bounded() {
        let now = Instant::now();
        let mut map = HashMap::new();

        blacklist_insert(&mut map, "fresh", now);
        assert!(blacklist_blocks(&map, "fresh", now), "a fresh failure bars WEB_REMIX");
        assert!(!blacklist_blocks(&map, "never-failed", now));

        // Past the TTL the entry reads as absent, so the track gets its best client back.
        let later = now + WEB_REMIX_BLACKLIST_TTL + Duration::from_secs(1);
        assert!(!blacklist_blocks(&map, "fresh", later));

        // And inserting at that point drops it, so the map cannot grow across a long session.
        blacklist_insert(&mut map, "other", later);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("other"));
    }

    /// The HEAD probe and mpv share this, so what it returns has to be identical for both callers
    /// (that mismatch is issue #71): the cookie rides along for an upload and for nothing else.
    #[test]
    fn only_an_upload_carries_the_cookie() {
        let ua = || Some("UA/1".to_owned());
        let cookie = || Some("SAPISID=secret".to_owned());

        let up = stream_headers(ua(), cookie(), true);
        assert_eq!(up.get("User-Agent").map(String::as_str), Some("UA/1"));
        assert_eq!(up.get("Cookie").map(String::as_str), Some("SAPISID=secret"));

        let ordinary = stream_headers(ua(), cookie(), false);
        assert_eq!(ordinary.get("User-Agent").map(String::as_str), Some("UA/1"));
        assert!(!ordinary.contains_key("Cookie"), "an ordinary stream must not send the cookie");

        // Signed out: an upload cannot play at all, but it must not produce a bogus header.
        assert!(!stream_headers(ua(), None, true).contains_key("Cookie"));
    }
}
