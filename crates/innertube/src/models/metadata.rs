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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
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
    NextResult { items, continuation }
}

// --- node parsers -------------------------------------------------------------------------

fn parse_list_item(node: &Value) -> Option<SongItem> {
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
    Some(SongItem {
        video_id,
        title,
        artists,
        album,
        duration,
        thumbnail: last_thumbnail(node),
    })
}

fn parse_panel_video(node: &Value) -> Option<SongItem> {
    let video_id = node.get("videoId").and_then(Value::as_str)?.to_owned();
    let title = runs_text(node.get("title"))?;
    let artists = node
        .get("longBylineText")
        .or_else(|| node.get("shortBylineText"))
        .and_then(runs_text_opt)
        .unwrap_or_default();
    let duration = node.get("lengthText").and_then(runs_text_opt);
    Some(SongItem {
        video_id,
        title,
        artists,
        album: None,
        duration,
        thumbnail: last_thumbnail(node),
    })
}

/// videoId from any of the three known locations. context/08 / AlbumPage.kt.
fn list_item_video_id(node: &Value) -> Option<String> {
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

fn runs_text(v: Option<&Value>) -> Option<String> {
    v.and_then(runs_text_opt)
}

/// Join all `runs[].text` in a `{ runs: [...] }` object.
fn runs_text_opt(v: &Value) -> Option<String> {
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
fn last_thumbnail(node: &Value) -> Option<String> {
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
fn find_all<'a>(root: &'a Value, key: &str) -> Vec<&'a Value> {
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

/// First string value under any key named `key`.
fn find_first_str(root: &Value, key: &str) -> Option<String> {
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
                        { "text": "The Artist" }, { "text": " • " }, { "text": "The Album" }, { "text": " • " }, { "text": "3:21" }
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
        assert_eq!(s.album.as_deref(), Some("The Album"));
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
            ], "continuations": [{ "nextContinuationData": { "continuation": "CONT_TOKEN" } }] } }
        });
        let r = parse_next(&root);
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].video_id, "vid9");
        assert_eq!(r.items[0].title, "Next Song");
        assert_eq!(r.items[0].artists, "Artist A & Artist B");
        assert_eq!(r.items[0].duration.as_deref(), Some("4:05"));
        assert_eq!(r.continuation.as_deref(), Some("CONT_TOKEN"));
    }
}
