//! High-level endpoint facade over the transport. context/03, 08.

use serde::Serialize;

use crate::clients::YouTubeClient;
use crate::models::context::Context;
use crate::models::metadata::{self, NextResult, SearchResult};
use crate::models::player::{
    ContentPlaybackContext, PlaybackContext, PlayerBody, PlayerResponse, ServiceIntegrityDimensions,
};
use crate::transport::{Error, InnerTube};

/// Search filter params (opaque base64). context/08.
pub const FILTER_SONG: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D";

impl InnerTube {
    /// `/player` for one client. context/03, context/06.
    ///
    /// `sts` — signature timestamp from the deciphering player.js (context/05); sent as
    /// `playbackContext.contentPlaybackContext.signatureTimestamp` so ciphered clients return
    /// usable formats. `po_token` — the session/streaming PoToken (context/04); sent as
    /// `serviceIntegrityDimensions.poToken` for web clients. Both `None` for the plain
    /// direct-URL clients that need neither.
    pub async fn player(
        &self,
        client: &YouTubeClient,
        video_id: &str,
        playlist_id: Option<&str>,
        sts: Option<i32>,
        po_token: Option<&str>,
    ) -> Result<PlayerResponse, Error> {
        let mut context = self.context_for(client);
        if let Some(tp) = context.third_party.as_mut() {
            tp.embed_url = format!("https://www.youtube.com/watch?v={video_id}");
        }
        let body = PlayerBody {
            context,
            video_id: video_id.to_owned(),
            playlist_id: playlist_id.map(str::to_owned),
            playback_context: sts.map(|signature_timestamp| PlaybackContext {
                content_playback_context: ContentPlaybackContext { signature_timestamp },
            }),
            service_integrity_dimensions: po_token.map(|t| ServiceIntegrityDimensions {
                po_token: t.to_owned(),
            }),
            content_check_ok: true,
            racy_check_ok: true,
        };
        let value = self.post("player", client, &body, /* set_login */ true).await?;
        Ok(serde_json::from_value(value)?)
    }

    /// Search songs. Uses the metadata client (WEB_REMIX renderer shape). context/08.
    pub async fn search_songs(
        &self,
        metadata_client: &YouTubeClient,
        query: &str,
    ) -> Result<SearchResult, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct SearchBody {
            context: Context,
            query: String,
            params: String,
        }
        let body = SearchBody {
            context: self.context_for(metadata_client),
            query: query.to_owned(),
            params: FILTER_SONG.to_owned(),
        };
        let value = self.post("search", metadata_client, &body, true).await?;
        Ok(metadata::parse_search(&value))
    }

    /// Up-next queue / radio for a video. context/08. Uses the metadata client.
    pub async fn next(
        &self,
        metadata_client: &YouTubeClient,
        video_id: &str,
        playlist_id: Option<&str>,
    ) -> Result<NextResult, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct NextBody {
            context: Context,
            video_id: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            playlist_id: Option<String>,
            is_audio_only: bool,
        }
        let body = NextBody {
            context: self.context_for(metadata_client),
            video_id: video_id.to_owned(),
            playlist_id: playlist_id.map(str::to_owned),
            is_audio_only: true,
        };
        let value = self.post("next", metadata_client, &body, true).await?;
        Ok(metadata::parse_next(&value))
    }

    fn context_for(&self, client: &YouTubeClient) -> Context {
        client.to_context(
            &self.session.locale,
            self.session.visitor_data.as_deref(),
            self.session.data_sync_id.as_deref(),
        )
    }
}
