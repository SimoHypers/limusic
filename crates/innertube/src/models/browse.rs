//! Browse-surface parsing: home feed, library playlists, playlist/album pages. context/08.
//!
//! Same tolerant walk-the-tree approach as `metadata.rs` (reuses its helpers): we locate the few
//! renderer node types we care about anywhere in the response and pull only the fields the UI
//! needs — robust to YouTube reshuffling the surrounding container tree.
//! - `musicCarouselShelfRenderer` → a home section (a titled row of cards).
//! - `musicTwoRowItemRenderer`     → a card (playlist / album / artist / song).
//! - `musicResponsiveListItemRenderer` → a track row (shared with search; via `parse_list_item`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::metadata::{
    artist_runs, artists_from_runs, duration_from_runs, find_all, find_all_shallow, find_first_str,
    first_artist_id, flex_column_text, flex_runs, is_explicit, is_upload_endpoint, is_upload_row,
    is_video_endpoint, is_video_row, is_video_type, last_thumbnail, list_item_video_id,
    parse_list_item, play_count, runs_text, runs_text_opt, ArtistRun, SongItem,
};

/// One clickable card in a home carousel or library grid. Flat + `kind`-tagged so the UI can
/// switch cheaply: `song` plays `id` (a videoId); `playlist`/`album`/`artist` navigate to the
/// browse page for `id` (a browseId).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseItem {
    /// `song` | `playlist` | `album` | `artist`.
    pub kind: &'static str,
    /// videoId (song) or browseId (playlist/album/artist).
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// "3:47" — only on song rows from a list-style carousel (a card shelf carries none). Kept out
    /// of `subtitle` so the queue and the scrobbler still get a clean artist string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    /// Song cards only: the `subtitle` artist line run by run, each tagged with its channel id when
    /// it links one. Carried so a card that gets played (search rows, home shelves) reaches the
    /// player bar with the same navigable artists a track row has. Empty when nothing links.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artist_runs: Vec<ArtistRun>,
    /// Song rows only: the play count as YouTube abbreviates it ("2.5B"), from a search row's
    /// plays column. Absent on rows that carry no such column (home cards, carousels).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub play_count: Option<String>,
    /// Song cards only: this card links a music video, not the audio track. Drives the
    /// "hide music videos" setting.
    #[serde(default)]
    pub is_video: bool,
    /// Song cards only: one of the user's own uploads, which only an authenticated client can
    /// stream. Carried so a card played from a shelf reaches the orchestrator with the flag set.
    #[serde(default)]
    pub is_upload: bool,
    /// YouTube marks this card explicit ([`is_explicit`]) — a track, or an album whose tracks are.
    #[serde(default)]
    pub explicit: bool,
}

/// A titled row of cards on the home feed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub title: String,
    pub items: Vec<BrowseItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more_browse_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more_params: Option<String>,
}

/// A mood/genre filter chip above the home feed (`chipCloudChipRenderer`): its label plus the
/// `params` token to re-browse `FEmusic_home` with, which returns a home feed filtered to it.
#[derive(Debug, Clone, Serialize)]
pub struct HomeChip {
    pub title: String,
    pub params: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HomePage {
    /// Empty when YouTube sends no chip cloud (it does for the unfiltered and filtered feeds alike).
    #[serde(default)]
    pub chips: Vec<HomeChip>,
    pub sections: Vec<Section>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
}

/// The orders YouTube can put a playlist in. context/08.
///
/// YouTube stores this choice itself, which is why Limusic asks for a sorted page instead of
/// sorting one in the UI: the list then reads the same way here, in YouTube Music, and in every
/// other client on the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistSort {
    /// The playlist's own order — what dragging rows around in YouTube Music produces.
    Default,
    Newest,
    Oldest,
    Title,
    Artist,
    Album,
    /// YouTube's "Top voted". Limusic's own menu does not offer it, but a list can already be in
    /// it, so it still has to round-trip.
    Top,
}

impl PlaylistSort {
    const ALL: [Self; 7] = [
        Self::Default,
        Self::Newest,
        Self::Oldest,
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::Top,
    ];

    /// The `browse` `params` that returns the list in this order. Every list that has a sort menu
    /// honours these, in both directions, and asking for one writes nothing to the account (Liked
    /// Music is the exception: YouTube remembers whatever it was last asked for).
    ///
    /// The values are a protobuf literal — field 139 wrapping `{1: order, 2: kind}`, order 1
    /// ascending / 2 descending, kind 3 date / 4 top-voted / 5 title / 6 artist / 7 album, and an
    /// empty submessage for the stored order. Copied from YouTube's own menu rather than encoded,
    /// because these eleven are all of them.
    pub fn params(self, desc: bool) -> &'static str {
        match (self, desc) {
            (Self::Default, _) => "2ggA", // da 08 00 — empty, i.e. leave the order alone
            // "Newest" is already a descending sort on date, so reversing it *is* "Oldest".
            (Self::Newest, false) | (Self::Oldest, true) => "2ggECAIQAw==",
            (Self::Oldest, false) | (Self::Newest, true) => "2ggECAEQAw==",
            (Self::Top, false) => "2ggECAIQBA==",
            (Self::Top, true) => "2ggECAEQBA==",
            (Self::Title, false) => "2ggECAEQBQ==",
            (Self::Title, true) => "2ggECAIQBQ==",
            (Self::Artist, false) => "2ggECAEQBg==",
            (Self::Artist, true) => "2ggECAIQBg==",
            (Self::Album, false) => "2ggECAEQBw==",
            (Self::Album, true) => "2ggECAIQBw==",
        }
    }

    /// The `browse/edit_playlist` action that stores this order on a playlist you own.
    ///
    /// There is deliberately no descending Title/Artist/Album here: YouTube's
    /// `playlistDynamicSortPreference` accepts 1..3 and nothing else. Values outside that answer
    /// HTTP 500 *and* leave the playlist serving an order its own menu then reports as manual, so
    /// never widen this match to express a reversed sort.
    pub fn edit_action(self) -> Value {
        let (field, n) = match self {
            Self::Default => ("playlistVideoOrder", 0),
            Self::Newest => ("playlistVideoOrder", 1),
            Self::Oldest => ("playlistVideoOrder", 2),
            Self::Top => ("playlistVideoOrder", 6),
            Self::Title => ("playlistDynamicSortPreference", 1),
            Self::Artist => ("playlistDynamicSortPreference", 2),
            Self::Album => ("playlistDynamicSortPreference", 3),
        };
        let action = match field {
            "playlistVideoOrder" => "ACTION_SET_PLAYLIST_VIDEO_ORDER",
            _ => "ACTION_SET_PLAYLIST_DYNAMIC_SORT_PREFERENCE",
        };
        serde_json::json!({ "action": action, field: n })
    }

    fn from_params(params: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.params(false) == params || s.params(true) == params)
    }

    fn from_edit_action(action: &Value) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.edit_action() == *action)
    }
}

/// The sort menu in a playlist header, when YouTube offers one. context/08.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortMenu {
    /// The order the list is in right now, when it is one we have a name for.
    pub selected: Option<PlaylistSort>,
    /// The options are `playlistEditEndpoint`s, so the choice can be written back and every other
    /// client will follow it. Playlists you own only. Everywhere else the menu is a view-only
    /// `browseEndpoint`: Liked Music remembers the choice anyway, someone else's playlist does not.
    pub editable: bool,
}

/// A playlist or album detail page: header + tracks + a paging token.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistPage {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub thumbnail: Option<String>,
    /// The playlist's own blurb, as the edit dialog needs it back to leave it alone.
    pub description: Option<String>,
    /// `PUBLIC` / `PRIVATE` / `UNLISTED`. Only owned playlists carry the edit header it comes from.
    pub privacy: Option<String>,
    /// Custom artwork the user picked on this machine, filled in by the app (YouTube has no
    /// playlist-thumbnail API). `thumbnail` stays YouTube's, so dropping the custom one is free.
    pub cover: Option<String>,
    pub items: Vec<SongItem>,
    pub continuation: Option<String>,
    /// True only when the signed-in user owns this playlist (rename/delete allowed). YouTube wraps
    /// the header in `musicEditablePlaylistDetailHeaderRenderer` exactly for owned playlists.
    pub owned: bool,
    /// Collaboration is on: other people can add to this playlist, and each of you may remove only
    /// what you added (which is per row, see `SongItem::set_video_id`). YouTube says so by turning
    /// the header's facepile into the "Collaborate" panel's button.
    pub collaborative: bool,
    /// Absent on lists YouTube will not reorder at all: albums and its own radio mixes.
    pub sort_menu: Option<SortMenu>,
}

/// A page of extra tracks fetched via a continuation token.
#[derive(Debug, Clone, Serialize)]
pub struct PlaylistContinuation {
    pub items: Vec<SongItem>,
    pub continuation: Option<String>,
}

/// An artist detail page (`browse` on a `UC…` channel id). context/08.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistPage {
    pub name: Option<String>,
    /// The wide hero/banner image from the immersive header.
    pub thumbnail: Option<String>,
    pub description: Option<String>,
    /// e.g. "32.7M subscribers" (the long form; the short one is a bare "32.7M").
    pub subscribers: Option<String>,
    /// The header's other count line, e.g. "137M monthly audience". Absent on artists YouTube
    /// publishes no listener figure for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_listeners: Option<String>,
    /// Subscribe target — the channelId (falls back to the browseId, which is the same `UC…`).
    pub channel_id: String,
    pub subscribed: bool,
    /// This artist's radio (`RDEM…` / `RDAO…`), from the header's "start radio" button. Server-
    /// supplied: unlike a song or a playlist, an artist radio id can't be built client-side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radio_playlist_id: Option<String>,
    /// Top songs shelf (usually 5).
    pub top_songs: Vec<SongItem>,
    /// The shelf's "Show all" target: a `VL…` playlist of the artist's songs, pageable like any
    /// other playlist. Absent on artists whose shelf has no show-all link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_songs_id: Option<String>,
    /// Card carousels (Albums / Singles / Videos / …), each with an optional "More" browse target.
    pub sections: Vec<ArtistCarousel>,
}

/// One titled card row on an artist page, plus where its "More" button navigates.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistCarousel {
    pub title: String,
    pub items: Vec<BrowseItem>,
    pub more_browse_id: Option<String>,
    pub more_params: Option<String>,
}

/// An album detail page (`browse` on an `MPRE…` id). Like a playlist but with album-specific
/// header fields (artist link, type/year, description). context/08.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumPage {
    pub title: Option<String>,
    pub artist: Option<String>,
    /// The album artist's channel browseId (`UC…`) — links to the artist page.
    pub artist_id: Option<String>,
    /// The artist line run by run, so a collaborative album links each artist separately.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artist_runs: Vec<ArtistRun>,
    pub artist_thumbnail: Option<String>,
    /// e.g. "Album • 2026".
    pub subtitle: Option<String>,
    /// e.g. "18 songs • 1 hour, 8 minutes".
    pub second_subtitle: Option<String>,
    pub description: Option<String>,
    /// The album cover.
    pub thumbnail: Option<String>,
    pub items: Vec<SongItem>,
    pub continuation: Option<String>,
    /// The album itself is flagged explicit (its header wears the badge, not just some tracks).
    pub explicit: bool,
    /// The album's audio playlist id (`OLAK5uy_…`) — the radio seed for autoplay continuation and
    /// the target of the library save (an album in your library is a "like" on this playlist).
    pub playlist_id: Option<String>,
    /// Whether the album is already saved to the signed-in user's library.
    pub in_library: bool,
    /// The card shelves YouTube ships under the track list: other versions of this release, more
    /// from the artist, related albums. Same response as the tracks, so they cost no extra fetch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<ArtistCarousel>,
}

/// Parse a `FEmusic_home` response into filter chips + titled carousel sections. context/08.
/// A chip's `params` fed back into `browse(FEmusic_home)` yields that mood's feed (same shape,
/// hence the same parser — "Mixed for you" and friends are just more carousel shelves).
pub fn parse_home(root: &Value) -> HomePage {
    let chips: Vec<HomeChip> = find_all(root, "chipCloudChipRenderer")
        .into_iter()
        .filter_map(|c| {
            let title = runs_text(c.get("text"))?;
            let params = c
                .get("navigationEndpoint")?
                .get("browseEndpoint")?
                .get("params")?
                .as_str()?
                .to_owned();
            Some(HomeChip { title, params })
        })
        .collect();
    let mut sections = Vec::new();
    for shelf in find_all(root, "musicCarouselShelfRenderer") {
        let header = find_all(shelf, "musicCarouselShelfBasicHeaderRenderer").into_iter().next();
        let title = header.and_then(|h| runs_text(h.get("title"))).unwrap_or_default();
        let items: Vec<BrowseItem> = shelf
            .get("contents")
            .and_then(Value::as_array)
            .map(|c| c.iter().filter_map(parse_carousel_item).collect())
            .unwrap_or_default();
        if !items.is_empty() {
            let more = header
                .and_then(|h| h.get("moreContentButton"))
                .and_then(|b| find_all(b, "browseEndpoint").into_iter().next());
            let more_browse_id =
                more.and_then(|e| e.get("browseId")).and_then(Value::as_str).map(str::to_owned);
            let more_params =
                more.and_then(|e| e.get("params")).and_then(Value::as_str).map(str::to_owned);
            sections.push(Section { title, items, more_browse_id, more_params });
        }
    }
    HomePage { chips, sections, continuation: continuation_token(root) }
}

/// One date bucket of the play history ("Today", "Yesterday", "This week"). context/08.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryGroup {
    pub title: String,
    pub items: Vec<SongItem>,
}

/// Parse a `FEmusic_history` response: one `musicShelfRenderer` per date bucket, each holding that
/// day's track rows in most-recent-first order. context/08.
pub fn parse_history(root: &Value) -> Vec<HistoryGroup> {
    find_all(root, "musicShelfRenderer")
        .into_iter()
        .filter_map(|shelf| {
            let title = runs_text(shelf.get("title"))?;
            let items: Vec<SongItem> =
                find_all_shallow(shelf.get("contents")?, "musicResponsiveListItemRenderer")
                    .into_iter()
                    .filter_map(parse_list_item)
                    .collect();
            (!items.is_empty()).then_some(HistoryGroup { title, items })
        })
        .collect()
}

/// Parse a `FEmusic_liked_*` response into a flat grid of cards. context/08. Playlists and albums
/// come back as a grid of two-row cards; library artists come back as a shelf of list rows
/// instead, so fall back to those when the grid is empty.
pub fn parse_library(root: &Value) -> Vec<BrowseItem> {
    let cards: Vec<BrowseItem> = find_all(root, "musicTwoRowItemRenderer")
        .into_iter()
        .filter_map(parse_two_row_item)
        .collect();
    if !cards.is_empty() {
        return cards;
    }
    find_all(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(list_item_to_browse_item)
        .collect()
}

/// Parse a playlist/album (`VL…` / `MPRE…`) browse response. context/08.
pub fn parse_playlist(root: &Value) -> PlaylistPage {
    let header = playlist_header(root);
    let title = header.and_then(|h| runs_text(h.get("title")));
    // `secondSubtitle` usually carries "N songs • Xh Ym"; fall back to `subtitle`.
    let subtitle = header
        .and_then(|h| runs_text(h.get("secondSubtitle")).or_else(|| runs_text(h.get("subtitle"))));
    let thumbnail = header.and_then(last_thumbnail);
    // An uploaded album opens on this route, not the album one: YouTube gives a privately-owned
    // release a `FEmusic_library_privately_owned_release_detail…` browseId, which is not an
    // `MPRE…`, so `browse_kind_from_id` calls it a playlist. Its rows carry no per-track art (the
    // cover is shown once in the header), and a track played off the page reached the queue, the
    // player bar and the OS widget with nothing to draw. Fill the gap from the cover, exactly as
    // `parse_album` does. Scoped to the header's own thumbnail subtree for the same reason it is
    // there, and with no fallback to the unscoped `thumbnail` above: that walk returns whatever
    // thumbnails array it reaches first, so a header with only a `straplineThumbnail` would hand
    // every row the artist avatar. Issue #160.
    let cover = header.and_then(|h| h.get("thumbnail")).and_then(last_thumbnail);
    let items = find_all_shallow(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(parse_list_item)
        .map(|mut it| {
            if it.thumbnail.is_none() {
                it.thumbnail = cover.clone();
            }
            it
        })
        .collect();
    // Present only for playlists the signed-in user owns — the sole reliable ownership signal.
    let owned = !find_all(root, "musicEditablePlaylistDetailHeaderRenderer").is_empty();
    // The facepile is on every playlist header (the owner's avatar); only a collaborative one taps
    // through to the Collaborate panel, and it also carries an avatar per contributor. Note a
    // collaborative playlist you own has no editable header at all, so `owned` reads false on it.
    let collaborative = find_all(root, "facepile")
        .iter()
        .any(|f| find_all(f, "tag").iter().any(|t| t.as_str() == Some("PAplaylist_collaborate")));
    PlaylistPage {
        title,
        subtitle,
        thumbnail,
        description: header_description(root, header),
        privacy: playlist_privacy(root),
        cover: None, // the app fills this in from its own store
        items,
        continuation: shelf_continuation(root),
        owned,
        collaborative,
        sort_menu: sort_menu(root),
    }
}

/// The track shelf's sort menu. Scoped to that shelf for the same reason `shelf_continuation` is:
/// an owned playlist's page carries a suggestions section of its own, and the library grids this
/// parser never sees have a sort menu with an unrelated vocabulary.
fn sort_menu(root: &Value) -> Option<SortMenu> {
    let items = find_all(track_shelf(root)?, "sortFilterSubMenuRenderer")
        .into_iter()
        .next()?
        .get("subMenuItems")?
        .as_array()?;
    Some(SortMenu {
        selected: items
            .iter()
            .find(|i| i.get("selected").and_then(Value::as_bool).unwrap_or(false))
            .and_then(selected_sort),
        editable: items
            .iter()
            .any(|i| i.pointer("/serviceEndpoint/playlistEditEndpoint").is_some()),
    })
}

/// Read a sort out of a menu entry, from whichever endpoint flavour the list uses. Both are keyed
/// on the payload rather than the label, which YouTube localises.
fn selected_sort(item: &Value) -> Option<PlaylistSort> {
    let endpoint = item.get("serviceEndpoint")?;
    if let Some(action) = endpoint.pointer("/playlistEditEndpoint/actions/0") {
        return PlaylistSort::from_edit_action(action);
    }
    // The browse flavour hands its params back percent-encoded ("2ggECAEQBQ%3D%3D").
    let params = endpoint.pointer("/browseEndpoint/params")?.as_str()?;
    PlaylistSort::from_params(&urlencoding::decode(params).ok()?)
}

/// Parse a browse continuation response (more playlist tracks). context/08.
pub fn parse_playlist_continuation(root: &Value) -> PlaylistContinuation {
    // Shallow find: on an owned/editable playlist each track row embeds a nested copy of its own
    // renderer (an add-suggestion edit command), so a deep find_all would return every track twice.
    let items = find_all_shallow(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(parse_list_item)
        .collect();
    // A continuation response is mostly shelf already; the sweep stays as the fallback for the
    // `…ShelfContinuation` shapes that carry no `…ShelfRenderer` node to scope to.
    PlaylistContinuation {
        items,
        continuation: shelf_continuation(root).or_else(|| continuation_token(root)),
    }
}

/// Categorized results for an unfiltered search: a mix of a "top result" set plus the per-type
/// shelves YouTube returns (songs / albums / artists / playlists). context/08.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub top: Vec<BrowseItem>,
    pub songs: Vec<BrowseItem>,
    pub albums: Vec<BrowseItem>,
    pub artists: Vec<BrowseItem>,
    pub playlists: Vec<BrowseItem>,
}

/// Parse an unfiltered `search` response. The metadata client (WEB_REMIX) returns a **flat** list
/// (one `musicCardShelfRenderer` "top result" + many `itemSectionRenderer` rows), NOT titled
/// per-type shelves — so we classify each row by its own navigation target. context/08.
pub fn parse_search_all(root: &Value) -> SearchResults {
    let mut r = SearchResults::default();
    let Some(contents) = find_all(root, "sectionListRenderer")
        .into_iter()
        .find_map(|s| s.get("contents").and_then(Value::as_array))
    else {
        return r;
    };
    for node in contents {
        if let Some(card) = node.get("musicCardShelfRenderer") {
            // Top result: the primary match + its related rows.
            if let Some(main) = card_shelf_main(card) {
                r.top.push(main);
            }
            for c in card.get("contents").and_then(Value::as_array).into_iter().flatten() {
                if let Some(li) = c.get("musicResponsiveListItemRenderer") {
                    r.top.extend(list_item_to_browse_item(li));
                }
            }
        } else {
            // A flat result row (usually wrapped in an itemSectionRenderer) → bucket by its kind.
            for li in find_all(node, "musicResponsiveListItemRenderer") {
                bucket_item(li, &mut r);
            }
        }
    }
    r
}

/// Route a search row into its category bucket by the kind its navigation implies.
fn bucket_item(li: &Value, r: &mut SearchResults) {
    let Some(bi) = list_item_to_browse_item(li) else { return };
    match bi.kind {
        "album" => r.albums.push(bi),
        "artist" => r.artists.push(bi),
        "playlist" => r.playlists.push(bi),
        _ => r.songs.push(bi),
    }
}

/// Parse a filtered (album/artist/playlist) search into a flat card list — the "Show more" pages.
pub fn parse_search_cards(root: &Value) -> Vec<BrowseItem> {
    find_all(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(list_item_to_browse_item)
        .collect()
}

/// A search-result list row → a card. Album/artist/playlist rows navigate (item-level
/// `browseEndpoint`); a plain song row plays (`videoId`).
fn list_item_to_browse_item(node: &Value) -> Option<BrowseItem> {
    let title = flex_column_text(node, 0)?;
    let subtitle = flex_column_text(node, 1);
    let thumbnail = last_thumbnail(node);
    // Item-level browse target (not a subtitle artist link) classifies album/artist/playlist.
    if let Some(bid) = node
        .get("navigationEndpoint")
        .and_then(|n| n.get("browseEndpoint"))
        .and_then(|b| b.get("browseId"))
        .and_then(Value::as_str)
    {
        let (kind, id) = browse_target(bid);
        return Some(BrowseItem {
            kind,
            id,
            title,
            subtitle,
            thumbnail,
            duration: None,
            artist_runs: Vec::new(),
            play_count: None,
            is_video: false,
            is_upload: false,
            explicit: is_explicit(node),
        });
    }
    let vid = list_item_video_id(node)?;
    // A song card's subtitle doubles as its artist string once it's played (and scrobbled), so it
    // carries the artist alone, never the "Song • … • 3:02" descriptor YouTube puts on the row.
    let runs = flex_runs(node, 1);
    let subtitle = artists_from_runs(runs).or(subtitle);
    Some(BrowseItem {
        kind: "song",
        id: vid,
        title,
        subtitle,
        thumbnail,
        duration: duration_from_runs(runs),
        artist_runs: runs.map(|r| artist_runs(r)).unwrap_or_default(),
        play_count: play_count(node),
        is_video: is_video_row(node),
        is_upload: is_upload_row(node),
        explicit: is_explicit(node),
    })
}

/// The primary match of a top-result card shelf.
fn card_shelf_main(card: &Value) -> Option<BrowseItem> {
    let title = runs_text(card.get("title"))?;
    let subtitle = runs_text(card.get("subtitle"));
    let thumbnail = card.get("thumbnail").and_then(last_thumbnail);
    let nav = card
        .get("title")
        .and_then(|t| t.get("runs"))
        .and_then(Value::as_array)
        .and_then(|r| r.first())
        .and_then(|r0| r0.get("navigationEndpoint"))
        .or_else(|| card.get("onTap"))
        .or_else(|| card.get("navigationEndpoint"));
    if let Some(vid) = nav
        .and_then(|n| n.get("watchEndpoint"))
        .and_then(|w| w.get("videoId"))
        .and_then(Value::as_str)
    {
        // Same as a song row: the top-result card's subtitle becomes the artist when it plays.
        let runs = card.get("subtitle").and_then(|s| s.get("runs")).and_then(Value::as_array);
        let subtitle = artists_from_runs(runs).or(subtitle);
        return Some(BrowseItem {
            kind: "song",
            id: vid.to_owned(),
            title,
            subtitle,
            thumbnail,
            duration: duration_from_runs(runs),
            artist_runs: runs.map(|r| artist_runs(r)).unwrap_or_default(),
            play_count: None,
            is_video: nav.is_some_and(is_video_endpoint),
            is_upload: nav.is_some_and(is_upload_endpoint),
            explicit: is_explicit(card),
        });
    }
    let bid = nav
        .and_then(|n| n.get("browseEndpoint"))
        .and_then(|b| b.get("browseId"))
        .and_then(Value::as_str)?;
    let (kind, id) = browse_target(bid);
    Some(BrowseItem {
        kind,
        id,
        title,
        subtitle,
        thumbnail,
        duration: None,
        artist_runs: Vec::new(),
        play_count: None,
        is_video: false,
        is_upload: false,
        explicit: is_explicit(card),
    })
}

/// Parse an album (`MPRE…`) browse response. context/08.
pub fn parse_album(root: &Value) -> AlbumPage {
    let header = playlist_header(root);
    let title = header.and_then(|h| runs_text(h.get("title")));
    let subtitle = header.and_then(|h| runs_text(h.get("subtitle")));
    let second_subtitle = header.and_then(|h| runs_text(h.get("secondSubtitle")));

    // The artist link + avatar live in the header's "strapline".
    let strapline = header.and_then(|h| h.get("straplineTextOne"));
    let artist = strapline.and_then(runs_text_opt);
    let strapline_runs = strapline.and_then(|s| s.get("runs")).and_then(Value::as_array);
    let artist_id = strapline_runs.and_then(|r| first_artist_id(r));
    let runs = strapline_runs.map(|r| artist_runs(r)).unwrap_or_default();
    let artist_thumbnail =
        header.and_then(|h| h.get("straplineThumbnail")).and_then(last_thumbnail);

    // Target the header's own thumbnail subtree so we get the cover, not the artist avatar.
    let thumbnail = header.and_then(|h| h.get("thumbnail")).and_then(last_thumbnail);
    let description = header_description(root, header);

    // Album track rows carry no per-track thumbnail (every track shares the cover shown once in
    // the header), so parse_list_item leaves them None. Fill missing ones with the album cover so
    // the player bar + queue show it when a track plays.
    //
    // Same for the artist and the album name: on a single-artist album YouTube ships the artist
    // column empty (`"text": {}`) because the header already says it, so every track would arrive
    // with no artist at all, which is what the player bar shows and what Last.fm refuses to
    // scrobble. A compilation fills the column per row; keep those, they differ per track.
    let items = find_all(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(parse_list_item)
        .map(|mut it| {
            if it.thumbnail.is_none() {
                it.thumbnail = thumbnail.clone();
            }
            if it.artists.is_empty() {
                it.artists = artist.clone().unwrap_or_default();
                it.artist_id = it.artist_id.take().or_else(|| artist_id.clone());
                it.artist_runs = runs.clone();
            }
            if it.album.is_none() {
                it.album = title.clone();
            }
            it
        })
        .collect();

    AlbumPage {
        title,
        artist,
        artist_id,
        artist_runs: runs,
        artist_thumbnail,
        subtitle,
        second_subtitle,
        description,
        thumbnail,
        items,
        continuation: shelf_continuation(root),
        explicit: header.is_some_and(is_explicit),
        // Track rows carry the OLAK id; an album with no playable rows still has it on the
        // header's save-to-library button.
        playlist_id: album_playlist_id(root).or_else(|| library_toggle_playlist_id(header)),
        // Whole-tree, not the section list: an album page splits into two columns and the shelves
        // sit in the one the tracks don't.
        sections: find_all(root, "musicCarouselShelfRenderer")
            .into_iter()
            .filter_map(parse_artist_carousel)
            .collect(),
        in_library: library_toggle(header)
            .is_some_and(|t| t.get("isToggled").and_then(Value::as_bool).unwrap_or(false)),
    }
}

/// The header's save-to-library `toggleButtonRenderer` — the one whose action is a `likeEndpoint`
/// on the album's playlist (the header's other buttons are play/menu). Saving an album to the
/// library IS a like on its `OLAK5uy_` playlist; `isToggled` is the current state. Live-verified
/// 2026-07.
fn library_toggle(header: Option<&Value>) -> Option<&Value> {
    find_all(header?, "toggleButtonRenderer")
        .into_iter()
        .find(|t| !find_all(t, "likeEndpoint").is_empty())
}

fn library_toggle_playlist_id(header: Option<&Value>) -> Option<String> {
    find_first_str(library_toggle(header)?, "playlistId")
}

/// Per-track "this row links the music video, not the album audio" flags, aligned with
/// `parse_album().items` (same rows, same parse filter). A row is a video when its watch
/// endpoint's `musicVideoType` is present and is neither audio kind (a generated album track or
/// one of the user's own uploads) — absent means we can't tell, so we assume audio and leave it
/// alone.
pub fn album_video_flags(root: &Value) -> Vec<bool> {
    find_all(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter(|row| parse_list_item(row).is_some())
        .map(|row| find_first_str(row, "musicVideoType").is_some_and(|t| is_video_type(&t)))
        .collect()
}

/// The album's own audio playlist id (`OLAK5uy_…`), read from the track rows' watch endpoints.
/// Scoped to `musicResponsiveListItemRenderer` on purpose: the response also carries OTHER albums'
/// `OLAK5uy_` ids (the "more from artist" carousel play buttons, in `musicTwoRowItemRenderer`s),
/// so a whole-tree "first OLAK id" would be wrong. Live-verified 2026-07: no
/// `musicPlaylistShelfRenderer` exists anymore; every track row carries the id.
fn album_playlist_id(root: &Value) -> Option<String> {
    find_all(root, "musicResponsiveListItemRenderer").into_iter().find_map(|row| {
        find_all(row, "playlistId")
            .into_iter()
            .filter_map(Value::as_str)
            .find(|id| id.starts_with("OLAK5uy_"))
            .map(str::to_owned)
    })
}

/// The visibility of an owned playlist, off the header that wraps the edit form. Absent everywhere
/// else: someone else's playlist never says whether it is public.
fn playlist_privacy(root: &Value) -> Option<String> {
    find_all(root, "musicPlaylistEditHeaderRenderer")
        .into_iter()
        .find_map(|h| h.get("privacy").and_then(Value::as_str))
        .map(str::to_owned)
}

/// The blurb under an album's or playlist's title, wherever this response happens to keep it.
fn header_description(root: &Value, header: Option<&Value>) -> Option<String> {
    if let Some(d) = header.and_then(|h| runs_text(h.get("description"))) {
        return Some(d);
    }
    find_all(root, "musicDescriptionShelfRenderer")
        .into_iter()
        .find_map(|s| runs_text(s.get("description")))
}

/// Parse an artist (`UC…`) browse response. `browse_id` is used as the subscribe channelId
/// fallback (the artist browseId is itself the channelId). context/08.
pub fn parse_artist(root: &Value, browse_id: &str) -> ArtistPage {
    let header = find_all(root, "musicImmersiveHeaderRenderer")
        .into_iter()
        .next()
        .or_else(|| find_all(root, "musicHeaderRenderer").into_iter().next());

    let name = header.and_then(|h| runs_text(h.get("title")));
    let description = header.and_then(|h| runs_text(h.get("description")));
    // Target the header's own thumbnail subtree (avoids the subscribe-button avatar etc).
    let thumbnail = header
        .and_then(|h| h.get("thumbnail"))
        .and_then(last_thumbnail)
        .or_else(|| header.and_then(last_thumbnail));

    let sub = header.and_then(|h| find_all(h, "subscribeButtonRenderer").into_iter().next());
    let channel_id = sub
        .and_then(|s| s.get("channelId"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| browse_id.to_owned());
    let subscribed =
        sub.and_then(|s| s.get("subscribed")).and_then(Value::as_bool).unwrap_or(false);
    // Long form first: `subscriberCountText` is a bare "2.96M", `longSubscriberCountText` the
    // labelled "2.96M subscribers" (localized by the context's hl, so don't relabel it here).
    let subscribers = sub.and_then(|s| {
        text_or_runs(s.get("longSubscriberCountText"))
            .or_else(|| text_or_runs(s.get("subscriberCountText")))
    });
    let monthly_listeners = header.and_then(|h| text_or_runs(h.get("monthlyListenerCount")));

    // Walk the section list: the first list shelf = top songs; every carousel = a card row.
    let mut top_songs = Vec::new();
    let mut top_songs_id = None;
    let mut sections = Vec::new();
    if let Some(contents) = find_all(root, "sectionListRenderer")
        .into_iter()
        .find_map(|s| s.get("contents").and_then(Value::as_array))
    {
        for node in contents {
            if let Some(shelf) = node.get("musicShelfRenderer") {
                if top_songs.is_empty() {
                    top_songs = find_all(shelf, "musicResponsiveListItemRenderer")
                        .into_iter()
                        .filter_map(parse_list_item)
                        .collect();
                    // "Show all" (and the shelf title, same endpoint) points at a `VL…` playlist
                    // holding every top song. Prefix-checked: other shelves link a channel instead.
                    top_songs_id =
                        find_all(shelf.get("bottomEndpoint").unwrap_or(shelf), "browseId")
                            .into_iter()
                            .filter_map(Value::as_str)
                            .find(|id| id.starts_with("VL"))
                            .map(str::to_owned);
                }
            } else if let Some(carousel) = node.get("musicCarouselShelfRenderer") {
                if let Some(sec) = parse_artist_carousel(carousel) {
                    sections.push(sec);
                }
            }
        }
    }

    let radio_playlist_id = header.and_then(radio_playlist_id);

    ArtistPage {
        name,
        thumbnail,
        description,
        subscribers,
        monthly_listeners,
        channel_id,
        subscribed,
        radio_playlist_id,
        top_songs,
        top_songs_id,
        sections,
    }
}

/// The radio playlist behind a header or menu: the first watch endpoint pointing at an `RD…`
/// playlist. Metrolist reads two different spots for this (the immersive header's
/// `startRadioButton`, or a menu item whose icon is `MIX`); both are just a `watchEndpoint`
/// carrying a playlist id no other endpoint uses, so one prefix check covers them.
pub(crate) fn radio_playlist_id(node: &Value) -> Option<String> {
    find_all(node, "playlistId")
        .into_iter()
        .filter_map(Value::as_str)
        .find(|id| id.starts_with("RD"))
        .map(str::to_owned)
}

fn parse_artist_carousel(node: &Value) -> Option<ArtistCarousel> {
    let header = find_all(node, "musicCarouselShelfBasicHeaderRenderer").into_iter().next();
    let title = header.and_then(|h| runs_text(h.get("title"))).unwrap_or_default();
    let items: Vec<BrowseItem> = node
        .get("contents")
        .and_then(Value::as_array)
        .map(|c| c.iter().filter_map(parse_carousel_item).collect())
        .unwrap_or_default();
    if items.is_empty() {
        return None;
    }
    let more = header
        .and_then(|h| h.get("moreContentButton"))
        .and_then(|b| find_all(b, "browseEndpoint").into_iter().next());
    let more_browse_id =
        more.and_then(|e| e.get("browseId")).and_then(Value::as_str).map(str::to_owned);
    let more_params = more.and_then(|e| e.get("params")).and_then(Value::as_str).map(str::to_owned);
    Some(ArtistCarousel { title, items, more_browse_id, more_params })
}

/// Read a text field that may be `{ simpleText }` or `{ runs: [...] }`.
fn text_or_runs(v: Option<&Value>) -> Option<String> {
    let v = v?;
    v.get("simpleText").and_then(Value::as_str).map(str::to_owned).or_else(|| runs_text_opt(v))
}

/// True if a browse response is YouTube's logged-out "Sign in" empty state — which is what the
/// server returns when the cookie has gone stale (its `__Secure-*SIDTS` cookies rotate ~hourly).
/// The endpoints turn this into a clear "session expired" error instead of a silently-empty page.
pub(crate) fn is_signed_out(root: &Value) -> bool {
    !find_all(root, "signInEndpoint").is_empty()
}

// --- node parsers -------------------------------------------------------------------------

/// A carousel content node is either a two-row card or a track row.
fn parse_carousel_item(node: &Value) -> Option<BrowseItem> {
    if let Some(tr) = node.get("musicTwoRowItemRenderer") {
        return parse_two_row_item(tr);
    }
    if let Some(li) = node.get("musicResponsiveListItemRenderer") {
        let song = parse_list_item(li)?;
        return Some(BrowseItem {
            kind: "song",
            id: song.video_id,
            title: song.title,
            subtitle: Some(song.artists).filter(|s| !s.is_empty()),
            thumbnail: song.thumbnail,
            duration: song.duration,
            artist_runs: song.artist_runs,
            play_count: song.play_count,
            is_video: song.is_video,
            is_upload: song.is_upload,
            explicit: song.explicit,
        });
    }
    None
}

/// A `musicTwoRowItemRenderer` → one card. Kind inferred from its navigation endpoint.
fn parse_two_row_item(node: &Value) -> Option<BrowseItem> {
    let title = runs_text(node.get("title"))?;
    let subtitle = runs_text(node.get("subtitle"));
    let thumbnail = last_thumbnail(node);
    let nav = node.get("navigationEndpoint");

    // Song → watchEndpoint.videoId.
    if let Some(vid) = nav
        .and_then(|n| n.get("watchEndpoint"))
        .and_then(|w| w.get("videoId"))
        .and_then(Value::as_str)
    {
        // Same rule as a search row: a song card's subtitle becomes its artist once it plays (and
        // scrobbles), so keep the artist field alone, never the whole "Aqua • 1.7B views" or
        // "Miley Cyrus • Plastic Hearts • 2020" descriptor the card displays. Cards that navigate
        // keep the full subtitle below: there it is only ever text on screen.
        let runs = node.get("subtitle").and_then(|s| s.get("runs")).and_then(Value::as_array);
        let subtitle = artists_from_runs(runs).or(subtitle);
        return Some(BrowseItem {
            kind: "song",
            id: vid.to_owned(),
            title,
            subtitle,
            thumbnail,
            duration: duration_from_runs(runs),
            artist_runs: runs.map(|r| artist_runs(r)).unwrap_or_default(),
            play_count: None,
            is_video: is_video_row(node),
            is_upload: is_upload_row(node),
            explicit: is_explicit(node),
        });
    }
    // Playlist via watchPlaylistEndpoint (some carousels expose the raw playlistId).
    if let Some(pid) = nav
        .and_then(|n| n.get("watchPlaylistEndpoint"))
        .and_then(|w| w.get("playlistId"))
        .and_then(Value::as_str)
    {
        return Some(BrowseItem {
            kind: "playlist",
            id: format!("VL{pid}"),
            title,
            subtitle,
            thumbnail,
            duration: None,
            artist_runs: Vec::new(),
            play_count: None,
            is_video: false,
            is_upload: false,
            explicit: is_explicit(node),
        });
    }
    // Otherwise a browseEndpoint → playlist/album/artist by browseId prefix.
    let browse_id = nav
        .and_then(|n| n.get("browseEndpoint"))
        .and_then(|b| b.get("browseId"))
        .and_then(Value::as_str)?;
    let (kind, id) = browse_target(browse_id);
    Some(BrowseItem {
        kind,
        id,
        title,
        subtitle,
        thumbnail,
        duration: None,
        artist_runs: Vec::new(),
        play_count: None,
        is_video: false,
        is_upload: false,
        explicit: is_explicit(node),
    })
}

/// Classify a browseId: albums are `MPRE…`, artist/user channels are `UC…`, everything else
/// (`VL…`, `PL…`, `RD…`) is treated as a playlist. context/08.
fn browse_kind_from_id(id: &str) -> &'static str {
    if id.starts_with("MPRE") || id.starts_with("VLMPRE") {
        "album"
    } else if id.starts_with("UC") {
        "artist"
    } else {
        "playlist"
    }
}

/// A browseId as the UI should store it: kind plus the id to navigate with. Library-artist rows
/// link `MPLA` + the channelId (the "artist, filtered to your library" page); strip the prefix so
/// they open the normal artist page like every other artist card. context/08.
fn browse_target(id: &str) -> (&'static str, String) {
    let id = id.strip_prefix("MPLA").unwrap_or(id);
    (browse_kind_from_id(id), id.to_owned())
}

/// The playlist/album header node — recursion finds the detail renderer even when it's wrapped in
/// an editable-playlist header.
fn playlist_header(root: &Value) -> Option<&Value> {
    ["musicResponsiveHeaderRenderer", "musicDetailHeaderRenderer"]
        .into_iter()
        .find_map(|key| find_all(root, key).into_iter().next())
}

/// The shelf holding the playlist's/album's own tracks — the first one in the response, since the
/// suggestions section always comes after it.
fn track_shelf(root: &Value) -> Option<&Value> {
    find_all(root, "musicPlaylistShelfRenderer")
        .into_iter()
        .next()
        .or_else(|| find_all(root, "musicShelfRenderer").into_iter().next())
}

/// The track shelf's own paging token. Scoped on purpose: an owned playlist's page also carries a
/// "suggestions" section with a continuation of its own, and paging *that* appends recommended
/// songs the playlist doesn't contain (a 6-track playlist grew to 13). Falls back to nothing rather
/// than to a whole-tree sweep — a token from the wrong shelf is worse than no paging.
fn shelf_continuation(root: &Value) -> Option<String> {
    continuation_token(track_shelf(root)?)
}

/// Paging token, modern (`continuationCommand.token`) or legacy (`nextContinuationData`). context/08.
pub(crate) fn continuation_token(root: &Value) -> Option<String> {
    if let Some(t) = find_all(root, "continuationCommand")
        .into_iter()
        .find_map(|c| c.get("token").and_then(Value::as_str))
    {
        return Some(t.to_owned());
    }
    find_all(root, "nextContinuationData")
        .into_iter()
        .find_map(|c| c.get("continuation").and_then(Value::as_str))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // An artist radio id can't be built client-side (no `RD…` prefix trick) — it only exists in
    // the header's start-radio button, next to a pile of other playlist ids that aren't radios.
    #[test]
    fn parses_the_artist_radio_off_the_header() {
        let root = json!({
            "header": { "musicImmersiveHeaderRenderer": {
                "title": { "runs": [{ "text": "An Artist" }] },
                "playButton": { "buttonRenderer": {
                    "navigationEndpoint": { "watchEndpoint": { "playlistId": "OLAK5uy_notaradio" } }
                } },
                "startRadioButton": { "buttonRenderer": {
                    "navigationEndpoint": { "watchEndpoint": { "playlistId": "RDEMabc123" } }
                } }
            } }
        });
        assert_eq!(parse_artist(&root, "UCx").radio_playlist_id.as_deref(), Some("RDEMabc123"));
        // No radio button (some channels have none) → the caller falls back to a top-song radio.
        let bare = json!({ "header": { "musicImmersiveHeaderRenderer": {
            "title": { "runs": [{ "text": "An Artist" }] }
        } } });
        assert_eq!(parse_artist(&bare, "UCx").radio_playlist_id, None);
    }

    // History comes back as one `musicShelfRenderer` per day, not one shelf of rows: the day title
    // is the grouping, and a shelf with nothing playable in it must not become a bare heading.
    #[test]
    fn parses_history_day_buckets() {
        let row = |id: &str, title: &str| {
            json!({ "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": id },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{
                        "text": title
                    }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                        { "text": "An Artist" }, { "text": " \u{2022} " }, { "text": "3:12" }
                    ] } } }
                ]
            } })
        };
        let root = json!({ "contents": { "sectionListRenderer": { "contents": [
            { "musicShelfRenderer": {
                "title": { "runs": [{ "text": "Today" }] },
                "contents": [row("aaaaaaaaaaa", "First"), row("bbbbbbbbbbb", "Second")]
            } },
            { "musicShelfRenderer": {
                "title": { "runs": [{ "text": "Yesterday" }] },
                "contents": [row("ccccccccccc", "Third")]
            } },
            // Nothing playable in it (no videoId anywhere) — dropped rather than shown empty.
            { "musicShelfRenderer": {
                "title": { "runs": [{ "text": "Last week" }] },
                "contents": [json!({ "musicResponsiveListItemRenderer": {} })]
            } }
        ] } } });
        let groups = parse_history(&root);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title, "Today");
        assert_eq!(groups[0].items.len(), 2);
        assert_eq!(groups[0].items[0].video_id, "aaaaaaaaaaa");
        assert_eq!(groups[0].items[1].title, "Second");
        assert_eq!(groups[1].title, "Yesterday");
        assert_eq!(groups[1].items.len(), 1);
    }

    #[test]
    fn parses_home_carousel() {
        let root = json!({
            "header": { "chipCloudRenderer": { "chips": [
                { "chipCloudChipRenderer": {
                    "text": { "runs": [{ "text": "Workout" }] },
                    "navigationEndpoint": { "browseEndpoint": { "browseId": "FEmusic_home", "params": "ggNC0" } }
                } },
                // No browseEndpoint (e.g. the "clear filter" chip) → skipped.
                { "chipCloudChipRenderer": { "text": { "runs": [{ "text": "Nowhere" }] } } }
            ] } },
            "contents": { "sectionListRenderer": {
                "continuations": [{ "nextContinuationData": { "continuation": "HOME_MORE" } }],
                "contents": [
                { "musicCarouselShelfRenderer": {
                    "header": { "musicCarouselShelfBasicHeaderRenderer": {
                        "title": { "runs": [{ "text": "Mixed for you" }] },
                        "moreContentButton": { "buttonRenderer": { "navigationEndpoint": {
                            "browseEndpoint": { "browseId": "FEmusic_moods_and_genres_category", "params": "MOREPARAMS" }
                        } } }
                    } },
                    "contents": [
                        { "musicTwoRowItemRenderer": {
                            "title": { "runs": [{ "text": "My Mix", "navigationEndpoint": {} }] },
                            "subtitle": { "runs": [{ "text": "Playlist" }] },
                            "navigationEndpoint": { "browseEndpoint": { "browseId": "VLPL123" } },
                            "thumbnailRenderer": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                                { "url": "a.jpg" }, { "url": "b.jpg" }
                            ] } } }
                        } },
                        { "musicTwoRowItemRenderer": {
                            "title": { "runs": [{ "text": "Some Album" }] },
                            "subtitle": { "runs": [{ "text": "Album • Artist" }] },
                            "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_abc" } }
                        } }
                    ]
                } },
                { "musicCarouselShelfRenderer": {
                    "header": { "musicCarouselShelfBasicHeaderRenderer": {
                        "title": { "runs": [{ "text": "Recommended albums" }] }
                    } },
                    "contents": [
                        { "musicTwoRowItemRenderer": {
                            "title": { "runs": [{ "text": "Another Album" }] },
                            "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREc_xyz" } }
                        } }
                    ]
                } },
                // A list-style shelf ("Forgotten favourites", "Quick picks"): song rows, not cards.
                { "musicCarouselShelfRenderer": {
                    "header": { "musicCarouselShelfBasicHeaderRenderer": {
                        "title": { "runs": [{ "text": "Forgotten favourites" }] }
                    } },
                    "contents": [
                        { "musicResponsiveListItemRenderer": {
                            "playlistItemData": { "videoId": "vid123" },
                            "flexColumns": [
                                { "musicResponsiveListItemFlexColumnRenderer": {
                                    "text": { "runs": [{ "text": "Old Song" }] } } },
                                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                                    { "text": "The Artist" }, { "text": " • " },
                                    { "text": "The Album" }, { "text": " • " }, { "text": "3:47" }
                                ] } } }
                            ]
                        } }
                    ]
                } }
            ] } }
        });
        let home = parse_home(&root);
        assert_eq!(home.chips.len(), 1);
        assert_eq!(home.chips[0].title, "Workout");
        assert_eq!(home.chips[0].params, "ggNC0");
        assert_eq!(home.sections.len(), 3);
        let s = &home.sections[0];
        assert_eq!(s.title, "Mixed for you");
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.items[0].kind, "playlist");
        assert_eq!(s.items[0].id, "VLPL123");
        assert_eq!(s.items[0].title, "My Mix");
        assert_eq!(s.items[0].thumbnail.as_deref(), Some("b.jpg"));
        assert_eq!(s.items[1].kind, "album");
        assert_eq!(s.items[1].id, "MPREb_abc");
        assert_eq!(s.more_browse_id.as_deref(), Some("FEmusic_moods_and_genres_category"));
        assert_eq!(s.more_params.as_deref(), Some("MOREPARAMS"));
        let s2 = &home.sections[1];
        assert_eq!(s2.title, "Recommended albums");
        assert_eq!(s2.more_browse_id, None);
        assert_eq!(s2.more_params, None);
        // A song row keeps artist and duration apart: the row shows both, the queue gets the artist.
        let song = &home.sections[2].items[0];
        assert_eq!(song.kind, "song");
        assert_eq!(song.id, "vid123");
        assert_eq!(song.subtitle.as_deref(), Some("The Artist"));
        assert_eq!(song.duration.as_deref(), Some("3:47"));
        assert_eq!(home.continuation.as_deref(), Some("HOME_MORE"));
    }

    /// A song *card* (`musicTwoRowItemRenderer`, used by home shelves and the artist page's
    /// "Videos"/"Live performances" carousels) puts its whole display subtitle where the artist
    /// goes. Playing it scrobbled "Aqua • 1.7B views" as the artist. Cards that navigate keep the
    /// full subtitle: there it is only ever text under a cover.
    #[test]
    fn song_card_subtitle_is_the_artist_alone() {
        let card = |sub_runs: Value, nav: Value| {
            json!({ "musicCarouselShelfRenderer": {
                "header": { "musicCarouselShelfBasicHeaderRenderer": { "title": { "runs": [{ "text": "Videos" }] } } },
                "contents": [{ "musicTwoRowItemRenderer": {
                    "title": { "runs": [{ "text": "Barbie Girl" }] },
                    "subtitle": { "runs": sub_runs },
                    "navigationEndpoint": nav
                } }]
            } })
        };
        let song_nav = json!({ "watchEndpoint": { "videoId": "vidbarbie" } });
        let items = |root: &Value| parse_home(root).sections[0].items.clone();

        // "Artist • 1.7B views" (an artist page's Videos carousel).
        let root = card(
            json!([{ "text": "Aqua" }, { "text": " • " }, { "text": "1.7B views" }]),
            song_nav.clone(),
        );
        assert_eq!(items(&root)[0].subtitle.as_deref(), Some("Aqua"));

        // "Artist • Album • Year" (a home shelf card).
        let root = card(
            json!([
                { "text": "Miley Cyrus", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCmiley" } } },
                { "text": " • " }, { "text": "Plastic Hearts" }, { "text": " • " }, { "text": "2020" }
            ]),
            song_nav,
        );
        assert_eq!(items(&root)[0].subtitle.as_deref(), Some("Miley Cyrus"));

        // An album card is not a queue entry, so its subtitle stays whole.
        let root = card(
            json!([{ "text": "Album" }, { "text": " • " }, { "text": "Miley Cyrus" }]),
            json!({ "browseEndpoint": { "browseId": "MPREb_hearts" } }),
        );
        let card = &items(&root)[0];
        assert_eq!(card.kind, "album");
        assert_eq!(card.subtitle.as_deref(), Some("Album • Miley Cyrus"));
    }

    #[test]
    fn home_continuation_absent_when_no_token() {
        let root = json!({
            "contents": { "sectionListRenderer": { "contents": [
                { "musicCarouselShelfRenderer": {
                    "header": { "musicCarouselShelfBasicHeaderRenderer": {
                        "title": { "runs": [{ "text": "Mixed for you" }] }
                    } },
                    "contents": [
                        { "musicTwoRowItemRenderer": {
                            "title": { "runs": [{ "text": "My Mix" }] },
                            "navigationEndpoint": { "browseEndpoint": { "browseId": "VLPL123" } }
                        } }
                    ]
                } }
            ] } }
        });
        let home = parse_home(&root);
        assert_eq!(home.continuation, None);
    }

    #[test]
    fn parses_library_grid() {
        let root = json!({
            "gridRenderer": { "items": [
                { "musicTwoRowItemRenderer": {
                    "title": { "runs": [{ "text": "Chill" }] },
                    "subtitle": { "runs": [{ "text": "12 songs" }] },
                    "navigationEndpoint": { "browseEndpoint": { "browseId": "VLPLchill" } }
                } },
                { "musicTwoRowItemRenderer": {
                    "title": { "runs": [{ "text": "Focus" }] },
                    "navigationEndpoint": { "browseEndpoint": { "browseId": "VLPLfocus" } }
                } }
            ] }
        });
        let items = parse_library(&root);
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.kind == "playlist"));
        assert_eq!(items[0].id, "VLPLchill");
        assert_eq!(items[0].subtitle.as_deref(), Some("12 songs"));
    }

    /// Library artists come back as a shelf of list rows (no grid), and each links `MPLA` + the
    /// channelId — which has to become a plain `UC…` artist card.
    #[test]
    fn parses_library_artists_shelf() {
        let root = json!({
            "musicShelfRenderer": { "contents": [
                { "musicResponsiveListItemRenderer": {
                    "flexColumns": [
                        { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Yuki Kajiura" }] } } },
                        { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "181 songs" }] } } }
                    ],
                    "navigationEndpoint": { "browseEndpoint": { "browseId": "MPLAUCkajiura" } }
                } }
            ] }
        });
        let items = parse_library(&root);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "artist");
        assert_eq!(items[0].id, "UCkajiura");
        assert_eq!(items[0].subtitle.as_deref(), Some("181 songs"));
    }

    #[test]
    fn parses_playlist_page() {
        let root = json!({
            "header": { "musicResponsiveHeaderRenderer": {
                "title": { "runs": [{ "text": "Road Trip" }] },
                "secondSubtitle": { "runs": [{ "text": "2 songs • 7 min" }] },
                "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                    { "url": "cover.jpg" }
                ] } } }
            } },
            "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
                "sectionListRenderer": { "contents": [{ "musicPlaylistShelfRenderer": { "contents": [
                    { "musicResponsiveListItemRenderer": {
                        "playlistItemData": { "videoId": "vid1" },
                        "flexColumns": [
                            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Track One" }] } } },
                            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Artist X" }] } } }
                        ],
                        "fixedColumns": [
                            { "musicResponsiveListItemFixedColumnRenderer": { "text": { "runs": [{ "text": "3:47" }] } } }
                        ]
                    } },
                    { "musicResponsiveListItemRenderer": {
                        "playlistItemData": { "videoId": "vid2" },
                        "flexColumns": [
                            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Track Two" }] } } },
                            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Artist Y" }] } } }
                        ]
                    } }
                ], "continuations": [{ "nextContinuationData": { "continuation": "MORE_TOKEN" } }] } }] }
            } } }] } }
        });
        let p = parse_playlist(&root);
        assert_eq!(p.title.as_deref(), Some("Road Trip"));
        assert_eq!(p.subtitle.as_deref(), Some("2 songs • 7 min"));
        assert_eq!(p.thumbnail.as_deref(), Some("cover.jpg"));
        assert_eq!(p.items.len(), 2);
        assert_eq!(p.items[0].video_id, "vid1");
        // Length lives in the row's fixed column, not the subtitle.
        assert_eq!(p.items[0].duration.as_deref(), Some("3:47"));
        assert_eq!(p.items[1].title, "Track Two");
        assert_eq!(p.continuation.as_deref(), Some("MORE_TOKEN"));
        // A plain header (someone else's playlist) is not editable.
        assert!(!p.owned);
        assert!(!p.collaborative);
    }

    /// An uploaded album is a playlist as far as its browseId goes, and its rows carry no art of
    /// their own. Without the cover filled in, playing one leaves the player bar blank (#160).
    #[test]
    fn playlist_rows_with_no_art_of_their_own_inherit_the_cover() {
        let root = json!({
            "header": { "musicDetailHeaderRenderer": {
                "title": { "runs": [{ "text": "18 Months" }] },
                "straplineThumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                    { "url": "artist_avatar.jpg" }
                ] } } },
                "thumbnail": { "croppedSquareThumbnailRenderer": { "thumbnail": { "thumbnails": [
                    { "url": "cover.jpg" }
                ] } } }
            } },
            "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
                "sectionListRenderer": { "contents": [{ "musicShelfRenderer": { "contents": [
                    { "musicResponsiveListItemRenderer": {
                        "playlistItemData": { "videoId": "up1" },
                        "flexColumns": [
                            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Feel So Close" }] } } }
                        ]
                    } },
                    { "musicResponsiveListItemRenderer": {
                        "playlistItemData": { "videoId": "up2" },
                        "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                            { "url": "row.jpg" }
                        ] } } },
                        "flexColumns": [
                            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Bounce" }] } } }
                        ]
                    } }
                ] } }] }
            } } }] } }
        });
        let p = parse_playlist(&root);
        assert_eq!(p.items[0].thumbnail.as_deref(), Some("cover.jpg"), "no art of its own");
        assert_eq!(p.items[1].thumbnail.as_deref(), Some("row.jpg"), "its own art wins");

        // Drop the cover and the avatar is the only thumbnail left in the header. A row keeps
        // nothing rather than inheriting a face.
        let mut no_cover = root.clone();
        no_cover["header"]["musicDetailHeaderRenderer"]
            .as_object_mut()
            .unwrap()
            .remove("thumbnail");
        let p = parse_playlist(&no_cover);
        assert_eq!(p.items[0].thumbnail, None, "never the artist avatar");
    }

    /// Every playlist header has a facepile (the owner's avatar). Only a collaborative one makes it
    /// the button for the Collaborate panel, and that is the one signal for the badge: a
    /// collaborative playlist you own carries no editable header, so `owned` cannot stand in.
    #[test]
    fn a_facepile_that_opens_the_collaborate_panel_marks_the_playlist_collaborative() {
        let page = |facepile: serde_json::Value| {
            json!({ "contents": { "sectionListRenderer": { "contents": [
                { "musicResponsiveHeaderRenderer": { "facepile": facepile } }
            ] } } })
        };
        let owner_only =
            json!({ "avatarStackViewModel": { "avatars": [{ "avatarViewModel": {} }] } });
        let collab = json!({ "avatarStackViewModel": { "rendererContext": { "commandContext": {
            "onTap": { "innertubeCommand": { "showEngagementPanelEndpoint": {
                "identifier": { "tag": "PAplaylist_collaborate" } } } } } } } });
        assert!(!parse_playlist(&page(owner_only)).collaborative);
        assert!(parse_playlist(&page(collab)).collaborative);
    }

    /// An owned playlist's page carries a suggestions section ("add more to this playlist") with a
    /// continuation of its own. Paging that appended recommended songs to the track list — a
    /// 6-track playlist showed 13. Only the track shelf's own token counts.
    #[test]
    fn short_playlist_ignores_the_suggestion_shelf_token() {
        let root = json!({
            "header": { "musicResponsiveHeaderRenderer": {
                "title": { "runs": [{ "text": "Vibesss" }] }
            } },
            "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
                "sectionListRenderer": { "contents": [
                    // The tracks: no continuation, the playlist fits on one page.
                    { "musicPlaylistShelfRenderer": { "contents": [
                        { "musicResponsiveListItemRenderer": {
                            "playlistItemData": { "videoId": "mine1" },
                            "flexColumns": [
                                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "End Of The Road" }] } } }
                            ]
                        } }
                    ] } },
                    // "Suggestions" shelf — its token must never be treated as more tracks.
                    { "musicShelfRenderer": {
                        "title": { "runs": [{ "text": "Suggestions" }] },
                        "contents": [],
                        "continuations": [{ "nextContinuationData": { "continuation": "SUGGEST_TOKEN" } }]
                    } }
                ] }
            } } }] } }
        });
        let p = parse_playlist(&root);
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.continuation, None, "the suggestions token is not a track continuation");
    }

    /// What the "Edit playlist" dialog prefills from. Both fields have to survive the round trip:
    /// a description read back as `None` is one the dialog would offer to overwrite with nothing.
    #[test]
    fn an_owned_playlist_reports_its_description_and_privacy() {
        let root = json!({
            "header": { "musicEditablePlaylistDetailHeaderRenderer": {
                "header": { "musicResponsiveHeaderRenderer": {
                    "title": { "runs": [{ "text": "Late night" }] },
                    "description": { "runs": [{ "text": "for the drive home" }] }
                } },
                "editHeader": { "musicPlaylistEditHeaderRenderer": {
                    "title": { "runs": [{ "text": "Late night" }] },
                    "privacy": "PUBLIC"
                } }
            } }
        });
        let p = parse_playlist(&root);
        assert!(p.owned);
        assert_eq!(p.description.as_deref(), Some("for the drive home"));
        assert_eq!(p.privacy.as_deref(), Some("PUBLIC"));

        // Someone else's playlist has no edit header, so there is no visibility to report.
        let theirs = json!({
            "header": { "musicResponsiveHeaderRenderer": {
                "title": { "runs": [{ "text": "Late night" }] }
            } }
        });
        assert_eq!(parse_playlist(&theirs).privacy, None);
    }

    #[test]
    fn continuation_ignores_nested_edit_renderer() {
        // An owned/editable playlist's continuation embeds, inside each track row, a NESTED copy of
        // the same `musicResponsiveListItemRenderer` (an add-suggestion edit command). A deep sweep
        // would count every track twice — the real "load more" duplication bug.
        let editable_row = |vid: &str, title: &str| {
            json!({ "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": vid },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": title }] } } }
                ],
                // The edit affordance carries a nested duplicate renderer with the SAME videoId.
                "fixedColumns": [{ "musicResponsiveListItemFixedColumnRenderer": { "button": {
                    "buttonRenderer": { "command": { "playlistEditEndpoint": { "clientActions": [
                        { "musicAddSuggestionToPlaylistCommand": { "addToPlaylistCommand": {
                            "insertShelfItemCommand": { "item": {
                                "musicResponsiveListItemRenderer": { "playlistItemData": { "videoId": vid } }
                            } }
                        } } }
                    ] } } }
                } } }]
            } })
        };
        let root = json!({ "continuationContents": { "sectionListContinuation": {
            "contents": [{ "musicShelfRenderer": {
                "contents": [editable_row("a1", "Alpha"), editable_row("b2", "Beta")],
                "continuations": [{ "nextContinuationData": { "continuation": "NEXT" } }]
            } }]
        } } });
        let c = parse_playlist_continuation(&root);
        assert_eq!(c.items.len(), 2, "each track must appear once, not twice");
        assert_eq!(c.items[0].video_id, "a1");
        assert_eq!(c.items[1].video_id, "b2");
        assert_eq!(c.continuation.as_deref(), Some("NEXT"));
    }

    #[test]
    fn detects_owned_playlist() {
        // YouTube wraps an owned playlist's header in `musicEditablePlaylistDetailHeaderRenderer`.
        let root = json!({
            "header": { "musicEditablePlaylistDetailHeaderRenderer": {
                "header": { "musicResponsiveHeaderRenderer": {
                    "title": { "runs": [{ "text": "My Playlist" }] }
                } }
            } }
        });
        let p = parse_playlist(&root);
        assert_eq!(p.title.as_deref(), Some("My Playlist"));
        assert!(p.owned);
    }

    #[test]
    fn detects_signed_out_state() {
        let signed_out = json!({
            "contents": { "sectionListRenderer": { "contents": [{ "itemSectionRenderer": { "contents": [
                { "messageRenderer": {
                    "text": { "runs": [{ "text": "Looking for what you've liked?" }] },
                    "button": { "buttonRenderer": { "navigationEndpoint": { "signInEndpoint": { "hack": true } } } }
                } }
            ] } }] } }
        });
        assert!(is_signed_out(&signed_out));
        // A normal playlist response has no signInEndpoint.
        let ok = json!({ "contents": { "musicPlaylistShelfRenderer": { "contents": [] } } });
        assert!(!is_signed_out(&ok));
    }

    #[test]
    fn parses_search_all_sections() {
        // Helper to build a search list row.
        let song_row = json!({ "musicResponsiveListItemRenderer": {
            "playlistItemData": { "videoId": "svid" },
            "flexColumns": [
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "A Song" }] } } },
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                    { "text": "Song" },
                    { "text": " • " },
                    { "text": "An Artist", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCart" } } },
                    { "text": " & " },
                    { "text": "Another", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCtwo" } } },
                    { "text": " • " },
                    { "text": "3:02" }
                ] } } }
            ]
        } });
        let album_row = json!({ "musicResponsiveListItemRenderer": {
            "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREalb" } },
            "flexColumns": [
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "An Album" }] } } },
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "2026" }] } } }
            ]
        } });
        let artist_row = json!({ "musicResponsiveListItemRenderer": {
            "navigationEndpoint": { "browseEndpoint": { "browseId": "UCart" } },
            "flexColumns": [
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "The Artist" }] } } }
            ]
        } });
        // The real (WEB_REMIX) shape: a top-result card + flat itemSectionRenderer rows.
        let root = json!({ "contents": { "tabbedSearchResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
            "sectionListRenderer": { "contents": [
                { "musicCardShelfRenderer": {
                    "title": { "runs": [{ "text": "The Artist", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCart" } } }] },
                    "subtitle": { "runs": [{ "text": "Artist" }] },
                    "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "top.jpg" }] } } },
                    "contents": []
                } },
                { "itemSectionRenderer": { "contents": [song_row] } },
                { "itemSectionRenderer": { "contents": [album_row] } },
                { "itemSectionRenderer": { "contents": [artist_row] } }
            ] }
        } } }] } } });

        let r = parse_search_all(&root);
        assert_eq!(r.top.len(), 1);
        assert_eq!(r.top[0].kind, "artist");
        assert_eq!(r.top[0].id, "UCart");
        assert_eq!(r.songs.len(), 1);
        assert_eq!(r.songs[0].kind, "song");
        assert_eq!(r.songs[0].id, "svid");
        // A song card keeps its artist links: played from search, the player bar navigates.
        assert_eq!(
            r.songs[0]
                .artist_runs
                .iter()
                .map(|x| (x.text.as_str(), x.id.as_deref()))
                .collect::<Vec<_>>(),
            [("An Artist", Some("UCart")), (" & ", None), ("Another", Some("UCtwo"))]
        );
        // A card that navigates carries none — its subtitle is a descriptor, not an artist line.
        assert!(r.albums.is_empty() || r.albums[0].artist_runs.is_empty());
        assert_eq!(r.albums.len(), 1);
        assert_eq!(r.albums[0].kind, "album");
        assert_eq!(r.albums[0].id, "MPREalb");
        assert_eq!(r.artists.len(), 1);
        assert_eq!(r.artists[0].kind, "artist");
        assert_eq!(r.artists[0].title, "The Artist");
    }

    #[test]
    fn parses_album_page() {
        let root = json!({
            "header": { "musicResponsiveHeaderRenderer": {
                "title": { "runs": [{ "text": "ICEMAN" }] },
                "subtitle": { "runs": [{ "text": "Album • 2026" }] },
                "secondSubtitle": { "runs": [{ "text": "18 songs • 1 hour, 8 minutes" }] },
                "straplineTextOne": { "runs": [
                    { "text": "Drake", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdrake" } } },
                    { "text": " & " },
                    { "text": "Metro", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCmetro" } } }
                ] },
                "straplineThumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                    { "url": "artist_avatar.jpg" }
                ] } } },
                "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                    { "url": "cover_small.jpg" }, { "url": "cover_big.jpg" }
                ] } } },
                "description": { "runs": [{ "text": "Iceman is one of three studio albums." }] }
            } },
            "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
                "sectionListRenderer": { "contents": [
                    { "musicShelfRenderer": { "contents": [
                        { "musicResponsiveListItemRenderer": {
                            "playlistItemData": { "videoId": "trk1" },
                            "flexColumns": [
                                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{
                                    "text": "Make Them Cry",
                                    "navigationEndpoint": { "watchEndpoint": { "videoId": "trk1", "playlistId": "OLAK5uy_iceman" } }
                                }] } } },
                                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Drake" }] } } }
                            ]
                        } }
                    ] } },
                    // "More from artist" carousel: a DIFFERENT album's OLAK id that must not win.
                    { "musicCarouselShelfRenderer": {
                      "header": { "musicCarouselShelfBasicHeaderRenderer": {
                          "title": { "runs": [{ "text": "Other versions" }] }
                      } },
                      "contents": [
                        { "musicTwoRowItemRenderer": {
                            "title": { "runs": [{ "text": "Other Album" }] },
                            "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREother" } },
                            "thumbnailOverlay": { "musicItemThumbnailOverlayRenderer": { "content": {
                                "musicPlayButtonRenderer": { "playNavigationEndpoint": {
                                    "watchPlaylistEndpoint": { "playlistId": "OLAK5uy_other" }
                                } }
                            } } }
                        } }
                    ] } }
                ] }
            } } }] } }
        });
        let a = parse_album(&root);
        assert_eq!(a.title.as_deref(), Some("ICEMAN"));
        assert_eq!(a.subtitle.as_deref(), Some("Album • 2026"));
        assert_eq!(a.second_subtitle.as_deref(), Some("18 songs • 1 hour, 8 minutes"));
        assert_eq!(a.artist.as_deref(), Some("Drake & Metro"));
        assert_eq!(a.artist_id.as_deref(), Some("UCdrake"));
        // A collab links each artist to its own page, separator run kept unlinked.
        assert_eq!(
            a.artist_runs.iter().map(|r| (r.text.as_str(), r.id.as_deref())).collect::<Vec<_>>(),
            vec![("Drake", Some("UCdrake")), (" & ", None), ("Metro", Some("UCmetro"))]
        );
        assert_eq!(a.artist_thumbnail.as_deref(), Some("artist_avatar.jpg"));
        assert_eq!(a.thumbnail.as_deref(), Some("cover_big.jpg"));
        assert_eq!(a.description.as_deref(), Some("Iceman is one of three studio albums."));
        assert_eq!(a.items.len(), 1);
        assert_eq!(a.items[0].video_id, "trk1");
        // Track row has no thumbnail of its own → falls back to the album cover (for the player bar).
        assert_eq!(a.items[0].thumbnail.as_deref(), Some("cover_big.jpg"));
        // The album's own OLAK id from the track rows — never the carousel's other-album id.
        assert_eq!(a.playlist_id.as_deref(), Some("OLAK5uy_iceman"));
        assert!(!a.in_library); // no save button in this fixture

        // The carousel becomes a shelf under the tracks; its card stays out of the track list.
        assert_eq!(a.sections.len(), 1);
        assert_eq!(a.sections[0].title, "Other versions");
        assert_eq!(a.sections[0].items.len(), 1);
        assert_eq!(a.sections[0].items[0].title, "Other Album");
    }

    /// On a single-artist album YouTube ships the per-track artist column *empty* (`"text": {}`,
    /// live-verified 2026-08 on Rumours / SOS / Nevermind / IGOR / Midnights / RENAISSANCE), because
    /// the header already names the artist. Left as-is those tracks play with no artist at all: the
    /// player bar shows a bare title and Last.fm drops the scrobble. A compilation fills the column
    /// per row, and those differ from the header, so they must survive untouched.
    #[test]
    fn album_tracks_inherit_the_header_artist_only_when_they_have_none() {
        let row = |id: &str, title: &str, artist: Option<&str>| {
            let col = match artist {
                Some(a) => json!({ "text": { "runs": [{ "text": a }] } }),
                None => json!({ "text": {} }), // YouTube's empty artist column, verbatim
            };
            json!({ "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": id },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": title }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": col },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "11K plays" }] } } }
                ]
            } })
        };
        let root = json!({
            "header": { "musicResponsiveHeaderRenderer": {
                "title": { "runs": [{ "text": "Sjelen" }] },
                "straplineTextOne": { "runs": [
                    { "text": "Delara", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdelara" } } },
                    { "text": " & " },
                    { "text": "Guest", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCguest" } } }
                ] },
                "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "cover.jpg" }] } } }
            } },
            "contents": { "musicShelfRenderer": { "contents": [
                row("t1", "Hele uka", None),
                row("t2", "Feature Track", Some("Someone Else"))
            ] } }
        });

        let a = parse_album(&root);
        assert_eq!(a.items.len(), 2);
        // Empty column → the header's artist, links and all, so the row behaves like a search row.
        assert_eq!(a.items[0].artists, "Delara & Guest");
        assert_eq!(a.items[0].artist_id.as_deref(), Some("UCdelara"));
        assert_eq!(
            a.items[0].artist_runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
            vec!["Delara", " & ", "Guest"]
        );
        // A row that names its own artist keeps it (compilations, features).
        assert_eq!(a.items[1].artists, "Someone Else");
        assert!(
            a.items[1].artist_runs.is_empty(),
            "an unlinked row artist must not borrow the header's links"
        );
        // Every track on an album page is on *this* album; Last.fm takes the album too.
        assert_eq!(a.items[0].album.as_deref(), Some("Sjelen"));
        assert_eq!(a.items[1].album.as_deref(), Some("Sjelen"));
    }

    /// The header's save-to-library toggle: `isToggled` is the state, and its like target carries
    /// the OLAK id even when no track row does (an album with nothing playable).
    #[test]
    fn reads_album_library_toggle() {
        let header = |toggled: bool| {
            json!({ "header": { "musicResponsiveHeaderRenderer": {
                "title": { "runs": [{ "text": "Saved" }] },
                "buttons": [{ "toggleButtonRenderer": {
                    "isToggled": toggled,
                    "defaultServiceEndpoint": { "likeEndpoint": {
                        "status": "LIKE", "target": { "playlistId": "OLAK5uy_saved" }
                    } }
                } }]
            } } })
        };
        let a = parse_album(&header(true));
        assert!(a.in_library);
        assert_eq!(a.playlist_id.as_deref(), Some("OLAK5uy_saved"));
        assert!(!parse_album(&header(false)).in_library);
    }

    #[test]
    fn album_video_flags_align_with_parsed_items() {
        let row = |id: &str, title: &str, vtype: Option<&str>| {
            let mut nav = json!({ "watchEndpoint": { "videoId": id } });
            if let Some(t) = vtype {
                nav["watchEndpoint"]["watchEndpointMusicSupportedConfigs"] =
                    json!({ "watchEndpointMusicConfig": { "musicVideoType": t } });
            }
            json!({ "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": id },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{
                        "text": title, "navigationEndpoint": nav
                    }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Drake" }] } } }
                ]
            } })
        };
        let root = json!({ "contents": [
            row("t1", "Audio Track", Some("MUSIC_VIDEO_TYPE_ATV")),
            row("t2", "MV Track", Some("MUSIC_VIDEO_TYPE_OMV")),
            // Unparseable row (no title) — dropped by parse_album, so flags must skip it too.
            row("t3", "", Some("MUSIC_VIDEO_TYPE_OMV")),
            row("t4", "Untyped Track", None),
        ] });
        assert_eq!(album_video_flags(&root), [false, true, false]);
        // Alignment invariant: one flag per parsed item.
        assert_eq!(parse_album(&root).items.len(), album_video_flags(&root).len());
    }

    #[test]
    fn parses_artist_page() {
        let root = json!({
            "header": { "musicImmersiveHeaderRenderer": {
                "title": { "runs": [{ "text": "Drake" }] },
                "description": { "runs": [{ "text": "Aubrey Drake Graham is a Canadian rapper." }] },
                "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                    { "url": "small.jpg" }, { "url": "hero.jpg" }
                ] } } },
                "subscriptionButton": { "subscribeButtonRenderer": {
                    "channelId": "UCdrake",
                    "subscribed": true,
                    "subscriberCountText": { "runs": [{ "text": "32.7M" }] },
                    "longSubscriberCountText": { "runs": [{ "text": "32.7M subscribers" }] }
                } },
                "monthlyListenerCount": { "runs": [{ "text": "137M monthly audience" }] }
            } },
            "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
                "sectionListRenderer": { "contents": [
                    { "musicShelfRenderer": { "title": { "runs": [{ "text": "Songs" }] }, "contents": [
                        { "musicResponsiveListItemRenderer": {
                            "playlistItemData": { "videoId": "song1" },
                            "flexColumns": [
                                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "God's Plan" }] } } },
                                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Drake" }] } } }
                            ]
                        } }
                    ],
                    "bottomEndpoint": { "browseEndpoint": { "browseId": "VLOLAK5uy_drake" } } } },
                    { "musicCarouselShelfRenderer": {
                        "header": { "musicCarouselShelfBasicHeaderRenderer": {
                            "title": { "runs": [{ "text": "Albums" }] },
                            "moreContentButton": { "buttonRenderer": { "navigationEndpoint": {
                                "browseEndpoint": { "browseId": "UCdrake", "params": "ALBUMS_PARAMS" }
                            } } }
                        } },
                        "contents": [
                            { "musicTwoRowItemRenderer": {
                                "title": { "runs": [{ "text": "ICEMAN" }] },
                                "subtitle": { "runs": [{ "text": "Album • 2026" }] },
                                "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREiceman" } }
                            } }
                        ]
                    } }
                ] }
            } } }] } }
        });
        let a = parse_artist(&root, "UCdrake");
        assert_eq!(a.name.as_deref(), Some("Drake"));
        assert_eq!(a.thumbnail.as_deref(), Some("hero.jpg"));
        assert_eq!(a.channel_id, "UCdrake");
        assert!(a.subscribed);
        assert_eq!(a.subscribers.as_deref(), Some("32.7M subscribers"));
        assert_eq!(a.monthly_listeners.as_deref(), Some("137M monthly audience"));
        assert_eq!(a.top_songs.len(), 1);
        assert_eq!(a.top_songs[0].video_id, "song1");
        assert_eq!(a.top_songs_id.as_deref(), Some("VLOLAK5uy_drake"));
        assert_eq!(a.sections.len(), 1);
        assert_eq!(a.sections[0].title, "Albums");
        assert_eq!(a.sections[0].items[0].kind, "album");
        assert_eq!(a.sections[0].more_browse_id.as_deref(), Some("UCdrake"));
        assert_eq!(a.sections[0].more_params.as_deref(), Some("ALBUMS_PARAMS"));
    }

    #[test]
    fn continuation_prefers_modern_token() {
        let root = json!({
            "continuationItemRenderer": { "continuationEndpoint": {
                "continuationCommand": { "token": "MODERN" }
            } }
        });
        assert_eq!(continuation_token(&root).as_deref(), Some("MODERN"));
    }

    /// The exact params YouTube's own menu hands out, so a typo in the table cannot pass. Only the
    /// ascending side of title/artist/album is ever sent by YouTube; the descending ones are the
    /// same message with the order byte flipped, which the server accepts (verified live).
    #[test]
    fn sort_params_match_the_menu_youtube_sends() {
        use PlaylistSort::*;
        assert_eq!(Default.params(false), "2ggA");
        assert_eq!(Default.params(true), "2ggA"); // no reversed manual order exists server-side
        assert_eq!(Newest.params(false), "2ggECAIQAw==");
        assert_eq!(Oldest.params(false), "2ggECAEQAw==");
        assert_eq!(Top.params(false), "2ggECAIQBA==");
        assert_eq!(Title.params(false), "2ggECAEQBQ==");
        assert_eq!(Artist.params(false), "2ggECAEQBg==");
        assert_eq!(Album.params(false), "2ggECAEQBw==");
        // Date is already directional, so reversing one date sort has to land on the other.
        assert_eq!(Newest.params(true), Oldest.params(false));
        assert_eq!(Oldest.params(true), Newest.params(false));
        // Everything else keeps its kind and only flips the order byte.
        for s in [Title, Artist, Album, Top] {
            assert_ne!(s.params(true), s.params(false), "{s:?} must have a reversed form");
            assert_eq!(PlaylistSort::from_params(s.params(true)), Some(s));
        }
    }

    /// 4/5/6 answer HTTP 500 *and* wedge the playlist into an order its own menu then reports as
    /// manual, so `playlistDynamicSortPreference` must never leave 1..3.
    #[test]
    fn edit_actions_stay_inside_the_enums_youtube_accepts() {
        for s in PlaylistSort::ALL {
            let a = s.edit_action();
            if let Some(n) = a.get("playlistDynamicSortPreference").and_then(Value::as_i64) {
                assert!((1..=3).contains(&n), "{s:?} would 500 with pref {n}");
            } else {
                let n = a.get("playlistVideoOrder").and_then(Value::as_i64).unwrap();
                assert!([0, 1, 2, 6].contains(&n), "{s:?} sends an unknown video order {n}");
            }
            assert_eq!(PlaylistSort::from_edit_action(&a), Some(s));
        }
    }

    fn shelf(menu: Value) -> Value {
        json!({ "musicPlaylistShelfRenderer": { "contents": [], "header": {
            "musicSideAlignedItemRenderer": { "startItems": [{ "sortFilterSubMenuRenderer": menu }] }
        } } })
    }

    /// A playlist you own: the options are writes, so the choice carries to every other client.
    #[test]
    fn reads_the_selected_sort_off_an_owned_playlists_menu() {
        let root = shelf(json!({ "subMenuItems": [
            { "title": "Manual ordering", "selected": false, "serviceEndpoint": {
                "playlistEditEndpoint": { "playlistId": "PLx", "actions": [
                    { "action": "ACTION_SET_PLAYLIST_VIDEO_ORDER", "playlistVideoOrder": 0 }
                ] } } },
            { "title": "Artist", "selected": true, "serviceEndpoint": {
                "playlistEditEndpoint": { "playlistId": "PLx", "actions": [
                    { "action": "ACTION_SET_PLAYLIST_DYNAMIC_SORT_PREFERENCE",
                      "playlistDynamicSortPreference": 2 }
                ] } } }
        ] }));
        let m = sort_menu(&root).expect("menu");
        assert_eq!(m.selected, Some(PlaylistSort::Artist));
        assert!(m.editable);
    }

    /// Liked Music and other people's playlists: view-only endpoints, and the params come back
    /// percent-encoded, which the plain table lookup would miss.
    #[test]
    fn reads_the_selected_sort_off_a_view_only_menu() {
        let root = shelf(json!({ "subMenuItems": [
            { "title": "Default ordering", "selected": false, "serviceEndpoint": {
                "browseEndpoint": { "browseId": "VLLM", "params": "2ggA" } } },
            { "title": "Title", "selected": true, "serviceEndpoint": {
                "browseEndpoint": { "browseId": "VLLM", "params": "2ggECAEQBQ%3D%3D" } } }
        ] }));
        let m = sort_menu(&root).expect("menu");
        assert_eq!(m.selected, Some(PlaylistSort::Title));
        assert!(!m.editable, "a browseEndpoint menu cannot be written back");
    }

    /// Albums and YouTube's radio mixes carry no menu at all — the UI falls back to sorting the
    /// rows it has rather than asking for an order the server will not give.
    #[test]
    fn no_menu_on_a_list_youtube_will_not_reorder() {
        let root = json!({ "musicPlaylistShelfRenderer": { "contents": [] } });
        assert!(sort_menu(&root).is_none());
        assert!(parse_playlist(&root).sort_menu.is_none());
    }
}
