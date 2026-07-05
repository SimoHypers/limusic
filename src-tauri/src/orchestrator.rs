//! The brain: videoId → a playable stream. Full context/06 algorithm.
//!
//! Phase 2: WEB_REMIX is the primary client (STS + PoToken + cipher/n-transform), with the
//! direct-URL clients (VISIONOS → ANDROID_VR → IOS) as graceful fallback and rustypipe as the
//! last-ditch net. All seven context/06 critical behaviors are preserved: metadata from MAIN,
//! WEB_REMIX skips HEAD (with per-videoId failure memory), last client accepted unvalidated,
//! HIGH two-pass, off-hot-path self-heal, and graceful PoToken/cipher degradation.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use innertube::{
    find_format, rustypipe_fallback, AudioQuality, Clients, Format, InnerTube, PlayerResponse,
    MAIN_CLIENT, STREAM_FALLBACK_ORDER,
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
    pub title: Option<String>,
    pub artists: Option<String>,
    pub duration: Option<String>,
    pub thumbnail: Option<String>,
    /// Which client produced the stream (diagnostics). context/06.
    pub stream_client: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("no client could resolve a playable stream for {0}")]
    AllClientsFailed(String),
}

/// Client keys that need the `n`-transform applied to their stream URLs. context/06.
const NEEDS_N_TRANSFORM: [&str; 4] = ["WEB", "WEB_REMIX", "WEB_CREATOR", "TVHTML5"];

/// A remembered best-but-not-ideal stream, for the HIGH two-pass (context/06 §4).
struct Candidate {
    format: Format,
    url: String,
    expires: i64,
    client: String,
}

pub struct Orchestrator {
    it: InnerTube,
    clients: Clients,
    cipher: Arc<CipherDeobfuscator>,
    potoken: Arc<PoTokenGenerator>,
    http: reqwest::Client,
    /// videoIds whose WEB_REMIX stream 403'd on the real GET → skip WEB_REMIX next time for them
    /// (context/06 §2). Cleared when the cipher self-heals. `Arc` so the off-hot-path self-heal
    /// task can clear it.
    web_remix_failed: Arc<Mutex<HashSet<String>>>,
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
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
            web_remix_failed: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Record that a WEB_REMIX stream for `video_id` failed on the real GET (called by the player
    /// layer on a playback 403). The next resolve for this id bypasses WEB_REMIX. context/06 §2.
    pub async fn mark_web_remix_failed(&self, video_id: &str) {
        self.web_remix_failed.lock().await.insert(video_id.to_owned());
    }

    /// Resolve a videoId to a playable stream. context/06 full algorithm.
    pub async fn resolve(
        &self,
        video_id: &str,
        quality: AudioQuality,
        disabled: &HashSet<String>,
    ) -> Result<PlaybackData, ResolveError> {
        let prefer_high = matches!(quality, AudioQuality::High | AudioQuality::Auto);
        let logged_in = self.it.is_logged_in();
        let visitor = self.it.visitor_data();

        // 1. Signature timestamp from the deciphering player.js (context/05).
        let sts = self.cipher.signature_timestamp().await;

        // 2. PoToken for the main web client (context/04). May be None (timeout / broken webview).
        let main_client = self.clients.get(MAIN_CLIENT);
        let po = match (main_client, &visitor) {
            (Some(c), Some(vd)) if c.use_web_po_tokens && !disabled.contains(MAIN_CLIENT) => {
                self.potoken.get_web_client_po_token(video_id, vd).await
            }
            _ => None,
        };
        let session_pot = po.as_ref().map(|p| p.player_request_po_token.as_str());
        let stream_pot = po.as_ref().map(|p| p.streaming_data_po_token.clone());

        // 3. Main request as WEB_REMIX (metadata source even when a fallback wins the stream).
        let mut main_resp = match main_client {
            Some(c) if !disabled.contains(MAIN_CLIENT) => {
                self.it.player(c, video_id, None, sts, session_pot).await.ok()
            }
            _ => None,
        };

        // Age/login gate on WEB_REMIX → retry with WEB_CREATOR (login-only). context/06 §4, seam #7.
        // ponytail: metadata + structure are correct now, but WEB_CREATOR streams are ciphered, so
        // this only becomes *audible* once KI-1 (sig/n extraction) is solved. Until then it degrades
        // exactly as before (falls through to the direct clients / rustypipe) — no regression.
        if logged_in && main_resp.as_ref().is_some_and(|r| r.playability_status.is_age_gated()) {
            if let Some(cc) = self.clients.get("WEB_CREATOR") {
                let cc_pot = if cc.use_web_po_tokens { session_pot } else { None };
                let cc_sts = if cc.use_signature_timestamp { sts } else { None };
                tracing::info!(video_id, "WEB_REMIX age/login-gated → retrying WEB_CREATOR");
                if let Ok(r) = self.it.player(cc, video_id, None, cc_sts, cc_pot).await {
                    main_resp = Some(r);
                }
            }
        }

        let main_ok = main_resp.as_ref().is_some_and(|r| r.playability_status.is_ok());
        let has_high = main_resp
            .as_ref()
            .and_then(|r| r.streaming_data.as_ref())
            .is_some_and(|s| s.adaptive_formats.iter().any(is_high));
        let mut audio_config_loudness = main_resp.as_ref().and_then(main_loudness);

        // 4. Fallback loop. idx == -1 reuses the main response; 0.. are the fallback clients.
        let mut best: Option<Candidate> = None;
        let last_idx = STREAM_FALLBACK_ORDER.len() as isize - 1;

        for idx in -1..=last_idx {
            let (key, resp): (String, PlayerResponse) = if idx == -1 {
                if !main_ok || disabled.contains(MAIN_CLIENT) {
                    continue;
                }
                (MAIN_CLIENT.to_owned(), main_resp.clone().unwrap())
            } else {
                let key = STREAM_FALLBACK_ORDER[idx as usize];
                if disabled.contains(key) {
                    continue;
                }
                let Some(client) = self.clients.get(key) else { continue };
                if client.login_required && !logged_in {
                    continue;
                }
                let client_pot = if client.use_web_po_tokens { session_pot } else { None };
                let client_sts = if client.use_signature_timestamp { sts } else { None };
                match self.it.player(client, video_id, None, client_sts, client_pot).await {
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

            // Resolve the URL: direct, else decipher (context/05).
            let Some(mut url) = self.find_url(format, video_id).await else {
                continue;
            };

            // n-transform + &pot= for web clients (context/05, 06).
            let client = self.clients.get(&key);
            let needs_n = client.is_some_and(|c| c.use_web_po_tokens)
                || NEEDS_N_TRANSFORM.contains(&key.as_str());
            if needs_n {
                url = self.cipher.transform_n_param_in_url(&url).await;
                if client.is_some_and(|c| c.use_web_po_tokens) {
                    if let Some(pot) = &stream_pot {
                        let sep = if url.contains('?') { '&' } else { '?' };
                        url = format!("{url}{sep}pot={}", urlencoding::encode(pot));
                    }
                }
            }

            // HIGH two-pass: remember the best non-HIGH and keep looking if a HIGH exists elsewhere.
            if prefer_high && !is_high(format) && has_high {
                if better(format, best.as_ref().map(|c| &c.format)) {
                    best = Some(Candidate { format: format.clone(), url, expires, client: key });
                }
                continue;
            }

            // Accept without validation on the last client (last-ditch).
            if idx == last_idx {
                return Ok(self.build(video_id, format, url, expires, &key, audio_config_loudness, &main_resp));
            }
            // WEB_REMIX skips HEAD (its authed URL 403s on HEAD, streams on GET) unless it already
            // failed for this id.
            if idx == -1
                && key == MAIN_CLIENT
                && !self.web_remix_failed.lock().await.contains(video_id)
            {
                return Ok(self.build(video_id, format, url, expires, &key, audio_config_loudness, &main_resp));
            }
            // Otherwise validate with a HEAD request.
            if self.validate_head(&url, client.map(|c| c.user_agent.as_str())).await {
                return Ok(self.build(video_id, format, url, expires, &key, audio_config_loudness, &main_resp));
            } else if needs_n {
                // A cipher client that fails validation may have a stale config → self-heal off
                // the hot path so it never blocks falling through (context/06 §7). If the heal
                // changes the config table, clear the WEB_REMIX failure memory (context/06 §2).
                let cipher = self.cipher.clone();
                let failed = self.web_remix_failed.clone();
                tauri::async_runtime::spawn(async move {
                    if cipher.on_stream_rejected().await {
                        failed.lock().await.clear();
                    }
                });
            }
        }

        // 6. HIGH wanted but only a non-HIGH found → use the remembered best.
        if let Some(c) = best {
            return Ok(self.build(video_id, &c.format, c.url, c.expires, &c.client, audio_config_loudness, &main_resp));
        }

        // 7. Net: rustypipe whole-videoId resolution (last-ditch). context/06, seam #11.
        tracing::info!(video_id, "all InnerTube clients exhausted → rustypipe fallback");
        match rustypipe_fallback::resolve(video_id, prefer_high).await {
            Ok(c) => Ok(PlaybackData {
                video_id: video_id.to_owned(),
                stream_url: c.url,
                itag: c.itag as i64,
                headers: std::collections::HashMap::new(),
                expires_in_seconds: c.expires_in_seconds as i64,
                loudness_db: c.loudness_db.map(|f| f as f64),
                title: c.title,
                artists: None,
                duration: c.duration_secs.map(|s| s.to_string()),
                thumbnail: None,
                stream_client: "rustypipe".to_owned(),
            }),
            Err(e) => {
                tracing::error!(video_id, error = %e, "rustypipe fallback failed");
                Err(ResolveError::AllClientsFailed(video_id.to_owned()))
            }
        }
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
    async fn validate_head(&self, url: &str, ua: Option<&str>) -> bool {
        let mut req = self.http.head(url);
        if let Some(ua) = ua {
            req = req.header("User-Agent", ua);
        }
        if let Some(cookie) = self.it.cookie() {
            req = req.header("Cookie", cookie.as_str());
        }
        matches!(req.send().await, Ok(r) if r.status().is_success())
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
    ) -> PlaybackData {
        let ua = self.clients.get(client).map(|c| c.user_agent.clone());
        let mut headers = std::collections::HashMap::new();
        if let Some(ua) = ua {
            headers.insert("User-Agent".to_owned(), ua);
        }
        let vd = main_resp.as_ref().and_then(|r| r.video_details.as_ref());
        tracing::info!(client, itag = format.itag, "resolved stream");
        PlaybackData {
            video_id: video_id.to_owned(),
            stream_url: url,
            itag: format.itag as i64,
            headers,
            expires_in_seconds: expires,
            loudness_db: format.loudness_db.or(loudness),
            title: vd.and_then(|v| v.title.clone()),
            artists: vd.and_then(|v| v.author.clone()),
            duration: vd.and_then(|v| v.length_seconds.clone()),
            thumbnail: main_resp.as_ref().and_then(best_thumbnail),
            stream_client: client.to_owned(),
        }
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
    (
        rank(a),
        a.audio_channels.unwrap_or(2),
        codec(a),
        a.bitrate,
    ) > (rank(b), b.audio_channels.unwrap_or(2), codec(b), b.bitrate)
}

fn main_loudness(resp: &PlayerResponse) -> Option<f64> {
    resp.player_config
        .as_ref()
        .and_then(|c| c.audio_config.as_ref())
        .and_then(|a| a.loudness_db)
}

fn best_thumbnail(resp: &PlayerResponse) -> Option<String> {
    resp.video_details
        .as_ref()
        .and_then(|v| v.thumbnail.as_ref())
        .and_then(|t| t.thumbnails.last())
        .map(|t| t.url.clone())
}
