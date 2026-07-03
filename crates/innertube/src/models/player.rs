//! `/player` request + response models and audio format selection. context/03.

use serde::{Deserialize, Deserializer, Serialize};

use super::context::Context;

/// Some innertube clients (VISIONOS, ANDROID_VR, IOS) send numeric fields like `bitrate` as
/// JSON strings instead of numbers. Accept either so a client quirk doesn't fail the whole
/// response and force a fallback.
fn deserialize_i64_lenient<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrI64 {
        String(String),
        I64(i64),
    }
    match StringOrI64::deserialize(deserializer)? {
        StringOrI64::String(s) => s.parse().map_err(serde::de::Error::custom),
        StringOrI64::I64(n) => Ok(n),
    }
}

/// Option variant — `expiresInSeconds` comes back as the string `"21540"` on every client.
fn deserialize_opt_i64_lenient<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<i64>, D::Error> {
    deserialize_i64_lenient(deserializer).map(Some)
}

/// `/player` request body. context/03.
///
/// Phase 1 never sends `playbackContext` (needs STS/cipher) or `serviceIntegrityDimensions`
/// (needs PoToken) — both are Phase 2 seams, kept as `None`-able fields so the shape is ready.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerBody {
    pub context: Context,
    pub video_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback_context: Option<PlaybackContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_integrity_dimensions: Option<ServiceIntegrityDimensions>,
    pub content_check_ok: bool,
    pub racy_check_ok: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackContext {
    pub content_playback_context: ContentPlaybackContext,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentPlaybackContext {
    pub signature_timestamp: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceIntegrityDimensions {
    pub po_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResponse {
    pub playability_status: PlayabilityStatus,
    #[serde(default)]
    pub player_config: Option<PlayerConfig>,
    #[serde(default)]
    pub streaming_data: Option<StreamingData>,
    #[serde(default)]
    pub video_details: Option<VideoDetails>,
    #[serde(default)]
    pub playback_tracking: Option<PlaybackTracking>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayabilityStatus {
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
}

impl PlayabilityStatus {
    pub fn is_ok(&self) -> bool {
        self.status == "OK"
    }
    /// Age/login gate — Phase 2 retries with WEB_CREATOR / embedded. context/03.
    pub fn is_age_gated(&self) -> bool {
        matches!(
            self.status.as_str(),
            "AGE_CHECK_REQUIRED"
                | "AGE_VERIFICATION_REQUIRED"
                | "LOGIN_REQUIRED"
                | "CONTENT_CHECK_REQUIRED"
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerConfig {
    #[serde(default)]
    pub audio_config: Option<AudioConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioConfig {
    #[serde(default)]
    pub loudness_db: Option<f64>,
    #[serde(default)]
    pub perceptual_loudness_db: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingData {
    #[serde(default)]
    pub formats: Option<Vec<Format>>,
    #[serde(default)]
    pub adaptive_formats: Vec<Format>,
    #[serde(default, deserialize_with = "deserialize_opt_i64_lenient")]
    pub expires_in_seconds: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    pub itag: i32,
    /// Direct URL, or `None` when `signature_cipher` is used (needs decipher — Phase 2).
    #[serde(default)]
    pub url: Option<String>,
    pub mime_type: String,
    #[serde(default, deserialize_with = "deserialize_i64_lenient")]
    pub bitrate: i64,
    /// `None` for audio-only formats → used to detect audio.
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
    #[serde(default)]
    pub content_length: Option<String>,
    #[serde(default)]
    pub audio_quality: Option<String>,
    #[serde(default)]
    pub approx_duration_ms: Option<String>,
    #[serde(default)]
    pub audio_channels: Option<i32>,
    #[serde(default)]
    pub loudness_db: Option<f64>,
    #[serde(default)]
    pub signature_cipher: Option<String>,
    #[serde(default)]
    pub cipher: Option<String>,
    #[serde(default)]
    pub audio_track: Option<AudioTrack>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    #[serde(default)]
    pub is_auto_dubbed: Option<bool>,
}

impl Format {
    /// Audio-only formats have no width. context/03.
    pub fn is_audio(&self) -> bool {
        self.width.is_none()
    }
    /// Not an auto-dubbed foreign-language track. context/03.
    pub fn is_original(&self) -> bool {
        self.audio_track.as_ref().and_then(|t| t.is_auto_dubbed).is_none()
    }
    /// Direct, playable URL with no cipher required (Phase 1 only accepts these).
    pub fn direct_url(&self) -> Option<&str> {
        if self.signature_cipher.is_some() || self.cipher.is_some() {
            return None;
        }
        self.url.as_deref()
    }
    fn quality_rank(&self) -> u8 {
        match self.audio_quality.as_deref() {
            Some("AUDIO_QUALITY_HIGH") => 3,
            Some("AUDIO_QUALITY_MEDIUM") => 2,
            Some("AUDIO_QUALITY_LOW") => 1,
            _ => 0,
        }
    }
    fn codec_score(&self) -> u8 {
        if self.mime_type.contains("opus") {
            2
        } else if self.mime_type.contains("mp4a") {
            1
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioQuality {
    High,
    Low,
    /// Desktop has no metered-network concept → treat AUTO as "prefer HIGH" (context/12).
    Auto,
}

/// Pick the best audio format for the requested quality. Port of `YTPlayerUtils.findFormat`,
/// context/03. Returns a reference into `adaptive_formats`.
pub fn find_format(data: &StreamingData, quality: AudioQuality) -> Option<&Format> {
    let audio: Vec<&Format> = data.adaptive_formats.iter().filter(|f| f.is_audio()).collect();
    if audio.is_empty() {
        return None;
    }
    match quality {
        AudioQuality::High | AudioQuality::Auto => audio.into_iter().max_by(|a, b| {
            a.quality_rank()
                .cmp(&b.quality_rank())
                .then(a.audio_channels.unwrap_or(2).cmp(&b.audio_channels.unwrap_or(2)))
                .then(a.codec_score().cmp(&b.codec_score()))
                .then(a.bitrate.cmp(&b.bitrate))
        }),
        AudioQuality::Low => {
            let capped: Vec<&&Format> = audio.iter().filter(|f| f.bitrate <= 128_000).collect();
            let pool = if capped.is_empty() { audio.iter().collect() } else { capped };
            // Prefer original (non-dubbed), then highest bitrate under the cap.
            pool.into_iter()
                .max_by(|a, b| {
                    a.is_original().cmp(&b.is_original()).then(a.bitrate.cmp(&b.bitrate))
                })
                .copied()
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoDetails {
    pub video_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub length_seconds: Option<String>,
    #[serde(default)]
    pub music_video_type: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<Thumbnails>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Thumbnails {
    #[serde(default)]
    pub thumbnails: Vec<Thumbnail>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Thumbnail {
    pub url: String,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: YouTube sends `expiresInSeconds` as a STRING ("21540") on every client.
    /// Parsing it as i64 rejected the whole response and exhausted all direct clients.
    #[test]
    fn streaming_data_string_expiry_parses() {
        let json = r#"{
            "playabilityStatus": { "status": "OK" },
            "streamingData": {
                "expiresInSeconds": "21540",
                "adaptiveFormats": [{
                    "itag": 251,
                    "url": "https://example.com/a",
                    "mimeType": "audio/webm; codecs=\"opus\"",
                    "bitrate": "141210",
                    "audioQuality": "AUDIO_QUALITY_MEDIUM"
                }]
            }
        }"#;
        let resp: PlayerResponse = serde_json::from_str(json).unwrap();
        let sd = resp.streaming_data.unwrap();
        assert_eq!(sd.expires_in_seconds, Some(21540));
        assert_eq!(sd.adaptive_formats[0].bitrate, 141210);
        assert!(find_format(&sd, AudioQuality::High).is_some());
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackTracking {
    #[serde(default)]
    pub videostats_playback_url: Option<BaseUrl>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseUrl {
    #[serde(default)]
    pub base_url: Option<String>,
}
