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

use super::metadata::{find_all, last_thumbnail, parse_list_item, runs_text, SongItem};

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
pub struct Section {
    pub title: String,
    pub items: Vec<BrowseItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HomePage {
    pub sections: Vec<Section>,
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
}

/// A page of extra tracks fetched via a continuation token.
#[derive(Debug, Clone, Serialize)]
pub struct PlaylistContinuation {
    pub items: Vec<SongItem>,
    pub continuation: Option<String>,
}

/// Parse a `FEmusic_home` response into titled carousel sections. context/08.
pub fn parse_home(root: &Value) -> HomePage {
    let mut sections = Vec::new();
    for shelf in find_all(root, "musicCarouselShelfRenderer") {
        let title = find_all(shelf, "musicCarouselShelfBasicHeaderRenderer")
            .into_iter()
            .next()
            .and_then(|h| runs_text(h.get("title")))
            .unwrap_or_default();
        let items: Vec<BrowseItem> = shelf
            .get("contents")
            .and_then(Value::as_array)
            .map(|c| c.iter().filter_map(parse_carousel_item).collect())
            .unwrap_or_default();
        if !items.is_empty() {
            sections.push(Section { title, items });
        }
    }
    HomePage { sections }
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
    let items = find_all(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(parse_list_item)
        .collect();
    PlaylistPage { title, subtitle, thumbnail, items, continuation: continuation_token(root) }
}

/// Parse a browse continuation response (more playlist tracks). context/08.
pub fn parse_playlist_continuation(root: &Value) -> PlaylistContinuation {
    let items = find_all(root, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(parse_list_item)
        .collect();
    PlaylistContinuation { items, continuation: continuation_token(root) }
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
            "contents": { "sectionListRenderer": { "contents": [
                { "musicCarouselShelfRenderer": {
                    "header": { "musicCarouselShelfBasicHeaderRenderer": {
                        "title": { "runs": [{ "text": "Mixed for you" }] }
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
                } }
            ] } }
        });
        let home = parse_home(&root);
        assert_eq!(home.sections.len(), 1);
        let s = &home.sections[0];
        assert_eq!(s.title, "Mixed for you");
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.items[0].kind, "playlist");
        assert_eq!(s.items[0].id, "VLPL123");
        assert_eq!(s.items[0].title, "My Mix");
        assert_eq!(s.items[0].thumbnail.as_deref(), Some("b.jpg"));
        assert_eq!(s.items[1].kind, "album");
        assert_eq!(s.items[1].id, "MPREb_abc");
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
    fn continuation_prefers_modern_token() {
        let root = json!({
            "continuationItemRenderer": { "continuationEndpoint": {
                "continuationCommand": { "token": "MODERN" }
            } }
        });
        assert_eq!(continuation_token(&root).as_deref(), Some("MODERN"));
    }
}
