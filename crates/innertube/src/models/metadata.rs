//! Search + next(queue) parsing. context/08.
//!
//! YouTube's response is a deeply-nested "renderer" tree. Rather than port Metrolist's ~40
//! renderer classes, we walk the raw JSON for the two node types we need
//! (`musicResponsiveListItemRenderer` for search, `playlistPanelVideoRenderer` for next) and
//! pull only the handful of fields the playback path uses. Targeted and robust to the tree
//! moving around (fixture reality wins over spec — see plan risks).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A song item — the minimum the playback path (context/06) needs. context/08.
/// Round-trips through the UI (serialized into search results, deserialized back into `play`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SongItem {
    pub video_id: String,
    pub title: String,
    pub artists: String,
    /// The primary artist's channel browseId (`UC…`), when the row links one — lets the UI make
    /// the artist name navigate to its artist page. context/08.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_id: Option<String>,
    /// The artist line split into its original runs, each tagged with its own channel id when it
    /// links one — a collab ("Future & Metro Boomin") links each name separately. Empty when the
    /// row links no artist at all; the UI then falls back to plain `artists`. context/08.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artist_runs: Vec<ArtistRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    /// The album's browseId (`MPRE…`), when the row links one — lets the UI navigate to the album.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    /// The row's play count as YouTube abbreviates it ("53M"), from an album page's plays column.
    /// Absent on rows that don't carry one (playlists, search, queue).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_count: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// `playlistSetVideoId` — the item's id *within a playlist*, needed to remove it (context/01
    /// edit_playlist). Only present when the item came from a playlist page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_video_id: Option<String>,
    /// Who put this track in the playlist, on a collaborative one: the contributor's name and
    /// avatar off the row's `contributorsAvatars`. `None` everywhere else, since an ordinary
    /// playlist has one contributor and YouTube doesn't send the stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by_avatar: Option<String>,
    /// The signed-in user's rating of this track, from the row's `likeStatus`. `None` when the
    /// response didn't carry one, which the UI treats the same as [`Rating::Indifferent`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<Rating>,
    /// Listen Together: username of the guest who added this queue item (`None` for the user's own
    /// tracks). Never parsed from YouTube — pure queue metadata, carried for attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_by: Option<String>,
    /// "Play next" (or a guest's session add): marks the "up next" block so successive adds stack
    /// FIFO right after the current song. Pure queue metadata, never parsed.
    #[serde(default)]
    pub queued: bool,
    /// "Add to queue": appended at the tail, after everything the user picked. Its own block in the
    /// queue panel — without this it would read as part of the playlist that's playing. Pure queue
    /// metadata, never parsed.
    #[serde(default)]
    pub queued_end: bool,
    /// What either block was added from ("Nightcore Bangers"), when it came from an album/playlist.
    /// The panel heads the block with it instead of the playing playlist's name. `None` for
    /// single-song adds. Pure queue metadata, never parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_from: Option<String>,
    /// Appended by autoplay radio continuation (vs. chosen by the user). Drives the queue's
    /// "Autoplay" divider + player-bar badge. Pure queue metadata, never parsed.
    #[serde(default)]
    pub autoplay: bool,
    /// This row links a music video, not the audio track ([`is_video_row`]). Drives the
    /// "hide music videos" setting; computed once here, never re-derived downstream.
    #[serde(default)]
    pub is_video: bool,
    /// One of the signed-in user's own uploads ([`is_upload_row`]). Only an authenticated client
    /// can stream these, so the orchestrator swaps in a login-only fallback chain. Issue #71.
    #[serde(default)]
    pub is_upload: bool,
    /// YouTube marks this track explicit ([`is_explicit`]). Only browse/search rows carry the
    /// badge: `/next` panel rows and `/player` don't, so a radio- or autoplay-appended track
    /// reads false even when the same song shows the badge on an album page.
    #[serde(default)]
    pub explicit: bool,
    /// "Add to library" / "Remove from library" off this row's own menu ([`library_toggle`]).
    /// `None` on rows YouTube sent no such menu for, and on the ones this app builds itself (local
    /// files, On Repeat), where there is nothing to send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<LibraryToggle>,
}

/// A track's library membership as its menu states it, with the tokens for both directions.
///
/// Library ▸ Songs is not Liked Music: adding a song to the library is a `feedbackEndpoint`, a
/// separate write from the rating, and the only handle on it is an opaque token minted with the
/// row. That is why this rides along on every `SongItem` instead of being asked for later, and why
/// a row we built ourselves can't offer the action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LibraryToggle {
    /// Whether the song is in the library right now, per the menu YouTube sent with the row.
    pub in_library: bool,
    /// One token per direction. Both are present when the row's menu is a *toggle*; a row that
    /// offers the action one way only (a plain menu item, which is how some surfaces send it)
    /// carries just the one it offers, and the way back has to come from a re-fetched row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_token: Option<String>,
}

/// How the signed-in user rated a track. One type for both directions: it's what a row's
/// `likeStatus` parses into, and what the write action ([`crate::InnerTube::rate`]) takes. The three
/// values are mutually exclusive on YouTube's side, so rating a liked track "dislike" un-likes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    Like,
    Dislike,
    /// No rating: what `like/removelike` leaves behind, and what an unrated row reports.
    Indifferent,
}

/// `MUSIC_VIDEO_TYPE_ATV` — the audio track YouTube Music generates for a release. Anything else
/// (`_OMV`, `_UGC`) is a video upload, except [`UPLOADED_TRACK_TYPE`].
pub(crate) const AUDIO_TRACK_TYPE: &str = "MUSIC_VIDEO_TYPE_ATV";

/// A track the signed-in user uploaded to their own library. Audio, not a music video: counting
/// it as one hid every upload under the "hide music videos" setting, and put the player view into
/// video mode on a track that has no video at all. Issue #71.
pub(crate) const UPLOADED_TRACK_TYPE: &str = "MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK";

/// True for a `musicVideoType` that means "a music video", i.e. neither of the two audio kinds.
pub(crate) fn is_video_type(t: &str) -> bool {
    t != AUDIO_TRACK_TYPE && t != UPLOADED_TRACK_TYPE
}

/// The `musicVideoType` a watch endpoint carries, if any.
fn endpoint_video_type(endpoint: &Value) -> Option<&str> {
    endpoint
        .get("watchEndpoint")
        .or_else(|| endpoint.get("watchPlaylistEndpoint"))?
        .get("watchEndpointMusicSupportedConfigs")?
        .get("watchEndpointMusicConfig")?
        .get("musicVideoType")?
        .as_str()
}

/// True when a watch endpoint points at a music video rather than the audio track.
pub(crate) fn is_video_endpoint(endpoint: &Value) -> bool {
    matches!(endpoint_video_type(endpoint), Some(t) if is_video_type(t))
}

/// True when a watch endpoint points at one of the user's own uploads.
pub(crate) fn is_upload_endpoint(endpoint: &Value) -> bool {
    endpoint_video_type(endpoint) == Some(UPLOADED_TRACK_TYPE)
}

/// True when a renderer row (list row, two-row card, or queue panel row) links a music video.
///
/// The authoritative tag sits on the thumbnail overlay's play button (`overlay` on list rows,
/// `thumbnailOverlay` on cards); queue-panel rows have no overlay, so the row's own navigation
/// endpoint is the fallback. Absent ⇒ we can't tell ⇒ audio, so a parse that misses the tag
/// degrades to "keep everything" rather than hiding the library.
pub(crate) fn is_video_row(node: &Value) -> bool {
    matches!(row_video_type(node), Some(t) if is_video_type(t))
}

/// True when a renderer row links one of the user's own uploads. Same lookup as [`is_video_row`].
pub(crate) fn is_upload_row(node: &Value) -> bool {
    row_video_type(node) == Some(UPLOADED_TRACK_TYPE)
}

/// The `musicVideoType` a row claims, from the overlay play button when it carries one.
fn row_video_type(node: &Value) -> Option<&str> {
    let overlay = node
        .get("overlay")
        .or_else(|| node.get("thumbnailOverlay"))
        .and_then(|o| o.get("musicItemThumbnailOverlayRenderer"))
        .and_then(|o| o.get("content"))
        .and_then(|c| c.get("musicPlayButtonRenderer"))
        .and_then(|p| p.get("playNavigationEndpoint"))
        .filter(|ep| endpoint_video_type(ep).is_some());
    match overlay {
        Some(ep) => endpoint_video_type(ep),
        None => node.get("navigationEndpoint").and_then(endpoint_video_type),
    }
}

/// True when a renderer node wears YouTube's explicit badge. The badge is the same
/// `musicInlineBadgeRenderer` everywhere, under one of three keys depending on the node: `badges`
/// on a list row, `subtitleBadges` on a two-row card, `subtitleBadge` (singular) on a
/// playlist/album header. Scoped to those keys rather than swept from the whole node, so a card's
/// badge can't leak onto the row that contains it. Live-verified 2026-08.
pub(crate) fn is_explicit(node: &Value) -> bool {
    ["badges", "subtitleBadges", "subtitleBadge"].into_iter().filter_map(|key| node.get(key)).any(
        |b| find_all(b, "iconType").into_iter().any(|t| t.as_str() == Some("MUSIC_EXPLICIT_BADGE")),
    )
}

/// One run of an artist line: the literal text plus its channel browseId when it links one
/// (separators like " & " carry no id).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtistRun {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub items: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NextResult {
    pub items: Vec<SongItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
    /// The lyrics tab's browseId (`MPLYt…`) — feed it to a lyrics `browse` (models::lyrics).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lyrics_browse_id: Option<String>,
    /// The mix the panel says it continues into (`automixPreviewVideoRenderer`, context/08): a
    /// radio playlist id to re-request `next` with. Present on a bare `next(videoId)`, which is
    /// otherwise just the seed song — that's how a dead `RDAMVM` radio finds a live one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automix_playlist_id: Option<String>,
    /// The signed-in user's rating of the **requested** video, off `playerOverlays`. The panel
    /// rows carry no `likeStatus` of their own (0 of 50, live-checked 2026-08-24), so this is the
    /// only place a `next` response states one, and it is what YouTube Music's own player bar
    /// draws its thumbs-up from. `None` when signed out or when YouTube omits the overlay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Rating>,
}

/// Logged-in account summary from `account/account_menu`. context/01, context/04A, context/15.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountInfo {
    pub name: Option<String>,
    pub handle: Option<String>,
    pub email: Option<String>,
    pub thumbnail: Option<String>,
    /// The channel browse id (`UC…`) when the response links one. Never used for delegation.
    pub channel_id: Option<String>,
    /// `onBehalfOfUser` id, `||`-split (context/04A). None when absent / single-account.
    pub data_sync_id: Option<String>,
    /// A login-bound visitorData, if the response carried one (context/15).
    pub visitor_data: Option<String>,
}

/// One usable YouTube identity returned by `account/accounts_list`.
///
/// `data_sync_id` is server-issued identity material from `datasyncIdToken` or `pageIdToken`, not
/// something inferred from the channel browse id. Rows without such a token are intentionally not
/// returned: displaying an identity that requests cannot actually select is worse than omitting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub name: String,
    pub handle: Option<String>,
    pub email: Option<String>,
    pub thumbnail: Option<String>,
    pub channel_id: Option<String>,
    pub data_sync_id: String,
    /// YouTube's current/default marker. The app's persisted choice may deliberately differ.
    pub is_selected: bool,
}

/// Parse a `search` response into song items. context/08.
pub fn parse_search(root: &Value) -> SearchResult {
    let mut items = Vec::new();
    for node in find_all(root, "musicResponsiveListItemRenderer") {
        if let Some(item) = parse_list_item(node) {
            items.push(item);
        }
    }
    SearchResult { items }
}

/// Parse a `next` response into the up-next queue + continuation token. context/08.
pub fn parse_next(root: &Value) -> NextResult {
    let mut items = Vec::new();
    let mut rows = Vec::new();
    panel_rows(root, &mut rows);
    for node in rows {
        if let Some(item) = parse_panel_video(node) {
            items.push(item);
        }
    }
    // The automix/radio continuation (context/08): the panel ends with a continuation token
    // used to fetch the endless mix. Take the first continuation token we find.
    let continuation = find_first_str(root, "continuation");
    let automix_playlist_id = find_all(root, "automixPreviewVideoRenderer")
        .into_iter()
        .find_map(|n| find_first_str(n, "playlistId"));
    NextResult {
        items,
        continuation,
        lyrics_browse_id: lyrics_browse_id(root),
        automix_playlist_id,
        // Scoped to the overlay on purpose: a bare `find_first_str` over the whole response would
        // read whatever a future panel row starts carrying instead of the requested video's own.
        rating: root.get("playerOverlays").and_then(like_status),
    }
}

/// Every panel row in order, ignoring the `counterpart` arm of a
/// `playlistPanelVideoWrapperRenderer`. A signed-in radio wraps each track's audio row together
/// with its music-video twin (what YouTube's own Song/Video toggle switches between), so a plain
/// [`find_all`] takes both and the queue gets every song twice. Issue #170.
fn panel_rows<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
    match node {
        Value::Object(map) => {
            for (k, v) in map {
                if k == "counterpart" {
                    continue;
                }
                if k == "playlistPanelVideoRenderer" {
                    out.push(v);
                }
                panel_rows(v, out);
            }
        }
        Value::Array(arr) => arr.iter().for_each(|e| panel_rows(e, out)),
        _ => {}
    }
}

/// The lyrics tab's browseId from a `next` response: the browseEndpoint whose pageType is
/// `MUSIC_PAGE_TYPE_TRACK_LYRICS`. context/08 §lyrics.
fn lyrics_browse_id(root: &Value) -> Option<String> {
    find_all(root, "browseEndpoint").into_iter().find_map(|be| {
        (find_first_str(be, "pageType").as_deref() == Some("MUSIC_PAGE_TYPE_TRACK_LYRICS"))
            .then(|| be.get("browseId").and_then(Value::as_str).map(str::to_owned))
            .flatten()
    })
}

/// Parse an `account/account_menu` response into an account summary. context/01, context/15.
pub fn parse_account_menu(root: &Value) -> AccountInfo {
    let header = find_all(root, "activeAccountHeaderRenderer").into_iter().next();
    let name = header.and_then(|h| account_text(h.get("accountName")));
    let handle = header.and_then(|h| account_text(h.get("channelHandle")));
    let email = header.and_then(|h| account_text(h.get("email")));
    let thumbnail = header.and_then(last_thumbnail);
    let channel_id = header.and_then(channel_browse_id);

    let rc = root.get("responseContext");
    // dataSyncId lives in the response context, not the menu header. context/04A.
    let data_sync_id = response_data_sync_id(root);
    let visitor_data = rc
        .and_then(|r| r.get("visitorData"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    AccountInfo { name, handle, email, thumbnail, channel_id, data_sync_id, visitor_data }
}

/// Parse every selectable channel from `account/accounts_list`.
///
/// The action envelope differs between YouTube clients, so this deliberately anchors on
/// `accountSectionListRenderer` / `accountItem` rather than one absolute JSON path. Section headers
/// provide the Google-account email; each row provides its channel metadata and supported identity
/// tokens.
pub fn parse_account_identities(root: &Value) -> Vec<AccountIdentity> {
    let response_id = response_data_sync_id(root);
    let mut identities = Vec::new();
    let sections = find_all(root, "accountSectionListRenderer");

    if sections.is_empty() {
        for item in find_all(root, "accountItem") {
            push_account_identity(&mut identities, item, None, response_id.as_deref());
        }
        return identities;
    }

    for section in sections {
        // Only the Google-account header. A section *title* is a group label ("Brand accounts"),
        // not an address, and putting one in `email` shows it under a channel name in the picker.
        let email = find_all(section, "googleAccountHeaderRenderer")
            .into_iter()
            .find_map(|h| account_text(h.get("email")));
        for item in find_all(section, "accountItem") {
            push_account_identity(&mut identities, item, email.as_deref(), response_id.as_deref());
        }
    }
    identities
}

fn push_account_identity(
    identities: &mut Vec<AccountIdentity>,
    item: &Value,
    email: Option<&str>,
    response_id: Option<&str>,
) {
    let Some(identity) = parse_account_identity(item, email, response_id) else { return };
    if identities.iter().all(|i| i.data_sync_id != identity.data_sync_id) {
        identities.push(identity);
    }
}

fn parse_account_identity(
    item: &Value,
    email: Option<&str>,
    response_id: Option<&str>,
) -> Option<AccountIdentity> {
    if item.get("isDisabled").and_then(Value::as_bool) == Some(true)
        || item.get("hasChannel").and_then(Value::as_bool) == Some(false)
    {
        return None;
    }
    let name = account_text(item.get("accountName"))?;
    let endpoint = item.get("serviceEndpoint")?.get("selectActiveIdentityEndpoint")?;
    let tokens = endpoint
        .get("supportedTokens")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    // Prefer the purpose-built data-sync token. Some WEB responses only provide pageIdToken;
    // that is still an explicit server-issued identity token and is the value YouTube's web
    // switcher uses for onBehalfOfUser. A `UC…` browse id is never substituted here.
    let token_data_sync_id = tokens.iter().find_map(|token| {
        let value = token.get("datasyncIdToken").or_else(|| token.get("dataSyncIdToken"))?;
        value
            .get("datasyncIdToken")
            .or_else(|| value.get("datasyncId"))
            .or_else(|| value.get("dataSyncId"))
            .and_then(Value::as_str)
            .and_then(normalize_data_sync_id)
    });
    let page_id = tokens.iter().find_map(|token| {
        token
            .get("pageIdToken")
            .and_then(|value| value.get("pageId"))
            .and_then(Value::as_str)
            .and_then(nonempty_string)
    });
    let is_selected = item.get("isSelected").and_then(Value::as_bool).unwrap_or(false);
    let selected_response_id = is_selected.then_some(response_id).flatten().map(str::to_owned);
    let data_sync_id = token_data_sync_id.or(page_id).or(selected_response_id)?;

    Some(AccountIdentity {
        name,
        handle: account_text(item.get("channelHandle")),
        email: email.and_then(nonempty_string),
        thumbnail: last_thumbnail(item),
        channel_id: channel_browse_id(item),
        data_sync_id,
        is_selected,
    })
}

fn account_text(value: Option<&Value>) -> Option<String> {
    value.and_then(|v| {
        runs_text_opt(v)
            .or_else(|| v.get("simpleText").and_then(Value::as_str).and_then(nonempty_string))
            .or_else(|| v.as_str().and_then(nonempty_string))
    })
}

fn channel_browse_id(node: &Value) -> Option<String> {
    find_all(node, "browseEndpoint").into_iter().find_map(|endpoint| {
        endpoint
            .get("browseId")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("UC"))
            .map(str::to_owned)
    })
}

fn response_data_sync_id(root: &Value) -> Option<String> {
    root.get("responseContext")
        .and_then(|r| r.get("mainAppWebResponseContext"))
        .and_then(|m| m.get("datasyncId").or_else(|| m.get("dataSyncId")))
        .and_then(Value::as_str)
        .and_then(normalize_data_sync_id)
}

fn normalize_data_sync_id(raw: &str) -> Option<String> {
    nonempty_string(&split_datasync_id(raw))
}

fn nonempty_string(raw: &str) -> Option<String> {
    (!raw.trim().is_empty()).then(|| raw.trim().to_owned())
}

/// Split a `dataSyncId` (`"<id>||<other>"`): prefer the part before `||`, else after. context/04A.
fn split_datasync_id(raw: &str) -> String {
    match raw.split_once("||") {
        Some((before, _)) if !before.is_empty() => before.to_owned(),
        Some((_, after)) => after.to_owned(),
        None => raw.to_owned(),
    }
}

// --- node parsers -------------------------------------------------------------------------

pub(crate) fn parse_list_item(node: &Value) -> Option<SongItem> {
    let video_id = list_item_video_id(node)?;
    let flex = node.get("flexColumns").and_then(Value::as_array);
    let title = flex.and_then(|c| c.first()).and_then(flex_text).unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    // Second flex column holds subtitle runs: "Artist • Album • duration" (• separated).
    let subtitle_runs = flex_runs(node, 1);
    let (artists, album, duration) = split_subtitle(subtitle_runs);
    // Playlist/album rows keep the length in a fixed column instead of the subtitle. context/08.
    let duration = duration.or_else(|| fixed_column_text(node));
    // …and the album in a column of its own rather than in the subtitle runs.
    let album = album.or_else(|| album_column(node));
    let artist_id = subtitle_runs.and_then(|r| first_artist_id(r));
    // Only when this row's own menu offers the remove action. Every playlist row carries a
    // playlistSetVideoId, including rows on a playlist you can't edit (live-checked 2026-08-23),
    // so the id alone is not permission. The menu is: a collaborative playlist gives the remove
    // action on the rows you added and withholds it on everyone else's, which is exactly what
    // YouTube Music itself shows.
    let set_video_id = removable(node)
        .then(|| {
            node.get("playlistItemData")
                .and_then(|d| d.get("playlistSetVideoId"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .flatten();
    let (added_by, added_by_avatar) = match contributor(node) {
        Some((name, avatar)) => (Some(name), Some(avatar)),
        None => (None, None),
    };
    Some(SongItem {
        video_id,
        title,
        artists,
        artist_id,
        artist_runs: subtitle_runs.map(|r| artist_runs(r)).unwrap_or_default(),
        album,
        album_id: album_id(node),
        duration,
        play_count: play_count(node),
        thumbnail: last_thumbnail(node),
        set_video_id,
        added_by,
        added_by_avatar,
        rating: like_status(node),
        queued_by: None,
        queued: false,
        queued_end: false,
        queued_from: None,
        autoplay: false,
        is_video: is_video_row(node),
        is_upload: is_upload_row(node),
        explicit: is_explicit(node),
        library: library_toggle(node),
    })
}

/// The play count from an album row's third flex column ("53M plays" → "53M"). Playlist rows put
/// the album name in that column instead, so the trailing "plays" is the discriminator — the
/// locale is pinned to en (models::context), so it's always that word. Live-verified 2026-08.
pub(crate) fn play_count(node: &Value) -> Option<String> {
    let text = flex_column_text(node, 2)?;
    let (count, unit) = text.trim().rsplit_once(' ')?;
    unit.eq_ignore_ascii_case("plays").then(|| count.to_owned())
}

/// The album name from that same third flex column — the other thing it can hold. A playlist row's
/// subtitle is the artist alone ("Artist"), not the search/next form ("Artist • Album • 3:02"), so
/// without this every playlist track comes back album-less. Same discriminator as [`play_count`],
/// read the other way round. context/08.
fn album_column(node: &Value) -> Option<String> {
    let text = flex_column_text(node, 2)?;
    let text = text.trim();
    (!text.is_empty() && play_count(node).is_none()).then(|| text.to_owned())
}

/// The album's browseId (`MPRE…`): either the linked album run or the row menu's "Go to album"
/// entry — whichever the renderer carries. Tolerant: first `MPRE…` browseId in the node. context/08.
fn album_id(node: &Value) -> Option<String> {
    find_all(node, "browseId")
        .into_iter()
        .filter_map(Value::as_str)
        .find(|id| id.starts_with("MPRE"))
        .map(str::to_owned)
}

/// The artist field of a run list, kept run by run so each linked artist of a collab navigates to
/// its own page. Empty when nothing links a channel. Cut at the "•" separators and dropping a
/// leading type label exactly like `split_subtitle`, so these runs describe the same field as the
/// `artists` string beside them: a search row reads "Song • Delara • 3:02", and taking everything
/// before the first "•" would hand back the unlinked word "Song". context/08.
pub(crate) fn artist_runs(runs: &[Value]) -> Vec<ArtistRun> {
    let mut fields: Vec<Vec<ArtistRun>> = vec![Vec::new()];
    for run in runs {
        let text = run.get("text").and_then(Value::as_str).unwrap_or_default();
        if text.trim() == "•" {
            fields.push(Vec::new());
        } else {
            fields.last_mut().expect("never empty").push(ArtistRun {
                text: text.to_owned(),
                id: first_artist_id(std::slice::from_ref(run)),
            });
        }
    }
    let linked = |f: &Vec<ArtistRun>| f.iter().any(|r| r.id.is_some());
    if fields.len() > 1 && !linked(&fields[0]) {
        let label: String = fields[0].iter().map(|r| r.text.as_str()).collect();
        if is_type_label(label.trim()) || fields[1..].iter().any(linked) {
            fields.remove(0);
        }
    }
    let out = fields.swap_remove(0);
    if !linked(&out) {
        return Vec::new();
    }
    out
}

/// First run that links an artist channel (`browseEndpoint.browseId` starting with `UC`). context/08.
pub(crate) fn first_artist_id(runs: &[Value]) -> Option<String> {
    runs.iter().find_map(|r| {
        let id = r.get("navigationEndpoint")?.get("browseEndpoint")?.get("browseId")?.as_str()?;
        id.starts_with("UC").then(|| id.to_owned())
    })
}

/// The track's rating from its menu's `likeStatus` (`LIKE` / `INDIFFERENT` / `DISLIKE`).
/// Tolerant: grabs the first `likeStatus` anywhere in the node, and reads anything it doesn't
/// recognise as unrated rather than dropping the row. context/08.
/// The contributor who added this row, on a collaborative playlist: their display name and avatar
/// URL. One avatar per row (whoever added it), in a stack renderer YouTube only sends there.
fn contributor(node: &Value) -> Option<(String, String)> {
    let stack = find_all(node, "contributorsAvatars").into_iter().next()?;
    let avatar = find_all(stack, "avatarViewModel").into_iter().next()?;
    let name = avatar.get("accessibilityText").and_then(Value::as_str)?;
    let url = avatar.pointer("/image/sources/0/url").and_then(Value::as_str)?;
    Some((name.to_owned(), url.to_owned()))
}

/// Does this row's menu carry "Remove from playlist"? The menu item is a `playlistEditEndpoint`
/// whose action is `ACTION_REMOVE_VIDEO`; YouTube only sends it where the account may remove that
/// item, so it is the per-row edit permission (owner, or collaborator on their own additions).
fn removable(node: &Value) -> bool {
    find_all(node, "action").iter().any(|a| a.as_str() == Some("ACTION_REMOVE_VIDEO"))
}

/// The row's "Save to library" action, in either shape YouTube sends it.
///
/// Usually a `toggleMenuServiceItemRenderer` holding both directions, one `feedbackEndpoint` each.
/// It carries no state field, so which way round it is comes from the icons: YouTube puts the
/// action available *now* in `defaultIcon`/`defaultServiceEndpoint`, so a song already in the
/// library arrives with the filled bookmark as its default. Some surfaces send a plain
/// `menuServiceItemRenderer` with one direction only; that is the fallback below.
///
/// Icons, not the label, because the label is localised. A track menu carries other feedback
/// toggles that must not match: "Add to liked songs" (FAVORITE/UNFAVORITE, a rating and a
/// different list) and "Pin to Listen again" (KEEP/KEEP_OFF). Live-checked 2026-08-28: search rows
/// send BOOKMARK_BORDER/BOOKMARK; the LIBRARY_* spellings are kept as YouTube has used them too.
fn library_toggle(node: &Value) -> Option<LibraryToggle> {
    /// The bookmark is empty when the song is not in the library — that direction adds it.
    const ADDS: [&str; 2] = ["BOOKMARK_BORDER", "LIBRARY_ADD"];
    /// Filled: it is in there, and this direction takes it out.
    const REMOVES: [&str; 3] = ["BOOKMARK", "LIBRARY_REMOVE", "LIBRARY_SAVED"];

    // Scoped to the row's menu where there is one: `find_all` walks everything, and a row can
    // carry other people's endpoints (an album's, an artist's) elsewhere in its tree.
    let menu = node.get("menu").unwrap_or(node);
    let feedback_token = |endpoint: Option<&Value>| {
        find_first_str(endpoint?.get("feedbackEndpoint")?, "feedbackToken")
    };
    let both = find_all(menu, "toggleMenuServiceItemRenderer").into_iter().find_map(|toggle| {
        let icon = |key: &str| find_first_str(toggle.get(key)?, "iconType");
        let default = feedback_token(toggle.get("defaultServiceEndpoint"))?;
        let toggled = feedback_token(toggle.get("toggledServiceEndpoint"))?;
        let (default_icon, toggled_icon) = (icon("defaultIcon")?, icon("toggledIcon")?);
        let (default_icon, toggled_icon) = (default_icon.as_str(), toggled_icon.as_str());
        if ADDS.contains(&default_icon) && REMOVES.contains(&toggled_icon) {
            Some(LibraryToggle {
                in_library: false,
                add_token: Some(default),
                remove_token: Some(toggled),
            })
        } else if REMOVES.contains(&default_icon) && ADDS.contains(&toggled_icon) {
            Some(LibraryToggle {
                in_library: true,
                add_token: Some(toggled),
                remove_token: Some(default),
            })
        } else {
            None
        }
    });
    both.or_else(|| {
        find_all(menu, "menuServiceItemRenderer").into_iter().find_map(|item| {
            let token = feedback_token(item.get("serviceEndpoint"))?;
            let icon = find_first_str(item.get("icon")?, "iconType")?;
            if ADDS.contains(&icon.as_str()) {
                Some(LibraryToggle {
                    in_library: false,
                    add_token: Some(token),
                    remove_token: None,
                })
            } else if REMOVES.contains(&icon.as_str()) {
                Some(LibraryToggle { in_library: true, add_token: None, remove_token: Some(token) })
            } else {
                None
            }
        })
    })
}

fn like_status(node: &Value) -> Option<Rating> {
    find_first_str(node, "likeStatus").map(|s| match s.as_str() {
        "LIKE" => Rating::Like,
        "DISLIKE" => Rating::Dislike,
        _ => Rating::Indifferent,
    })
}

fn parse_panel_video(node: &Value) -> Option<SongItem> {
    let video_id = node.get("videoId").and_then(Value::as_str)?.to_owned();
    let title = runs_text(node.get("title"))?;
    let byline = node.get("longBylineText").or_else(|| node.get("shortBylineText"));
    let byline_runs = byline.and_then(|b| b.get("runs")).and_then(Value::as_array);
    // The byline is a full descriptor ("Delara • Sjelen • 2026"), not a name: take its artist
    // field only, or the queue (and the scrobbler behind it) gets the whole string as the artist.
    let artists = artists_from_runs(byline_runs).unwrap_or_default();
    let artist_id = byline_runs.and_then(|r| first_artist_id(r));
    let duration = node.get("lengthText").and_then(runs_text_opt);
    Some(SongItem {
        video_id,
        title,
        artists,
        artist_id,
        artist_runs: byline_runs.map(|r| artist_runs(r)).unwrap_or_default(),
        album: None,
        album_id: album_id(node),
        duration,
        play_count: None,
        thumbnail: last_thumbnail(node),
        set_video_id: None,
        added_by: None,
        added_by_avatar: None,
        rating: like_status(node),
        queued_by: None,
        queued: false,
        queued_end: false,
        queued_from: None,
        autoplay: false,
        is_video: is_video_row(node),
        is_upload: is_upload_row(node),
        // Queue-panel rows carry no badges today; asking anyway costs a map lookup and picks the
        // flag up for free if YouTube ever adds one.
        explicit: is_explicit(node),
        library: library_toggle(node),
    })
}

/// Joined text of a `musicResponsiveListItemRenderer` flex column (0 = title, 1 = subtitle). Used
/// by the search-section parser to build cards from list rows. context/08.
/// The raw runs of a list row's `i`th flex column (the text with its per-run links intact).
pub(crate) fn flex_runs(node: &Value, i: usize) -> Option<&Vec<Value>> {
    node.get("flexColumns")
        .and_then(Value::as_array)
        .and_then(|c| c.get(i))
        .and_then(|c| c.get("musicResponsiveListItemFlexColumnRenderer"))
        .and_then(|r| r.get("text"))
        .and_then(|t| t.get("runs"))
        .and_then(Value::as_array)
}

pub(crate) fn flex_column_text(node: &Value, i: usize) -> Option<String> {
    node.get("flexColumns").and_then(Value::as_array).and_then(|c| c.get(i)).and_then(flex_text)
}

/// videoId from any of the three known locations. context/08 / AlbumPage.kt.
pub(crate) fn list_item_video_id(node: &Value) -> Option<String> {
    let direct = node
        .get("playlistItemData")
        .and_then(|d| d.get("videoId"))
        .and_then(Value::as_str)
        .or_else(|| {
            node.get("navigationEndpoint")
                .and_then(|n| n.get("watchEndpoint"))
                .and_then(|w| w.get("videoId"))
                .and_then(Value::as_str)
        });
    match direct {
        Some(id) => Some(id.to_owned()),
        // Last resort: the play-button overlay's watchEndpoint videoId.
        None => node.get("overlay").and_then(|o| find_first_str(o, "videoId")),
    }
}

// --- small helpers ------------------------------------------------------------------------

/// The row's `fixedColumns` duration ("3:47"). Playlist and album track rows carry it here.
fn fixed_column_text(node: &Value) -> Option<String> {
    node.get("fixedColumns")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|c| {
            c.get("musicResponsiveListItemFixedColumnRenderer")
                .and_then(|r| r.get("text"))
                .and_then(runs_text_opt)
        })
        .filter(|s| is_duration(s))
}

fn flex_text(col: &Value) -> Option<String> {
    col.get("musicResponsiveListItemFlexColumnRenderer")
        .and_then(|r| r.get("text"))
        .and_then(runs_text_opt)
}

pub(crate) fn runs_text(v: Option<&Value>) -> Option<String> {
    v.and_then(runs_text_opt)
}

/// Join all `runs[].text` in a `{ runs: [...] }` object.
pub(crate) fn runs_text_opt(v: &Value) -> Option<String> {
    let runs = v.get("runs").and_then(Value::as_array)?;
    let s: String = runs.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect();
    (!s.is_empty()).then_some(s)
}

/// One "•"-separated field of a subtitle, plus whether it links an artist channel (`UC…`).
struct Group {
    text: String,
    artist_link: bool,
}

/// Cut a subtitle run list at its "•" separators, keeping each field's artist link.
fn subtitle_groups(runs: &[Value]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut cur = Group { text: String::new(), artist_link: false };
    for run in runs {
        let t = run.get("text").and_then(Value::as_str).unwrap_or("");
        if t.trim() == "•" {
            groups.push(std::mem::replace(
                &mut cur,
                Group { text: String::new(), artist_link: false },
            ));
        } else {
            cur.text.push_str(t);
            cur.artist_link |= first_artist_id(std::slice::from_ref(run)).is_some();
        }
    }
    groups.push(cur);
    for g in &mut groups {
        g.text = g.text.trim().to_string();
    }
    groups
}

/// Result rows on an unfiltered search lead with the result type: "Song • Delara • 3:02". Nothing
/// downstream wants that word, and taken as an artist it lands in the user's Last.fm scrobbles.
fn is_type_label(s: &str) -> bool {
    matches!(
        s,
        "Song"
            | "Video"
            | "Album"
            | "Single"
            | "EP"
            | "Playlist"
            | "Artist"
            | "Episode"
            | "Podcast"
    )
}

/// "3:02" / "1:04:11", and nothing else. A colon alone doesn't make a duration: an artist or album
/// can carry one ("Jorge Rivera-Herrans & Cast of EPIC: The Musical"), and taking that as the
/// length prints the whole title where the time goes and squeezes the rest of the row out.
fn is_duration(s: &str) -> bool {
    let s = s.trim();
    s.contains(':') && s.chars().all(|c| c.is_ascii_digit() || c == ':')
}

/// Split a "• "-separated subtitle run list into (artists, album, duration). context/08.
fn split_subtitle(runs: Option<&Vec<Value>>) -> (String, Option<String>, Option<String>) {
    let Some(runs) = runs else { return (String::new(), None, None) };
    let mut groups = subtitle_groups(runs);
    // Drop a leading type label so artist/album don't both shift one field to the right. A later
    // field linking an artist channel proves the first one isn't the artist; the word list covers
    // the rows where nothing is linked at all.
    if groups.len() > 1
        && !groups[0].artist_link
        && (is_type_label(&groups[0].text) || groups[1..].iter().any(|g| g.artist_link))
    {
        groups.remove(0);
    }
    let groups: Vec<String> = groups.into_iter().map(|g| g.text).collect();
    let artists = groups.first().cloned().unwrap_or_default();
    // Last group that is a duration is the duration; the middle is album.
    let duration = groups.iter().rev().find(|g| is_duration(g)).cloned();
    let album = groups.get(1).filter(|g| Some(*g) != duration.as_ref()).cloned();
    (artists, album, duration)
}

/// Just the artist field of a subtitle run list, for the surfaces that keep a flat string.
pub(crate) fn artists_from_runs(runs: Option<&Vec<Value>>) -> Option<String> {
    Some(split_subtitle(runs).0).filter(|s| !s.is_empty())
}

/// Just the duration field ("Song • Delara • 3:02" → "3:02"). Song cards carry it in the same
/// subtitle they take their artist from, and dropping it leaves the row blank where the time goes
/// once the card is queued.
pub(crate) fn duration_from_runs(runs: Option<&Vec<Value>>) -> Option<String> {
    split_subtitle(runs).2
}

/// Deepest/last thumbnail URL under this node (highest resolution).
pub(crate) fn last_thumbnail(node: &Value) -> Option<String> {
    // Find any `thumbnails: [ { url }, ... ]` array and take the last url.
    fn walk(v: &Value) -> Option<String> {
        match v {
            Value::Object(map) => {
                if let Some(arr) = map.get("thumbnails").and_then(Value::as_array) {
                    if let Some(url) = arr.last().and_then(|t| t.get("url")).and_then(Value::as_str)
                    {
                        return Some(url.to_owned());
                    }
                }
                map.values().find_map(walk)
            }
            Value::Array(arr) => arr.iter().find_map(walk),
            _ => None,
        }
    }
    walk(node)
}

/// Recursively collect every object that is the value of a key named `key`.
pub(crate) fn find_all<'a>(root: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    fn walk<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
        match v {
            Value::Object(map) => {
                for (k, val) in map {
                    if k == key {
                        out.push(val);
                    }
                    walk(val, key, out);
                }
            }
            Value::Array(arr) => arr.iter().for_each(|e| walk(e, key, out)),
            _ => {}
        }
    }
    walk(root, key, &mut out);
    out
}

/// Like [`find_all`], but does not descend into a node once it matches `key`. Use when collecting
/// "top-level" renderers (e.g. playlist track rows): an *editable* playlist item embeds a nested
/// copy of its own `musicResponsiveListItemRenderer` inside an add-suggestion edit command, so a
/// deep search counts every track twice. Stopping at the first match avoids that double-count.
pub(crate) fn find_all_shallow<'a>(root: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    fn walk<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
        match v {
            Value::Object(map) => {
                for (k, val) in map {
                    if k == key {
                        out.push(val); // matched — do NOT recurse into it
                    } else {
                        walk(val, key, out);
                    }
                }
            }
            Value::Array(arr) => arr.iter().for_each(|e| walk(e, key, out)),
            _ => {}
        }
    }
    walk(root, key, &mut out);
    out
}

/// First string value under any key named `key`.
pub(crate) fn find_first_str(root: &Value, key: &str) -> Option<String> {
    match root {
        Value::Object(map) => {
            for (k, v) in map {
                if k == key {
                    if let Some(s) = v.as_str() {
                        return Some(s.to_owned());
                    }
                }
                if let Some(s) = find_first_str(v, key) {
                    return Some(s);
                }
            }
            None
        }
        Value::Array(arr) => arr.iter().find_map(|e| find_first_str(e, key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The whole "hide music videos" feature is this predicate, and the way it fails is silent: a
    // wrong JSON path reads `None` everywhere, the filter quietly becomes a no-op, and the setting
    // just looks broken. So: every shape a row arrives in, plus the fail-open case.
    #[test]
    fn video_rows_are_recognised_in_every_renderer_shape() {
        let cfg = |t: &str| {
            json!({ "watchEndpoint": { "videoId": "v",
                "watchEndpointMusicSupportedConfigs": {
                    "watchEndpointMusicConfig": { "musicVideoType": t } } } })
        };
        let overlay = |t: &str| {
            json!({ "musicItemThumbnailOverlayRenderer": { "content": {
                "musicPlayButtonRenderer": { "playNavigationEndpoint": cfg(t) } } } })
        };

        // Queue-panel row (`/next`): no overlay, the tag sits on the row's own endpoint.
        assert!(is_video_row(&json!({ "navigationEndpoint": cfg("MUSIC_VIDEO_TYPE_OMV") })));
        assert!(is_video_row(&json!({ "navigationEndpoint": cfg("MUSIC_VIDEO_TYPE_UGC") })));
        assert!(!is_video_row(&json!({ "navigationEndpoint": cfg("MUSIC_VIDEO_TYPE_ATV") })));

        // List row: `overlay` wins over the row endpoint. Card: same, under `thumbnailOverlay`.
        assert!(is_video_row(&json!({
            "overlay": overlay("MUSIC_VIDEO_TYPE_OMV"),
            "navigationEndpoint": cfg("MUSIC_VIDEO_TYPE_ATV"),
        })));
        assert!(!is_video_row(&json!({
            "thumbnailOverlay": overlay("MUSIC_VIDEO_TYPE_ATV"),
            "navigationEndpoint": cfg("MUSIC_VIDEO_TYPE_OMV"),
        })));

        // Fail open: no tag (or an overlay carrying none) means audio, never hide.
        assert!(!is_video_row(
            &json!({ "navigationEndpoint": { "watchEndpoint": { "videoId": "v" } } })
        ));
        assert!(!is_video_row(&json!({})));

        // A personal upload is audio, so it must not read as a music video. Issue #71: with
        // "hide music videos" on, treating it as one dropped every upload out of the library.
        let up = "MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK";
        assert!(!is_video_row(&json!({ "navigationEndpoint": cfg(up) })));
        assert!(!is_video_row(&json!({ "overlay": overlay(up) })));
    }

    // The upload flag picks the orchestrator's login-only client chain (issue #71), so it has to
    // come off the same lookup `is_video_row` uses: overlay first, row endpoint as the fallback.
    #[test]
    fn upload_rows_are_recognised_in_every_renderer_shape() {
        let cfg = |t: &str| {
            json!({ "watchEndpoint": { "videoId": "v",
                "watchEndpointMusicSupportedConfigs": {
                    "watchEndpointMusicConfig": { "musicVideoType": t } } } })
        };
        let overlay = |t: &str| {
            json!({ "musicItemThumbnailOverlayRenderer": { "content": {
                "musicPlayButtonRenderer": { "playNavigationEndpoint": cfg(t) } } } })
        };
        let up = "MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK";

        assert!(is_upload_row(&json!({ "navigationEndpoint": cfg(up) })));
        assert!(is_upload_row(&json!({ "overlay": overlay(up) })));
        assert!(is_upload_row(&json!({ "thumbnailOverlay": overlay(up) })));
        // The overlay wins over the row endpoint, the same way round as for videos.
        assert!(is_upload_row(&json!({
            "overlay": overlay(up),
            "navigationEndpoint": cfg("MUSIC_VIDEO_TYPE_ATV"),
        })));
        assert!(!is_upload_row(&json!({
            "overlay": overlay("MUSIC_VIDEO_TYPE_ATV"),
            "navigationEndpoint": cfg(up),
        })));

        // Fail closed: no tag means an ordinary track, so the normal client chain still runs.
        assert!(!is_upload_row(&json!({ "navigationEndpoint": cfg("MUSIC_VIDEO_TYPE_OMV") })));
        assert!(!is_upload_row(&json!({})));
    }

    // The library toggle is opaque tokens and no state field: which one is live is told by the
    // icons alone, so a mixed-up pair would send "add" for a song already in the library (a no-op
    // the UI would report as success). Both orientations, the one-way shape, and the two feedback
    // toggles that sit right next to it in a real track menu and must not be mistaken for it.
    #[test]
    fn reads_the_library_action_in_every_shape() {
        let toggle = |default_icon: &str, toggled_icon: &str| {
            json!({ "menu": { "menuRenderer": { "items": [
                { "toggleMenuServiceItemRenderer": {
                    "defaultIcon": { "iconType": default_icon },
                    "defaultServiceEndpoint": { "feedbackEndpoint": { "feedbackToken": "TOK_DEFAULT" } },
                    "toggledIcon": { "iconType": toggled_icon },
                    "toggledServiceEndpoint": { "feedbackEndpoint": { "feedbackToken": "TOK_TOGGLED" } }
                } }
            ] } } })
        };

        // Not in the library: the empty bookmark is the action on offer, so its token is default.
        let out = library_toggle(&toggle("BOOKMARK_BORDER", "BOOKMARK")).unwrap();
        assert!(!out.in_library);
        assert_eq!(out.add_token.as_deref(), Some("TOK_DEFAULT"));
        assert_eq!(out.remove_token.as_deref(), Some("TOK_TOGGLED"));

        // Already in it: the same renderer arrives the other way round.
        let out = library_toggle(&toggle("BOOKMARK", "BOOKMARK_BORDER")).unwrap();
        assert!(out.in_library);
        assert_eq!(out.add_token.as_deref(), Some("TOK_TOGGLED"));
        assert_eq!(out.remove_token.as_deref(), Some("TOK_DEFAULT"));

        // The older spelling, still accepted.
        assert!(!library_toggle(&toggle("LIBRARY_ADD", "LIBRARY_REMOVE")).unwrap().in_library);

        // One direction only, as a plain menu item: still worth offering, with no way back until
        // the row is fetched again.
        let one_way = json!({ "menu": { "menuRenderer": { "items": [
            { "menuServiceItemRenderer": {
                "icon": { "iconType": "BOOKMARK_BORDER" },
                "serviceEndpoint": { "feedbackEndpoint": { "feedbackToken": "TOK_ONE_WAY" } }
            } }
        ] } } });
        let out = library_toggle(&one_way).unwrap();
        assert!(!out.in_library);
        assert_eq!(out.add_token.as_deref(), Some("TOK_ONE_WAY"));
        assert_eq!(out.remove_token, None);

        // Its neighbours in a real menu: "Add to liked songs" is a rating on a different list, and
        // "Pin to Listen again" is a feedback toggle too. Neither is this.
        assert_eq!(library_toggle(&toggle("FAVORITE", "UNFAVORITE")), None);
        assert_eq!(library_toggle(&toggle("KEEP", "KEEP_OFF")), None);
        assert_eq!(library_toggle(&json!({})), None);
    }

    // The rating drives both thumbs on every row, and the third state is new: `DISLIKE` used to
    // collapse into "not liked". An unknown value must read as unrated rather than as a dislike.
    #[test]
    fn reads_all_three_rating_states() {
        let row = |status: &str| {
            json!({ "menu": { "menuRenderer": { "topLevelButtons": [
                { "likeButtonRenderer": { "likeStatus": status } }
            ] } } })
        };
        assert_eq!(like_status(&row("LIKE")), Some(Rating::Like));
        assert_eq!(like_status(&row("DISLIKE")), Some(Rating::Dislike));
        assert_eq!(like_status(&row("INDIFFERENT")), Some(Rating::Indifferent));
        assert_eq!(like_status(&row("SOMETHING_NEW")), Some(Rating::Indifferent));
        assert_eq!(like_status(&json!({})), None);
    }

    // A dead `RDAMVM` radio answers with the seed song plus this marker, which names the mix the
    // song really belongs to. It's the escalation the "start radio did nothing" case runs on.
    #[test]
    fn parses_the_automix_the_panel_continues_into() {
        let root = json!({
            "contents": { "playlistPanelRenderer": { "contents": [
                { "playlistPanelVideoRenderer": {
                    "videoId": "seed1",
                    "title": { "runs": [{ "text": "Only Song" }] },
                    "longBylineText": { "runs": [{ "text": "An Artist" }] },
                    "lengthText": { "runs": [{ "text": "3:00" }] },
                    "thumbnail": { "thumbnails": [{ "url": "https://t/1" }] }
                } },
                { "automixPreviewVideoRenderer": { "content": { "automixPlaylistVideoRenderer": {
                    "navigationEndpoint": { "watchPlaylistEndpoint": { "playlistId": "RDAMVMseed1" } }
                } } } }
            ] } }
        });
        let out = parse_next(&root);
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.automix_playlist_id.as_deref(), Some("RDAMVMseed1"));
    }

    // Signed in, YouTube pairs a radio track's audio row with its music-video twin under one
    // wrapper. Only the primary arm is the queue row; taking the counterpart too put every song
    // in the radio twice, with two different durations. Issue #170.
    #[test]
    fn a_wrapped_row_contributes_only_its_primary() {
        let renderer = |id: &str, len: &str| {
            json!({ "playlistPanelVideoRenderer": {
                "videoId": id,
                "title": { "runs": [{ "text": "Sunflower" }] },
                "longBylineText": { "runs": [{ "text": "Post Malone" }] },
                "lengthText": { "runs": [{ "text": len }] },
                "thumbnail": { "thumbnails": [{ "url": "https://t/1" }] }
            } })
        };
        let root = json!({
            "contents": { "playlistPanelRenderer": { "contents": [
                { "playlistPanelVideoWrapperRenderer": {
                    "primaryRenderer": renderer("audio1", "2:39"),
                    "counterpart": [{ "counterpartRenderer": renderer("video1", "2:42") }]
                } },
                renderer("plain1", "3:00")
            ] } }
        });
        let out = parse_next(&root);
        let ids: Vec<&str> = out.items.iter().map(|i| i.video_id.as_str()).collect();
        assert_eq!(ids, ["audio1", "plain1"]);
    }

    // A real radio page has no automix marker — nothing to escalate to, and nothing to mistake a
    // regular playlist id for.
    #[test]
    fn no_automix_on_a_live_radio_page() {
        let root = json!({
            "contents": { "playlistPanelRenderer": {
                "playlistId": "RDAMVMseed1",
                "contents": []
            } }
        });
        assert_eq!(parse_next(&root).automix_playlist_id, None);
    }

    #[test]
    fn parses_search_item() {
        let root = json!({
            "a": { "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": "abc123" },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Song Title" }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                        { "text": "The Artist", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCartist1" } } },
                        { "text": " & " },
                        { "text": "Guest", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCartist2" } } },
                        { "text": " • " },
                        { "text": "The Album", "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREalbum1" } } },
                        { "text": " • " }, { "text": "3:21" }
                    ] } } }
                ],
                "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                    { "url": "small.jpg" }, { "url": "big.jpg" }
                ] } } }
            }}
        });
        let r = parse_search(&root);
        assert_eq!(r.items.len(), 1);
        let s = &r.items[0];
        assert_eq!(s.video_id, "abc123");
        assert_eq!(s.title, "Song Title");
        assert_eq!(s.artists, "The Artist & Guest");
        assert_eq!(s.artist_id.as_deref(), Some("UCartist1"));
        // Each artist keeps its own link; the run list stops at the first "•" (album/duration).
        assert_eq!(
            s.artist_runs.iter().map(|r| (r.text.as_str(), r.id.as_deref())).collect::<Vec<_>>(),
            vec![("The Artist", Some("UCartist1")), (" & ", None), ("Guest", Some("UCartist2"))]
        );
        assert_eq!(s.album.as_deref(), Some("The Album"));
        assert_eq!(s.album_id.as_deref(), Some("MPREalbum1"));
        assert_eq!(s.duration.as_deref(), Some("3:21"));
        assert_eq!(s.thumbnail.as_deref(), Some("big.jpg"));
    }

    // Album rows carry "53M plays" in the third flex column; playlist rows put the album name
    // there. Confusing the two would print an album title where the play count goes.
    #[test]
    fn play_count_comes_only_from_a_plays_column() {
        let row = |third: Value| {
            json!({ "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": "abc123" },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Song Title" }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "The Artist" }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": third }] } } }
                ]
            }})
        };
        let plays = |third: Value| parse_list_item(&row(third)["musicResponsiveListItemRenderer"]);
        assert_eq!(plays(json!("53M plays")).unwrap().play_count.as_deref(), Some("53M"));
        assert_eq!(plays(json!("1,234 plays")).unwrap().play_count.as_deref(), Some("1,234"));
        assert_eq!(plays(json!("The Album")).unwrap().play_count, None);
        assert_eq!(plays(json!("")).unwrap().play_count, None);
        // The other half of the same discriminator: what isn't a play count is the album, which is
        // the only place a playlist row carries one (its subtitle is the artist alone). Without it
        // every playlist track parses album-less and grouping a playlist by album does nothing.
        assert_eq!(plays(json!("The Album")).unwrap().album.as_deref(), Some("The Album"));
        assert_eq!(plays(json!("53M plays")).unwrap().album, None);
        assert_eq!(plays(json!("")).unwrap().album, None);
    }

    // An artist or album with a colon in its name ("Cast of EPIC: The Musical") used to read as the
    // duration, so the row printed that whole string where the time goes and lost the real length.
    #[test]
    fn a_colon_in_a_name_is_not_a_duration() {
        let row = json!({
            "playlistItemData": { "videoId": "abc123" },
            "flexColumns": [
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Monster" }] } } },
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                    { "text": "Jorge Rivera-Herrans & Cast of EPIC: The Musical",
                      "navigationEndpoint": { "browseEndpoint": { "browseId": "UCjorge" } } },
                    { "text": " • " }, { "text": "EPIC: The Musical" }
                ] } } }
            ],
            "fixedColumns": [
                { "musicResponsiveListItemFixedColumnRenderer": { "text": { "runs": [{ "text": "4:32" }] } } }
            ]
        });
        let s = parse_list_item(&row).unwrap();
        assert_eq!(s.artists, "Jorge Rivera-Herrans & Cast of EPIC: The Musical");
        assert_eq!(s.album.as_deref(), Some("EPIC: The Musical"));
        assert_eq!(s.duration.as_deref(), Some("4:32"));
    }

    #[test]
    fn parses_next_panel_video() {
        let root = json!({
            "contents": { "playlistPanelRenderer": { "contents": [
                { "playlistPanelVideoRenderer": {
                    "videoId": "vid9",
                    "title": { "runs": [{ "text": "Next Song" }] },
                    "longBylineText": { "runs": [{ "text": "Artist A" }, { "text": " & " }, { "text": "Artist B" }] },
                    "lengthText": { "runs": [{ "text": "4:05" }] },
                    "thumbnail": { "thumbnails": [{ "url": "t.jpg" }] }
                }}
            ], "continuations": [{ "nextContinuationData": { "continuation": "CONT_TOKEN" } }] } },
            "tabs": [{ "tabRenderer": { "title": "Lyrics", "endpoint": { "browseEndpoint": {
                "browseId": "MPLYt_abc123",
                "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": {
                    "pageType": "MUSIC_PAGE_TYPE_TRACK_LYRICS" } }
            } } } }]
        });
        let r = parse_next(&root);
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].video_id, "vid9");
        assert_eq!(r.items[0].title, "Next Song");
        assert_eq!(r.items[0].artists, "Artist A & Artist B");
        assert_eq!(r.items[0].duration.as_deref(), Some("4:05"));
        assert_eq!(r.continuation.as_deref(), Some("CONT_TOKEN"));
        assert_eq!(r.lyrics_browse_id.as_deref(), Some("MPLYt_abc123"));
        // No overlay in this response, and the panel row carries no `likeStatus` either: the
        // rating has to read as unknown, not as "not liked".
        assert_eq!(r.rating, None);
    }

    /// The one place a `next` response states the requested video's rating (issue #93). It has to
    /// come from the player overlay, not from a panel row, and not from the *seed* row's absence.
    #[test]
    fn next_reads_the_requested_videos_rating_from_the_player_overlay() {
        let root = json!({
            "contents": { "playlistPanelRenderer": { "contents": [
                { "playlistPanelVideoRenderer": {
                    "videoId": "vid9",
                    "title": { "runs": [{ "text": "Next Song" }] }
                }}
            ] } },
            "playerOverlays": { "playerOverlayRenderer": { "actions": [
                { "likeButtonRenderer": { "likeStatus": "LIKE" } }
            ] } }
        });
        let r = parse_next(&root);
        assert_eq!(r.rating, Some(Rating::Like));
        // The panel row keeps its own (absent) rating: the overlay speaks for the seed alone.
        assert_eq!(r.items[0].rating, None);
    }

    /// An unfiltered search row leads with the result type ("Song • Delara • 3:02"). It must not
    /// end up in `artists` — that string is what gets scrobbled.
    #[test]
    fn drops_the_result_type_from_a_search_row() {
        let root = json!({
            "a": { "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": "abc123" },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Hele uka" }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                        { "text": "Song" }, { "text": " • " },
                        { "text": "Delara", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdelara" } } },
                        { "text": " • " }, { "text": "3:02" }
                    ] } } }
                ]
            }}
        });
        let s = &parse_search(&root).items[0];
        assert_eq!(s.artists, "Delara");
        assert_eq!(s.album, None);
        assert_eq!(s.duration.as_deref(), Some("3:02"));
        // The links describe the same field as `artists` — never the "Song" label in front of it.
        assert_eq!(
            s.artist_runs.iter().map(|r| (r.text.as_str(), r.id.as_deref())).collect::<Vec<_>>(),
            [("Delara", Some("UCdelara"))]
        );
    }

    /// A queue row's byline is a whole descriptor; only its artist field is the artist.
    #[test]
    fn panel_byline_keeps_only_the_artist() {
        let root = json!({
            "contents": { "playlistPanelRenderer": { "contents": [
                { "playlistPanelVideoRenderer": {
                    "videoId": "vid9",
                    "title": { "runs": [{ "text": "Hele uka" }] },
                    "longBylineText": { "runs": [
                        { "text": "Delara", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdelara" } } },
                        { "text": " • " }, { "text": "Sjelen" }, { "text": " • " }, { "text": "2026" }
                    ] }
                }}
            ] } }
        });
        assert_eq!(parse_next(&root).items[0].artists, "Delara");
    }

    #[test]
    fn splits_datasync_id() {
        assert_eq!(split_datasync_id("realid||other"), "realid");
        assert_eq!(split_datasync_id("||fallback"), "fallback");
        assert_eq!(split_datasync_id("plain"), "plain");
    }

    #[test]
    fn parses_account_menu() {
        let root: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/account_menu_single.json"))
                .unwrap();
        let a = parse_account_menu(&root);
        assert_eq!(a.name.as_deref(), Some("Personal channel"));
        assert_eq!(a.handle.as_deref(), Some("@personal"));
        assert_eq!(a.email.as_deref(), Some("listener@example.invalid"));
        assert_eq!(a.thumbnail.as_deref(), Some("https://example.invalid/avatar-large.jpg"));
        assert_eq!(a.channel_id.as_deref(), Some("UCSANITIZEDPERSONAL"));
        assert_eq!(a.data_sync_id.as_deref(), Some("personal-sync"));
        assert_eq!(a.visitor_data.as_deref(), Some("CgtSANITIZEDVISITOR"));
    }

    #[test]
    fn parses_one_selectable_identity() {
        let root: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/accounts_list_one.json"))
                .unwrap();
        let identities = parse_account_identities(&root);
        assert_eq!(identities.len(), 1);
        let identity = &identities[0];
        assert_eq!(identity.name, "Personal channel");
        assert_eq!(identity.handle.as_deref(), Some("@personal"));
        assert_eq!(identity.email.as_deref(), Some("listener@example.invalid"));
        assert_eq!(identity.channel_id.as_deref(), Some("UCSANITIZEDPERSONAL"));
        assert_eq!(identity.data_sync_id, "personal-sync");
        assert!(identity.is_selected);
    }

    #[test]
    fn parses_multiple_identities_and_keeps_a_persisted_non_default_choice() {
        let root: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/accounts_list_multiple.json"))
                .unwrap();
        let identities = parse_account_identities(&root);
        assert_eq!(identities.len(), 2);
        assert!(identities[0].is_selected);

        // The app restores by server-issued id, not YouTube's current/default marker.
        let persisted = identities.iter().find(|i| i.data_sync_id == "brand-page-id").unwrap();
        assert_eq!(persisted.name, "Brand channel");
        assert_eq!(persisted.channel_id.as_deref(), Some("UCSANITIZEDBRAND"));
        assert!(!persisted.is_selected);
    }

    #[test]
    fn missing_identity_metadata_stays_optional() {
        let root: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/accounts_list_missing_optional.json"
        ))
        .unwrap();
        let identities = parse_account_identities(&root);
        assert_eq!(identities.len(), 1);
        let identity = &identities[0];
        assert_eq!(identity.name, "No optional metadata");
        assert_eq!(identity.handle, None);
        assert_eq!(identity.email, None);
        assert_eq!(identity.thumbnail, None);
        assert_eq!(identity.channel_id, None);
    }

    #[test]
    fn malformed_or_unusable_identity_tokens_are_not_selectable() {
        let root: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/accounts_list_malformed_ids.json"
        ))
        .unwrap();
        let identities = parse_account_identities(&root);
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].name, "Missing token");
        assert_eq!(identities[0].data_sync_id, "active-response-sync");
        assert_eq!(identities[1].name, "Fallback token");
        assert_eq!(identities[1].data_sync_id, "fallback-sync");
    }

    // Three keys, one badge shape, and a badge array that can hold other badges alongside it.
    // Getting the key wrong reads false everywhere and the flag silently never shows.
    #[test]
    fn explicit_badge_is_read_from_every_key_it_arrives_under() {
        let badge = json!([{ "musicInlineBadgeRenderer": {
            "icon": { "iconType": "MUSIC_EXPLICIT_BADGE" },
            "accessibilityData": { "accessibilityData": { "label": "Explicit" } }
        } }]);
        // List row (search / playlist / album tracks), two-row card, playlist+album header.
        assert!(is_explicit(&json!({ "badges": badge })));
        assert!(is_explicit(&json!({ "subtitleBadges": badge })));
        assert!(is_explicit(&json!({ "subtitleBadge": badge })));
        // Alongside another badge, and with no badge at all.
        assert!(is_explicit(&json!({ "badges": [
            { "musicInlineBadgeRenderer": { "icon": { "iconType": "MUSIC_NEW_RELEASE_BADGE" } } },
            badge[0]
        ] })));
        assert!(!is_explicit(&json!({ "badges": [
            { "musicInlineBadgeRenderer": { "icon": { "iconType": "MUSIC_NEW_RELEASE_BADGE" } } }
        ] })));
        assert!(!is_explicit(&json!({})));
        // Never from a nested row: an album card inside a shelf must not badge the shelf's row.
        assert!(!is_explicit(&json!({ "contents": [{ "subtitleBadges": badge }] })));
    }

    #[test]
    fn absent_or_empty_response_datasync_id_is_none() {
        assert_eq!(parse_account_menu(&json!({})).data_sync_id, None);
        assert_eq!(
            parse_account_menu(&json!({
                "responseContext": { "mainAppWebResponseContext": { "datasyncId": "||" } }
            }))
            .data_sync_id,
            None
        );
    }

    /// A row on a playlist you can't edit still carries a playlistSetVideoId, so the remove action
    /// in its own menu is what decides whether the row can be removed (issue #86: a collaborator
    /// can remove the tracks they added, and only those).
    #[test]
    fn set_video_id_only_comes_from_a_row_that_may_be_removed() {
        let row = |menu: serde_json::Value| {
            json!({
                "playlistItemData": { "playlistSetVideoId": "SVID", "videoId": "vid1" },
                "flexColumns": [{ "musicResponsiveListItemFlexColumnRenderer": {
                    "text": { "runs": [{ "text": "Alpha" }] } } }],
                "menu": menu
            })
        };
        let remove = json!({ "menuRenderer": { "items": [{ "menuServiceItemRenderer": {
            "serviceEndpoint": { "playlistEditEndpoint": {
                "actions": [{ "setVideoId": "SVID", "action": "ACTION_REMOVE_VIDEO" }] } } } }] } });
        let share = json!({ "menuRenderer": { "items": [{ "menuNavigationItemRenderer": {} }] } });
        assert_eq!(parse_list_item(&row(remove)).unwrap().set_video_id.as_deref(), Some("SVID"));
        assert_eq!(parse_list_item(&row(share)).unwrap().set_video_id, None);
    }

    /// A collaborative playlist stamps each row with whoever added it.
    #[test]
    fn a_row_carries_the_contributor_who_added_it() {
        let node = json!({
            "playlistItemData": { "videoId": "vid1" },
            "flexColumns": [{ "musicResponsiveListItemFlexColumnRenderer": {
                "text": { "runs": [{ "text": "Alpha" }] } } }],
            "contributorsAvatars": { "avatarStackViewModel": { "avatars": [{ "avatarViewModel": {
                "image": { "sources": [{ "url": "https://yt3.ggpht.com/abc=s48" }] },
                "accessibilityText": "Rajat"
            } }] } }
        });
        let item = parse_list_item(&node).unwrap();
        assert_eq!(item.added_by.as_deref(), Some("Rajat"));
        assert_eq!(item.added_by_avatar.as_deref(), Some("https://yt3.ggpht.com/abc=s48"));
        // An ordinary playlist row has no stack, so no name to draw.
        assert_eq!(
            parse_list_item(&json!({
                "playlistItemData": { "videoId": "vid1" },
                "flexColumns": [{ "musicResponsiveListItemFlexColumnRenderer": {
                    "text": { "runs": [{ "text": "Alpha" }] } } }]
            }))
            .unwrap()
            .added_by,
            None
        );
    }
}
