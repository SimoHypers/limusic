//! Reading a Spotify library without Spotify's Web API (#375).
//!
//! The Web API is out for an app like this one: since February 2026 a development-mode app is
//! capped at five users, and it only sees the tracks of playlists the user owns. So this reads what
//! Spotify hands anyone, and nothing here needs a Spotify account:
//!
//! - **a link.** The embed page (`open.spotify.com/embed/<kind>/<id>`) carries the first 100
//!   tracks and an anonymous token. That token is spent on `api.spotify.com` (429 for everyone),
//!   but `pathfinder`, the GraphQL API Spotify's own web player pages playlists out of, takes it.
//! - **the data export** ("Account data" on spotify.com/account/privacy). A zip of JSON with every
//!   playlist, private ones included, and Liked Songs. Read entirely offline.
//! - **a CSV** from Exportify, TuneMyMusic, Soundiiz and the like.
//!
//! `pathfinder` is private the way InnerTube is, so it is treated the same way: the one value that
//! rotates (the persisted-query hash) is re-read from Spotify's web player bundle when the built-in
//! one stops working, and a failure past that keeps the embed's 100 tracks with `truncated` set
//! instead of failing the import.
//!
//! Errors are short codes (`private`, `not_spotify`, ...) the UI words in the user's language;
//! anything else is a network error and goes through as text.

use std::sync::Mutex;
use std::time::Duration;

use reqwest::header::USER_AGENT;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http;

/// One song as the source describes it. Everything but the title is optional because the sources
/// disagree on what they know: the export has no duration, the embed has no album.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceTrack {
    /// Spotify's track id, when the source names one. The match cache is filed under it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ListKind {
    Playlist,
    Album,
    /// Liked Songs. Its own kind so the UI can name it in the user's language.
    Liked,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceList {
    pub kind: ListKind,
    pub name: String,
    /// The playlist's owner, or the album's artist.
    pub owner: Option<String>,
    pub cover: Option<String>,
    /// The share link, for a list read off one: what "Update from Spotify" reads again. `None` for
    /// an export or a CSV, which are snapshots with nothing to go back to.
    pub url: Option<String>,
    pub tracks: Vec<SourceTrack>,
    /// Rows that aren't songs (podcast episodes, audiobooks) or carry no title.
    pub skipped: usize,
    /// Only the embed's first 100 tracks could be read.
    pub truncated: bool,
}

/// An album by name, for matching a pasted album link to its YouTube Music page.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SavedAlbum {
    pub title: String,
    pub artist: String,
}

/// Everything one source holds: one list for a link, every playlist and Liked Songs for the export.
#[derive(Debug, Clone, Default)]
pub struct Library {
    pub lists: Vec<SourceList>,
}

// --- links ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Playlist,
    Album,
    Track,
    Artist,
}

impl LinkKind {
    fn path(self) -> &'static str {
        match self {
            LinkKind::Playlist => "playlist",
            LinkKind::Album => "album",
            LinkKind::Track => "track",
            LinkKind::Artist => "artist",
        }
    }
}

/// `None` for anything that isn't a Spotify playlist, album, track or artist. Takes what people
/// actually paste: `open.spotify.com/playlist/<id>?si=…`, the `/intl-de/` and `/embed/` forms,
/// the old `/user/<name>/playlist/<id>`, a bare host with no scheme, and `spotify:playlist:<id>`.
pub fn parse_link(input: &str) -> Option<(LinkKind, String)> {
    let text = input.trim();
    let segments: Vec<String> = if let Some(rest) = text.strip_prefix("spotify:") {
        rest.split(':').map(str::to_owned).collect()
    } else {
        let with_scheme =
            if text.contains("://") { text.to_owned() } else { format!("https://{text}") };
        let url = reqwest::Url::parse(&with_scheme).ok()?;
        if !matches!(url.host_str()?, "open.spotify.com" | "play.spotify.com") {
            return None;
        }
        url.path_segments()?.filter(|s| !s.is_empty()).map(str::to_owned).collect()
    };
    // The kind is whichever known word comes first; the id is the segment after it. That skips
    // `intl-xx`, `embed` and `user/<name>` without listing them.
    segments.windows(2).find_map(|pair| {
        let kind = match pair[0].as_str() {
            "playlist" => LinkKind::Playlist,
            "album" => LinkKind::Album,
            "track" => LinkKind::Track,
            "artist" => LinkKind::Artist,
            _ => return None,
        };
        let id = pair[1].as_str();
        (!id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric()))
            .then(|| (kind, id.to_owned()))
    })
}

pub fn share_url(kind: LinkKind, id: &str) -> String {
    format!("https://open.spotify.com/{}/{id}", kind.path())
}

// --- the embed page ------------------------------------------------------------------------------

async fn get_text(url: &str) -> Result<String, String> {
    let resp = http::client()
        .get(url)
        .header(USER_AGENT, http::WEB_UA)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    // A private or deleted playlist is a 404 on the embed page, the same as a mistyped id.
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("private".into());
    }
    resp.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())
}

/// The JSON a Next.js page boots from. Everything the embed shows is in it.
fn next_data(html: &str) -> Option<Value> {
    const OPEN: &str = r#"<script id="__NEXT_DATA__" type="application/json">"#;
    let start = html.find(OPEN)? + OPEN.len();
    let end = start + html[start..].find("</script>")?;
    serde_json::from_str(&html[start..end]).ok()
}

struct Embed {
    entity: Value,
    token: Option<String>,
}

fn parse_embed(html: &str) -> Result<Embed, String> {
    let data = next_data(html).ok_or("spotify_changed")?;
    // A private or deleted playlist is usually not a real 404: the page is a 200 whose props say
    // `status: 404` and carry no `state`.
    if data.pointer("/props/pageProps/status").and_then(Value::as_u64) == Some(404) {
        return Err("private".into());
    }
    let state = data.pointer("/props/pageProps/state").ok_or("spotify_changed")?;
    let entity = state.pointer("/data/entity").cloned().ok_or("private")?;
    let token =
        state.pointer("/settings/session/accessToken").and_then(Value::as_str).map(str::to_owned);
    Ok(Embed { entity, token })
}

async fn embed(kind: LinkKind, id: &str) -> Result<Embed, String> {
    parse_embed(&get_text(&format!("https://open.spotify.com/embed/{}/{id}", kind.path())).await?)
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The largest picture the entity offers: `visualIdentity.image` (`maxWidth`) or
/// `coverArt.sources` (`width`), whichever this entity type carries.
fn largest_image(entity: &Value) -> Option<String> {
    let pick = |list: Option<&Value>, width: &str| -> Option<(u64, String)> {
        list?
            .as_array()?
            .iter()
            .filter_map(|i| {
                Some((
                    i.get(width).and_then(Value::as_u64).unwrap_or(0),
                    str_at(i, "url")?.to_owned(),
                ))
            })
            .max_by_key(|(w, _)| *w)
    };
    pick(entity.pointer("/visualIdentity/image"), "maxWidth")
        .or_else(|| pick(entity.pointer("/coverArt/sources"), "width"))
        .map(|(_, url)| url)
}

/// The embed's artist line is the names joined with ", ". Splitting it back breaks a name that
/// has a comma in it ("Tyler, The Creator"); the matcher only uses it for overlap, where half a
/// name still counts, and `pathfinder` replaces it with the real list for playlists.
fn split_artists(line: &str) -> Vec<String> {
    line.split(", ").map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

fn track_id(uri: &str) -> Option<String> {
    uri.strip_prefix("spotify:track:").map(str::to_owned)
}

fn embed_list(kind: LinkKind, id: &str, entity: &Value) -> SourceList {
    let name = str_at(entity, "name").or_else(|| str_at(entity, "title")).unwrap_or_default();
    let album = (kind == LinkKind::Album).then(|| name.to_owned());
    let mut skipped = 0;
    let tracks = entity
        .get("trackList")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|t| {
            let title = str_at(t, "title").filter(|_| {
                t.get("entityType").and_then(Value::as_str).unwrap_or("track") == "track"
            });
            let Some(title) = title else {
                skipped += 1;
                return None;
            };
            Some(SourceTrack {
                id: str_at(t, "uri").and_then(track_id),
                title: title.to_owned(),
                artists: str_at(t, "subtitle").map(split_artists).unwrap_or_default(),
                album: album.clone(),
                duration_ms: t.get("duration").and_then(Value::as_u64).filter(|d| *d > 0),
                explicit: t.get("isExplicit").and_then(Value::as_bool),
            })
        })
        .collect();
    SourceList {
        kind: if kind == LinkKind::Album { ListKind::Album } else { ListKind::Playlist },
        name: name.to_owned(),
        owner: str_at(entity, "subtitle").map(str::to_owned),
        cover: largest_image(entity),
        url: Some(share_url(kind, id)),
        tracks,
        skipped,
        truncated: false,
    }
}

/// A single track off its embed page: what a pasted track link resolves from.
fn embed_track(entity: &Value) -> Option<SourceTrack> {
    Some(SourceTrack {
        id: str_at(entity, "uri").and_then(track_id),
        title: str_at(entity, "name").or_else(|| str_at(entity, "title"))?.to_owned(),
        artists: entity
            .get("artists")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| str_at(x, "name")).map(str::to_owned).collect())
            .unwrap_or_default(),
        album: None,
        duration_ms: entity.get("duration").and_then(Value::as_u64).filter(|d| *d > 0),
        explicit: entity.get("isExplicit").and_then(Value::as_bool),
    })
}

// --- pathfinder ----------------------------------------------------------------------------------

const PATHFINDER: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
/// `fetchPlaylistContents`, as Spotify's web player bundle named it on 2026-10-04. Old hashes
/// keep working for a while after a new one ships, which is why one baked in here is worth having.
const PLAYLIST_HASH: &str = "8964e8eafb21aa992a7d951d256d83285c04be2105d209262901de70cb97584a";
const PAGE: usize = 100;

/// A hash re-read from the web player this run, after the built-in one stopped working.
// ponytail: process memory only. A stale built-in hash costs two requests per launch to heal;
// persist it in `settings` if that ever matters.
static HEALED_HASH: Mutex<Option<String>> = Mutex::new(None);

fn playlist_hash() -> String {
    HEALED_HASH.lock().unwrap().clone().unwrap_or_else(|| PLAYLIST_HASH.to_owned())
}

/// Read the current `fetchPlaylistContents` hash out of the web player's main bundle, the way the
/// cipher reads its functions out of `player.js`.
async fn heal_hash() -> Option<String> {
    let shell = get_text("https://open.spotify.com/").await.ok()?;
    let bundle = regex::Regex::new(
        r"https://open\.spotifycdn\.com/cdn/build/web-player/web-player\.[0-9a-f]+\.js",
    )
    .ok()?
    .find(&shell)?
    .as_str()
    .to_owned();
    let js = get_text(&bundle).await.ok()?;
    let hash = regex::Regex::new(r#""fetchPlaylistContents","query","([0-9a-f]{64})""#)
        .ok()?
        .captures(&js)?
        .get(1)?
        .as_str()
        .to_owned();
    tracing::info!(hash, "spotify: re-read the playlist query hash");
    *HEALED_HASH.lock().unwrap() = Some(hash.clone());
    Some(hash)
}

async fn pathfinder_page(token: &str, uri: &str, offset: usize, hash: &str) -> Option<Value> {
    let body = serde_json::json!({
        "variables": { "uri": uri, "offset": offset, "limit": PAGE },
        "operationName": "fetchPlaylistContents",
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
    });
    let resp = http::client()
        .post(PATHFINDER)
        .bearer_auth(token)
        .header(USER_AGENT, http::WEB_UA)
        .header("app-platform", "WebPlayer")
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .ok()?;
    let v: Value = resp.json().await.ok()?;
    v.pointer("/data/playlistV2/content").is_some().then_some(v)
}

/// One page: the playlist's total, and its rows (`None` for an episode or anything else that is not
/// a song).
fn parse_page(v: &Value) -> Option<(usize, Vec<Option<SourceTrack>>)> {
    let content = v.pointer("/data/playlistV2/content")?;
    let total = content.get("totalCount").and_then(Value::as_u64)? as usize;
    let rows = content
        .get("items")?
        .as_array()?
        .iter()
        .map(|item| {
            let d = item.pointer("/itemV2/data")?;
            let uri = str_at(d, "uri").unwrap_or_default();
            if uri.starts_with("spotify:local:") {
                return parse_local_uri(uri);
            }
            if d.get("__typename").and_then(Value::as_str) != Some("Track") {
                return None;
            }
            let names = |p: &str| -> Vec<String> {
                d.pointer(p)
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.pointer("/profile/name").and_then(Value::as_str))
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            Some(SourceTrack {
                id: track_id(uri),
                title: str_at(d, "name")?.to_owned(),
                artists: names("/artists/items"),
                album: d.pointer("/albumOfTrack/name").and_then(Value::as_str).map(str::to_owned),
                duration_ms: d.pointer("/trackDuration/totalMilliseconds").and_then(Value::as_u64),
                explicit: d
                    .pointer("/contentRating/label")
                    .and_then(Value::as_str)
                    .map(|l| l == "EXPLICIT"),
            })
        })
        .collect();
    Some((total, rows))
}

/// Every track of a playlist, page by page. `None` when pathfinder won't answer even with a
/// freshly read hash, and the caller keeps what the embed had.
async fn pathfinder_playlist(token: &str, id: &str) -> Option<(Vec<SourceTrack>, usize)> {
    let uri = format!("spotify:playlist:{id}");
    let mut hash = playlist_hash();
    let mut tracks = Vec::new();
    let mut skipped = 0;
    let mut offset = 0;
    loop {
        // Paced like the web player paging as you scroll, not as fast as the network allows.
        if offset > 0 {
            tokio::time::sleep(Duration::from_millis(400 + rand::random::<u64>() % 400)).await;
        }
        let page = match pathfinder_page(token, &uri, offset, &hash).await {
            Some(p) => p,
            // Once per read, and only before anything came back: a failure halfway through a
            // long playlist is the network, not the hash.
            None if offset == 0 && hash == playlist_hash() => {
                let fresh = heal_hash().await.filter(|h| *h != hash)?;
                hash = fresh;
                pathfinder_page(token, &uri, offset, &hash).await?
            }
            // One more try after a pause, so a single dropped page doesn't cut a long playlist
            // down to the embed's 100.
            None => {
                tokio::time::sleep(Duration::from_secs(3)).await;
                pathfinder_page(token, &uri, offset, &hash).await?
            }
        };
        let (total, rows) = parse_page(&page)?;
        let got = rows.len();
        for row in rows {
            match row {
                Some(t) => tracks.push(t),
                None => skipped += 1,
            }
        }
        offset += got;
        if got == 0 || offset >= total {
            return Some((tracks, skipped));
        }
    }
}

/// Read a playlist or an album off its link.
pub async fn read_link(kind: LinkKind, id: &str) -> Result<SourceList, String> {
    if !matches!(kind, LinkKind::Playlist | LinkKind::Album) {
        return Err("not_a_list".into());
    }
    let Embed { entity, token } = embed(kind, id).await?;
    let mut list = embed_list(kind, id, &entity);
    if kind == LinkKind::Playlist {
        // From the first page rather than only past the embed's 100: pathfinder knows each
        // track's album, which the embed doesn't, and the album is one more thing to match on.
        let full = match token.as_deref() {
            Some(t) => pathfinder_playlist(t, id).await,
            None => None,
        };
        match full {
            Some((tracks, skipped)) => {
                list.tracks = tracks;
                list.skipped = skipped;
            }
            // The embed stops at exactly 100, so a full one may well have had more.
            None => list.truncated = list.tracks.len() + list.skipped >= PAGE,
        }
    }
    if list.tracks.is_empty() {
        return Err("empty".into());
    }
    Ok(list)
}

/// A track link, for "Spotify links open in Limusic".
pub async fn read_track(id: &str) -> Result<SourceTrack, String> {
    let Embed { entity, .. } = embed(LinkKind::Track, id).await?;
    embed_track(&entity).ok_or_else(|| "spotify_changed".into())
}

/// An album or artist link: just the name (and the artist line, for an album).
pub async fn read_name(kind: LinkKind, id: &str) -> Result<(String, Option<String>), String> {
    let Embed { entity, .. } = embed(kind, id).await?;
    let name = str_at(&entity, "name").ok_or("spotify_changed")?.to_owned();
    let subtitle = match kind {
        LinkKind::Album => str_at(&entity, "subtitle").map(str::to_owned),
        _ => None,
    };
    Ok((name, subtitle))
}

// --- the data export -----------------------------------------------------------------------------

#[derive(Deserialize)]
struct ExportPlaylists {
    #[serde(default)]
    playlists: Vec<ExportPlaylist>,
}

#[derive(Deserialize)]
struct ExportPlaylist {
    #[serde(default)]
    name: String,
    #[serde(default)]
    items: Vec<ExportItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportItem {
    #[serde(default)]
    track: Option<ExportTrack>,
    #[serde(default)]
    local_track: Option<ExportLocal>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportTrack {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    album_name: String,
    #[serde(default)]
    track_uri: String,
}

#[derive(Deserialize)]
struct ExportLocal {
    #[serde(default)]
    uri: String,
}

/// `YourLibrary.json`. It also lists saved albums and followed artists, which are deliberately
/// not brought over: following or saving hundreds at once is the bulk engagement YouTube's spam
/// filters look for (see `import.rs`, "staying welcome on YouTube").
#[derive(Deserialize)]
struct ExportLibrary {
    #[serde(default)]
    tracks: Vec<LibraryTrack>,
}

#[derive(Deserialize)]
struct LibraryTrack {
    #[serde(default)]
    artist: String,
    #[serde(default)]
    album: String,
    #[serde(default)]
    track: String,
    #[serde(default)]
    uri: String,
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// `spotify:local:<artist>:<album>:<title>:<seconds>`, form-encoded: a file the user added to a
/// playlist from their own disk. It is still a song, and often one YouTube Music has.
fn parse_local_uri(uri: &str) -> Option<SourceTrack> {
    let fields: Vec<String> = uri
        .strip_prefix("spotify:local:")?
        .split(':')
        .map(|f| {
            urlencoding::decode(&f.replace('+', " ")).map(|s| s.into_owned()).unwrap_or_default()
        })
        .collect();
    let [artist, album, title, secs] = fields.as_slice() else {
        return None;
    };
    Some(SourceTrack {
        id: None,
        title: non_empty(title)?,
        artists: non_empty(artist).into_iter().collect(),
        album: non_empty(album),
        duration_ms: secs.parse::<u64>().ok().filter(|s| *s > 0).map(|s| s * 1000),
        explicit: None,
    })
}

fn export_list(kind: ListKind, name: String, rows: Vec<Option<SourceTrack>>) -> SourceList {
    let skipped = rows.iter().filter(|r| r.is_none()).count();
    SourceList {
        kind,
        name,
        owner: None,
        cover: None,
        url: None,
        tracks: rows.into_iter().flatten().collect(),
        skipped,
        truncated: false,
    }
}

fn read_playlists_json(bytes: &[u8], lib: &mut Library) -> Result<(), String> {
    let parsed: ExportPlaylists = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    for p in parsed.playlists {
        let rows = p
            .items
            .into_iter()
            .map(|i| match (i.track, i.local_track) {
                (Some(t), _) => Some(SourceTrack {
                    id: track_id(&t.track_uri),
                    title: non_empty(&t.track_name)?,
                    artists: non_empty(&t.artist_name).into_iter().collect(),
                    album: non_empty(&t.album_name),
                    ..Default::default()
                }),
                (None, Some(l)) => parse_local_uri(&l.uri),
                // An episode, an audiobook chapter, or a row Spotify exported blank.
                _ => None,
            })
            .collect();
        lib.lists.push(export_list(ListKind::Playlist, p.name, rows));
    }
    Ok(())
}

fn read_library_json(bytes: &[u8], lib: &mut Library) -> Result<(), String> {
    let parsed: ExportLibrary = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if !parsed.tracks.is_empty() {
        let rows = parsed
            .tracks
            .into_iter()
            .map(|t| {
                Some(SourceTrack {
                    id: track_id(&t.uri),
                    title: non_empty(&t.track)?,
                    artists: non_empty(&t.artist).into_iter().collect(),
                    album: non_empty(&t.album),
                    ..Default::default()
                })
            })
            .collect();
        // First, ahead of the playlists: it's the list someone switching cares about most.
        lib.lists.insert(0, export_list(ListKind::Liked, "Liked Songs".into(), rows));
    }
    Ok(())
}

/// What a JSON file from the export holds, told apart by its keys rather than its name, so a
/// renamed file still works.
fn read_json(bytes: &[u8], lib: &mut Library) -> Result<(), String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| "unreadable")?;
    if v.get("playlists").is_some() {
        read_playlists_json(bytes, lib)
    } else if v.get("tracks").is_some() || v.get("artists").is_some() {
        read_library_json(bytes, lib)
    } else {
        Err("unreadable".into())
    }
}

fn read_zip(bytes: &[u8], lib: &mut Library) -> Result<(), String> {
    use std::io::Read;
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "unreadable".to_string())?;
    // Playlist1.json, Playlist2.json, … and YourLibrary.json, under a folder whose name has
    // changed over the years ("MyData", "Spotify Account Data"). By number, so Playlist10 comes
    // after Playlist9 and the playlists keep Spotify's order. Liked Songs is read last so it ends
    // up first whatever order the zip lists things in.
    let mut names: Vec<String> = zip
        .file_names()
        .filter(|n| {
            let base = n.rsplit('/').next().unwrap_or(n);
            (base.starts_with("Playlist") && base.ends_with(".json")) || base == "YourLibrary.json"
        })
        .map(str::to_owned)
        .collect();
    let number = |n: &str| -> u32 {
        let base = n.rsplit('/').next().unwrap_or(n);
        base.trim_start_matches("Playlist").trim_end_matches(".json").parse().unwrap_or(0)
    };
    names.sort_by_key(|n| (n.ends_with("YourLibrary.json"), number(n), n.clone()));
    for name in names {
        let mut buf = Vec::new();
        zip.by_name(&name)
            .map_err(|e| e.to_string())?
            .read_to_end(&mut buf)
            .map_err(|e| e.to_string())?;
        read_json(&buf, lib)?;
    }
    Ok(())
}

// --- CSV -----------------------------------------------------------------------------------------

/// Header names lowercased with everything but letters dropped: "Artist Name(s)" → "artistnames".
fn header_key(h: &str) -> String {
    h.chars().filter(char::is_ascii_alphabetic).map(|c| c.to_ascii_lowercase()).collect()
}

fn column(headers: &[String], names: &[&str]) -> Option<usize> {
    names.iter().find_map(|n| headers.iter().position(|h| h == n))
}

/// "3:45", "225000" (ms) or "225" (seconds).
fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some((m, sec)) = s.split_once(':') {
        let (m, sec): (u64, u64) = (m.trim().parse().ok()?, sec.trim().parse().ok()?);
        return Some((m * 60 + sec) * 1000);
    }
    let n: f64 = s.parse().ok()?;
    let n = n as u64;
    (n > 0).then_some(if n > 10_000 { n } else { n * 1000 })
}

fn read_csv(bytes: &[u8], file_name: &str, lib: &mut Library) -> Result<(), String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);
    let headers: Vec<String> =
        rdr.headers().map_err(|_| "unreadable")?.iter().map(header_key).collect();
    let title = column(&headers, &["trackname", "title", "songname", "name", "song", "track"])
        .ok_or("csv_columns")?;
    let artist = column(&headers, &["artistnames", "artistname", "artists", "artist"]);
    let album = column(&headers, &["albumname", "album"]);
    let duration =
        column(&headers, &["durationms", "trackdurationms", "duration", "length", "time"]);
    let explicit = column(&headers, &["explicit"]);
    let uri = column(&headers, &["trackuri", "spotifyid", "spotifyuri", "uri"]);
    let playlist = column(&headers, &["playlistname", "playlist"]);
    let fallback = std::path::Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Imported playlist")
        .to_owned();

    // One CSV can hold several playlists (TuneMyMusic's column), kept in first-seen order.
    let mut lists: Vec<(String, Vec<Option<SourceTrack>>)> = Vec::new();
    for record in rdr.records().flatten() {
        let get = |i: Option<usize>| i.and_then(|i| record.get(i)).map(str::trim).unwrap_or("");
        let name = non_empty(get(playlist)).unwrap_or_else(|| fallback.clone());
        let row = non_empty(get(Some(title))).map(|title| {
            let artists = get(artist);
            let sep = if artists.contains(';') { ';' } else { ',' };
            let id = get(uri);
            SourceTrack {
                id: track_id(id).or_else(|| {
                    (id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric()))
                        .then(|| id.to_owned())
                }),
                title,
                artists: artists.split(sep).filter_map(non_empty).collect(),
                album: non_empty(get(album)),
                duration_ms: parse_duration(get(duration)),
                explicit: match get(explicit).to_ascii_lowercase().as_str() {
                    "true" | "yes" | "1" => Some(true),
                    "false" | "no" | "0" => Some(false),
                    _ => None,
                },
            }
        });
        match lists.iter_mut().find(|(n, _)| *n == name) {
            Some((_, rows)) => rows.push(row),
            None => lists.push((name, vec![row])),
        }
    }
    lib.lists.extend(lists.into_iter().map(|(n, rows)| export_list(ListKind::Playlist, n, rows)));
    Ok(())
}

/// Read a dropped or picked file: the export zip, one of its JSON files, or a CSV.
pub fn read_file(bytes: &[u8], file_name: &str) -> Result<Library, String> {
    let mut lib = Library::default();
    let lower = file_name.to_ascii_lowercase();
    if bytes.starts_with(b"PK\x03\x04") {
        read_zip(bytes, &mut lib)?;
    } else if lower.ends_with(".csv") {
        read_csv(bytes, file_name, &mut lib)?;
    } else {
        read_json(bytes, &mut lib)?;
    }
    lib.lists.retain(|l| !l.tracks.is_empty());
    if lib.lists.is_empty() {
        return Err("empty".into());
    }
    Ok(lib)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMBED_PLAYLIST: &str = include_str!("../tests/fixtures/spotify/embed_playlist.html");
    const EMBED_ALBUM: &str = include_str!("../tests/fixtures/spotify/embed_album.html");
    const EMBED_TRACK: &str = include_str!("../tests/fixtures/spotify/embed_track.html");
    const PATHFINDER_PAGE: &str =
        include_str!("../tests/fixtures/spotify/pathfinder_playlist_page.json");

    /// Live, against Spotify only (no YouTube): a 150-track editorial playlist comes back whole,
    /// with albums, which only pathfinder knows. `cargo test -p limusic-app --lib -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn reads_past_the_embed_limit() {
        let list = read_link(LinkKind::Playlist, "37i9dQZF1DX4UtSsGT1Sbe").await.unwrap();
        assert!(list.tracks.len() > 100, "{} tracks", list.tracks.len());
        assert!(!list.truncated);
        assert!(list.tracks.iter().all(|t| t.album.is_some() && t.duration_ms.is_some()));
    }

    /// Live: the hash can still be read out of the web player bundle, which is the fallback for
    /// the day the built-in one stops working.
    #[tokio::test]
    #[ignore]
    async fn heals_the_playlist_hash() {
        let hash = heal_hash().await.expect("hash not found in the web player bundle");
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn links() {
        let id = "37i9dQZF1DX4UtSsGT1Sbe";
        let want = Some((LinkKind::Playlist, id.to_owned()));
        for s in [
            "https://open.spotify.com/playlist/37i9dQZF1DX4UtSsGT1Sbe?si=abc123",
            "open.spotify.com/playlist/37i9dQZF1DX4UtSsGT1Sbe",
            "https://open.spotify.com/intl-de/playlist/37i9dQZF1DX4UtSsGT1Sbe",
            "https://open.spotify.com/embed/playlist/37i9dQZF1DX4UtSsGT1Sbe",
            "https://open.spotify.com/user/spotify/playlist/37i9dQZF1DX4UtSsGT1Sbe",
            "  spotify:playlist:37i9dQZF1DX4UtSsGT1Sbe ",
        ] {
            assert_eq!(parse_link(s), want, "{s}");
        }
        assert_eq!(
            parse_link("https://open.spotify.com/track/05RgAMGypEvqhNs5hPCbMS"),
            Some((LinkKind::Track, "05RgAMGypEvqhNs5hPCbMS".into()))
        );
        assert_eq!(parse_link("https://music.youtube.com/playlist?list=PL123"), None);
        assert_eq!(parse_link("https://open.spotify.com/show/abc"), None);
        assert_eq!(parse_link("https://open.spotify.com/playlist/"), None);
        assert_eq!(parse_link("not a link"), None);
    }

    #[test]
    fn embed_playlist_page() {
        let e = parse_embed(EMBED_PLAYLIST).unwrap();
        assert_eq!(e.token.as_deref(), Some("ANON-TOKEN"));
        let list = embed_list(LinkKind::Playlist, "37i9dQZF1DX4UtSsGT1Sbe", &e.entity);
        assert_eq!(list.name, "All Out 80s");
        assert_eq!(list.owner.as_deref(), Some("Spotify"));
        assert!(list.cover.as_deref().is_some_and(|c| c.starts_with("https://")));
        assert_eq!(list.tracks.len(), 3);
        let t = &list.tracks[0];
        assert_eq!(t.title, "La Isla Bonita");
        assert_eq!(t.artists, ["Madonna"]);
        assert_eq!(t.duration_ms, Some(242733));
        assert_eq!(t.id.as_deref(), Some("6r8k1vznHrzlEKYxL4dZEe"));
        assert_eq!(t.album, None);
    }

    #[test]
    fn embed_private_page() {
        let html = r#"<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"status":404,"title":"Page not found"}}}</script>"#;
        assert_eq!(parse_embed(html).err().as_deref(), Some("private"));
    }

    #[test]
    fn embed_album_and_track() {
        let e = parse_embed(EMBED_ALBUM).unwrap();
        let list = embed_list(LinkKind::Album, "3REUXdj5OPKhuDTrTtCBU0", &e.entity);
        assert_eq!(list.kind, ListKind::Album);
        assert_eq!(list.owner.as_deref(), Some("Van Halen"));
        assert!(list.tracks.iter().all(|t| t.album.as_deref() == Some("1984 (Remastered)")));

        let t = embed_track(&parse_embed(EMBED_TRACK).unwrap().entity).unwrap();
        assert_eq!(t.title, "Panama - 2015 Remaster");
        assert_eq!(t.artists, ["Van Halen"]);
        assert_eq!(t.duration_ms, Some(210226));
    }

    #[test]
    fn pathfinder_rows() {
        let v: Value = serde_json::from_str(PATHFINDER_PAGE).unwrap();
        let (total, rows) = parse_page(&v).unwrap();
        assert_eq!(total, 150);
        assert_eq!(rows.len(), 3);
        let t = rows[0].as_ref().unwrap();
        assert_eq!(t.title, "Panama - 2015 Remaster");
        assert_eq!(t.album.as_deref(), Some("1984 (Remastered)"));
        assert_eq!(t.explicit, Some(false));
        assert_eq!(rows[1].as_ref().unwrap().explicit, Some(true));
        assert!(rows[2].is_none(), "an episode is not a song");
    }

    #[test]
    fn local_files() {
        let t = parse_local_uri("spotify:local:Daft+Punk:Discovery:One+More+Time:320").unwrap();
        assert_eq!(t.title, "One More Time");
        assert_eq!(t.artists, ["Daft Punk"]);
        assert_eq!(t.album.as_deref(), Some("Discovery"));
        assert_eq!(t.duration_ms, Some(320_000));
        assert_eq!(parse_local_uri("spotify:local:::%C3%89t%C3%A9:0").unwrap().title, "Été");
        assert!(parse_local_uri("spotify:local:a:b").is_none());
    }

    fn export_zip() -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            z.start_file("Spotify Account Data/YourLibrary.json", opts).unwrap();
            z.write_all(
                br#"{"tracks":[{"artist":"Madonna","album":"True Blue","track":"La Isla Bonita","uri":"spotify:track:6r8k1vznHrzlEKYxL4dZEe"}],
                    "albums":[{"artist":"Van Halen","album":"1984","uri":"spotify:album:x"}],
                    "artists":[{"name":"Van Halen","uri":"spotify:artist:y"}],"shows":[],"episodes":[]}"#,
            )
            .unwrap();
            z.start_file("Spotify Account Data/Playlist1.json", opts).unwrap();
            z.write_all(
                br#"{"playlists":[{"name":"Road trip","lastModifiedDate":"2024-01-01","items":[
                    {"track":{"trackName":"Panama","artistName":"Van Halen","albumName":"1984","trackUri":"spotify:track:05RgAMGypEvqhNs5hPCbMS"},"episode":null,"localTrack":null,"addedDate":"2024-01-01"},
                    {"track":null,"episode":{"episodeName":"Pod","showName":"Show","episodeUri":"spotify:episode:z"},"localTrack":null},
                    {"track":null,"episode":null,"localTrack":{"uri":"spotify:local:Daft+Punk:Discovery:One+More+Time:320"}}
                ]},{"name":"Empty","items":[]}]}"#,
            )
            .unwrap();
            // Big exports split across files; zipped in name order, Playlist10 before Playlist2.
            for (file, name) in [("Playlist10", "Tenth"), ("Playlist2", "Second")] {
                z.start_file(format!("Spotify Account Data/{file}.json"), opts).unwrap();
                let json = format!(
                    r#"{{"playlists":[{{"name":"{name}","items":[{{"track":{{"trackName":"Jump","artistName":"Van Halen","albumName":"1984","trackUri":"spotify:track:7N3PAbqfTjSEU1edb2tY8j"}}}}]}}]}}"#
                );
                z.write_all(json.as_bytes()).unwrap();
            }
            z.start_file("Spotify Account Data/Userdata.json", opts).unwrap();
            z.write_all(b"{}").unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn data_export() {
        let lib = read_file(&export_zip(), "my_spotify_data.zip").unwrap();
        let names: Vec<&str> = lib.lists.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            ["Liked Songs", "Road trip", "Second", "Tenth"],
            "the empty one is dropped"
        );
        assert_eq!(lib.lists[0].kind, ListKind::Liked);
        assert_eq!(lib.lists[0].tracks[0].id.as_deref(), Some("6r8k1vznHrzlEKYxL4dZEe"));
        let road = &lib.lists[1];
        assert_eq!(road.name, "Road trip");
        assert_eq!(road.tracks.len(), 2);
        assert_eq!(road.skipped, 1);
        assert_eq!(road.tracks[1].title, "One More Time");
    }

    #[test]
    fn csv_exports() {
        // Exportify's columns.
        let exportify =
            "\u{feff}Track URI,Track Name,Artist Name(s),Album Name,Duration (ms),Explicit\n\
            spotify:track:05RgAMGypEvqhNs5hPCbMS,Panama,\"Van Halen\",1984,210226,false\n\
            spotify:track:abc,Collab,\"Future, Metro Boomin\",We Don't Trust You,183000,true\n";
        let lib = read_file(exportify.as_bytes(), "Road Trip.csv").unwrap();
        assert_eq!(lib.lists.len(), 1);
        assert_eq!(lib.lists[0].name, "Road Trip");
        let t = &lib.lists[0].tracks;
        assert_eq!(t[0].id.as_deref(), Some("05RgAMGypEvqhNs5hPCbMS"));
        assert_eq!(t[0].duration_ms, Some(210226));
        assert_eq!(t[1].artists, ["Future", "Metro Boomin"]);
        assert_eq!(t[1].explicit, Some(true));

        // TuneMyMusic: several playlists in one file, told apart by a column.
        let tmm = "Track name,Artist name,Album,Playlist name,Type,ISRC,Spotify - id\n\
            Panama,Van Halen,1984,Rock,Playlist,USWB10400044,05RgAMGypEvqhNs5hPCbMS\n\
            Jump,Van Halen,1984,Rock,Playlist,,\n\
            Vogue,Madonna,I'm Breathless,Pop,Playlist,,\n";
        let lib = read_file(tmm.as_bytes(), "export.csv").unwrap();
        let names: Vec<&str> = lib.lists.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Rock", "Pop"]);
        assert_eq!(lib.lists[0].tracks[0].id.as_deref(), Some("05RgAMGypEvqhNs5hPCbMS"));

        assert_eq!(read_file(b"foo,bar\n1,2\n", "x.csv").unwrap_err(), "csv_columns");
        assert_eq!(parse_duration("3:45"), Some(225_000));
        assert_eq!(parse_duration("225"), Some(225_000));
    }
}
