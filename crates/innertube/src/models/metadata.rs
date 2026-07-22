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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SongItem {
    pub video_id: String,
    pub title: String,
    pub artists: String,
    /// The primary artist's channel browseId (`UC…`), when the row links one — lets the UI make
    /// the artist name navigate to its artist page. context/08.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    /// The album's browseId (`MPRE…`), when the row links one — lets the UI navigate to the album.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// `playlistSetVideoId` — the item's id *within a playlist*, needed to remove it (context/01
    /// edit_playlist). Only present when the item came from a playlist page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_video_id: Option<String>,
    /// Whether the signed-in user has liked this track (`likeStatus == "LIKE"`). `None` when the
    /// response didn't carry a like status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liked: Option<bool>,
    /// Listen Together: username of the guest who added this queue item (`None` for the user's own
    /// tracks). Never parsed from YouTube — pure queue metadata, carried for attribution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_by: Option<String>,
    /// Manually "added to queue" (vs. a playlist/radio track). Marks the "up next" block so
    /// successive adds stack FIFO right after the current song. Pure queue metadata, never parsed.
    #[serde(default)]
    pub queued: bool,
    /// Appended by autoplay radio continuation (vs. chosen by the user). Drives the queue's
    /// "Autoplay" divider + player-bar badge. Pure queue metadata, never parsed.
    #[serde(default)]
    pub autoplay: bool,
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
}

/// Logged-in account summary from `account/account_menu`. context/01, context/04A, context/15.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountInfo {
    pub name: Option<String>,
    /// Channel handle or email (whichever the header carries).
    pub handle: Option<String>,
    pub thumbnail: Option<String>,
    /// `onBehalfOfUser` id, `||`-split (context/04A). None when absent / single-account.
    pub data_sync_id: Option<String>,
    /// A login-bound visitorData, if the response carried one (context/15).
    pub visitor_data: Option<String>,
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
    for node in find_all(root, "playlistPanelVideoRenderer") {
        if let Some(item) = parse_panel_video(node) {
            items.push(item);
        }
    }
    // The automix/radio continuation (context/08): the panel ends with a continuation token
    // used to fetch the endless mix. Take the first continuation token we find.
    let continuation = find_first_str(root, "continuation");
    NextResult { items, continuation, lyrics_browse_id: lyrics_browse_id(root) }
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
    let name = header.and_then(|h| runs_text(h.get("accountName")));
    // YTM labels the second line `channelHandle` on newer accounts, `email` on older ones.
    let handle =
        header.and_then(|h| runs_text(h.get("channelHandle")).or_else(|| runs_text(h.get("email"))));
    let thumbnail = header.and_then(last_thumbnail);

    let rc = root.get("responseContext");
    // dataSyncId lives in the response context, not the menu header. context/04A.
    let data_sync_id = rc
        .and_then(|r| r.get("mainAppWebResponseContext"))
        .and_then(|m| m.get("datasyncId"))
        .and_then(Value::as_str)
        .map(split_datasync_id);
    let visitor_data = rc
        .and_then(|r| r.get("visitorData"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    AccountInfo { name, handle, thumbnail, data_sync_id, visitor_data }
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
    let title = flex
        .and_then(|c| c.first())
        .and_then(flex_text)
        .unwrap_or_default();
    if title.is_empty() {
        return None;
    }
    // Second flex column holds subtitle runs: "Artist • Album • duration" (• separated).
    let subtitle_runs = flex
        .and_then(|c| c.get(1))
        .and_then(|c| c.get("musicResponsiveListItemFlexColumnRenderer"))
        .and_then(|r| r.get("text"))
        .and_then(|t| t.get("runs"))
        .and_then(Value::as_array);
    let (artists, album, duration) = split_subtitle(subtitle_runs);
    let artist_id = subtitle_runs.and_then(|r| first_artist_id(r));
    let set_video_id = node
        .get("playlistItemData")
        .and_then(|d| d.get("playlistSetVideoId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(SongItem {
        video_id,
        title,
        artists,
        artist_id,
        album,
        album_id: album_id(node),
        duration,
        thumbnail: last_thumbnail(node),
        set_video_id,
        liked: like_status(node),
        queued_by: None,
        queued: false,
        autoplay: false,
    })
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

/// First run that links an artist channel (`browseEndpoint.browseId` starting with `UC`). context/08.
pub(crate) fn first_artist_id(runs: &[Value]) -> Option<String> {
    runs.iter().find_map(|r| {
        let id = r.get("navigationEndpoint")?.get("browseEndpoint")?.get("browseId")?.as_str()?;
        id.starts_with("UC").then(|| id.to_owned())
    })
}

/// The track's like state from its menu's `likeStatus` (`LIKE` / `INDIFFERENT` / `DISLIKE`).
/// Tolerant: grabs the first `likeStatus` anywhere in the node. context/08.
fn like_status(node: &Value) -> Option<bool> {
    find_first_str(node, "likeStatus").map(|s| s == "LIKE")
}

fn parse_panel_video(node: &Value) -> Option<SongItem> {
    let video_id = node.get("videoId").and_then(Value::as_str)?.to_owned();
    let title = runs_text(node.get("title"))?;
    let byline = node.get("longBylineText").or_else(|| node.get("shortBylineText"));
    let artists = byline.and_then(runs_text_opt).unwrap_or_default();
    let artist_id =
        byline.and_then(|b| b.get("runs")).and_then(Value::as_array).and_then(|r| first_artist_id(r));
    let duration = node.get("lengthText").and_then(runs_text_opt);
    Some(SongItem {
        video_id,
        title,
        artists,
        artist_id,
        album: None,
        album_id: album_id(node),
        duration,
        thumbnail: last_thumbnail(node),
        set_video_id: None,
        liked: like_status(node),
        queued_by: None,
        queued: false,
        autoplay: false,
    })
}

/// Joined text of a `musicResponsiveListItemRenderer` flex column (0 = title, 1 = subtitle). Used
/// by the search-section parser to build cards from list rows. context/08.
pub(crate) fn flex_column_text(node: &Value, i: usize) -> Option<String> {
    node.get("flexColumns")
        .and_then(Value::as_array)
        .and_then(|c| c.get(i))
        .and_then(flex_text)
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

/// Split a "• "-separated subtitle run list into (artists, album, duration). context/08.
fn split_subtitle(runs: Option<&Vec<Value>>) -> (String, Option<String>, Option<String>) {
    let Some(runs) = runs else { return (String::new(), None, None) };
    let mut groups: Vec<String> = Vec::new();
    let mut cur = String::new();
    for run in runs {
        let t = run.get("text").and_then(Value::as_str).unwrap_or("");
        if t.trim() == "•" {
            groups.push(std::mem::take(&mut cur));
        } else {
            cur.push_str(t);
        }
    }
    groups.push(cur);
    let groups: Vec<String> = groups.into_iter().map(|g| g.trim().to_string()).collect();
    let artists = groups.first().cloned().unwrap_or_default();
    // Last group that looks like a duration (contains ':') is the duration; the middle is album.
    let duration = groups.iter().rev().find(|g| g.contains(':')).cloned();
    let album = groups.get(1).filter(|g| Some(*g) != duration.as_ref()).cloned();
    (artists, album, duration)
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

    #[test]
    fn parses_search_item() {
        let root = json!({
            "a": { "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": "abc123" },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Song Title" }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                        { "text": "The Artist", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCartist1" } } },
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
        assert_eq!(s.artists, "The Artist");
        assert_eq!(s.artist_id.as_deref(), Some("UCartist1"));
        assert_eq!(s.album.as_deref(), Some("The Album"));
        assert_eq!(s.album_id.as_deref(), Some("MPREalbum1"));
        assert_eq!(s.duration.as_deref(), Some("3:21"));
        assert_eq!(s.thumbnail.as_deref(), Some("big.jpg"));
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
    }

    #[test]
    fn splits_datasync_id() {
        assert_eq!(split_datasync_id("realid||other"), "realid");
        assert_eq!(split_datasync_id("||fallback"), "fallback");
        assert_eq!(split_datasync_id("plain"), "plain");
    }

    #[test]
    fn parses_account_menu() {
        let root = json!({
            "responseContext": {
                "visitorData": "CgtNEWVISITOR",
                "mainAppWebResponseContext": { "datasyncId": "1234||5678" }
            },
            "actions": [{ "openPopupAction": { "popup": { "multiPageMenuRenderer": { "sections": [{
                "activeAccountHeaderRenderer": {
                    "accountName": { "runs": [{ "text": "Jane Doe" }] },
                    "channelHandle": { "runs": [{ "text": "@janedoe" }] },
                    "accountPhoto": { "thumbnails": [{ "url": "small.jpg" }, { "url": "big.jpg" }] }
                }
            }] } } } }]
        });
        let a = parse_account_menu(&root);
        assert_eq!(a.name.as_deref(), Some("Jane Doe"));
        assert_eq!(a.handle.as_deref(), Some("@janedoe"));
        assert_eq!(a.thumbnail.as_deref(), Some("big.jpg"));
        assert_eq!(a.data_sync_id.as_deref(), Some("1234"));
        assert_eq!(a.visitor_data.as_deref(), Some("CgtNEWVISITOR"));
    }
}
