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
    /// Video formats only. Used to prefer 30fps over the 60fps twin at the same height.
    #[serde(default)]
    pub fps: Option<i32>,
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
    /// Set on the video's own language track. A dubbed video lists every language at the same
    /// qualities, so without this the bitrate tiebreak picks one at random (#286 follow-up).
    #[serde(default)]
    pub audio_is_default: Option<bool>,
}

impl Format {
    /// Audio-only formats have no width. context/03.
    pub fn is_audio(&self) -> bool {
        self.width.is_none()
    }
    /// The video's own audio, not a dub. context/03 checks only `isAutoDubbed`, which a human dub
    /// and some auto dubs don't carry; `audioIsDefault` marks the original on every multi-track
    /// response seen so far. A format with no `audioTrack` is the only track there is.
    pub fn is_original(&self) -> bool {
        self.audio_track
            .as_ref()
            .is_none_or(|t| t.is_auto_dubbed != Some(true) && t.audio_is_default == Some(true))
    }
    /// Direct, playable URL with no cipher required (present on the non-web fallback clients).
    pub fn direct_url(&self) -> Option<&str> {
        if self.signature_cipher.is_some() || self.cipher.is_some() {
            return None;
        }
        self.url.as_deref()
    }
    /// The raw `signatureCipher` (or legacy `cipher`) query string, when the format is ciphered.
    /// The orchestrator hands this to the cipher webview to deobfuscate (context/05, Phase 2).
    pub fn cipher_string(&self) -> Option<&str> {
        self.signature_cipher.as_deref().or(self.cipher.as_deref())
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
/// context/03. Returns a reference into `adaptive_formats`, or into `formats` when there is no
/// adaptive audio: a podcast added to the library by RSS feed is not hosted by YouTube, and WEB_REMIX
/// answers with one progressive `audio/mpeg` entry pointing at the feed's own enclosure (#294).
pub fn find_format(data: &StreamingData, quality: AudioQuality) -> Option<&Format> {
    let mut audio: Vec<&Format> = data.adaptive_formats.iter().filter(|f| f.is_audio()).collect();
    if audio.is_empty() {
        audio = data.formats.iter().flatten().filter(|f| f.is_audio()).collect();
    }
    if audio.is_empty() {
        return None;
    }
    // A dubbed video lists every language at the same qualities, so the original is picked
    // before anything else: a dub is never the better stream, at any bitrate.
    let original: Vec<&Format> = audio.iter().copied().filter(|f| f.is_original()).collect();
    let audio = if original.is_empty() { audio } else { original };
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

/// Best VP9 video-only format at or below `max_height`, for the player view's music-video mode.
///
/// Video-only: the audio still comes from mpv, which is playing the same videoId. VP9 rather than
/// MP4 because a stock Fedora only ships openh264 (constrained baseline) and YouTube's 720p/1080p
/// MP4 is High profile, so itag 137 would fail for those users. context/03.
pub fn find_video_format(data: &StreamingData, max_height: i32) -> Option<&Format> {
    data.adaptive_formats
        .iter()
        .filter(|f| {
            !f.is_audio()
                && f.mime_type.contains("vp9")
                && f.height.is_some_and(|h| h <= max_height)
        })
        // Biggest picture that fits, then the smoother of a 30/60fps pair, then the better encode.
        .max_by(|a, b| {
            a.height
                .cmp(&b.height)
                .then(a.fps.unwrap_or(30).cmp(&b.fps.unwrap_or(30)))
                .then(a.bitrate.cmp(&b.bitrate))
        })
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

impl VideoDetails {
    /// Whether this videoId is a video upload rather than the audio track YouTube generates for a
    /// release, from YouTube's own `musicVideoType`. `None` when the response didn't say, which is
    /// every non-Music client: only WEB_REMIX carries the field, so a fallback client's answer
    /// must not be read as "not a video". Authoritative for the player view's music-video mode,
    /// where the queue row's flag is only as good as the parse that built it.
    pub fn is_music_video(&self) -> Option<bool> {
        Some(super::metadata::is_video_type(self.music_video_type.as_deref()?))
    }

    /// Whether this videoId is a podcast episode. `None` when the response didn't say, as above.
    pub fn is_podcast_episode(&self) -> Option<bool> {
        Some(self.music_video_type.as_deref()? == super::metadata::PODCAST_EPISODE_TYPE)
    }
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

    /// A dubbed video lists each language at the same qualities; the higher-bitrate dub must not
    /// win over the original, at either quality setting. Track shape from a live VISIONOS response.
    #[test]
    fn find_format_keeps_the_original_language() {
        let json = r#"{
            "playabilityStatus": { "status": "OK" },
            "streamingData": { "adaptiveFormats": [
                { "itag": 251, "url": "hi", "mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 150000, "audioQuality": "AUDIO_QUALITY_MEDIUM",
                  "audioTrack": { "displayName": "Hindi", "id": "hi.3" } },
                { "itag": 251, "url": "en", "mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 140000, "audioQuality": "AUDIO_QUALITY_MEDIUM",
                  "audioTrack": { "displayName": "English original", "id": "en.4", "audioIsDefault": true, "isAutoDubbed": false } },
                { "itag": 249, "url": "es", "mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 90000, "audioQuality": "AUDIO_QUALITY_LOW",
                  "audioTrack": { "displayName": "Spanish", "id": "es.3", "audioIsDefault": false, "isAutoDubbed": true } }
            ] }
        }"#;
        let sd = serde_json::from_str::<PlayerResponse>(json).unwrap().streaming_data.unwrap();
        assert_eq!(find_format(&sd, AudioQuality::High).unwrap().url.as_deref(), Some("en"));
        assert_eq!(find_format(&sd, AudioQuality::Low).unwrap().url.as_deref(), Some("en"));
    }

    /// An RSS-feed podcast episode (#294): no adaptive formats, only the feed's own mp3 as a
    /// progressive format. Shape from a live WEB_REMIX response.
    #[test]
    fn find_format_falls_back_to_a_progressive_rss_enclosure() {
        let json = r#"{
            "playabilityStatus": { "status": "OK" },
            "streamingData": { "expiresInSeconds": "21540", "formats": [
                { "itag": 15, "url": "https://www.podtrac.com/pts/redirect.mp3/x.mp3", "mimeType": "audio/mpeg",
                  "contentLength": "0", "approxDurationMs": "5656000",
                  "audioTrack": { "displayName": "Ep. 079", "audioIsDefault": true } }
            ] }
        }"#;
        let sd = serde_json::from_str::<PlayerResponse>(json).unwrap().streaming_data.unwrap();
        assert_eq!(find_format(&sd, AudioQuality::High).unwrap().itag, 15);
        assert_eq!(find_format(&sd, AudioQuality::Low).unwrap().itag, 15);
    }

    /// Video-only picker (plan 031): VP9 only, capped by height, 60fps preferred over the 30fps
    /// twin (plan 032: decode was never the bottleneck). AV1 and MP4 are deliberately not
    /// fallbacks; see `find_video_format`.
    #[test]
    fn find_video_format_picks_capped_vp9() {
        let json = r#"{
            "playabilityStatus": { "status": "OK" },
            "streamingData": { "adaptiveFormats": [
                { "itag": 251, "url": "a", "mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 141210 },
                { "itag": 243, "url": "v360", "mimeType": "video/webm; codecs=\"vp9\"", "bitrate": 300000, "width": 640, "height": 360, "fps": 30 },
                { "itag": 244, "url": "v480", "mimeType": "video/webm; codecs=\"vp9\"", "bitrate": 600000, "width": 854, "height": 480, "fps": 30 },
                { "itag": 247, "url": "v720", "mimeType": "video/webm; codecs=\"vp9\"", "bitrate": 1500000, "width": 1280, "height": 720, "fps": 24 },
                { "itag": 302, "url": "v720-60", "mimeType": "video/webm; codecs=\"vp9\"", "bitrate": 2500000, "width": 1280, "height": 720, "fps": 60 },
                { "itag": 248, "url": "v1080", "mimeType": "video/webm; codecs=\"vp9\"", "bitrate": 3000000, "width": 1920, "height": 1080, "fps": 30 },
                { "itag": 399, "url": "av1", "mimeType": "video/mp4; codecs=\"av01.0.08M.08\"", "bitrate": 2000000, "width": 1920, "height": 1080, "fps": 30 },
                { "itag": 137, "url": "h264", "mimeType": "video/mp4; codecs=\"avc1.640028\"", "bitrate": 4000000, "width": 1920, "height": 1080, "fps": 30 }
            ] }
        }"#;
        let sd = serde_json::from_str::<PlayerResponse>(json).unwrap().streaming_data.unwrap();
        assert_eq!(find_video_format(&sd, 720).unwrap().itag, 302); // 720p60 over the 720p24
        assert_eq!(find_video_format(&sd, 480).unwrap().itag, 244);
        assert_eq!(find_video_format(&sd, 2160).unwrap().itag, 248); // never the AV1/H.264 1080p
    }

    /// A real WEB_REMIX `/player` reply for an OMV (dQw4w9WgXcQ, 2026-09-30, signed in; trimmed to
    /// the fields read here, cipher strings scrubbed). The picture sits in the same
    /// `adaptiveFormats` as the audio, which is what lets the orchestrator reuse the audio's reply,
    /// and every format is ciphered, so the video URL has to be deciphered like the audio's.
    #[test]
    fn web_remix_reply_carries_the_music_video() {
        let r: PlayerResponse =
            serde_json::from_str(include_str!("../../tests/fixtures/player_web_remix_omv.json"))
                .unwrap();
        assert!(r.playability_status.is_ok());
        assert_eq!(r.video_details.as_ref().and_then(|v| v.is_music_video()), Some(true));
        let sd = r.streaming_data.unwrap();
        let v = find_video_format(&sd, 720).unwrap();
        assert_eq!(v.itag, 247); // the 720p VP9, never the H.264 136 beside it
        assert!(v.direct_url().is_none() && v.cipher_string().is_some());
        assert_eq!(find_video_format(&sd, 480).unwrap().itag, 244);
        assert!(find_format(&sd, AudioQuality::High).unwrap().is_audio());
    }

    /// Audio-only response (the ordinary case for a song): no video, so the view keeps the artwork.
    #[test]
    fn find_video_format_none_without_vp9() {
        let json = r#"{
            "playabilityStatus": { "status": "OK" },
            "streamingData": { "adaptiveFormats": [
                { "itag": 251, "url": "a", "mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 141210 },
                { "itag": 137, "url": "h264", "mimeType": "video/mp4; codecs=\"avc1.640028\"", "bitrate": 4000000, "width": 1920, "height": 1080, "fps": 30 }
            ] }
        }"#;
        let sd = serde_json::from_str::<PlayerResponse>(json).unwrap().streaming_data.unwrap();
        assert!(find_video_format(&sd, 720).is_none());
    }

    /// The player view's video mode believes this over the queue row's flag, so ATV must classify
    /// as audio and a response without the field must stay undecided rather than answer "audio".
    #[test]
    fn music_video_type_classifies_the_track() {
        let details = |kind: &str| {
            let t = if kind.is_empty() {
                String::new()
            } else {
                format!(r#", "musicVideoType": "{kind}""#)
            };
            let body = format!(
                r#"{{ "playabilityStatus": {{ "status": "OK" }},
                      "videoDetails": {{ "videoId": "a"{t} }} }}"#
            );
            serde_json::from_str::<PlayerResponse>(&body).unwrap().video_details.unwrap()
        };
        let atv = details("MUSIC_VIDEO_TYPE_ATV");
        let omv = details("MUSIC_VIDEO_TYPE_OMV");
        let ugc = details("MUSIC_VIDEO_TYPE_UGC");
        let bare = details("");
        assert_eq!(atv.is_music_video(), Some(false));
        assert_eq!(omv.is_music_video(), Some(true));
        assert_eq!(ugc.is_music_video(), Some(true));
        assert_eq!(bare.is_music_video(), None);
        // A personal upload is audio, not a music video (issue #71).
        let upload = details("MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK");
        assert_eq!(upload.is_music_video(), Some(false));
        // A podcast episode (#500): its own kind, and unknown when the client didn't say.
        assert_eq!(details("MUSIC_VIDEO_TYPE_PODCAST_EPISODE").is_podcast_episode(), Some(true));
        assert_eq!(omv.is_podcast_episode(), Some(false));
        assert_eq!(bare.is_podcast_episode(), None);
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
