//! High-level endpoint facade over the transport. context/03, 08.

use serde::Serialize;

use crate::blocklist;
use crate::clients::YouTubeClient;
use crate::models::browse::{
    self, AlbumPage, ArtistPage, BrowseItem, HistoryGroup, HomePage, MoodSection,
    PlaylistContinuation, PlaylistPage, PlaylistSort, SearchResults, SearchSuggestions,
};
use crate::models::comment_write::WriteCommand;
use crate::models::comments::{self, CommentReplies, CommentsPage};
use crate::models::context::Context;
use crate::models::lyrics::{self, PlainLyrics, TimedLyricLine};
use crate::models::metadata::{
    self, AccountIdentity, AccountInfo, NextResult, Rating, SearchResult, SongItem,
};
use crate::models::player::{
    ContentPlaybackContext, PlaybackContext, PlayerBody, PlayerResponse, ServiceIntegrityDimensions,
};
use crate::transport::{enum_like, Error, InnerTube};

/// Search filter params (opaque base64). context/08.
pub const FILTER_SONG: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D";
pub const FILTER_VIDEO: &str = "EgWKAQIQAWoKEAkQChAFEAMQBA%3D%3D";
pub const FILTER_ALBUM: &str = "EgWKAQIYAWoKEAkQChAFEAMQBA%3D%3D";
pub const FILTER_ARTIST: &str = "EgWKAQIgAWoKEAkQChAFEAMQBA%3D%3D";
pub const FILTER_COMMUNITY_PLAYLIST: &str = "EgeKAQQoAEABagoQAxAEEAoQCRAF";

/// A comment's like/unlike/dislike/undislike and delete: `{context, actions: [<token>]}`.
pub const COMMENT_ACTION_PATH: &str = "comment/perform_comment_action";

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
    ///
    /// `record_history` decides whether the request carries the account at all. A signed-in
    /// `search` is written to the account's YouTube search history, and the typeahead fires one
    /// per debounce, so every half-typed prefix used to show up later in YouTube's and YTM's own
    /// search box (#203). Only the query the user actually submitted passes `true`; a preview goes
    /// out with no cookie and no `onBehalfOfUser`, which YouTube cannot attribute to anyone.
    /// Nothing is lost by that: a search response carries no per-account field, not even
    /// `likeStatus`, so the anonymous rows are the same rows minus personalised ranking.
    async fn search_raw(
        &self,
        client: &YouTubeClient,
        query: &str,
        params: Option<&str>,
        record_history: bool,
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
            context: if record_history {
                self.context_for(client)
            } else {
                self.context_anonymous(client)
            },
            query: query.to_owned(),
            params: params.map(str::to_owned),
        };
        self.post("search", client, &body, /* set_login */ record_history).await
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

    // --- blocked artists (user list, empty by default) -----------------------------------------
    //
    // Same boundary and the same reason as "hide music videos", but a narrower set of surfaces:
    // only what YouTube generates. A playlist, an album, a search and the history are things the
    // user asked for by name, and silently dropping rows out of them is a bug, not a feature.
    // See plan 046.

    fn drop_blocked_songs(&self, items: &mut Vec<SongItem>, keep: Option<&str>) {
        blocklist::retain_songs(items, &self.blocked(), keep);
    }

    fn drop_blocked_cards(&self, items: &mut Vec<BrowseItem>) {
        blocklist::retain_cards(items, &self.blocked());
    }

    /// Search songs only (`FILTER_SONG`). context/08.
    pub async fn search_songs(
        &self,
        metadata_client: &YouTubeClient,
        query: &str,
        record_history: bool,
    ) -> Result<SearchResult, Error> {
        let value =
            self.search_raw(metadata_client, query, Some(FILTER_SONG), record_history).await?;
        let mut r = metadata::parse_search(&value);
        self.drop_video_songs(&mut r.items);
        Ok(r)
    }

    /// Search video uploads only (`FILTER_VIDEO`): the covers, live sets and remixes that never
    /// got an official release, which `FILTER_SONG` cannot return by definition (#209, #266).
    /// context/08.
    pub async fn search_videos(
        &self,
        metadata_client: &YouTubeClient,
        query: &str,
    ) -> Result<SearchResult, Error> {
        // Not a filter on the rows, a refusal to ask: with music videos hidden there is no such
        // thing as a video search, and the caller's shelf disappears on an empty list.
        if self.hide_videos() {
            return Ok(SearchResult { items: Vec::new() });
        }
        // No history: the search page already recorded this query with its other two searches.
        let value = self.search_raw(metadata_client, query, Some(FILTER_VIDEO), false).await?;
        Ok(metadata::parse_search(&value))
    }

    /// Unfiltered search → categorized sections (top / songs / albums / artists / playlists).
    pub async fn search_all(
        &self,
        client: &YouTubeClient,
        query: &str,
        record_history: bool,
    ) -> Result<SearchResults, Error> {
        let value = self.search_raw(client, query, None, record_history).await?;
        let mut r = browse::parse_search_all(&value);
        self.drop_video_cards(&mut r.top);
        self.drop_video_cards(&mut r.songs);
        Ok(r)
    }

    /// Typeahead for the search field and Ctrl+K: query completions plus a few matching songs,
    /// artists and albums. context/08.
    ///
    /// Sent with the account, unlike a preview `search` (#203): this is the request YTM's own
    /// search box sends signed in on every keystroke, so it is not written to search history, and
    /// the account is what ranks the user's own listening first and returns their past searches.
    pub async fn search_suggestions(
        &self,
        client: &YouTubeClient,
        input: &str,
    ) -> Result<SearchSuggestions, Error> {
        #[derive(Serialize)]
        struct Body {
            context: Context,
            input: String,
        }
        let body = Body { context: self.context_for(client), input: input.to_owned() };
        let value = self.post("music/get_search_suggestions", client, &body, true).await?;
        let mut r = browse::parse_search_suggestions(&value);
        self.drop_video_cards(&mut r.items);
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
        // No history: "Show more" repeats the query the search page already recorded.
        let value = self.search_raw(client, query, Some(filter), false).await?;
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
        self.drop_blocked_songs(&mut next.items, video_id);
        Ok(next)
    }

    // --- comments. Reads go through `next`: a comments token sent to `browse` is answered with
    // the Home feed. -----------------------------------------------------------------------

    /// First page of comments for a video (the Comments tab's token, then its page); no token is
    /// `CommentsState::Disabled`. Signed in, both requests go as the account without the healer;
    /// a 401/403 redoes the whole chain anonymously, so an account's token is never sent without it.
    pub async fn comments(
        &self,
        client: &YouTubeClient,
        video_id: &str,
    ) -> Result<CommentsPage, Error> {
        if self.can_read_as_account(client) {
            match self.comments_first_page(client, video_id, true).await {
                Ok(mut page) => {
                    page.read_as_account = true;
                    return Ok(page);
                }
                Err(e) if is_refusal(&e) => {
                    // Nothing about the request or the account is logged.
                    tracing::debug!("signed-in comments read refused, redoing it anonymously");
                }
                Err(e) => return Err(e),
            }
        }
        self.comments_first_page(client, video_id, false).await
    }

    /// The tab lookup and the first page, both sent the same way (`as_account`).
    async fn comments_first_page(
        &self,
        client: &YouTubeClient,
        video_id: &str,
        as_account: bool,
    ) -> Result<CommentsPage, Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct NextBody {
            context: Context,
            video_id: String,
            is_audio_only: bool,
        }
        let tabs = self
            .comments_send(client, as_account, |context| NextBody {
                context,
                video_id: video_id.to_owned(),
                is_audio_only: true,
            })
            .await?;
        let Some(token) = comments::parse_comments_token(&tabs) else {
            return Ok(CommentsPage::disabled());
        };
        let value = self.next_continuation(client, &token, as_account).await?;
        let page = comments::parse_comments_page(&value, true);
        if as_account {
            log_own_comment_commands("first page", page_comments(&page));
        }
        Ok(page)
    }

    /// The next page of comments, or another sort (the header's sort entries carry tokens).
    /// `as_account` is how the page that issued `token` was read; a refusal is not retried
    /// anonymously.
    pub async fn comments_continuation(
        &self,
        client: &YouTubeClient,
        token: &str,
        as_account: bool,
    ) -> Result<CommentsPage, Error> {
        let value = self.next_continuation(client, token, as_account).await?;
        // A sort switch answers with a header and a body, a page with the body alone.
        let mut page = comments::parse_comments_page(&value, false);
        page.read_as_account = as_account;
        if as_account {
            log_own_comment_commands("continuation", page_comments(&page));
        }
        Ok(page)
    }

    /// One page of a thread's replies, via the token on the thread (or on the previous page's
    /// "Show more replies"). `as_account` and refusals as for [`Self::comments_continuation`].
    pub async fn comment_replies(
        &self,
        client: &YouTubeClient,
        token: &str,
        as_account: bool,
    ) -> Result<CommentReplies, Error> {
        let value = self.next_continuation(client, token, as_account).await?;
        let replies = comments::parse_comment_replies(&value);
        if as_account {
            log_own_comment_commands("replies", replies.replies.iter());
        }
        Ok(replies)
    }

    /// `next` with a bare continuation token. Not `browse_continuation`, see above.
    async fn next_continuation(
        &self,
        client: &YouTubeClient,
        token: &str,
        as_account: bool,
    ) -> Result<serde_json::Value, Error> {
        #[derive(Serialize)]
        struct ContinuationBody {
            context: Context,
            continuation: String,
        }
        self.comments_send(client, as_account, |context| ContinuationBody {
            context,
            continuation: token.to_owned(),
        })
        .await
    }

    /// Like, unlike, dislike or undislike: replays the comment's own toolbar token, through the
    /// ordinary path like `rate`. Needs `STATUS_SUCCEEDED` ([`answer_verdict`]).
    pub async fn comment_action(
        &self,
        client: &YouTubeClient,
        token: &str,
        kind: &'static str,
    ) -> Result<(), Error> {
        self.send_action(client, token, kind, false).await.map(|_| ())
    }

    /// [`Self::comment_action`] and a delete. `missing_ok` accepts an answer with no status (a
    /// delete's). Returns the answer.
    async fn send_action(
        &self,
        client: &YouTubeClient,
        token: &str,
        kind: &'static str,
        missing_ok: bool,
    ) -> Result<serde_json::Value, Error> {
        #[derive(Serialize)]
        struct ActionBody {
            context: Context,
            actions: Vec<String>,
        }
        let body =
            ActionBody { context: self.context_for(client), actions: vec![token.to_owned()] };
        let value = self.post(COMMENT_ACTION_PATH, client, &body, true).await?;
        accept_answer(kind, value, missing_ok)
    }

    /// Replays a write command from a signed-in read, with `text` in the command's text field.
    /// `replayable` writes (edit, delete) use `post`; the others use [`InnerTube::post_write`]. A
    /// 404 on a delete is success (`Null`). Returns the answer.
    pub async fn comment_write(
        &self,
        client: &YouTubeClient,
        command: &WriteCommand,
        text: Option<&str>,
        replayable: bool,
    ) -> Result<serde_json::Value, Error> {
        let kind = command.log_kind();
        let (path, body) = match command {
            WriteCommand::Action(token) => {
                return match self.send_action(client, token, kind, true).await {
                    // A token replay is a delete: a 404 means it is already gone.
                    Err(e) if e.is_not_found() => Ok(serde_json::Value::Null),
                    other => other,
                };
            }
            WriteCommand::Endpoint { path, payload, text_field, .. } => {
                if !crate::models::comment_write::is_plain_comment_path(path) {
                    tracing::debug!(kind, "comment write refused: not a plain comment path");
                    return Err(Error::Other("Not a comment path.".into()));
                }
                let mut body = serde_json::Map::new();
                body.insert("context".into(), serde_json::to_value(self.context_for(client))?);
                for (key, value) in payload {
                    // What the server issued never replaces the context.
                    body.entry(key.clone()).or_insert_with(|| value.clone());
                }
                if let Some(text) = text {
                    body.insert((*text_field).into(), text.into());
                }
                (path.as_str(), serde_json::Value::Object(body))
            }
        };
        let value = if replayable {
            self.post(path, client, &body, true).await?
        } else {
            self.post_write(path, client, &body, true).await?
        };
        accept_answer(kind, value, true)
    }

    /// Whether a comments request can go out as the account right now.
    fn can_read_as_account(&self, client: &YouTubeClient) -> bool {
        client.login_supported && self.is_logged_in()
    }

    /// One comments `next` request, as the account through [`InnerTube::post_no_heal`] or
    /// anonymously. An account request with nobody signed in is an error, never an anonymous one.
    async fn comments_send<B: Serialize>(
        &self,
        client: &YouTubeClient,
        as_account: bool,
        build: impl Fn(Context) -> B,
    ) -> Result<serde_json::Value, Error> {
        if as_account {
            if !self.can_read_as_account(client) {
                return Err(Error::AccountChanged);
            }
            self.post_no_heal("next", client, &build(self.context_for(client)), true).await
        } else {
            self.post("next", client, &build(self.context_anonymous(client)), false).await
        }
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
        let mut healed = false;
        loop {
            let value = self.post("browse", client, &body, true).await?;

            // A stale cookie authenticates transport-wise but YouTube returns a logged-out "Sign
            // in" state for account-scoped browse. Same reasoning as the transport's 401: let the
            // healer have a go and retry once before telling the user their session expired.
            if self.is_logged_in() && browse::is_signed_out(&value) {
                if !healed && !self.healing_suspended() {
                    healed = true;
                    tracing::warn!("InnerTube browse returned the signed-out state, healing");
                    self.wait_for_session_heal().await?;
                    tracing::info!("heal finished, retrying browse");
                    continue;
                }
                return Err(self.reject_session());
            }

            return Ok(value);
        }
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
        // A card the grid hands out twice is fatal on the UI side: every library list is keyed by
        // browseId, and one repeat blanks the whole list (the sidebar, the Library page, the
        // add-to-playlist picker) with an each_key_duplicate. Only libraries past the first page
        // saw it, which is why removing a few playlists in YouTube Music "fixed" it. Issues #258,
        // #260.
        let mut seen = std::collections::HashSet::new();
        let mut dropped = 0usize;
        let mut keep = |items: &mut Vec<BrowseItem>| {
            items.retain(|i: &BrowseItem| {
                let fresh = seen.insert(i.id.clone());
                dropped += usize::from(!fresh);
                fresh
            })
        };
        let mut items = browse::parse_library(&value);
        keep(&mut items);
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
            let mut page = browse::parse_library(&value);
            if page.is_empty() {
                // A spurious token: some grids carry one that resolves to nothing.
                break;
            }
            // Dedupe after the emptiness test, not before: a page that is entirely cards we
            // already have still carries the token for the page after it, and that one can hold
            // cards found nowhere else. A token that cycles is bounded by the self-reference
            // filter below and by the page cap above.
            keep(&mut page);
            items.extend(page);
            token = browse::continuation_token(&value).filter(|next| *next != t);
        }
        // Says so in a log the next reporter pastes: without it, a library that still looks wrong
        // can't be told apart from one that was never duplicating in the first place.
        if dropped > 0 {
            tracing::warn!(
                browse_id,
                dropped,
                "library grid repeated cards; kept the first of each"
            );
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
            self.drop_blocked_cards(&mut s.items);
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
            self.drop_blocked_cards(&mut s.items);
        }
        page.sections.retain(|s| !s.items.is_empty());
        Ok(page)
    }

    /// Moods & Genres (`FEmusic_moods_and_genres`): the tiles the search page browses from. A tile's
    /// `params` opens its playlists via `browse_grid("FEmusic_moods_and_genres_category", ..)`.
    /// Works signed out. context/08.
    pub async fn moods(&self, client: &YouTubeClient) -> Result<Vec<MoodSection>, Error> {
        let value = self.browse(client, Some("FEmusic_moods_and_genres"), None).await?;
        Ok(browse::parse_moods(&value))
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

    /// The artists the signed-in user subscribes to (`FEmusic_library_corpus_artists`), YouTube
    /// Music's Library ▸ Artists ▸ Subscriptions. Same list rows as `library_artists`, but they
    /// link the channel itself rather than `MPLA` + it. Needs login.
    pub async fn library_subscriptions(
        &self,
        client: &YouTubeClient,
    ) -> Result<Vec<BrowseItem>, Error> {
        self.library_grid(client, "FEmusic_library_corpus_artists").await
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
        if let Some(show) = browse::podcast_show_id(browse_id, &value) {
            let value = self.browse(client, Some(&show), params).await?;
            return Ok(browse::parse_playlist(&value));
        }
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
            self.drop_blocked_cards(&mut c.items);
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
        // `top_songs` deliberately keeps everything: filtering a blocked artist off their own
        // page is absurd, and the page is where you go to unblock them.
        self.drop_video_songs(&mut page.top_songs);
        for c in &mut page.sections {
            self.drop_video_cards(&mut c.items);
            self.drop_blocked_cards(&mut c.items);
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
        // A mood category is several shelves flattened into one grid, and YouTube puts the same
        // playlist on more than one of them (Chill had 85 repeats). A repeat is fatal on the UI
        // side for the same reason as in `library_grid`: the grid is keyed, and one duplicate key
        // leaves the page on its skeleton forever. Issue #355.
        let mut seen = std::collections::HashSet::new();
        items.retain(|i| seen.insert(i.id.clone()));
        self.drop_video_cards(&mut items);
        self.drop_blocked_cards(&mut items);
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
        self.search_raw(client, query, params, false).await
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
    /// `edit_rejection`) rather than storing a second copy, unless `allow_duplicates` asks for one.
    pub async fn playlist_add(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        video_id: &str,
        allow_duplicates: bool,
    ) -> Result<bool, Error> {
        match self
            .edit_playlist(client, playlist_id, add_video_action(video_id, allow_duplicates))
            .await
        {
            Ok(()) => Ok(true),
            Err(Error::AlreadyInPlaylist) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Add many videos in one `browse/edit_playlist` call, in order. The edit is all or nothing,
    /// and one track the playlist already holds fails the whole batch (`edit_rejection`), so the
    /// caller passes only videos it knows are new. Used by the Spotify import (#375).
    pub async fn playlist_add_many(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        video_ids: &[String],
    ) -> Result<(), Error> {
        let actions = video_ids.iter().map(|v| add_video_action(v, false)).collect();
        self.edit_playlist_actions(client, playlist_id, actions).await
    }

    /// Remove a video from a playlist. Needs `set_video_id` (the item's playlistSetVideoId).
    pub async fn playlist_remove(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        video_id: &str,
        set_video_id: &str,
    ) -> Result<(), Error> {
        self.playlist_remove_many(
            client,
            playlist_id,
            &[(video_id.to_owned(), set_video_id.to_owned())],
        )
        .await
    }

    /// Remove several videos in one `edit_playlist` request: the endpoint takes an array of
    /// actions, so a bulk removal is one round trip rather than one per track.
    // ponytail: no chunking. YouTube has taken a few hundred actions in one body; split into
    // batches here if a very long selection ever comes back rejected.
    pub async fn playlist_remove_many(
        &self,
        client: &YouTubeClient,
        playlist_id: &str,
        tracks: &[(String, String)],
    ) -> Result<(), Error> {
        let actions = tracks
            .iter()
            .map(|(video_id, set_video_id)| {
                serde_json::json!({
                    "action": "ACTION_REMOVE_VIDEO",
                    "setVideoId": set_video_id,
                    "removedVideoId": video_id,
                })
            })
            .collect();
        self.edit_playlist_actions(client, playlist_id, actions).await
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

/// What a comment action's or write's answer says about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Succeeded,
    /// A status that is not `STATUS_SUCCEEDED`, or a top-level `error`.
    Rejected,
    /// A 200 with no status to read.
    Missing,
}

/// Read an answer to a comment action or write: `STATUS_SUCCEEDED` in `actionResults[0]` (a vote)
/// or `actionResult` (an edit) succeeds; another status or a top-level `error` is a rejection;
/// none is `Missing`, for the caller to decide. Also the status, as it may be logged.
fn answer_verdict(value: &serde_json::Value) -> (Verdict, &str) {
    let result = value.pointer("/actionResults/0").or_else(|| value.get("actionResult"));
    let status = result.and_then(|r| r.get("status")).and_then(serde_json::Value::as_str);
    let verdict = match status {
        _ if value.get("error").is_some() => Verdict::Rejected,
        Some("STATUS_SUCCEEDED") => Verdict::Succeeded,
        Some(_) => Verdict::Rejected,
        None => Verdict::Missing,
    };
    (verdict, status.map_or("missing", |s| enum_like(s).unwrap_or("unexpected")))
}

/// The answer, unless it is a rejection. `missing_ok`: an answer with no status is accepted.
fn accept_answer(
    kind: &'static str,
    value: serde_json::Value,
    missing_ok: bool,
) -> Result<serde_json::Value, Error> {
    match answer_verdict(&value) {
        (Verdict::Succeeded, _) => Ok(value),
        (Verdict::Missing, _) if missing_ok => Ok(value),
        (_, status) => {
            tracing::debug!(kind, status, "comment action rejected");
            Err(Error::ActionRejected)
        }
    }
}

/// Every comment on a page, top-level and inline replies.
fn page_comments(page: &CommentsPage) -> impl Iterator<Item = &comments::Comment> {
    page.threads.iter().flat_map(|t| std::iter::once(&t.comment).chain(&t.replies))
}

/// One line per signed-in page that has the viewer's own comments: how many, and how many of them
/// offer edit and delete. Counts only.
fn log_own_comment_commands<'a>(
    kind: &'static str,
    comments: impl Iterator<Item = &'a comments::Comment>,
) {
    let (mut own, mut with_edit, mut with_delete) = (0, 0, 0);
    for c in comments.filter(|c| c.own) {
        own += 1;
        with_edit += usize::from(c.commands.edit.is_some());
        with_delete += usize::from(c.commands.delete.is_some());
    }
    if own > 0 {
        tracing::debug!(kind, own, with_edit, with_delete, "own comment commands");
    }
}

/// A 401/403: YouTube said no to this request.
fn is_refusal(e: &Error) -> bool {
    matches!(e, Error::Http(h) if h.status().is_some_and(|s| s == 401 || s == 403))
}

// Playlist edit/delete want the raw playlistId; browse gives it `VL`-prefixed. context/01.
fn strip_vl(id: &str) -> &str {
    id.strip_prefix("VL").unwrap_or(id)
}

/// `DEDUPE_OPTION_SKIP` skips YouTube's duplicate check: it is what the refusal's own "Add anyway"
/// button sends (the capture in `duplicate_add_is_rejected`). Sent on an ordinary add, it stores a
/// second copy instead of refusing.
fn add_video_action(video_id: &str, allow_duplicates: bool) -> serde_json::Value {
    let mut action = serde_json::json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": video_id });
    if allow_duplicates {
        action["dedupeOption"] = "DEDUPE_OPTION_SKIP".into();
    }
    action
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
    fn add_video_action_json() {
        assert_eq!(
            add_video_action("dQw4w9WgXcQ", false),
            json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": "dQw4w9WgXcQ" })
        );
        assert_eq!(
            add_video_action("dQw4w9WgXcQ", true),
            json!({
                "action": "ACTION_ADD_VIDEO",
                "addedVideoId": "dQw4w9WgXcQ",
                "dedupeOption": "DEDUPE_OPTION_SKIP"
            })
        );
    }

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

    // --- comments reads: as the account without the healer, never a token across the line ---

    use crate::clients::{Clients, METADATA_CLIENT};
    use crate::models::comment_write::{REPLY_EDIT_TEXT_FIELD, TEXT_FIELD};
    use crate::models::comments::CommentsState;
    use crate::test_server::{MockServer, Seen};
    use crate::transport::Session;
    use std::time::Duration;

    fn it_against(server: &MockServer, signed_in: bool) -> InnerTube {
        let session = Session {
            cookie: signed_in.then(|| "SAPISID=fixture-sapisid; PREF=x".to_owned()),
            data_sync_id: signed_in.then(|| "fixture-dsid".to_owned()),
            visitor_data: Some("fixture-visitor".into()),
            ..Default::default()
        };
        let mut it = InnerTube::new(session, None).unwrap();
        it.set_base_url(&server.base_url);
        it
    }

    fn web() -> YouTubeClient {
        Clients::bundled().get(METADATA_CLIENT).unwrap().clone()
    }

    fn as_account(req: &Seen) -> bool {
        req.header("cookie").is_some()
            || req.header("authorization").is_some()
            || req.body.contains("onBehalfOfUser")
    }

    fn is_tab_lookup(req: &Seen) -> bool {
        req.body.contains("\"videoId\"")
    }

    /// A tab-lookup answer whose comments token records who it was issued to.
    fn tab_answer(req: &Seen) -> String {
        let issued_to = if as_account(req) { "tok-account" } else { "tok-anon" };
        json!({ "tabs": [{ "tabRenderer": { "content": { "sectionListRenderer": {
            "continuations": [{ "reloadContinuationData": { "continuation": issued_to } }] } } } }] })
        .to_string()
    }

    /// A first page that has a header and no threads: parses as `Empty`.
    const PAGE: &str = r#"{"x":{"continuationItems":[{"commentsHeaderRenderer":{}}]}}"#;

    async fn no_heal(it: &InnerTube) {
        assert!(
            tokio::time::timeout(Duration::from_millis(100), it.session_rejected().notified())
                .await
                .is_err(),
            "a comments request must never wake the healer"
        );
        assert!(it.is_logged_in(), "and the session is untouched");
    }

    /// A refusal comes back as the HTTP error it was, never as an expired session.
    fn is_plain_refusal(e: &Error) -> bool {
        is_refusal(e) && !matches!(e, Error::SessionExpired)
    }

    #[tokio::test]
    async fn a_signed_in_first_page_is_read_as_the_account_and_says_so() {
        let server = MockServer::start(|req| {
            (200, if is_tab_lookup(req) { tab_answer(req) } else { PAGE.to_owned() })
        });
        let it = it_against(&server, true);
        let page = it.comments(&web(), "vid").await.unwrap();
        assert!(page.read_as_account);
        assert_eq!(page.state, CommentsState::Empty);
        let sent = server.requests();
        assert_eq!(sent.len(), 2, "tab lookup, first page");
        assert!(sent.iter().all(as_account));
        assert!(sent.iter().all(|r| r.path.starts_with("/youtubei/v1/next")));
        assert!(sent[0].header("authorization").is_some_and(|a| a.starts_with("SAPISIDHASH ")));
        assert!(sent[1].body.contains("tok-account"), "the token it was issued");
        no_heal(&it).await;
    }

    /// The first page is refused: the whole chain restarts anonymously, with a FRESH tab lookup,
    /// and the account's token is never sent without the account.
    #[tokio::test]
    async fn a_refused_first_page_redoes_the_whole_chain_anonymously() {
        for status in [401, 403] {
            let server = MockServer::start(move |req| {
                if is_tab_lookup(req) {
                    (200, tab_answer(req))
                } else if as_account(req) {
                    (status, "{}".into())
                } else if req.body.contains("tok-anon") {
                    (200, PAGE.to_owned())
                } else {
                    (400, "an account's token in an anonymous request".into())
                }
            });
            let it = it_against(&server, true);
            let page = it.comments(&web(), "vid").await.unwrap();
            assert!(!page.read_as_account);
            assert_eq!(page.state, CommentsState::Empty);

            let sent = server.requests();
            let shape: Vec<(bool, bool)> =
                sent.iter().map(|r| (is_tab_lookup(r), as_account(r))).collect();
            assert_eq!(
                shape,
                [(true, true), (false, true), (true, false), (false, false)],
                "account tab, refused page, then a fresh anonymous tab and page"
            );
            assert!(
                sent.iter().filter(|r| !as_account(r)).all(|r| !r.body.contains("tok-account")),
                "an account's token was replayed anonymously"
            );
            no_heal(&it).await;
        }
    }

    /// The tab lookup itself is refused: the chain restarts anonymously from the same place.
    #[tokio::test]
    async fn a_refused_tab_lookup_falls_back_anonymously() {
        let server = MockServer::start(|req| {
            if is_tab_lookup(req) && as_account(req) {
                (403, "{}".into())
            } else if is_tab_lookup(req) {
                (200, tab_answer(req))
            } else {
                (200, PAGE.to_owned())
            }
        });
        let it = it_against(&server, true);
        let page = it.comments(&web(), "vid").await.unwrap();
        assert!(!page.read_as_account);
        let sent = server.requests();
        assert_eq!(sent.len(), 3, "refused tab, anonymous tab, anonymous page");
        assert!(!as_account(&sent[1]) && !as_account(&sent[2]));
        no_heal(&it).await;
    }

    /// Only a refusal restarts the chain: a 500 would just repeat itself.
    #[tokio::test]
    async fn other_failures_do_not_restart_the_chain() {
        let server = MockServer::start(|_| (500, "{}".into()));
        let it = it_against(&server, true);
        assert!(matches!(it.comments(&web(), "vid").await, Err(Error::Http(_))));
        assert_eq!(server.requests().len(), 1);
        no_heal(&it).await;
    }

    #[tokio::test]
    async fn a_refusal_on_both_sides_is_a_plain_error_not_a_session_one() {
        let server = MockServer::start(|_| (403, "{}".into()));
        let it = it_against(&server, true);
        let err = it.comments(&web(), "vid").await.unwrap_err();
        assert!(is_plain_refusal(&err), "{err:?}");
        assert_eq!(server.requests().len(), 2, "one account attempt, one anonymous");
        no_heal(&it).await;
    }

    #[tokio::test]
    async fn signed_out_is_one_anonymous_chain() {
        let server = MockServer::start(|req| {
            (200, if is_tab_lookup(req) { tab_answer(req) } else { PAGE.to_owned() })
        });
        let it = it_against(&server, false);
        let page = it.comments(&web(), "vid").await.unwrap();
        assert!(!page.read_as_account);
        let sent = server.requests();
        assert_eq!(sent.len(), 2);
        assert!(!sent.iter().any(as_account));
    }

    /// A sort switch, a page and a replies page carry tokens issued earlier: refused, they come
    /// back as a plain error with exactly one request, never as an anonymous retry.
    #[tokio::test]
    async fn a_refused_continuation_or_replies_page_is_not_retried_anonymously() {
        for status in [401, 403] {
            let server = MockServer::start(move |_| (status, "{}".into()));
            let it = it_against(&server, true);
            let page = it.comments_continuation(&web(), "tok-account", true).await;
            let replies = it.comment_replies(&web(), "tok-account", true).await;
            for err in [page.unwrap_err(), replies.unwrap_err()] {
                assert!(is_plain_refusal(&err), "{err:?}");
            }
            assert_eq!(server.requests().len(), 2, "one request each, no retry");
            assert!(server.requests().iter().all(as_account));
            no_heal(&it).await;
        }
    }

    /// A token goes back the way it came: issued to an anonymous read, it is sent anonymously
    /// even though the app is signed in now.
    #[tokio::test]
    async fn a_token_goes_back_the_way_it_came() {
        let server = MockServer::start(|_| (200, PAGE.to_owned()));
        let it = it_against(&server, true);
        let anon = it.comments_continuation(&web(), "tok-anon", false).await.unwrap();
        assert!(!anon.read_as_account);
        let acct = it.comments_continuation(&web(), "tok-account", true).await.unwrap();
        assert!(acct.read_as_account);
        it.comment_replies(&web(), "tok-account", true).await.unwrap();
        let sent = server.requests();
        assert!(!as_account(&sent[0]) && as_account(&sent[1]) && as_account(&sent[2]));
    }

    /// The session went away between the first page and a later one: the account's token is not
    /// sent anonymously, nothing goes out, and the caller is told the account changed.
    #[tokio::test]
    async fn an_account_token_without_the_account_is_an_error_and_sends_nothing() {
        let server = MockServer::start(|_| (200, PAGE.to_owned()));
        let it = it_against(&server, false);
        let err = it.comments_continuation(&web(), "tok-account", true).await.unwrap_err();
        assert!(matches!(err, Error::AccountChanged), "{err:?}");
        assert!(server.requests().is_empty());
    }

    // --- comment writes ----------------------------------------------------------------------

    /// An `Endpoint` command as a read would build it, sending its text as `commentText`.
    fn endpoint(path: &str, kind: &'static str, payload: serde_json::Value) -> WriteCommand {
        WriteCommand::Endpoint {
            path: path.to_owned(),
            payload: payload.as_object().cloned().unwrap_or_default(),
            text_field: TEXT_FIELD,
            kind,
        }
    }

    fn create() -> WriteCommand {
        endpoint("comment/create_comment", "create", json!({ "createCommentParams": "P" }))
    }

    fn edit() -> WriteCommand {
        endpoint("comment/update_comment", "edit", json!({ "updateCommentParams": "P" }))
    }

    fn delete() -> WriteCommand {
        WriteCommand::Action("fixture-token".into())
    }

    /// The request as the server sees it: the server-issued fields and the text, the context ours
    /// (a payload can never replace it), the path the command named, and the account's headers.
    #[tokio::test]
    async fn a_write_replays_the_servers_command_with_the_text() {
        let server =
            MockServer::start(|_| (200, r#"{"actions":[{"createCommentAction":{}}]}"#.into()));
        let it = it_against(&server, true);
        let cmd = endpoint(
            "comment/create_comment",
            "create",
            json!({ "createCommentParams": "fixture-params", "context": "must-not-win" }),
        );
        it.comment_write(&web(), &cmd, Some("hello"), false).await.unwrap();
        let sent = server.requests();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].path.starts_with("/youtubei/v1/comment/create_comment?"));
        let body: serde_json::Value = serde_json::from_str(&sent[0].body).unwrap();
        assert_eq!(body["createCommentParams"], "fixture-params");
        assert_eq!(body["commentText"], "hello");
        assert!(body["context"].is_object(), "the context is ours: {body}");
        assert!(as_account(&sent[0]));
        assert!(sent[0].header("authorization").is_some_and(|a| a.starts_with("SAPISIDHASH ")));
    }

    /// The answers: a clean 200 is accepted; an `error`, a failed result, or a non-200 is not; and
    /// none of them carries the params or the text in what comes back.
    #[tokio::test]
    async fn write_answers_are_checked_and_errors_never_hold_the_params_or_text() {
        for (status, reply, ok) in [
            (200, r#"{"actions":[{"createCommentAction":{}}]}"#, true),
            (200, r#"{}"#, true),
            (200, r#"{"error":{"code":400}}"#, false),
            (200, r#"{"actionResults":[{"status":"STATUS_FAILED"}]}"#, false),
            (200, r#"{"actionResults":[{"status":"STATUS_SUCCEEDED"}]}"#, true),
            (400, r#"{}"#, false),
            (500, r#"{}"#, false),
        ] {
            let server = MockServer::start(move |_: &Seen| (status, reply.to_owned()));
            let it = it_against(&server, true);
            let cmd = endpoint("comment/x", "reply", json!({ "p": "secret-params" }));
            let got = it.comment_write(&web(), &cmd, Some("secret text"), false).await;
            assert_eq!(got.is_ok(), ok, "{status} {reply}");
            if let Err(e) = &got {
                for shown in [e.to_string(), format!("{e:?}")] {
                    assert!(!shown.contains("secret"), "{shown}");
                }
            }
            assert_eq!(server.requests().len(), 1);
        }
    }

    /// Never sent twice: a lost answer to a post is `WriteUncertain` after exactly one request.
    #[tokio::test]
    async fn a_lost_answer_to_a_post_is_uncertain_and_never_resent() {
        let server = MockServer::start(|_: &Seen| (0, String::new())); // hang up
        let it = it_against(&server, true);
        let got = it.comment_write(&web(), &create(), Some("hello"), false).await;
        assert!(matches!(got, Err(Error::WriteUncertain)), "{got:?}");
        assert_eq!(server.requests().len(), 1, "exactly one request");
    }

    /// A path that is not a plain `comment/…` one never reaches the network, even in a command
    /// that was built by hand: the host, scheme and port are the fixed base's alone.
    #[tokio::test]
    async fn a_hostile_command_path_sends_nothing() {
        let server = MockServer::start(|_: &Seen| (200, "{}".into()));
        let it = it_against(&server, true);
        for path in [
            "../browse",
            "comment/../browse",
            "comment//x",
            "comment/x?y=1",
            "comment/x#frag",
            "https://evil.example/comment/x",
            "//evil.example/comment/x",
            "evil.example/comment/x",
            "comment/x@evil.example",
            "comment/%2e%2e/x",
            "browse",
            "",
        ] {
            let got = it
                .comment_write(&web(), &endpoint(path, "edit", json!({})), Some("t"), false)
                .await;
            assert!(matches!(got, Err(Error::Other(_))), "{path}: {got:?}");
        }
        assert!(server.requests().is_empty(), "something was sent");
    }

    /// A delete that is a perform-action token is the same replay as a vote.
    #[tokio::test]
    async fn an_action_token_write_goes_to_the_perform_action_path() {
        let server = MockServer::start(|_: &Seen| {
            (200, r#"{"actionResults":[{"status":"STATUS_SUCCEEDED"}]}"#.into())
        });
        let it = it_against(&server, true);
        it.comment_write(&web(), &delete(), None, true).await.unwrap();
        let sent = server.requests();
        assert!(sent[0].path.starts_with(&format!("/youtubei/v1/{COMMENT_ACTION_PATH}")));
        let body: serde_json::Value = serde_json::from_str(&sent[0].body).unwrap();
        assert_eq!(body["actions"], json!(["fixture-token"]));
    }

    // --- comment actions ---------------------------------------------------------------------

    #[test]
    fn success_is_a_succeeded_status_in_either_answer_shape() {
        let verdict = |v: &serde_json::Value| answer_verdict(v).0;
        // `actionResults: [..]` (a like) and `actionResult: {..}` (an edit).
        let like = json!({ "actionResults": [{ "status": "STATUS_SUCCEEDED", "feedback": "FEEDBACK_LIKE" }] });
        let edit = json!({ "actionResult": { "status": "STATUS_SUCCEEDED", "feedbackText": { "runs": [] } } });
        for ok in [&like, &edit] {
            assert_eq!(answer_verdict(ok), (Verdict::Succeeded, "STATUS_SUCCEEDED"), "{ok}");
        }

        // A different status is a rejection in either shape, and so is a top-level error, even
        // beside a success.
        for rejected in [
            json!({ "actionResults": [{ "status": "STATUS_FAILED", "feedback": "FEEDBACK_LIKE" }] }),
            json!({ "actionResult": { "status": "STATUS_FAILED" } }),
            json!({ "actionResults": [{ "status": "nope" }] }),
            json!({ "error": { "code": 400 }, "actionResult": { "status": "STATUS_SUCCEEDED" } }),
        ] {
            assert_eq!(verdict(&rejected), Verdict::Rejected, "{rejected}");
        }
        // The plural array wins when both are there, and only its first entry is the answer.
        let both = json!({ "actionResults": [{ "status": "STATUS_FAILED" }], "actionResult": { "status": "STATUS_SUCCEEDED" } });
        assert_eq!(verdict(&both), Verdict::Rejected);
        let late = json!({ "actionResults": [{ "status": "STATUS_FAILED" }, { "status": "STATUS_SUCCEEDED" }] });
        assert_eq!(verdict(&late), Verdict::Rejected);

        // No status to read: missing, for the caller to decide.
        for missing in [
            json!({}),
            json!(null),
            json!([]),
            json!({ "actionResults": [] }),
            json!({ "actionResults": [{ "feedback": "FEEDBACK_LIKE" }] }),
            json!({ "actionResults": "STATUS_SUCCEEDED" }),
            json!({ "actionResult": {} }),
            json!({ "actionResult": { "feedbackText": "x" } }),
            json!({ "responseContext": {} }),
        ] {
            assert_eq!(answer_verdict(&missing), (Verdict::Missing, "missing"), "{missing}");
        }
        // Only enum-like words are echoed for the log.
        let odd = json!({ "actionResult": { "status": "STATUS_secret value" } });
        assert_eq!(answer_verdict(&odd), (Verdict::Rejected, "unexpected"));
    }

    /// Both answer shapes through a real request, for a vote, a write and a delete; and what a
    /// missing status means for each.
    #[tokio::test]
    async fn every_action_and_write_accepts_either_shape_and_handles_a_missing_status() {
        const PLURAL: &str = r#"{"actionResults":[{"status":"STATUS_SUCCEEDED"}]}"#;
        const SINGULAR: &str =
            r#"{"actionResult":{"status":"STATUS_SUCCEEDED","feedbackText":{}}}"#;
        const FAILED: &str = r#"{"actionResult":{"status":"STATUS_FAILED"}}"#;
        const NONE: &str = r#"{"responseContext":{},"trackingParams":"x"}"#;

        for (reply, vote, write, del) in [
            (PLURAL, "ok", "ok", "ok"),
            (SINGULAR, "ok", "ok", "ok"),
            (FAILED, "rejected", "rejected", "rejected"),
            // No status: a vote is a rejection; a write or a delete is accepted.
            (NONE, "rejected", "ok", "ok"),
        ] {
            let server = MockServer::start(move |_: &Seen| (200, reply.to_owned()));
            let it = it_against(&server, true);
            let label = |r: Result<_, Error>| match r {
                Ok(_) => "ok",
                Err(Error::ActionRejected) => "rejected",
                Err(Error::WriteUncertain) => "uncertain",
                Err(_) => "other",
            };
            let got_vote = label(it.comment_action(&web(), "T", "like").await);
            let got_write =
                label(it.comment_write(&web(), &edit(), Some("t"), false).await.map(|_| ()));
            let got_delete =
                label(it.comment_write(&web(), &delete(), None, true).await.map(|_| ()));
            assert_eq!((got_vote, got_write, got_delete), (vote, write, del), "{reply}");
        }
    }

    /// An edit is idempotent, so it takes the ordinary path: the transport retries a dropped
    /// connection, and a final failure is a plain failure, never `uncertain` (a post or a reply
    /// still is, and is still sent once).
    #[tokio::test]
    async fn an_edit_is_retried_and_never_uncertain_while_a_post_is_sent_once() {
        let server = MockServer::start(|_: &Seen| (0, String::new())); // hang up
        let it = it_against(&server, true);
        let got = it.comment_write(&web(), &edit(), Some("t"), true).await;
        assert!(matches!(&got, Err(Error::Http(_))), "{got:?}");
        assert!(server.requests().len() > 1, "the edit was not retried");

        let before = server.requests().len();
        let got = it.comment_write(&web(), &create(), Some("t"), false).await;
        assert!(matches!(got, Err(Error::WriteUncertain)), "{got:?}");
        assert_eq!(server.requests().len() - before, 1, "a post is sent exactly once");
    }

    /// A reply's edit goes out as `{context, replyText, updateReplyParams}` to its path, by the
    /// ordinary (retrying) path; a 404 or a 400 there is an ordinary error, never success.
    #[tokio::test]
    async fn a_reply_edit_sends_update_reply_params_and_a_refusal_is_an_error() {
        for status in [404, 400] {
            let server = MockServer::start(move |_: &Seen| {
                (
                    status,
                    r#"{"error":{"code":400,"message":"m","status":"INVALID_ARGUMENT"}}"#.into(),
                )
            });
            let it = it_against(&server, true);
            let edit = WriteCommand::Endpoint {
                path: crate::models::comment_write::COMMENT_UPDATE_REPLY_PATH.into(),
                payload: json!({ "updateReplyParams": "P" }).as_object().cloned().unwrap(),
                text_field: REPLY_EDIT_TEXT_FIELD,
                kind: "edit_reply",
            };
            let got = it.comment_write(&web(), &edit, Some("new text"), true).await;
            assert!(
                matches!(&got, Err(Error::Http(e)) if e.status().map(|s| s.as_u16()) == Some(status)),
                "{got:?}"
            );
            let sent = server.requests();
            assert_eq!(sent.len(), 1, "a refusal is not retried");
            assert!(sent[0].path.starts_with("/youtubei/v1/comment/update_comment_reply?"));
            let body: serde_json::Value = serde_json::from_str(&sent[0].body).unwrap();
            let mut keys: Vec<&str> =
                body.as_object().unwrap().keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["context", "replyText", "updateReplyParams"]);
            assert_eq!(body[REPLY_EDIT_TEXT_FIELD], "new text");
        }
    }

    /// A delete's answer has no status (it carries a `removeCommentAction` and the web client's
    /// toast): accepted, as is a bare 200; an `error` is not. A vote with the same answer still
    /// needs a status.
    #[tokio::test]
    async fn a_delete_answer_without_a_status_is_accepted() {
        let answer = r#"{"responseContext":{},"actions":[
            {"clickTrackingParams":"x","removeCommentAction":{"actionResult":{"status":"STATUS_SUCCEEDED"}}},
            {"clickTrackingParams":"y","openPopupAction":{"popup":{}}}]}"#;
        let popup_only = r#"{"responseContext":{},"actions":[{"clickTrackingParams":"y","openPopupAction":{}}]}"#;
        let bare = r#"{"responseContext":{}}"#;
        let failed = r#"{"actions":[{"removeCommentAction":{}}],"error":{"code":400}}"#;
        for (reply, ok) in [(answer, true), (popup_only, true), (bare, true), (failed, false)] {
            let server = MockServer::start(move |_: &Seen| (200, reply.to_owned()));
            let it = it_against(&server, true);
            let got = it.comment_write(&web(), &delete(), None, true).await;
            assert_eq!(got.is_ok(), ok, "{reply}");
        }
        let server = MockServer::start(move |_: &Seen| (200, answer.to_owned()));
        let it = it_against(&server, true);
        assert!(matches!(it.comment_action(&web(), "T", "like").await, Err(Error::ActionRejected)));
    }

    /// Delete is idempotent: a 404 is success, a failure leaves nothing unknown (it can simply
    /// be pressed again), a rejection is a rejection, and none of them is `uncertain`.
    #[tokio::test]
    async fn a_delete_succeeds_on_a_404_and_is_never_uncertain() {
        // 404: already gone, so done.
        let server = MockServer::start(|_: &Seen| (404, "{}".into()));
        let it = it_against(&server, true);
        let got = it.comment_write(&web(), &delete(), None, true).await;
        assert!(matches!(got, Ok(serde_json::Value::Null)), "{got:?}");
        assert_eq!(server.requests().len(), 1);

        // The same 404 on an edit is an error (the comment is gone, the edit did not happen).
        let it = it_against(&server, true);
        let got = it.comment_write(&web(), &edit(), Some("t"), true).await;
        assert!(matches!(&got, Err(e) if e.is_not_found()), "{got:?}");

        // A 500 (what a dead connection looks like after the retries) is a plain failure.
        let server = MockServer::start(|_: &Seen| (500, "{}".into()));
        let it = it_against(&server, true);
        let got = it.comment_write(&web(), &delete(), None, true).await;
        assert!(matches!(&got, Err(Error::Http(_))), "{got:?}");

        // A rejected status is a rejection, in both shapes, and so is an `error` with no status.
        for reply in [
            r#"{"actionResult":{"status":"STATUS_FAILED"}}"#,
            r#"{"actionResults":[{"status":"STATUS_FAILED"}]}"#,
            r#"{"error":{"code":400}}"#,
        ] {
            let server = MockServer::start(move |_: &Seen| (200, reply.to_owned()));
            let it = it_against(&server, true);
            let got = it.comment_write(&web(), &delete(), None, true).await;
            assert!(matches!(got, Err(Error::ActionRejected)), "{reply}");
        }
    }

    /// The request as the server sees it, and what each answer turns into. The token is in the
    /// body and nowhere else, and never in what comes back to the caller.
    #[tokio::test]
    async fn a_comment_action_posts_the_token_and_checks_the_answer() {
        const TOKEN: &str = "fixture-secret-token";
        for (status, reply, ok) in [
            (
                200,
                r#"{"actionResults":[{"status":"STATUS_SUCCEEDED","feedback":"FEEDBACK_LIKE"}]}"#,
                true,
            ),
            (200, r#"{"actionResults":[{"status":"STATUS_FAILED"}]}"#, false),
            (200, r#"{}"#, false),
            (400, r#"{}"#, false),
            (500, r#"{}"#, false),
        ] {
            let server = MockServer::start(move |_: &Seen| (status, reply.to_owned()));
            let it = it_against(&server, true);
            let got = it.comment_action(&web(), TOKEN, "like").await;
            assert_eq!(got.is_ok(), ok, "{status} {reply}");
            if let Err(e) = &got {
                assert!(!e.to_string().contains(TOKEN), "the token is in an error: {e}");
                assert!(!format!("{e:?}").contains(TOKEN));
            }
            let sent = server.requests();
            assert_eq!(sent.len(), 1);
            assert!(sent[0].path.starts_with(&format!("/youtubei/v1/{COMMENT_ACTION_PATH}")));
            assert!(sent[0].path.contains("prettyPrint=false"));
            let body: serde_json::Value = serde_json::from_str(&sent[0].body).unwrap();
            assert_eq!(body["actions"], json!([TOKEN]), "{{context, actions: [token]}}");
            assert!(body.get("context").is_some());
            assert!(as_account(&sent[0]), "an action is the account's own");
        }
    }
}
