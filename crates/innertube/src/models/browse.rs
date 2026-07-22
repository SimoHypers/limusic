//! Browse-surface parsing: home feed, library playlists, playlist/album pages. context/08.
//!
//! Same tolerant walk-the-tree approach as `metadata.rs` (reuses its helpers): we locate the few
//! renderer node types we care about anywhere in the response and pull only the fields the UI
//! needs — robust to YouTube reshuffling the surrounding container tree.
//! - `musicCarouselShelfRenderer` → a home section (a titled row of cards).
//! - `musicTwoRowItemRenderer`     → a card (playlist / album / artist / song).
//! - `musicResponsiveListItemRenderer` → a track row (shared with search; via `parse_list_item`).

use serde::Serialize;
use serde_json::Value;

use super::metadata::{
    find_all, find_all_shallow, first_artist_id, flex_column_text, last_thumbnail,
    list_item_video_id, parse_list_item, runs_text, runs_text_opt, SongItem,
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

/// A playlist or album detail page: header + tracks + a paging token.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistPage {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub thumbnail: Option<String>,
    pub items: Vec<SongItem>,
    pub continuation: Option<String>,
    /// True only when the signed-in user owns this playlist (rename/delete allowed). YouTube wraps
    /// the header in `musicEditablePlaylistDetailHeaderRenderer` exactly for owned playlists.
    pub owned: bool,
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
    /// e.g. "137M monthly listeners" / "32.7M subscribers".
    pub subscribers: Option<String>,
    /// Subscribe target — the channelId (falls back to the browseId, which is the same `UC…`).
    pub channel_id: String,
    pub subscribed: bool,
    /// Top songs shelf (usually 5).
    pub top_songs: Vec<SongItem>,
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
    /// The album's audio playlist id (`OLAK5uy_…`) — the radio seed for autoplay continuation.
    pub playlist_id: Option<String>,
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
            let more_browse_id = more.and_then(|e| e.get("browseId")).and_then(Value::as_str).map(str::to_owned);
            let more_params = more.and_then(|e| e.get("params")).and_then(Value::as_str).map(str::to_owned);
            sections.push(Section { title, items, more_browse_id, more_params });
        }
    }
    HomePage { chips, sections, continuation: continuation_token(root) }
}

/// Parse a `FEmusic_liked_playlists` response into a flat grid of playlist cards. context/08.
pub fn parse_library(root: &Value) -> Vec<BrowseItem> {
    find_all(root, "musicTwoRowItemRenderer").into_iter().filter_map(parse_two_row_item).collect()
}

/// Parse a playlist/album (`VL…` / `MPRE…`) browse response. context/08.
pub fn parse_playlist(root: &Value) -> PlaylistPage {
    let header = playlist_header(root);
    let title = header.and_then(|h| runs_text(h.get("title")));
    // `secondSubtitle` usually carries "N songs • Xh Ym"; fall back to `subtitle`.
    let subtitle =
        header.and_then(|h| runs_text(h.get("secondSubtitle")).or_else(|| runs_text(h.get("subtitle"))));
    let thumbnail = header.and_then(last_thumbnail);
    let items = find_all_shallow(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(parse_list_item)
        .collect();
    // Present only for playlists the signed-in user owns — the sole reliable ownership signal.
    let owned = !find_all(root, "musicEditablePlaylistDetailHeaderRenderer").is_empty();
    PlaylistPage { title, subtitle, thumbnail, items, continuation: continuation_token(root), owned }
}

/// Parse a browse continuation response (more playlist tracks). context/08.
pub fn parse_playlist_continuation(root: &Value) -> PlaylistContinuation {
    // Shallow find: on an owned/editable playlist each track row embeds a nested copy of its own
    // renderer (an add-suggestion edit command), so a deep find_all would return every track twice.
    let items = find_all_shallow(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(parse_list_item)
        .collect();
    PlaylistContinuation { items, continuation: continuation_token(root) }
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
        return Some(BrowseItem { kind: browse_kind_from_id(bid), id: bid.to_owned(), title, subtitle, thumbnail });
    }
    let vid = list_item_video_id(node)?;
    Some(BrowseItem { kind: "song", id: vid, title, subtitle, thumbnail })
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
    if let Some(vid) =
        nav.and_then(|n| n.get("watchEndpoint")).and_then(|w| w.get("videoId")).and_then(Value::as_str)
    {
        return Some(BrowseItem { kind: "song", id: vid.to_owned(), title, subtitle, thumbnail });
    }
    let bid = nav
        .and_then(|n| n.get("browseEndpoint"))
        .and_then(|b| b.get("browseId"))
        .and_then(Value::as_str)?;
    Some(BrowseItem { kind: browse_kind_from_id(bid), id: bid.to_owned(), title, subtitle, thumbnail })
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
    let artist_id =
        strapline.and_then(|s| s.get("runs")).and_then(Value::as_array).and_then(|r| first_artist_id(r));
    let artist_thumbnail = header.and_then(|h| h.get("straplineThumbnail")).and_then(last_thumbnail);

    // Target the header's own thumbnail subtree so we get the cover, not the artist avatar.
    let thumbnail = header.and_then(|h| h.get("thumbnail")).and_then(last_thumbnail);
    let description = album_description(root, header);

    // Album track rows carry no per-track thumbnail (every track shares the cover shown once in
    // the header), so parse_list_item leaves them None. Fill missing ones with the album cover so
    // the player bar + queue show it when a track plays.
    let items = find_all(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(parse_list_item)
        .map(|mut it| {
            if it.thumbnail.is_none() {
                it.thumbnail = thumbnail.clone();
            }
            it
        })
        .collect();

    AlbumPage {
        title,
        artist,
        artist_id,
        artist_thumbnail,
        subtitle,
        second_subtitle,
        description,
        thumbnail,
        items,
        continuation: continuation_token(root),
        playlist_id: album_playlist_id(root),
    }
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

fn album_description(root: &Value, header: Option<&Value>) -> Option<String> {
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
    let thumbnail =
        header.and_then(|h| h.get("thumbnail")).and_then(last_thumbnail).or_else(|| header.and_then(last_thumbnail));

    let sub = header.and_then(|h| find_all(h, "subscribeButtonRenderer").into_iter().next());
    let channel_id = sub
        .and_then(|s| s.get("channelId"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| browse_id.to_owned());
    let subscribed = sub.and_then(|s| s.get("subscribed")).and_then(Value::as_bool).unwrap_or(false);
    let subscribers = sub.and_then(|s| {
        text_or_runs(s.get("subscriberCountText")).or_else(|| text_or_runs(s.get("longSubscriberCountText")))
    });

    // Walk the section list: the first list shelf = top songs; every carousel = a card row.
    let mut top_songs = Vec::new();
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
                }
            } else if let Some(carousel) = node.get("musicCarouselShelfRenderer") {
                if let Some(sec) = parse_artist_carousel(carousel) {
                    sections.push(sec);
                }
            }
        }
    }

    ArtistPage { name, thumbnail, description, subscribers, channel_id, subscribed, top_songs, sections }
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
    let more_browse_id = more.and_then(|e| e.get("browseId")).and_then(Value::as_str).map(str::to_owned);
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
    if let Some(vid) =
        nav.and_then(|n| n.get("watchEndpoint")).and_then(|w| w.get("videoId")).and_then(Value::as_str)
    {
        return Some(BrowseItem { kind: "song", id: vid.to_owned(), title, subtitle, thumbnail });
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
        });
    }
    // Otherwise a browseEndpoint → playlist/album/artist by browseId prefix.
    let browse_id = nav
        .and_then(|n| n.get("browseEndpoint"))
        .and_then(|b| b.get("browseId"))
        .and_then(Value::as_str)?;
    Some(BrowseItem {
        kind: browse_kind_from_id(browse_id),
        id: browse_id.to_owned(),
        title,
        subtitle,
        thumbnail,
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

/// The playlist/album header node — recursion finds the detail renderer even when it's wrapped in
/// an editable-playlist header.
fn playlist_header(root: &Value) -> Option<&Value> {
    ["musicResponsiveHeaderRenderer", "musicDetailHeaderRenderer"]
        .into_iter()
        .find_map(|key| find_all(root, key).into_iter().next())
}

/// Paging token, modern (`continuationCommand.token`) or legacy (`nextContinuationData`). context/08.
fn continuation_token(root: &Value) -> Option<String> {
    if let Some(t) =
        find_all(root, "continuationCommand").into_iter().find_map(|c| c.get("token").and_then(Value::as_str))
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
                } }
            ] } }
        });
        let home = parse_home(&root);
        assert_eq!(home.chips.len(), 1);
        assert_eq!(home.chips[0].title, "Workout");
        assert_eq!(home.chips[0].params, "ggNC0");
        assert_eq!(home.sections.len(), 2);
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
        assert_eq!(home.continuation.as_deref(), Some("HOME_MORE"));
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
        assert_eq!(p.items[1].title, "Track Two");
        assert_eq!(p.continuation.as_deref(), Some("MORE_TOKEN"));
        // A plain header (someone else's playlist) is not editable.
        assert!(!p.owned);
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
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "An Artist" }] } } }
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
                    { "text": "Drake", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdrake" } } }
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
                    { "musicCarouselShelfRenderer": { "contents": [
                        { "musicTwoRowItemRenderer": {
                            "title": { "runs": [{ "text": "Other Album" }] },
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
        assert_eq!(a.artist.as_deref(), Some("Drake"));
        assert_eq!(a.artist_id.as_deref(), Some("UCdrake"));
        assert_eq!(a.artist_thumbnail.as_deref(), Some("artist_avatar.jpg"));
        assert_eq!(a.thumbnail.as_deref(), Some("cover_big.jpg"));
        assert_eq!(a.description.as_deref(), Some("Iceman is one of three studio albums."));
        assert_eq!(a.items.len(), 1);
        assert_eq!(a.items[0].video_id, "trk1");
        // Track row has no thumbnail of its own → falls back to the album cover (for the player bar).
        assert_eq!(a.items[0].thumbnail.as_deref(), Some("cover_big.jpg"));
        // The album's own OLAK id from the track rows — never the carousel's other-album id.
        assert_eq!(a.playlist_id.as_deref(), Some("OLAK5uy_iceman"));
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
                    "subscriberCountText": { "runs": [{ "text": "32.7M subscribers" }] }
                } }
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
                    ] } },
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
        assert_eq!(a.top_songs.len(), 1);
        assert_eq!(a.top_songs[0].video_id, "song1");
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
}
