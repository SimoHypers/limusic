//! High-level endpoint facade over the transport. context/03, 08.

use serde::Serialize;

use crate::clients::YouTubeClient;
use crate::models::browse::{
    self, AlbumPage, ArtistPage, BrowseItem, HomePage, PlaylistContinuation, PlaylistPage,
    SearchResults,
};
use crate::models::context::Context;
use crate::models::metadata::{self, AccountInfo, NextResult, SearchResult};
use crate::models::player::{
    ContentPlaybackContext, PlaybackContext, PlayerBody, PlayerResponse, ServiceIntegrityDimensions,
};
use crate::transport::{Error, InnerTube};

/// Search filter params (opaque base64). context/08.
pub const FILTER_SONG: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D";
pub const FILTER_ALBUM: &str = "EgWKAQIYAWoKEAkQChAFEAMQBA%3D%3D";
pub const FILTER_ARTIST: &str = "EgWKAQIgAWoKEAkQChAFEAMQBA%3D%3D";
pub const FILTER_COMMUNITY_PLAYLIST: &str = "EgeKAQQoAEABagoQAxAEEAoQCRAF";

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

    /// Raw `search` POST. `params` = a filter (None = the mixed, unfiltered search). context/08.
    async fn search_raw(
        &self,
        client: &YouTubeClient,
        query: &str,
        params: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct SearchBody {
            context: Context,
            query: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            params: Option<String>,
        }
        let body = SearchBody {
            context: self.context_for(client),
            query: query.to_owned(),
            params: params.map(str::to_owned),
        };
        self.post("search", client, &body, true).await
    }

    /// Search songs only (`FILTER_SONG`). context/08.
    pub async fn search_songs(
        &self,
        metadata_client: &YouTubeClient,
        query: &str,
    ) -> Result<SearchResult, Error> {
        let value = self.search_raw(metadata_client, query, Some(FILTER_SONG)).await?;
        Ok(metadata::parse_search(&value))
    }

    /// Unfiltered search → categorized sections (top / songs / albums / artists / playlists).
    pub async fn search_all(
        &self,
        client: &YouTubeClient,
        query: &str,
    ) -> Result<SearchResults, Error> {
        let value = self.search_raw(client, query, None).await?;
        Ok(browse::parse_search_all(&value))
    }

    /// Filtered card search for a "Show more" page. `category` ∈ albums / artists / playlists.
    pub async fn search_cards(
        &self,
        client: &YouTubeClient,
        query: &str,
        category: &str,
    ) -> Result<Vec<BrowseItem>, Error> {
        let filter = match category {
            "albums" => FILTER_ALBUM,
            "artists" => FILTER_ARTIST,
            "playlists" => FILTER_COMMUNITY_PLAYLIST,
            other => return Err(Error::Other(format!("unknown search category: {other}"))),
        };
        let value = self.search_raw(client, query, Some(filter)).await?;
        Ok(browse::parse_search_cards(&value))
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

    /// Logged-in account summary (`account/account_menu`, context/01). Requires a cookie. Also the
    /// source of `dataSyncId` (context/04A) and a login-bound visitorData (context/15).
    pub async fn account_menu(&self, client: &YouTubeClient) -> Result<AccountInfo, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct AccountMenuBody {
            context: Context,
        }
        let body = AccountMenuBody { context: self.context_for(client) };
        let value = self.post("account/account_menu", client, &body, true).await?;
        Ok(metadata::parse_account_menu(&value))
    }

    /// Raw `browse` call (context/01, context/08). `browse_id`/`params` optional; response is the
    /// deeply-nested renderer tree the browse parsers walk.
    async fn browse(
        &self,
        client: &YouTubeClient,
        browse_id: Option<&str>,
        params: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct BrowseBody {
            context: Context,
            #[serde(skip_serializing_if = "Option::is_none")]
            browse_id: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            params: Option<String>,
        }
        let body = BrowseBody {
            context: self.context_for(client),
            browse_id: browse_id.map(str::to_owned),
            params: params.map(str::to_owned),
        };
        let value = self.post("browse", client, &body, true).await?;
        // A stale cookie authenticates transport-wise but YouTube returns a logged-out "Sign in"
        // state for account-scoped browse. Surface it as a clear error, not a blank page.
        if self.is_logged_in() && browse::is_signed_out(&value) {
            return Err(Error::SessionExpired);
        }
        Ok(value)
    }

    /// Home feed (`FEmusic_home`). context/08.
    pub async fn home(&self, client: &YouTubeClient) -> Result<HomePage, Error> {
        let value = self.browse(client, Some("FEmusic_home"), None).await?;
        Ok(browse::parse_home(&value))
    }

    /// Library playlists grid (`FEmusic_liked_playlists`). context/08. Needs login.
    pub async fn library_playlists(&self, client: &YouTubeClient) -> Result<Vec<BrowseItem>, Error> {
        let value = self.browse(client, Some("FEmusic_liked_playlists"), None).await?;
        Ok(browse::parse_library(&value))
    }

    /// A playlist or album page by browseId (`VL…` / `MPRE…`). context/08.
    pub async fn playlist(
        &self,
        client: &YouTubeClient,
        browse_id: &str,
    ) -> Result<PlaylistPage, Error> {
        let value = self.browse(client, Some(browse_id), None).await?;
        Ok(browse::parse_playlist(&value))
    }

    /// An album page by album browseId (`MPRE…`). context/08.
    pub async fn album(&self, client: &YouTubeClient, browse_id: &str) -> Result<AlbumPage, Error> {
        let value = self.browse(client, Some(browse_id), None).await?;
        Ok(browse::parse_album(&value))
    }

    /// An artist page by channel browseId (`UC…`). context/08.
    pub async fn artist(
        &self,
        client: &YouTubeClient,
        browse_id: &str,
    ) -> Result<ArtistPage, Error> {
        let value = self.browse(client, Some(browse_id), None).await?;
        Ok(browse::parse_artist(&value, browse_id))
    }

    /// A browse target that returns a grid of cards (e.g. an artist's "all albums" page reached
    /// via a carousel's "More" button). context/08.
    pub async fn browse_grid(
        &self,
        client: &YouTubeClient,
        browse_id: &str,
        params: Option<&str>,
    ) -> Result<Vec<BrowseItem>, Error> {
        let value = self.browse(client, Some(browse_id), params).await?;
        Ok(browse::parse_library(&value))
    }

    /// Next page of playlist tracks via a continuation token. context/08.
    pub async fn playlist_continuation(
        &self,
        client: &YouTubeClient,
        token: &str,
    ) -> Result<PlaylistContinuation, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct ContinuationBody {
            context: Context,
        }
        let body = ContinuationBody { context: self.context_for(client) };
        // Continuation is carried in the query, matching Metrolist's browse-continuation call.
        let enc = urlencoding::encode(token);
        let path = format!("browse?ctoken={enc}&continuation={enc}&type=next");
        let value = self.post(&path, client, &body, true).await?;
        Ok(browse::parse_playlist_continuation(&value))
    }

    // --- write actions (context/01 ✎, context/15 D7). All auth-gated (SAPISIDHASH). ---------

    /// Like or un-like a video. context/01.
    pub async fn like(
        &self,
        client: &YouTubeClient,
        video_id: &str,
        liked: bool,
    ) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct LikeBody {
            context: Context,
            target: Target,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Target {
            video_id: String,
        }
        let path = if liked { "like/like" } else { "like/removelike" };
        let body = LikeBody {
            context: self.context_for(client),
            target: Target { video_id: video_id.to_owned() },
        };
        self.post(path, client, &body, true).await?;
        Ok(())
    }

    /// Add a video to a playlist. context/01 `browse/edit_playlist`.
    pub async fn playlist_add(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        video_id: &str,
    ) -> Result<(), Error> {
        self.edit_playlist(
            client,
            playlist_id,
            serde_json::json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": video_id }),
        )
        .await
    }

    /// Remove a video from a playlist. Needs `set_video_id` (the item's playlistSetVideoId).
    pub async fn playlist_remove(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        video_id: &str,
        set_video_id: &str,
    ) -> Result<(), Error> {
        self.edit_playlist(
            client,
            playlist_id,
            serde_json::json!({
                "action": "ACTION_REMOVE_VIDEO",
                "setVideoId": set_video_id,
                "removedVideoId": video_id,
            }),
        )
        .await
    }

    /// Rename a playlist you own. context/01 `browse/edit_playlist`.
    pub async fn playlist_rename(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        name: &str,
    ) -> Result<(), Error> {
        self.edit_playlist(
            client,
            playlist_id,
            serde_json::json!({ "action": "ACTION_SET_PLAYLIST_NAME", "playlistName": name }),
        )
        .await
    }

    async fn edit_playlist(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        action: serde_json::Value,
    ) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct EditBody {
            context: Context,
            playlist_id: String,
            actions: Vec<serde_json::Value>,
        }
        let body = EditBody {
            context: self.context_for(client),
            playlist_id: strip_vl(playlist_id).to_owned(),
            actions: vec![action],
        };
        self.post("browse/edit_playlist", client, &body, true).await?;
        Ok(())
    }

    /// Create a private playlist; returns the new playlistId. context/01 `playlist/create`.
    pub async fn create_playlist(
        &self,
        client: &YouTubeClient,
        title: &str,
    ) -> Result<String, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateBody {
            context: Context,
            title: String,
            privacy_status: String,
        }
        let body = CreateBody {
            context: self.context_for(client),
            title: title.to_owned(),
            privacy_status: "PRIVATE".to_owned(),
        };
        let value = self.post("playlist/create", client, &body, true).await?;
        metadata::find_first_str(&value, "playlistId")
            .ok_or_else(|| Error::Other("create_playlist: no playlistId in response".into()))
    }

    /// Delete a playlist you own. context/01 `playlist/delete`.
    pub async fn delete_playlist(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
    ) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct DeleteBody {
            context: Context,
            playlist_id: String,
        }
        let body = DeleteBody {
            context: self.context_for(client),
            playlist_id: strip_vl(playlist_id).to_owned(),
        };
        self.post("playlist/delete", client, &body, true).await?;
        Ok(())
    }

    /// Subscribe / unsubscribe to a channel (artist). context/01 `subscription/*`.
    pub async fn subscribe(
        &self,
        client: &YouTubeClient,
        channel_id: &str,
        subscribed: bool,
    ) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct SubBody {
            context: Context,
            channel_ids: Vec<String>,
        }
        let path = if subscribed { "subscription/subscribe" } else { "subscription/unsubscribe" };
        let body =
            SubBody { context: self.context_for(client), channel_ids: vec![channel_id.to_owned()] };
        self.post(path, client, &body, true).await?;
        Ok(())
    }
}

/// Playlist edit/delete want the raw playlistId; browse gives it `VL`-prefixed. context/01.
fn strip_vl(id: &str) -> &str {
    id.strip_prefix("VL").unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_vl_prefix() {
        assert_eq!(strip_vl("VLPL123"), "PL123");
        assert_eq!(strip_vl("PL123"), "PL123");
    }

    #[test]
    fn create_playlist_id_parsed() {
        let resp = json!({ "playlistId": "PLnew123", "status": "STATUS_SUCCEEDED" });
        assert_eq!(metadata::find_first_str(&resp, "playlistId").as_deref(), Some("PLnew123"));
    }
}
