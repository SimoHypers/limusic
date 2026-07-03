//! The brain: videoId → a playable stream. context/06 §"minimal-but-correct".
//!
//! Phase 1 scope (PHASE1-PROMPT): direct-URL clients VISIONOS → ANDROID_VR_1_43_32 → IOS
//! (no cipher, no PoToken), then rustypipe whole-videoId resolution as the net. Graceful
//! degradation is structural — every failure `continue`s, nothing aborts. DEFERRED to Phase 2:
//! WEB_REMIX streams, STS/cipher, PoToken, HEAD validation, HIGH two-pass, self-heal.

use std::collections::HashMap;
use std::collections::HashSet;

use innertube::{
    find_format, rustypipe_fallback, AudioQuality, Clients, InnerTube, STREAM_FALLBACK_ORDER,
};

/// Everything the player + UI + media layer need for one track. context/06 PlaybackData.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaybackData {
    pub video_id: String,
    pub stream_url: String,
    pub itag: i64,
    /// HTTP headers mpv must send (User-Agent; Phase 3 adds Cookie).
    #[serde(skip)]
    pub headers: HashMap<String, String>,
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

/// Resolve a videoId to a playable stream. `disabled`: client keys the user turned off (also the
/// force-fail lever for the rustypipe-solo acceptance test). context/06.
pub async fn resolve(
    it: &InnerTube,
    clients: &Clients,
    video_id: &str,
    quality: AudioQuality,
    disabled: &HashSet<String>,
) -> Result<PlaybackData, ResolveError> {
    let prefer_high = matches!(quality, AudioQuality::High | AudioQuality::Auto);

    // 1. Direct-URL clients, in order. No STS/PoToken this phase.
    for key in STREAM_FALLBACK_ORDER {
        if disabled.contains(key) {
            tracing::debug!(client = key, "skipped (disabled)");
            continue;
        }
        let Some(client) = clients.get(key) else { continue };

        let resp = match it.player(client, video_id, None).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(client = key, error = %e, "player call failed");
                continue;
            }
        };
        if !resp.playability_status.is_ok() {
            tracing::debug!(client = key, status = %resp.playability_status.status, "not OK");
            continue;
        }
        let Some(streaming) = resp.streaming_data.as_ref() else { continue };
        let Some(expires) = streaming.expires_in_seconds else { continue };
        let Some(format) = find_format(streaming, quality) else { continue };
        // Phase 1 only accepts direct URLs — ciphered formats need Phase 2's decipher.
        let Some(url) = format.direct_url() else {
            tracing::debug!(client = key, itag = format.itag, "format is cipher-only, skipping");
            continue;
        };

        tracing::info!(client = key, itag = format.itag, "resolved via direct client");
        let loudness = format.loudness_db.or_else(|| audio_config_loudness(&resp));
        return Ok(PlaybackData {
            video_id: video_id.to_owned(),
            stream_url: url.to_owned(),
            itag: format.itag as i64,
            headers: header_map(&client.user_agent),
            expires_in_seconds: expires,
            loudness_db: loudness,
            title: resp.video_details.as_ref().and_then(|v| v.title.clone()),
            artists: resp.video_details.as_ref().and_then(|v| v.author.clone()),
            duration: resp
                .video_details
                .as_ref()
                .and_then(|v| v.length_seconds.clone()),
            thumbnail: best_thumbnail(&resp),
            stream_client: key.to_owned(),
        });
    }

    // 2. The net: rustypipe whole-videoId resolution. Must be able to carry the queue SOLO.
    tracing::info!(video_id, "direct clients exhausted → rustypipe fallback");
    match rustypipe_fallback::resolve(video_id, prefer_high).await {
        Ok(c) => Ok(PlaybackData {
            video_id: video_id.to_owned(),
            stream_url: c.url,
            itag: c.itag as i64,
            // rustypipe's URLs are pre-signed and CDN-direct; no special headers required.
            headers: HashMap::new(),
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

fn header_map(user_agent: &str) -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("User-Agent".to_owned(), user_agent.to_owned());
    h
}

fn audio_config_loudness(resp: &innertube::PlayerResponse) -> Option<f64> {
    resp.player_config
        .as_ref()
        .and_then(|c| c.audio_config.as_ref())
        .and_then(|a| a.loudness_db)
}

fn best_thumbnail(resp: &innertube::PlayerResponse) -> Option<String> {
    resp.video_details
        .as_ref()
        .and_then(|v| v.thumbnail.as_ref())
        .and_then(|t| t.thumbnails.last())
        .map(|t| t.url.clone())
}
