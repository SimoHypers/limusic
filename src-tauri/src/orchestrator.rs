//! The brain: videoId → a playable stream. Full context/06 algorithm.
//!
//! Phase 2: WEB_REMIX is the primary client (STS + PoToken + cipher/n-transform), with the
//! direct-URL client (VISIONOS) as graceful fallback and rustypipe as the
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
    PlayerResponse, StreamingData, YouTubeClient, MAIN_CLIENT, STREAM_FALLBACK_ORDER,
    UPLOAD_FALLBACK_ORDER,
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
    /// The format's MIME type, when a `Format` produced this stream. Used by offline downloads to
    /// name the saved file and its extension; `None` on a cache replay, a local file and rustypipe
    /// (which reports no mime).
    pub mime_type: Option<String>,
    /// The format's bitrate in bits/s, when it reported one. Downloads show it as part of the
    /// actual quality label ("256 kbps AAC").
    pub bitrate: Option<i64>,
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
    /// Every client YouTube answered for this track wanted an account, and there is no session.
    /// Distinct from `AllClientsFailed` because the user can fix this one: some networks and
    /// regions get `LOGIN_REQUIRED` from every anonymous client, and signing in is the whole fix
    /// (issue #292).
    #[error("YouTube would not serve {0} without an account. Sign in from the account menu.")]
    SignInRequired(String),
    /// A local file that was in the library but is no longer on disk (context: local.rs).
    #[error("this file is no longer on your disk: {0}")]
    LocalMissing(String),
    /// Nothing answered at all: no client's `/player` call came back, and neither did the
    /// rustypipe net. That is the network, a dead proxy or a captive portal, and it says nothing
    /// about this particular track, so the queue must not skip past it or drop it.
    #[error("could not reach YouTube. Check your connection and try again ({0})")]
    Unreachable(String),
}

impl ResolveError {
    /// Would every other track in the queue fail this way too? A caller deciding whether to skip
    /// forward, or to delete a row, has to know: skipping is right for a track YouTube refused and
    /// wrong for an outage, where it walks the whole queue and deletes what it passes.
    pub fn affects_every_track(&self) -> bool {
        matches!(self, ResolveError::Unreachable(_) | ResolveError::SignInRequired(_))
    }
}

/// Client keys that need the `n`-transform applied to their stream URLs. context/06.
const NEEDS_N_TRANSFORM: [&str; 4] = ["WEB", "WEB_REMIX", "WEB_CREATOR", "TVHTML5"];

// WEB_REMIX is validated like every other client, see `validate_stream`.

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
    /// videoId → (usable until, the `streamingData` of a music video's WEB_REMIX reply). `resolve`
    /// already paid for that reply and it lists the picture next to the audio, so
    /// [`resolve_video`](Self::resolve_video) takes it from here instead of asking again. Taken
    /// once: the URL made from it is cached upstream (`AppState::video_urls`).
    /// ponytail: cleared wholesale at 8 entries, like `video_urls`.
    video_replies: std::sync::Mutex<HashMap<String, (Instant, StreamingData)>>,
    /// videoId → when its picture failed to load after it was handed out. Same TTL as
    /// `web_remix_failed`; see [`mark_video_failed`](Self::mark_video_failed).
    video_failed: std::sync::Mutex<HashMap<String, Instant>>,
}

const WEB_REMIX_BLACKLIST_TTL: Duration = Duration::from_secs(30 * 60);

/// How often the off-hot-path self-heal may run. It deletes the session PoToken, the cached
/// `player.js` and the cipher webview, so re-running it per failed probe is expensive; and the
/// probe it reacts to cannot tell a stale signature from a video whose URL googlevideo simply caps
/// (see `validate_stream` and KNOWN-ISSUES KI-11), so most of what used to trigger it was not
/// evidence about the cipher at all. Matches `cipher::config`'s own registry cooldown.
const HEAL_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// One self-heal at a time, and not more than one per [`HEAL_COOLDOWN`]. Same shape as
/// `session::claim_refresh`, and for the same reason: one bad minute throws off a burst of
/// identical signals and each one used to pay the full price.
/// ponytail: process-global, fine with one `Orchestrator` per process; move it onto `self` if a
/// second one is ever built.
fn claim_heal() -> bool {
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let Ok(mut last) = LAST.lock() else { return false };
    if last.is_some_and(|t| t.elapsed() < HEAL_COOLDOWN) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

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
            video_replies: Default::default(),
            video_failed: Default::default(),
        }
    }

    /// The picture for `video_id` failed after it was handed out: the `<video>` element errored, or
    /// mpv could not open it. Its next [`resolve_video`](Self::resolve_video) tries VISIONOS first,
    /// and the kept WEB_REMIX reply goes, because the dead URL came out of it. `true` only the
    /// first time within the TTL, so a caller that retries on failure retries once, not forever.
    pub fn mark_video_failed(&self, video_id: &str) -> bool {
        if let Ok(mut kept) = self.video_replies.lock() {
            kept.remove(video_id);
        }
        let Ok(mut failed) = self.video_failed.lock() else { return false };
        let now = Instant::now();
        let first = !blacklist_blocks(&failed, video_id, now);
        blacklist_insert(&mut failed, video_id, now);
        first
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
        // The token belongs to the *session*, not to WEB_REMIX. Gating the mint on the main
        // client was harmless while WEB_REMIX was the only thing that wanted one; with
        // TVHTML5_SIMPLY in the chain it meant that turning WEB_REMIX off silently turned off
        // every other PoToken client with it, and they then skipped themselves for want of a
        // token that was sitting valid in the cache. So ask whether anything we are actually
        // going to try wants one.
        let wants_pot = std::iter::once(MAIN_CLIENT)
            .chain(order.iter().copied())
            .filter(|k| !disabled.contains(*k))
            .any(|k| self.clients.get(k).is_some_and(|c| c.use_web_po_tokens));
        let session_pot_owned = match &visitor {
            Some(vd) if wants_pot => self.potoken.get_session_po_token(vd).await,
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
        if main_ok && main_key == MAIN_CLIENT {
            self.keep_video_reply(video_id, main_resp.as_ref().unwrap());
        }
        // The main response's status was the one thing the log never showed, so a report where
        // nothing played could not be told apart from one where only the fallbacks failed
        // (issue #292). `reason` is YouTube's own sentence, which is what separates a country
        // block from a bot check.
        let mut login_wanted = false;
        if let Some(r) = main_resp.as_ref().filter(|_| !main_ok) {
            login_wanted = r.playability_status.status == "LOGIN_REQUIRED";
            crate::import::note_playability(r.playability_status.reason.as_deref());
            tracing::debug!(
                client = main_key,
                status = %r.playability_status.status,
                reason = r.playability_status.reason.as_deref().unwrap_or(""),
                "not OK"
            );
        }
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
        // Did YouTube answer anything at all? A refusal is information about the track; silence is
        // information about the network, and the two want opposite handling upstream.
        let mut reached = main_resp.is_some();
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
                // A PoToken client with no token is a guaranteed UNPLAYABLE ("The page needs to
                // be reloaded"), so spend nothing on it when minting degraded. The direct clients
                // ahead of it need no token, which is what keeps the chain alive in that state
                // (context/06 §graceful PoToken degradation).
                if client.use_web_po_tokens && client_pot.is_none() {
                    tracing::debug!(client = key, "no PoToken, skipping");
                    continue;
                }
                let client_sts = if client.use_signature_timestamp { sts } else { None };
                let answered =
                    self.it.player(client, video_id, playlist_id, client_sts, client_pot).await;
                reached |= answered.is_ok();
                match answered {
                    Ok(r) if r.playability_status.is_ok() => (key.to_owned(), r),
                    Ok(r) => {
                        login_wanted |= r.playability_status.status == "LOGIN_REQUIRED";
                        crate::import::note_playability(r.playability_status.reason.as_deref());
                        tracing::debug!(
                            client = key,
                            status = %r.playability_status.status,
                            reason = r.playability_status.reason.as_deref().unwrap_or(""),
                            "not OK"
                        );
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

            // n-transform + &pot= for web clients (context/05, 06). YouTube's own stream hosts
            // only: an RSS-feed podcast's URL is the feed's own host, which must not be handed a
            // PoToken (#294). Uploads stream from `c.youtube.com` and do need both (#308).
            let client = self.clients.get(&key);
            let needs_n = is_youtube_stream(&url)
                && (client.is_some_and(|c| c.use_web_po_tokens)
                    || NEEDS_N_TRANSFORM.contains(&key.as_str()));
            if needs_n {
                url = self.sign(url, client, video_id, visitor.as_deref()).await;
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
            //   stream on GET. That holds for ExoPlayer, which fetches in bounded ranges, and
            //   for the videos where googlevideo caps a WEB_REMIX URL (only the first ~768 KiB is
            //   served, in <=256 KiB pieces) nothing past that opening ever arrives.
            //
            // Measured on fresh URLs when this was a HEAD, it agreed with what mpv got every time:
            // 200/206 for dQw4w9WgXcQ, 403/403 for XqZsoesa55w and D07O_cbJ_Rw. So the check costs
            // one round trip and turns a guaranteed failed load, an error toast, a retry and a
            // round of cipher/PoToken self-heal churn into a silent fall-through at resolve time.
            //
            // It also stays correct if a valid PoToken lifts the cap on those videos: then the
            // probe passes and WEB_REMIX is used. Nothing here has to know which way that goes.
            //
            // The probe sends exactly the headers `build` will hand mpv (same UA, cookie only
            // where mpv gets one), because a probe that carries something the real GET does not
            // is not a prediction of anything. Issue #71.
            let headers =
                stream_headers(client.map(|c| c.user_agent.clone()), self.it.cookie(), is_upload);
            if self.validate_stream(&url, &headers, content_length(format)).await {
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

            // An upload's failed probe is a demotion, never a rejection. Metrolist stopped
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
                    tracing::info!(video_id, client = %key, "upload stream failed validation, trying the next login client");
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
            tracing::warn!(video_id, client = %c.client, "no upload stream passed validation, using the first anyway");
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
            Ok(c) if !self.validate_stream(&c.url, &HashMap::new(), Some(c.size)).await => {
                tracing::warn!(video_id, "rustypipe URL serves only its first MiB, not playable");
                // rustypipe answered, so the network is up and this failure is about the URL.
                Err(nothing_played(video_id, logged_in, login_wanted, true))
            }
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
                // rustypipe reports no mime or bitrate.
                mime_type: None,
                bitrate: None,
            }),
            Err(e) => {
                tracing::error!(video_id, error = %e, "rustypipe fallback failed");
                // rustypipe is the last thing that spoke to YouTube, so its verdict counts as an
                // answer even when every InnerTube client was skipped or errored (a disabled
                // MAIN/VISIONOS pair, or TVHTML5_SIMPLY passed over for want of a PoToken). Without
                // this, a genuinely dead video reads as an outage and the queue sits on it.
                let reached = reached || e.answered();
                Err(nothing_played(video_id, logged_in, login_wanted, reached))
            }
        }
    }

    /// A video-only stream URL for `video_id`, for the player view's music-video mode (plan 031).
    ///
    /// Deliberately not part of [`resolve`](Self::resolve): this runs only while someone is looking
    /// at the player view with video on, and it must never be able to make audio slower or less
    /// reliable. A `None` here just means the view keeps the artwork.
    ///
    /// WEB_REMIX first. It carries the session's PoToken, visitor data and login, so it passes the
    /// bot check that turns VISIONOS away from a flagged IP: on 2026-09-30 VISIONOS answered "Sign
    /// in to confirm you're not a bot" for every video while WEB_REMIX kept resolving the audio of
    /// the same ids. VISIONOS is the fallback, and goes first for a track whose picture already
    /// failed ([`mark_video_failed`](Self::mark_video_failed)).
    pub async fn resolve_video(
        &self,
        video_id: &str,
        max_height: i32,
        disabled: &HashSet<String>,
    ) -> Option<String> {
        let failed =
            self.video_failed.lock().is_ok_and(|m| blacklist_blocks(&m, video_id, Instant::now()));
        let order = if failed { ["VISIONOS", MAIN_CLIENT] } else { [MAIN_CLIENT, "VISIONOS"] };
        for key in order {
            // The "stream clients" setting covers this path too. It used to be read only by
            // `resolve`, so a user who turned a client off still got it here, which is both a
            // setting that does not do what it says and a client they had a reason to refuse.
            if disabled.contains(key) {
                continue;
            }
            let url = if key == MAIN_CLIENT {
                self.web_remix_video(video_id, max_height).await
            } else {
                self.direct_video(key, video_id, max_height).await
            };
            if let Some(url) = url {
                tracing::debug!(video_id, client = key, "video: resolved");
                return Some(url);
            }
        }
        tracing::debug!(video_id, "video: no usable format");
        None
    }

    /// Keep a music video's WEB_REMIX reply for [`resolve_video`](Self::resolve_video).
    fn keep_video_reply(&self, video_id: &str, resp: &PlayerResponse) {
        let is_video = resp.video_details.as_ref().and_then(|v| v.is_music_video());
        let Some(sd) = resp.streaming_data.as_ref().filter(|_| is_video == Some(true)) else {
            return;
        };
        let Some(secs) = sd.expires_in_seconds.and_then(|s| u64::try_from(s).ok()) else { return };
        let Ok(mut kept) = self.video_replies.lock() else { return };
        if kept.len() >= 8 {
            kept.clear();
        }
        kept.insert(video_id.to_owned(), (Instant::now() + Duration::from_secs(secs), sd.clone()));
    }

    /// The picture from WEB_REMIX: the reply `resolve` kept when there is one, else a `/player` of
    /// its own. Its URL gets everything the audio's gets (decipher, `n`, `&pot=`) and the same tail
    /// probe, because googlevideo caps some WEB_REMIX URLs at their first ~768 KiB
    /// (`validate_stream`). The probe sends no headers since neither proxy does, and googlevideo
    /// was measured to serve these URLs without a User-Agent or cookie.
    async fn web_remix_video(&self, video_id: &str, max_height: i32) -> Option<String> {
        let client = self.clients.get(MAIN_CLIENT)?;
        let visitor = self.it.visitor_data();
        let kept = self.video_replies.lock().ok().and_then(|mut m| m.remove(video_id));
        let sd = match kept.filter(|(until, _)| *until > Instant::now()) {
            Some((_, sd)) => sd,
            None => {
                let sts = self.cipher.signature_timestamp().await;
                let pot = match &visitor {
                    Some(vd) => self.potoken.get_session_po_token(vd).await,
                    None => None,
                };
                let resp =
                    self.it.player(client, video_id, None, sts, pot.as_deref()).await.ok()?;
                if !resp.playability_status.is_ok() {
                    crate::import::note_playability(resp.playability_status.reason.as_deref());
                    tracing::debug!(video_id, status = %resp.playability_status.status, "video: WEB_REMIX not OK");
                    return None;
                }
                resp.streaming_data?
            }
        };
        let format = find_video_format(&sd, max_height)?;
        let url = self.find_url(format, video_id).await?;
        let url = self.sign(url, Some(client), video_id, visitor.as_deref()).await;
        if !self.validate_stream(&url, &HashMap::new(), content_length(format)).await {
            tracing::debug!(video_id, itag = format.itag, "video: WEB_REMIX URL refused");
            return None;
        }
        Some(url)
    }

    /// The picture from a direct-URL client (VISIONOS): nothing to decipher or sign.
    ///
    /// Not ANDROID_VR: its video URLs are capped at the first mebibyte like its audio ones (issue
    /// #292), so it could only ever hand the view a stream that dies mid-clip.
    async fn direct_video(&self, key: &str, video_id: &str, max_height: i32) -> Option<String> {
        let client = self.clients.get(key)?;
        let resp = match self.it.player(client, video_id, None, None, None).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(video_id, client = key, error = %e, "video: /player failed");
                return None;
            }
        };
        if !resp.playability_status.is_ok() {
            crate::import::note_playability(resp.playability_status.reason.as_deref());
            tracing::debug!(
                video_id,
                client = key,
                reason = resp.playability_status.reason.as_deref().unwrap_or(""),
                "video: not OK"
            );
            return None;
        }
        let sd = resp.streaming_data.as_ref()?;
        find_video_format(sd, max_height).and_then(|f| f.direct_url()).map(str::to_owned)
    }

    /// `n`-transform, then `&pot=` for a PoToken client (context/05, 06). A step that fails leaves
    /// the URL as it was, so playback still tries it.
    async fn sign(
        &self,
        url: String,
        client: Option<&YouTubeClient>,
        video_id: &str,
        visitor: Option<&str>,
    ) -> String {
        let mut url = self.cipher.transform_n_param_in_url(&url).await;
        if client.is_some_and(|c| c.use_web_po_tokens) {
            if let Some(vd) = visitor {
                if let Some(pot) = self.potoken.get_streaming_po_token(video_id, vd).await {
                    let sep = if url.contains('?') { '&' } else { '?' };
                    url = format!("{url}{sep}pot={}", urlencoding::encode(&pot));
                }
            }
        }
        url
    }

    /// A format's playable URL: direct, else deciphered from its `signatureCipher`. context/05.
    async fn find_url(&self, format: &Format, video_id: &str) -> Option<String> {
        if let Some(u) = format.direct_url() {
            return Some(u.to_owned());
        }
        let cipher = format.cipher_string()?;
        self.cipher.deobfuscate_stream_url(cipher, video_id).await
    }

    /// Will googlevideo serve this URL, all the way to the end? (context/06 §validateStatus.)
    ///
    /// Probe shape, not just probe headers. mpv never opens a googlevideo URL directly any more:
    /// `state::mpv_stream_url` hands it a loopback URL and `audioproxy` fetches bounded ranges
    /// upstream, so a bounded range is the request this has to predict.
    ///
    /// And the range is the last 256 bytes, because that is the one question that separates a
    /// capped URL from a healthy one. Since 2026 googlevideo answers only the first mebibyte of
    /// some URLs (rustypipe's, ANDROID_VR's): measured 2026-09-22, a range ending inside that
    /// window returns 206 and every range ending past it returns 403, on every video tried and at
    /// any chunk size. HEAD and an opening range both pass, and mpv was handed a stream that
    /// delivered no bytes and blamed the audio format (issue #292). The unranged HEAD this replaced
    /// also caught capped direct-client URLs, but only as an accident of what the CDN refused that
    /// month (KNOWN-ISSUES KI-11/KI-12).
    ///
    /// `headers` is what `build` will hand mpv, so the probe carries exactly what the real fetch
    /// will (issue #71). Falls back to a HEAD when there is no length to aim at: an RSS-feed
    /// podcast's enclosure is not on googlevideo, reports `contentLength: "0"`, and may not honour
    /// ranges at all (#294). Success = 2xx, false on any error.
    async fn validate_stream(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        content_length: Option<u64>,
    ) -> bool {
        let req = match content_length.filter(|n| *n > 0).filter(|_| is_youtube_stream(url)) {
            Some(len) => crate::http::client()
                .get(url)
                .header("Range", format!("bytes={}-{}", len.saturating_sub(256), len - 1))
                .header("Accept-Encoding", "identity"),
            None => crate::http::client().head(url),
        };
        // The 10s budget is a property of this one probe, not of the app's HTTP.
        let mut req = req.timeout(Duration::from_secs(10));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        matches!(req.send().await, Ok(r) if r.status().is_success())
    }

    /// A cipher client's stream was refused, so its config may be stale. Heal off the hot path so
    /// it never blocks falling through (context/06 §7). If the heal changes the config table,
    /// clear the WEB_REMIX failure memory (context/06 §2).
    fn self_heal(&self) {
        if !claim_heal() {
            tracing::debug!("self-heal ran recently, not repeating it");
            return;
        }
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

    /// [`stream_headers`] for a client registry key. `pub(crate)` because a cache hit skips the
    /// resolve and has to rebuild the same headers from the client it recorded.
    pub(crate) fn headers_for(&self, client: &str, is_upload: bool) -> HashMap<String, String> {
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
            mime_type: Some(format.mime_type.clone()),
            bitrate: Some(format.bitrate),
        }
    }
}

/// A format's byte length, when it reported one. `"0"` (an RSS-feed enclosure, #294) reads as
/// absent, because a zero-length file has no tail to probe.
fn content_length(f: &Format) -> Option<u64> {
    f.content_length.as_deref()?.parse::<u64>().ok().filter(|n| *n > 0)
}

/// True for a URL served by YouTube's own stream CDN. Everything else (an RSS-feed podcast's
/// enclosure, #294) gets no n-transform, no PoToken and no chunking proxy.
///
/// **Two hosts, not one.** Ordinary tracks come back on `*.googlevideo.com`, but one of the user's
/// own uploads is served from `*.c.youtube.com` (measured on a 0.8.2 report, issue #308). Matching
/// only the first host meant an upload's URL was handed to mpv unsigned: no `n`-transform and no
/// `&pot=`, which googlevideo answers with 403, and mpv opened it directly because the chunking
/// proxy skipped it too. Uploads have no anonymous client behind them, so that was every upload.
pub(crate) fn is_youtube_stream(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str().map(|h| h.ends_with(".googlevideo.com") || h.ends_with(".c.youtube.com"))
        })
        .unwrap_or(false)
}

/// The headers mpv (and the validating HEAD) must send for one stream.
///
/// A privately-owned track's stream URL (`c.youtube.com`, #308) is only served to the session
/// that owns it, so an upload's GET has to carry the cookie. Uploads only: this is the hot path
/// and there is no evidence an ordinary stream wants one. Issue #71.
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

/// The error for a track nothing could stream. Signing in is a real fix when YouTube asked for an
/// account and there is no session, and useless noise otherwise (issue #292).
/// When no response came back at all (`reached` false), we never learned anything about this video.
fn nothing_played(
    video_id: &str,
    logged_in: bool,
    login_wanted: bool,
    reached: bool,
) -> ResolveError {
    if !reached {
        return ResolveError::Unreachable(video_id.to_owned());
    }
    if login_wanted && !logged_in {
        ResolveError::SignInRequired(video_id.to_owned())
    } else {
        ResolveError::AllClientsFailed(video_id.to_owned())
    }
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
    use super::{
        blacklist_blocks, blacklist_insert, claim_heal, content_length, is_youtube_stream,
        nothing_played, stream_headers, ResolveError, WEB_REMIX_BLACKLIST_TTL,
    };
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    // An RSS-feed podcast streams from the feed's own host (#294), which gets no PoToken. One of
    // the user's own uploads streams from `c.youtube.com` (#308), which needs everything a
    // googlevideo URL needs: miss it and the URL reaches mpv unsigned and 403s.
    #[test]
    fn both_of_youtubes_stream_hosts_count_and_nothing_else_does() {
        assert!(is_youtube_stream("https://rr5---sn-abc.googlevideo.com/videoplayback?n=x"));
        assert!(is_youtube_stream("https://rr2---sn-2onja5-5i.c.youtube.com/videoplayback?n=x"));
        assert!(!is_youtube_stream("https://www.podtrac.com/pts/redirect.mp3/x.mp3"));
        assert!(!is_youtube_stream("https://evil.com/googlevideo.com/videoplayback"));
        assert!(!is_youtube_stream("https://evil.com/rr2---sn-x.c.youtube.com/videoplayback"));
        assert!(!is_youtube_stream("https://www.youtube.com/watch?v=x"));
        assert!(!is_youtube_stream("/home/me/song.flac"));
    }

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

    /// Silence outranks a half-heard verdict: with nothing reached, even a `LOGIN_REQUIRED` seen
    /// earlier must not turn an outage into "sign in".
    #[test]
    fn nothing_played_prefers_unreachable_over_a_verdict() {
        for (logged_in, login_wanted) in
            [(false, true), (false, false), (true, true), (true, false)]
        {
            assert!(
                matches!(
                    nothing_played("v", logged_in, login_wanted, false),
                    ResolveError::Unreachable(_)
                ),
                "an outage must never read as a verdict on the track"
            );
        }
    }

    /// The regression guard for issue #292's fix, once YouTube did answer.
    #[test]
    fn nothing_played_keeps_its_old_answers_when_youtube_answered() {
        assert!(matches!(nothing_played("v", false, true, true), ResolveError::SignInRequired(_)));
        for (logged_in, login_wanted) in [(false, false), (true, true), (true, false)] {
            assert!(matches!(
                nothing_played("v", logged_in, login_wanted, true),
                ResolveError::AllClientsFailed(_)
            ));
        }
    }

    #[test]
    fn only_the_systemic_errors_affect_every_track() {
        let v = || "v".to_owned();
        assert!(
            ResolveError::Unreachable(v()).affects_every_track(),
            "an outage would walk the queue and delete rows"
        );
        assert!(
            ResolveError::SignInRequired(v()).affects_every_track(),
            "every anonymous track fails the same way until the user signs in"
        );
        assert!(
            !ResolveError::AllClientsFailed(v()).affects_every_track(),
            "an unavailable video must still be skipped, or the queue stalls on it"
        );
        assert!(
            !ResolveError::UploadUnavailable(v()).affects_every_track(),
            "a mixed queue must skip past a failed upload to the ordinary tracks"
        );
        assert!(
            !ResolveError::LocalMissing(v()).affects_every_track(),
            "a deleted local file must keep leaving the queue"
        );
    }

    #[test]
    fn content_length_reads_only_a_real_length() {
        let fmt = |len: &str| -> super::Format {
            let mut v = serde_json::json!({ "itag": 251, "mimeType": "audio/webm" });
            if !len.is_empty() {
                v["contentLength"] = len.into();
            }
            serde_json::from_value(v).unwrap()
        };
        assert_eq!(content_length(&fmt("4194304")), Some(4194304));
        assert_eq!(
            content_length(&fmt("0")),
            None,
            "a zero-length enclosure has no tail to probe, so it must fall back to the HEAD"
        );
        assert_eq!(content_length(&fmt("")), None);
        assert_eq!(content_length(&fmt("not a number")), None);
    }

    #[test]
    fn claim_heal_allows_one_and_then_holds_the_door() {
        assert!(claim_heal());
        assert!(
            !claim_heal(),
            "a burst of identical probe failures must cost one heal, not one heal per track"
        );
    }
}
