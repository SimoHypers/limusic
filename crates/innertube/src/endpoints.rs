//! High-level endpoint facade over the transport. context/03, 08.

use serde::Serialize;

use crate::clients::YouTubeClient;
use crate::models::browse::{
    self, AlbumPage, ArtistPage, BrowseItem, HistoryGroup, HomePage, PlaylistContinuation,
    PlaylistPage, PlaylistSort, SearchResults,
};
use crate::models::context::Context;
use crate::models::lyrics::{self, PlainLyrics, TimedLyricLine};
use crate::models::metadata::{
    self, AccountIdentity, AccountInfo, NextResult, Rating, SearchResult, SongItem,
};
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
            service_integrity_dimensions: po_token
                .map(|t| ServiceIntegrityDimensions { po_token: t.to_owned() }),
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

    // --- "hide music videos" (user setting, off by default) ------------------------------------
    //
    // Gated at the fetch boundary rather than at each queue/search call site: every consumer
    // inherits it and there is no second policy to keep in sync. A row is a video when YouTube's
    // own `musicVideoType` says so (see `metadata::is_video_row`) — never by matching the title.

    fn drop_video_songs(&self, items: &mut Vec<SongItem>) {
        if self.hide_videos() {
            items.retain(|i| !i.is_video);
        }
    }

    fn drop_video_cards(&self, items: &mut Vec<BrowseItem>) {
        if self.hide_videos() {
            items.retain(|i| !i.is_video);
        }
    }

    /// Search songs only (`FILTER_SONG`). context/08.
    pub async fn search_songs(
        &self,
        metadata_client: &YouTubeClient,
        query: &str,
    ) -> Result<SearchResult, Error> {
        let value = self.search_raw(metadata_client, query, Some(FILTER_SONG)).await?;
        let mut r = metadata::parse_search(&value);
        self.drop_video_songs(&mut r.items);
        Ok(r)
    }

    /// Unfiltered search → categorized sections (top / songs / albums / artists / playlists).
    pub async fn search_all(
        &self,
        client: &YouTubeClient,
        query: &str,
    ) -> Result<SearchResults, Error> {
        let value = self.search_raw(client, query, None).await?;
        let mut r = browse::parse_search_all(&value);
        self.drop_video_cards(&mut r.top);
        self.drop_video_cards(&mut r.songs);
        Ok(r)
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
    ///
    /// `video_id` is optional: an artist/mood radio is a playlist id with no seed track
    /// (`playlistId` alone), which is how YouTube itself opens one.
    pub async fn next(
        &self,
        metadata_client: &YouTubeClient,
        video_id: Option<&str>,
        playlist_id: Option<&str>,
    ) -> Result<NextResult, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct NextBody {
            context: Context,
            #[serde(skip_serializing_if = "Option::is_none")]
            video_id: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            playlist_id: Option<String>,
            is_audio_only: bool,
        }
        let body = NextBody {
            context: self.context_for(metadata_client),
            video_id: video_id.map(str::to_owned),
            playlist_id: playlist_id.map(str::to_owned),
            is_audio_only: true,
        };
        let value = self.post("next", metadata_client, &body, true).await?;
        let mut next = metadata::parse_next(&value);
        // The seed itself survives: "start radio from this video" must still open on that video,
        // and the track already playing is never yanked out from under the user.
        if self.hide_videos() {
            next.items.retain(|i| !i.is_video || Some(i.video_id.as_str()) == video_id);
        }
        Ok(next)
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

    /// Validate and refresh one delegated identity without mutating the transport's shared
    /// selection. This keeps unrelated in-flight requests on the previously committed channel.
    pub async fn account_menu_for_identity(
        &self,
        client: &YouTubeClient,
        data_sync_id: &str,
    ) -> Result<AccountInfo, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct AccountMenuBody {
            context: Context,
        }
        let body = AccountMenuBody { context: self.context_for_identity(client, data_sync_id) };
        let value = self.post("account/account_menu", client, &body, true).await?;
        Ok(metadata::parse_account_menu(&value))
    }

    /// Every usable YouTube identity under the signed-in Google account. The official web client
    /// opens this sibling endpoint from the account menu; `account/account_menu` itself only
    /// carries the active header.
    pub async fn account_identities(
        &self,
        client: &YouTubeClient,
    ) -> Result<Vec<AccountIdentity>, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct AccountsListBody {
            context: Context,
        }
        let body = AccountsListBody { context: self.context_for(client) };
        let value = self.post("account/accounts_list", client, &body, true).await?;
        Ok(metadata::parse_account_identities(&value))
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
            return Err(self.reject_session());
        }
        Ok(value)
    }

    /// POST a paging token. The ctoken is carried in the query, matching Metrolist's
    /// browse-continuation call; every paged surface (home, playlist tracks, library grids) uses
    /// this same carrier.
    async fn browse_continuation(
        &self,
        client: &YouTubeClient,
        token: &str,
    ) -> Result<serde_json::Value, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct ContinuationBody {
            context: Context,
        }
        let body = ContinuationBody { context: self.context_for(client) };
        let enc = urlencoding::encode(token);
        let path = format!("browse?ctoken={enc}&continuation={enc}&type=next");
        self.post(&path, client, &body, true).await
    }

    /// A library grid, paged to the end. YouTube hands these out ~25 at a time, so a single browse
    /// silently truncated anyone's bigger library (issue #72).
    ///
    /// Library grids only. `browse_grid`'s "More" targets deliberately do NOT page: they are read
    /// as cards, and some of them (an album's `VL…` shelf) are really 100-track pages, so paging
    /// one would spend a dozen sequential requests building a card wall nobody asked for. The
    /// card grids behind those buttons come back whole anyway (a 91-album discography arrived in
    /// one response, no token).
    async fn library_grid(
        &self,
        client: &YouTubeClient,
        browse_id: &str,
    ) -> Result<Vec<BrowseItem>, Error> {
        let value = self.browse(client, Some(browse_id), None).await?;
        let mut items = browse::parse_library(&value);
        let mut token = browse::continuation_token(&value);
        // ponytail: page cap, so a token that never resolves can't spin forever. Raise it if
        // anyone turns up with a library past ~500 entries.
        for _ in 0..20 {
            let Some(t) = token else { break };
            // A failed page must not throw away the ones that worked: a short library beats an
            // error where the grid should be.
            let Ok(value) = self.browse_continuation(client, &t).await.inspect_err(|e| {
                tracing::warn!(error = %e, browse_id, "grid continuation failed; keeping what loaded")
            }) else {
                break;
            };
            let page = browse::parse_library(&value);
            if page.is_empty() {
                break; // a spurious token (some grids carry one that resolves to nothing)
            }
            items.extend(page);
            token = browse::continuation_token(&value).filter(|next| *next != t);
        }
        Ok(items)
    }

    /// Home feed (`FEmusic_home`). `params` is a mood/genre chip token from a previous home
    /// response — pass it to get that chip's filtered feed. context/08.
    pub async fn home(
        &self,
        client: &YouTubeClient,
        params: Option<&str>,
    ) -> Result<HomePage, Error> {
        let value = self.browse(client, Some("FEmusic_home"), params).await?;
        let mut page = browse::parse_home(&value);
        for s in &mut page.sections {
            self.drop_video_cards(&mut s.items);
        }
        // A shelf the filter emptied (an all-videos row) would render as a bare heading.
        page.sections.retain(|s| !s.items.is_empty());
        Ok(page)
    }

    /// Next batch of home shelves via a continuation token. Same ctoken carrier as
    /// `playlist_continuation`; the response's shelves parse with `parse_home` (its find_all walk
    /// doesn't care whether shelves sit under `contents` or `continuationContents`).
    pub async fn home_continuation(
        &self,
        client: &YouTubeClient,
        token: &str,
    ) -> Result<HomePage, Error> {
        let value = self.browse_continuation(client, token).await?;
        let mut page = browse::parse_home(&value);
        for s in &mut page.sections {
            self.drop_video_cards(&mut s.items);
        }
        page.sections.retain(|s| !s.items.is_empty());
        Ok(page)
    }

    /// Play history (`FEmusic_history`), in YouTube's own date buckets (Today, Yesterday, …).
    /// context/08. Needs login: signed out, YouTube has nothing to return.
    pub async fn history(&self, client: &YouTubeClient) -> Result<Vec<HistoryGroup>, Error> {
        let value = self.browse(client, Some("FEmusic_history"), None).await?;
        let mut groups = browse::parse_history(&value);
        for g in &mut groups {
            self.drop_video_songs(&mut g.items);
        }
        groups.retain(|g| !g.items.is_empty()); // a bucket the filter emptied is a bare heading
        Ok(groups)
    }

    /// Library playlists grid (`FEmusic_liked_playlists`). context/08. Needs login.
    pub async fn library_playlists(
        &self,
        client: &YouTubeClient,
    ) -> Result<Vec<BrowseItem>, Error> {
        self.library_grid(client, "FEmusic_liked_playlists").await
    }

    /// Saved albums grid (`FEmusic_liked_albums`). context/08. Needs login.
    pub async fn library_albums(&self, client: &YouTubeClient) -> Result<Vec<BrowseItem>, Error> {
        self.library_grid(client, "FEmusic_liked_albums").await
    }

    /// The albums the signed-in user uploaded themselves
    /// (`FEmusic_library_privately_owned_releases`). Cards come back as ordinary `MPREb_…` album
    /// browseIds, so they open on the album page like any other. context/08. Needs login.
    pub async fn upload_albums(&self, client: &YouTubeClient) -> Result<Vec<BrowseItem>, Error> {
        self.library_grid(client, "FEmusic_library_privately_owned_releases").await
    }

    /// Library artists (`FEmusic_library_corpus_track_artists`) — the artists behind the songs and
    /// albums in your library, which is what YouTube Music's own Artists tab shows (subscriptions
    /// live under `FEmusic_library_corpus_artists`). context/08. Needs login.
    pub async fn library_artists(&self, client: &YouTubeClient) -> Result<Vec<BrowseItem>, Error> {
        self.library_grid(client, "FEmusic_library_corpus_track_artists").await
    }

    /// A playlist or album page by browseId (`VL…` / `MPRE…`). context/08.
    ///
    /// `sort` asks YouTube to order the tracks — see `PlaylistSort::params`. Passing `None` gets
    /// whatever order the account already has the list in, which is the one thing a fresh visit
    /// wants: it is what YouTube Music would show.
    pub async fn playlist(
        &self,
        client: &YouTubeClient,
        browse_id: &str,
        sort: Option<(PlaylistSort, bool)>,
    ) -> Result<PlaylistPage, Error> {
        let params = sort.map(|(s, desc)| s.params(desc));
        let value = self.browse(client, Some(browse_id), params).await?;
        Ok(browse::parse_playlist(&value))
    }

    /// An album page by album browseId (`MPRE…`). context/08.
    ///
    /// Some album rows link the official music video (`musicVideoType` ≠ ATV), so playing them
    /// streams the MV's audio (intros, skits, crowd) instead of the album track. The album's
    /// `OLAK5uy_` *audio* playlist carries the album-audio uploads in track order, so for those
    /// rows we swap in its videoId (one extra fetch, only for affected albums).
    pub async fn album(&self, client: &YouTubeClient, browse_id: &str) -> Result<AlbumPage, Error> {
        let value = self.browse(client, Some(browse_id), None).await?;
        let mut page = browse::parse_album(&value);
        // Album track rows never carry the album's own browseId (live-checked 2026-09-02, every
        // release type), so without this every track played off a release page reaches the queue
        // with no `album_id` and the ⋮ menu offers no "Go to album". We are holding the id.
        for item in &mut page.items {
            item.album_id.get_or_insert_with(|| browse_id.to_owned());
        }
        let video = browse::album_video_flags(&value);
        if video.contains(&true) {
            if let Some(pl) = &page.playlist_id {
                match self.playlist(client, &format!("VL{pl}"), None).await {
                    // ponytail: positional match, guarded on equal track counts — the OLAK
                    // playlist is the same album in the same order. Mismatch → keep the MV ids.
                    Ok(audio) if audio.items.len() == page.items.len() => {
                        for ((item, is_video), audio_item) in
                            page.items.iter_mut().zip(&video).zip(audio.items)
                        {
                            if *is_video {
                                item.video_id = audio_item.video_id;
                                item.duration = audio_item.duration.or(item.duration.take());
                                // The row linked the MV, but the id it now carries is the audio
                                // track: leaving the flag set would put the player view's video
                                // mode on a still album-art stream.
                                item.is_video = false;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "audio-playlist fetch failed; keeping MV ids")
                    }
                }
            }
        }
        for c in &mut page.sections {
            self.drop_video_cards(&mut c.items);
            // YouTube lists the album you are already on under "Other versions". A card that
            // reopens the current page is noise, so drop it.
            c.items.retain(|i| i.id != browse_id);
        }
        page.sections.retain(|c| !c.items.is_empty());
        Ok(page)
    }

    /// An artist page by channel browseId (`UC…`). context/08.
    pub async fn artist(
        &self,
        client: &YouTubeClient,
        browse_id: &str,
    ) -> Result<ArtistPage, Error> {
        let value = self.browse(client, Some(browse_id), None).await?;
        let mut page = browse::parse_artist(&value, browse_id);
        self.drop_video_songs(&mut page.top_songs);
        for c in &mut page.sections {
            self.drop_video_cards(&mut c.items);
        }
        page.sections.retain(|c| !c.items.is_empty()); // e.g. the artist's "Videos" carousel
        Ok(page)
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
        let mut items = browse::parse_library(&value);
        self.drop_video_cards(&mut items);
        Ok(items)
    }

    /// Next page of playlist tracks via a continuation token. context/08.
    pub async fn playlist_continuation(
        &self,
        client: &YouTubeClient,
        token: &str,
    ) -> Result<PlaylistContinuation, Error> {
        let value = self.browse_continuation(client, token).await?;
        Ok(browse::parse_playlist_continuation(&value))
    }

    // --- lyrics (context/08 §lyrics; browseId comes from `next`) -----------------------------

    /// Line-synced lyrics. `client` must be a mobile identity (`LYRICS_TIMED_CLIENT`) — web
    /// clients never return `timedLyricsData`. Empty vec = track has no timed lyrics.
    pub async fn lyrics_timed(
        &self,
        client: &YouTubeClient,
        browse_id: &str,
    ) -> Result<Vec<TimedLyricLine>, Error> {
        let value = self.browse(client, Some(browse_id), None).await?;
        Ok(lyrics::parse_lyrics_timed(&value))
    }

    /// Plain-text lyrics via WEB_REMIX (`musicDescriptionShelfRenderer`). `None` = none exist.
    pub async fn lyrics_plain(
        &self,
        client: &YouTubeClient,
        browse_id: &str,
    ) -> Result<Option<PlainLyrics>, Error> {
        let value = self.browse(client, Some(browse_id), None).await?;
        Ok(lyrics::parse_lyrics_plain(&value))
    }

    // --- write actions (context/01 ✎, context/15 D7). All auth-gated (SAPISIDHASH). ---------

    /// Rate a video, or clear its rating. context/01. The three states are mutually exclusive on
    /// YouTube's side: disliking a liked track removes it from Liked Music in the same call.
    pub async fn rate(
        &self,
        client: &YouTubeClient,
        video_id: &str,
        rating: Rating,
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
        let path = match rating {
            Rating::Like => "like/like",
            Rating::Dislike => "like/dislike",
            Rating::Indifferent => "like/removelike",
        };
        let body = LikeBody {
            context: self.context_for(client),
            target: Target { video_id: video_id.to_owned() },
        };
        self.post(path, client, &body, true).await?;
        Ok(())
    }

    /// The raw `search` / `browse` responses, for the live smoke tests only (feature-gated, never
    /// built into the app). Diagnosing a menu shape needs the JSON before the parsers drop it.
    #[cfg(feature = "integration-tests")]
    pub async fn search_json(
        &self,
        client: &YouTubeClient,
        query: &str,
        params: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        self.search_raw(client, query, params).await
    }

    #[cfg(feature = "integration-tests")]
    pub async fn browse_json(
        &self,
        client: &YouTubeClient,
        browse_id: &str,
    ) -> Result<serde_json::Value, Error> {
        self.browse(client, Some(browse_id), None).await
    }

    /// Add a track to the library, or take it out: `youtubei/v1/feedback` with a token minted on
    /// the row itself ([`crate::models::LibraryToggle`]). Not the same thing as a like, which is
    /// what `rate` does: Library ▸ Songs and Liked Music are separate lists on the account.
    ///
    /// YouTube answers 200 with a per-token result rather than an HTTP error, so a token that has
    /// gone stale (the row was fetched long enough ago) surfaces here, not as a silent no-op.
    pub async fn feedback(&self, client: &YouTubeClient, token: &str) -> Result<(), Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct FeedbackBody {
            context: Context,
            feedback_tokens: Vec<String>,
        }
        let body = FeedbackBody {
            context: self.context_for(client),
            feedback_tokens: vec![token.to_owned()],
        };
        let value = self.post("feedback", client, &body, true).await?;
        let processed = value
            .pointer("/feedbackResponses/0/isProcessed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if processed {
            Ok(())
        } else {
            Err(Error::Other(
                "YouTube turned down the library change — reopen the list and try again.".into(),
            ))
        }
    }

    /// Save an album/playlist to the library, or remove it. Same `like` endpoint as a track, with
    /// a playlist target: for an album pass its `OLAK5uy_…` audio playlist id. Live-verified.
    pub async fn like_playlist(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
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
            playlist_id: String,
        }
        let path = if liked { "like/like" } else { "like/removelike" };
        let body = LikeBody {
            context: self.context_for(client),
            target: Target { playlist_id: strip_vl(playlist_id).to_owned() },
        };
        self.post(path, client, &body, true).await?;
        Ok(())
    }

    /// Add a video to a playlist. context/01 `browse/edit_playlist`.
    ///
    /// Returns `false` when the track is already in the playlist: YouTube refuses the add (see
    /// `edit_rejection`) rather than storing a second copy.
    pub async fn playlist_add(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        video_id: &str,
    ) -> Result<bool, Error> {
        match self
            .edit_playlist(
                client,
                playlist_id,
                serde_json::json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": video_id }),
            )
            .await
        {
            Ok(()) => Ok(true),
            Err(Error::AlreadyInPlaylist) => Ok(false),
            Err(e) => Err(e),
        }
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

    /// Store a sort order on a playlist you own, so every other client on the account shows the
    /// list the same way. context/01 `browse/edit_playlist`.
    ///
    /// Only meaningful where `SortMenu::editable` said so. A `browseEndpoint`-flavoured list has
    /// no equivalent write: Liked Music persists whatever page was last asked for, and someone
    /// else's playlist keeps nothing at all.
    pub async fn playlist_set_sort(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        sort: PlaylistSort,
    ) -> Result<(), Error> {
        self.edit_playlist(client, playlist_id, sort.edit_action()).await
    }

    /// Edit the details of a playlist you own. context/01 `browse/edit_playlist`.
    ///
    /// Every field is optional and only the ones given are sent, so an edit of the name cannot
    /// blank a description this parser failed to read back. `privacy` is YouTube's own vocabulary:
    /// `PUBLIC` / `PRIVATE` / `UNLISTED`.
    pub async fn playlist_edit_details(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        name: Option<&str>,
        description: Option<&str>,
        privacy: Option<&str>,
    ) -> Result<(), Error> {
        let mut actions = Vec::new();
        if let Some(name) = name {
            actions.push(
                serde_json::json!({ "action": "ACTION_SET_PLAYLIST_NAME", "playlistName": name }),
            );
        }
        if let Some(description) = description {
            actions.push(serde_json::json!({
                "action": "ACTION_SET_PLAYLIST_DESCRIPTION",
                "playlistDescription": description,
            }));
        }
        if let Some(privacy) = privacy {
            actions.push(
                serde_json::json!({ "action": "ACTION_SET_PLAYLIST_PRIVACY", "playlistPrivacy": privacy }),
            );
        }
        if actions.is_empty() {
            return Ok(());
        }
        self.edit_playlist_actions(client, playlist_id, actions).await
    }

    /// Give a playlist you own a cover of your own, the way YouTube Music's web client does it:
    /// open a resumable ("Scotty") upload, send the whole image in one go, then attach the blob id
    /// that comes back. Two of the three calls are not InnerTube endpoints at all. context/01
    /// §custom playlist thumbnail.
    ///
    /// Signed in only, and YouTube treats the slot as square (`studio_square_thumbnail`): a photo
    /// that isn't gets cropped or refused at the far end, which the caller surfaces as-is.
    pub async fn playlist_set_cover(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        image: Vec<u8>,
    ) -> Result<(), Error> {
        // 1. Open the upload. The id comes back in a header; the body says nothing.
        let (headers, _) = self
            .post_upload(
                PLAYLIST_IMAGE_UPLOAD,
                client,
                &[
                    ("x-goog-upload-command", "start".to_owned()),
                    ("x-goog-upload-protocol", "resumable".to_owned()),
                    ("x-goog-upload-header-content-length", image.len().to_string()),
                ],
                Vec::new(),
            )
            .await?;
        let upload_id = headers
            .get("x-guploader-uploadid")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Error::Other("YouTube didn't open an upload for the artwork.".into()))?
            .to_owned();

        // 2. Send the bytes. "Resumable" in name only: one request, offset 0, finalized.
        let (_, body) = self
            .post_upload(
                &format!(
                    "{PLAYLIST_IMAGE_UPLOAD}?upload_id={}&upload_protocol=resumable",
                    urlencoding::encode(&upload_id)
                ),
                client,
                &[
                    ("x-goog-upload-command", "upload, finalize".to_owned()),
                    ("x-goog-upload-offset", "0".to_owned()),
                ],
                image,
            )
            .await?;
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Uploaded {
            encrypted_blob_id: String,
        }
        let uploaded: Uploaded = serde_json::from_slice(&body)
            .map_err(|_| Error::Other("YouTube rejected the artwork upload.".into()))?;

        // 3. Attach the blob to the playlist. This one is an ordinary edit_playlist action.
        self.edit_playlist(
            client,
            playlist_id,
            serde_json::json!({
                "action": "ACTION_SET_CUSTOM_THUMBNAIL",
                "addedCustomThumbnail": {
                    "imageKey": custom_thumbnail_key(),
                    "playlistScottyEncryptedBlobId": uploaded.encrypted_blob_id,
                },
            }),
        )
        .await
        .map_err(cover_refusal)
    }

    /// Drop the custom cover again, so YouTube goes back to building one out of the tracks.
    /// Answers that rebuilt thumbnail: nothing here can guess the collage's URL.
    /// context/01 `browse/edit_playlist`.
    pub async fn playlist_clear_cover(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
    ) -> Result<Option<String>, Error> {
        let value = self
            .edit_playlist_value(
                client,
                playlist_id,
                vec![serde_json::json!({
                    "action": "ACTION_REMOVE_CUSTOM_THUMBNAIL",
                    "deletedCustomThumbnail": custom_thumbnail_key(),
                })],
            )
            .await
            .map_err(cover_refusal)?;
        Ok(edited_thumbnail(&value))
    }

    async fn edit_playlist(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        action: serde_json::Value,
    ) -> Result<(), Error> {
        self.edit_playlist_actions(client, playlist_id, vec![action]).await
    }

    async fn edit_playlist_actions(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        actions: Vec<serde_json::Value>,
    ) -> Result<(), Error> {
        self.edit_playlist_value(client, playlist_id, actions).await.map(|_| ())
    }

    async fn edit_playlist_value(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        actions: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, Error> {
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
            actions,
        };
        let value = self.post("browse/edit_playlist", client, &body, true).await?;
        match edit_rejection(&value) {
            Some(e) => Err(e),
            None => Ok(value),
        }
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

/// Where custom playlist artwork goes up. Not a `youtubei/v1` endpoint: it is Google's generic
/// resumable uploader, sitting on `music.youtube.com` under its own path. context/01.
const PLAYLIST_IMAGE_UPLOAD: &str = "playlist_image_upload/playlist_custom_thumbnail";

/// YouTube saying no to a custom playlist image, as opposed to the network saying nothing at all.
///
/// The gate is phone verification, and it is invisible: the account uploads the bytes fine (the
/// credential is clearly good, the uploader took it), and then the attach comes back 4xx with
/// nothing in the body naming a reason. A 403 in particular must not reach the user as our
/// "session expired, sign in again", which is what the transport makes of one everywhere else.
/// Timeouts and connect failures are left alone: those really are try-again.
fn cover_refusal(e: Error) -> Error {
    match &e {
        Error::SessionExpired | Error::Other(_) => Error::CoverRefused,
        Error::Http(h) if h.status().is_some() => Error::CoverRefused,
        _ => e,
    }
}

/// The playlist's thumbnail as the edit left it, off the header YouTube echoes back. Both cover
/// actions answer with one, and after a removal it is the only way to learn the collage YouTube
/// rebuilt out of the tracks. Scoped to `newHeader` so an unrelated avatar can't stand in for it.
fn edited_thumbnail(response: &serde_json::Value) -> Option<String> {
    metadata::find_all(response, "newHeader").into_iter().find_map(metadata::last_thumbnail)
}

/// The single image slot a playlist has. Square by name and by what YouTube does with it.
fn custom_thumbnail_key() -> serde_json::Value {
    serde_json::json!({
        "name": "studio_square_thumbnail",
        "type": "PLAYLIST_IMAGE_TYPE_CUSTOM_THUMBNAIL",
    })
}

/// Playlist edit/delete want the raw playlistId; browse gives it `VL`-prefixed. context/01.
fn strip_vl(id: &str) -> &str {
    id.strip_prefix("VL").unwrap_or(id)
}

/// `browse/edit_playlist` answers HTTP 200 even when it applies nothing: the refusal is
/// `"status": "STATUS_FAILED"` in the body. Adding a track the playlist already holds is the
/// common one, and YouTube marks it by offering an "Add anyway" button whose endpoint repeats the
/// action with a `dedupeOption`. Left unread, the caller thinks the edit landed.
fn edit_rejection(v: &serde_json::Value) -> Option<Error> {
    if v.get("status").and_then(serde_json::Value::as_str) != Some("STATUS_FAILED") {
        return None;
    }
    Some(match metadata::find_first_str(v, "dedupeOption") {
        Some(_) => Error::AlreadyInPlaylist,
        None => Error::Other("YouTube refused the playlist edit.".into()),
    })
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

    /// The image bytes were accepted moments earlier, so the credential is good: a refusal on the
    /// attach is the account not being allowed a custom playlist image. Above all it must not come
    /// out as "your session expired", which is what a 403 means anywhere else in this crate.
    #[test]
    fn a_refused_cover_never_reads_as_a_dead_session() {
        assert!(matches!(cover_refusal(Error::SessionExpired), Error::CoverRefused));
        // `edit_playlist`'s STATUS_FAILED rejection, which is how a 200 says no.
        assert!(matches!(
            cover_refusal(Error::Other("YouTube refused the playlist edit.".into())),
            Error::CoverRefused
        ));
        // Nothing reached YouTube at all: still worth a retry, so leave it be.
        assert!(matches!(cover_refusal(Error::VisitorDataNotFound), Error::VisitorDataNotFound));
    }

    // Both bodies are trimmed captures of the live responses.
    #[test]
    fn duplicate_add_is_rejected() {
        let ok = json!({ "playlistEditResults": [{ "playlistEditVideoAddedResultData": {
            "setVideoId": "56B44F6D10557CC6", "videoId": "dQw4w9WgXcQ" } }],
            "status": "STATUS_SUCCEEDED" });
        assert!(edit_rejection(&ok).is_none());

        let dup = json!({ "actions": [{ "addToToastAction": { "item": {
            "notificationActionRenderer": { "actionButton": { "buttonRenderer": {
                "command": { "playlistEditEndpoint": { "actions": [{
                    "action": "ACTION_ADD_VIDEO",
                    "addedVideoId": "dQw4w9WgXcQ",
                    "dedupeOption": "DEDUPE_OPTION_SKIP" }] } },
                "text": { "runs": [{ "text": "Add anyway" }] } } },
            "responseText": { "runs": [{ "text": "This track is already in the playlist" }] } } } } }],
            "status": "STATUS_FAILED" });
        assert!(matches!(edit_rejection(&dup), Some(Error::AlreadyInPlaylist)));

        let failed = json!({ "status": "STATUS_FAILED" });
        assert!(matches!(edit_rejection(&failed), Some(Error::Other(_))));
    }

    #[test]
    fn create_playlist_id_parsed() {
        let resp = json!({ "playlistId": "PLnew123", "status": "STATUS_SUCCEEDED" });
        assert_eq!(metadata::find_first_str(&resp, "playlistId").as_deref(), Some("PLnew123"));
    }
}
