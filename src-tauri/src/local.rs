//! Local music: scan folders on disk, read tags, and hand mpv a file path instead of a stream URL.
//!
//! Everything local rides the surfaces that already exist. A file is a `SongItem` whose `video_id`
//! is `LOCAL:<absolute path>`, an album is a `BrowseItem` with id `LOCALALBUM:<key>`, and both are
//! intercepted before anything YouTube-shaped runs: `AppState::resolve` for playback,
//! `commands::get_album` for the album page. So queueing, gapless, shuffle, media keys, Shortcuts,
//! and drag-and-drop work on local music without knowing it exists — and work offline, because
//! nothing on this path touches the network.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use innertube::{AlbumPage, BrowseItem, SongItem};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{Accessor, ItemKey};

use crate::db::{Db, LocalTrack};

/// A local file's synthetic videoId: this prefix + the absolute path.
pub const SONG_PREFIX: &str = "LOCAL:";
/// A local album's synthetic browseId: this prefix + `LocalTrack::album_key`.
pub const ALBUM_PREFIX: &str = "LOCALALBUM:";
/// A local artist's synthetic browseId: this prefix + the artist name exactly as tagged. No key or
/// digest — the name is the id, so the page finds its tracks by matching it back.
pub const ARTIST_PREFIX: &str = "LOCALARTIST:";
/// Where the folder list lives (JSON array of absolute paths).
const FOLDERS_SETTING: &str = "local_folders";
/// Bumped whenever `read_track` starts producing different titles, artists or album keys. The scan
/// normally trusts stored rows whose file hasn't changed; after a bump it re-reads everything once,
/// so a library isn't left half-parsed by the old rules and half by the new ones.
const SCAN_VERSION: &str = "4";
const SCAN_VERSION_SETTING: &str = "local_scan_version";

/// Extensions we pick up. Playback itself is mpv, which decodes far more than this — the list is
/// only about what a music folder plausibly holds, so a scan doesn't try to probe every stray file.
const AUDIO_EXT: [&str; 15] = [
    "mp3", "flac", "m4a", "m4b", "aac", "ogg", "oga", "opus", "wav", "wma", "aiff", "aif", "ape",
    "wv", "mka",
];
/// Cover images sitting next to the tracks, in preference order (used when nothing is embedded).
const COVER_FILES: [&str; 6] = [
    "cover.jpg",
    "cover.png",
    "folder.jpg",
    "folder.png",
    "front.jpg",
    "album.jpg",
];

/// What an untagged file's artist reads as. Shared so the scrobbler can refuse to send it: a
/// Last.fm profile full of "Unknown artist" is worse than a gap.
pub const UNKNOWN_ARTIST: &str = "Unknown artist";

/// How deep a folder tree is walked. Music/Artist/Album/Disc 1 is four; ten is room to spare
/// without letting a symlink cycle spin forever.
const MAX_DEPTH: usize = 10;

pub fn is_local_song(video_id: &str) -> bool {
    video_id.starts_with(SONG_PREFIX)
}

/// The file behind a local videoId, or `None` when this isn't one.
pub fn song_path(video_id: &str) -> Option<&str> {
    video_id.strip_prefix(SONG_PREFIX)
}

/// What the Local tab renders. `removed` is what vanished from disk since the last scan — the UI
/// prunes those ids out of Shortcuts and recents immediately instead of leaving dead tiles.
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLibrary {
    pub folders: Vec<String>,
    pub albums: Vec<BrowseItem>,
    pub artists: Vec<BrowseItem>,
    pub songs: Vec<SongItem>,
    pub removed: Vec<String>,
}

// --- folders ------------------------------------------------------------------------------------

pub fn folders(db: &Db) -> Vec<String> {
    db.get_setting(FOLDERS_SETTING)
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

fn set_folders(db: &Db, folders: &[String]) {
    db.set_setting(
        FOLDERS_SETTING,
        &serde_json::to_string(folders).unwrap_or_else(|_| "[]".into()),
    );
}

/// Add a folder (no-op if it's already watched, or nested inside one that is). Nesting is compared
/// by path component, not by string prefix — `\` separators and a trailing slash both make the
/// textual version wrong, and a missed nesting means every file in it is scanned twice.
pub fn add_folder(db: &Db, path: String) {
    let mut list = folders(db);
    let new = Path::new(&path).to_path_buf();
    if list.iter().any(|f| new.starts_with(f)) {
        return;
    }
    // A new parent supersedes the children already in the list.
    list.retain(|f| !Path::new(f).starts_with(&new));
    list.push(path);
    set_folders(db, &list);
}

pub fn remove_folder(db: &Db, path: &str) {
    let mut list = folders(db);
    list.retain(|f| f != path);
    set_folders(db, &list);
}

// --- scanning -----------------------------------------------------------------------------------

/// Re-read the watched folders and return the whole library. Files whose mtime hasn't moved keep
/// their stored tags, so a rescan of an unchanged collection is a stat() per file.
///
/// Blocking (disk IO + tag parsing) — call it from `spawn_blocking`.
pub fn scan(db: &Db, covers_dir: &Path) -> LocalLibrary {
    let before: HashSet<String> = card_ids_of(&db.local_tracks(None));
    let known = db.local_mtimes();
    let reparse = db.get_setting(SCAN_VERSION_SETTING).as_deref() != Some(SCAN_VERSION);
    let mut found: HashSet<String> = HashSet::new();
    let mut fresh: Vec<LocalTrack> = Vec::new();

    let mut seen_dirs: HashSet<PathBuf> = HashSet::new();
    for folder in folders(db) {
        walk(Path::new(&folder), 0, &mut seen_dirs, &mut |file| {
            let path = file.to_string_lossy().to_string();
            let mtime = mtime_of(file);
            found.insert(path.clone());
            if !reparse && known.get(&path) == Some(&mtime) {
                return; // unchanged since the last scan
            }
            fresh.push(read_track(file, &path, mtime, covers_dir));
        });
    }
    // One transaction for the whole scan: a per-file INSERT is a per-file fsync, which turns a
    // first scan of a few thousand tracks into minutes of waiting.
    db.put_local_tracks(&fresh);
    if reparse {
        db.set_setting(SCAN_VERSION_SETTING, SCAN_VERSION);
    }

    let gone: Vec<String> = known
        .keys()
        .filter(|p| !found.contains(*p))
        .cloned()
        .collect();
    if !gone.is_empty() {
        tracing::info!(
            count = gone.len(),
            "local files disappeared — dropped from the library"
        );
        db.delete_local_tracks(&gone);
    }

    let tracks = db.local_tracks(None);
    let after: HashSet<String> = card_ids_of(&tracks);
    // Ids the UI may still be showing: the deleted songs, plus the albums and artists that lost
    // their last track.
    let mut removed: Vec<String> = gone
        .into_iter()
        .map(|p| format!("{SONG_PREFIX}{p}"))
        .collect();
    removed.extend(before.difference(&after).cloned());

    LocalLibrary {
        folders: folders(db),
        albums: albums_of(&tracks),
        artists: artists_of(&tracks),
        songs: songs_of(&tracks),
        removed,
    }
}

/// Recurse into `dir`, calling `on_file` for every audio file. Errors (permissions, a folder that
/// was unplugged) are skipped rather than failing the scan.
///
/// Symlinks are followed — a music folder full of links to an external drive is a normal setup, and
/// refusing to follow them means those users see an empty library. `seen` (canonical paths) is what
/// keeps a link that points back up the tree from recursing forever; the depth cap is the backstop
/// for paths that can't be canonicalized.
fn walk(dir: &Path, depth: usize, seen: &mut HashSet<PathBuf>, on_file: &mut impl FnMut(&Path)) {
    if depth > MAX_DEPTH || !seen.insert(dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf())) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // `metadata` follows symlinks (unlike `entry.file_type`), so a linked album folder or a
        // linked file is treated as what it points at.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            walk(&path, depth + 1, seen, on_file);
        } else if meta.is_file() && is_audio(&path) {
            on_file(&path);
        }
    }
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXT.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn mtime_of(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Read one file's tags. Missing tags fall back to the filename and the parent folder, and a file
/// lofty can't parse at all is still listed from its filename — mpv plays far more formats than
/// any tag reader understands, and a track that silently never appears is worse than one with a
/// thin label.
fn read_track(file: &Path, path: &str, mtime: i64, covers_dir: &Path) -> LocalTrack {
    let tagged = lofty::probe::Probe::open(file)
        .ok()
        .and_then(|p| p.read().ok());
    let duration_secs = tagged
        .as_ref()
        .map(|t| t.properties().duration().as_secs() as i64)
        .unwrap_or(0);
    let tag = tagged
        .as_ref()
        .and_then(|t| t.primary_tag().or_else(|| t.first_tag()));
    let str_tag = |get: fn(&lofty::tag::Tag) -> Option<std::borrow::Cow<'_, str>>| {
        tag.and_then(get)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let title = str_tag(|t| t.title());
    let artist = str_tag(|t| t.artist());
    let album = str_tag(|t| t.album());
    // The album artist is what groups a compilation into one album instead of one per track.
    let album_artist = tag
        .and_then(|t| {
            t.get_string(&ItemKey::AlbumArtist)
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty());

    let dir = file.parent();
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    // Untagged files are the common case for anything not ripped by a tagger, and "Artist - Title"
    // is what those filenames almost always say. Only consulted when the tags are silent.
    let (from_name_artist, from_name_title) = split_filename(&stem);
    let title = title.unwrap_or(from_name_title);
    let artist = artist
        .or(from_name_artist)
        .unwrap_or_else(|| UNKNOWN_ARTIST.into());

    // With no album tag the folder is the album: loose files in one directory belong together
    // whoever performs them, so the key is the folder, not the folder *plus* the artist (which
    // would split "Downloads" into one album per artist).
    let tagged_album = album.is_some();
    let (album, album_key) = match album {
        Some(a) => {
            let key = album_key(album_artist.as_ref().unwrap_or(&artist), &a);
            (a, key)
        }
        None => {
            let name = dir
                .and_then(|d| d.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Local music".into());
            let key = folder_key(dir, &name);
            (name, key)
        }
    };
    // Artwork comes from the file itself, or from a real album it belongs to. A file with no album
    // tag has neither: it sits in a folder next to unrelated downloads, so neither the folder's
    // picture nor a neighbour's embedded art is its cover. Its own picture (cached per file, not
    // per pseudo-album) or nothing, and the UI draws a music note for nothing.
    let cover = if tagged_album {
        cover_for(&album_key, tag, dir, covers_dir)
    } else {
        embedded_cover(&format!("file-{}", short_digest(path)), tag, covers_dir)
    };

    LocalTrack {
        path: path.to_owned(),
        title,
        artist,
        album,
        album_key,
        track_no: tag.and_then(|t| t.track()).unwrap_or(0) as i64,
        duration_secs,
        cover,
        mtime,
    }
}

/// "PARTYNEXTDOOR, Drake - CN TOWER" → (artist, title). Splits on the first " - " only, so
/// "Artist - Song - Live" keeps the suffix on the title where it belongs. No separator means the
/// whole name is the title and nothing is claimed about the artist.
/// ponytail: one separator, no track-number stripping. Real tags beat any of this, so the parsing
/// only has to be better than "Unknown artist".
fn split_filename(stem: &str) -> (Option<String>, String) {
    match stem.split_once(" - ") {
        Some((artist, title)) if !artist.trim().is_empty() && !title.trim().is_empty() => {
            (Some(artist.trim().to_string()), title.trim().to_string())
        }
        _ => (None, stem.to_string()),
    }
}

/// Forget a single file the moment it turns out to be gone (a play attempt found nothing there).
/// Returns the ids the UI might still be showing: the song, plus its album when that was the last
/// track left in it. Same shape as a scan's `removed`, so both feed one prune on the UI side.
pub fn forget_missing(db: &Db, path: &str) -> Vec<String> {
    let all = db.local_tracks(None);
    db.delete_local_tracks(&[path.to_owned()]);
    let mut ids = vec![format!("{SONG_PREFIX}{path}")];
    let Some(gone) = all.iter().find(|t| t.path == path) else {
        return ids;
    };
    let others: Vec<&LocalTrack> = all.iter().filter(|t| t.path != path).collect();
    if !others.iter().any(|t| t.album_key == gone.album_key) {
        ids.push(album_id_of(gone));
    }
    if !others.iter().any(|t| t.artist == gone.artist) {
        ids.push(artist_id_of(gone));
    }
    ids
}

/// A stable, readable album id: `artist--album`, sanitized to a safe filename (it doubles as the
/// extracted cover's name). Deliberately not a hash — this id is persisted in Shortcuts, so it has
/// to survive across releases.
fn album_key(artist: &str, album: &str) -> String {
    let raw = format!("{artist}--{album}");
    let key = sanitize(&raw);
    // Long names get cut to fit a filename, and two box-set volumes can be identical for the first
    // 80 characters. A digest of the full name keeps them apart.
    if raw.chars().count() > KEY_CHARS {
        format!("{key}-{}", short_digest(&raw))
    } else {
        key
    }
}

/// Key length cap: long enough to stay readable, short enough for every filesystem.
const KEY_CHARS: usize = 80;

/// The key for files with no album tag: the folder is the album. Includes a digest of the full
/// path, because two different "Downloads" folders are two different albums.
fn folder_key(dir: Option<&Path>, name: &str) -> String {
    let path = dir
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();
    format!("dir-{}-{}", short_digest(&path), sanitize(name))
}

/// Lowercase, keep letters and digits (any script — collapsing non-ASCII would make every Japanese
/// album the same id), everything else becomes `-`. All the characters that survive are legal in a
/// filename on every platform we ship to.
fn sanitize(raw: &str) -> String {
    raw.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        // `char`s, not bytes: truncating a String mid-codepoint panics.
        .take(KEY_CHARS)
        .collect()
}

/// Short, stable hash of a string (FNV-1a, hex). Only used to keep same-named folders apart, so
/// it needs to be deterministic across releases — which `DefaultHasher` explicitly is not.
fn short_digest(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    // Low 48 bits: exactly 12 hex digits, so a key's length is predictable.
    format!("{:012x}", h & 0xffff_ffff_ffff)
}

/// The album's cover: the first embedded picture (written once into `covers_dir`), else an image
/// sitting next to the tracks. Extraction is skipped when the file is already there, so a rescan
/// doesn't rewrite artwork.
fn cover_for(
    album_key: &str,
    tag: Option<&lofty::tag::Tag>,
    dir: Option<&Path>,
    covers_dir: &Path,
) -> Option<String> {
    embedded_cover(album_key, tag, covers_dir).or_else(|| {
        let dir = dir?;
        COVER_FILES
            .iter()
            .map(|name| dir.join(name))
            .find(|p| p.exists())
            .map(|p| p.to_string_lossy().to_string())
    })
}

/// The picture inside the file, written once into `covers_dir` under `key` and reused after that.
/// `key` is the album for real albums (one extraction per album) and the file for everything else.
fn embedded_cover(key: &str, tag: Option<&lofty::tag::Tag>, covers_dir: &Path) -> Option<String> {
    // The extension matters: the webview gets these over Tauri's asset protocol, which types the
    // response from the file name.
    for ext in ["jpg", "png"] {
        let cached = covers_dir.join(format!("{key}.{ext}"));
        if cached.exists() {
            return Some(cached.to_string_lossy().to_string());
        }
    }
    let pic = tag.and_then(|t| t.pictures().first())?;
    let ext = match pic.mime_type() {
        Some(lofty::picture::MimeType::Png) => "png",
        _ => "jpg",
    };
    let out = covers_dir.join(format!("{key}.{ext}"));
    std::fs::create_dir_all(covers_dir).ok();
    std::fs::write(&out, pic.data())
        .ok()
        .map(|_| out.to_string_lossy().to_string())
}

// --- shaping into the app's models ----------------------------------------------------------

fn album_id_of(t: &LocalTrack) -> String {
    format!("{ALBUM_PREFIX}{}", t.album_key)
}

fn artist_id_of(t: &LocalTrack) -> String {
    format!("{ARTIST_PREFIX}{}", primary_artist(&t.artist))
}

/// Every browseId a set of tracks puts on screen. A scan diffs this before and after to find the
/// cards that lost their last file, so Shortcuts tiles pointing at them go at the same moment.
fn card_ids_of(tracks: &[LocalTrack]) -> HashSet<String> {
    tracks
        .iter()
        .flat_map(|t| [album_id_of(t), artist_id_of(t)])
        .collect()
}

pub fn to_song(t: &LocalTrack) -> SongItem {
    SongItem {
        video_id: format!("{SONG_PREFIX}{}", t.path),
        title: t.title.clone(),
        artists: t.artist.clone(),
        album: Some(t.album.clone()),
        // No `album_id`: that field is what puts "Go to album" in a track's ⋯ menu, and a file on
        // this disk has no album page worth offering there. The Local tab's album grid is how you
        // get to one. The album *name* stays — scrobbling and lyrics matching both use it.
        album_id: None,
        // 0 means "the tag reader couldn't say"; mpv fills the real length in once it plays.
        duration: (t.duration_secs > 0).then(|| fmt_duration(t.duration_secs)),
        thumbnail: t.cover.clone(),
        ..Default::default()
    }
}

fn songs_of(tracks: &[LocalTrack]) -> Vec<SongItem> {
    tracks.iter().map(to_song).collect()
}

/// One card per album, sorted by title.
fn albums_of(tracks: &[LocalTrack]) -> Vec<BrowseItem> {
    let mut groups: HashMap<&str, Vec<&LocalTrack>> = HashMap::new();
    for t in tracks {
        groups.entry(t.album_key.as_str()).or_default().push(t);
    }
    let mut albums: Vec<BrowseItem> = groups
        .values()
        .map(|ts| {
            let n = ts.len();
            // The card's face is the first track that actually has artwork.
            let face = ts.iter().find(|t| t.cover.is_some()).unwrap_or(&ts[0]);
            BrowseItem {
                kind: "album",
                id: album_id_of(face),
                title: face.album.clone(),
                subtitle: Some(format!(
                    "{} • {} song{}",
                    album_artist(ts),
                    n,
                    if n == 1 { "" } else { "s" }
                )),
                thumbnail: face.cover.clone(),
                duration: None,
                artist_runs: Vec::new(),
                is_video: false,
            }
        })
        .collect();
    albums.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    albums
}

/// Who a track counts under. A tag naming a collaboration ("Drake; 21 Savage", "Drake feat.
/// Future") belongs to whoever it names first: a circle per guest list is one-song noise, and the
/// track's row still shows the whole line.
///
/// ponytail: splits on ";" and the feat. markers only. "," and "&" are left alone on purpose —
/// they sit inside real names ("Tyler, The Creator", "Earth, Wind & Fire") often enough that
/// splitting on them invents worse artists than it merges.
fn primary_artist(artist: &str) -> &str {
    let mut cut = artist.find(';').unwrap_or(artist.len());
    let lower = artist.to_lowercase();
    for marker in [" feat.", " feat ", " ft.", " ft ", " featuring "] {
        if let Some(i) = lower.find(marker) {
            cut = cut.min(i);
        }
    }
    // `get`, not a slice: lowercasing can shift byte offsets (İ and friends), and the safe way to
    // be wrong is to leave the name whole.
    artist.get(..cut).unwrap_or(artist).trim()
}

/// One circle per artist, sorted by name.
fn artists_of(tracks: &[LocalTrack]) -> Vec<BrowseItem> {
    let mut groups: HashMap<&str, Vec<&LocalTrack>> = HashMap::new();
    for t in tracks {
        groups.entry(primary_artist(&t.artist)).or_default().push(t);
    }
    let mut artists: Vec<BrowseItem> = groups
        .iter()
        .map(|(name, ts)| {
            let n = ts.len();
            BrowseItem {
                kind: "artist",
                id: format!("{ARTIST_PREFIX}{name}"),
                title: (*name).to_string(),
                subtitle: Some(format!("{} song{}", n, if n == 1 { "" } else { "s" })),
                // The face is the first track that has artwork — an artist has no portrait of
                // their own on disk, so a cover from their music is the closest thing.
                thumbnail: ts.iter().find_map(|t| t.cover.clone()),
                duration: None,
                artist_runs: Vec::new(),
                is_video: false,
            }
        })
        .collect();
    artists.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    artists
}

/// Who an album is by: the one artist on it, or "Various artists". A folder of loose downloads is
/// grouped as one album (see `read_track`), and claiming it belongs to whichever track happened to
/// be first would be wrong on most of its rows.
fn album_artist(tracks: &[&LocalTrack]) -> String {
    let first = tracks
        .first()
        .map(|t| t.artist.as_str())
        .unwrap_or(UNKNOWN_ARTIST);
    if tracks.iter().all(|t| t.artist == first) {
        first.to_string()
    } else {
        "Various artists".to_string()
    }
}

/// The album page for a local album, in the same shape the YouTube one uses so the route is shared.
pub fn album_page(db: &Db, album_key: &str) -> AlbumPage {
    let tracks = db.local_tracks(Some(album_key));
    let title = tracks.first().map(|t| t.album.clone());
    let artist = (!tracks.is_empty()).then(|| album_artist(&tracks.iter().collect::<Vec<_>>()));
    page_of(title, artist, &tracks)
}

/// One local artist's page: everything of theirs on this disk, grouped by album (the query's own
/// order). It renders through the *album* route, because a file on disk has no YouTube channel
/// behind it — nothing the real artist page offers (subscribe, radio, related) applies.
pub fn artist_page(db: &Db, name: &str) -> AlbumPage {
    let tracks: Vec<LocalTrack> = db
        .local_tracks(None)
        .into_iter()
        .filter(|t| primary_artist(&t.artist) == name)
        .collect();
    page_of(Some(name.to_owned()), None, &tracks)
}

/// The shared body of both: a title, a track list, and nothing YouTube-shaped.
fn page_of(title: Option<String>, artist: Option<String>, tracks: &[LocalTrack]) -> AlbumPage {
    let total: i64 = tracks.iter().map(|t| t.duration_secs).sum();
    AlbumPage {
        title,
        artist,
        artist_id: None,
        artist_runs: Vec::new(),
        artist_thumbnail: None,
        subtitle: Some("On this device".into()),
        second_subtitle: Some(format!(
            "{} song{} • {}",
            tracks.len(),
            if tracks.len() == 1 { "" } else { "s" },
            fmt_duration(total)
        )),
        description: None,
        thumbnail: tracks.iter().find_map(|t| t.cover.clone()),
        items: songs_of(&tracks),
        continuation: None,
        // No YouTube playlist behind it: no radio to seed, nothing to save to the library.
        playlist_id: None,
        in_library: false,
    }
}

/// Seconds → "3:47" / "1:08:20".
fn fmt_duration(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Everything the player needs for a local file. The "stream url" is just the path — mpv opens it
/// directly, so playback works with no network at all. Errs when the file is gone (the user
/// deleted it since the scan): the queue skips it like any other unplayable track.
pub fn playback_data(video_id: &str, path: &str) -> Result<crate::orchestrator::PlaybackData, ()> {
    if !Path::new(path).is_file() {
        return Err(());
    }
    Ok(crate::orchestrator::PlaybackData {
        video_id: video_id.to_owned(),
        stream_url: path.to_owned(),
        itag: 0,
        headers: HashMap::new(),
        // Never expires, and never enters the URL cache (see `AppState::resolve`).
        expires_in_seconds: i64::MAX / 2,
        // No loudness metadata in the general case; mpv plays the file as mastered.
        loudness_db: None,
        playback_url: None,
        title: None,
        artists: None,
        duration: None,
        thumbnail: None,
        stream_client: "local".to_owned(),
    })
}

/// Let the webview fetch exactly the artwork we hand it, and nothing else.
///
/// The asset protocol's static scope is empty on purpose: it can't name folders the user picks at
/// runtime, and the usual `"**"` shortcut would put every file on the machine behind a URL the page
/// could fetch. Both the stored path and its canonical form are allowed, because the scope check
/// canonicalizes what it is asked about — without that, a music folder that is a symlink to an
/// external drive gets its covers refused.
pub fn allow_covers(app: &tauri::AppHandle, songs: &[SongItem]) {
    use tauri::Manager;
    let scope = app.asset_protocol_scope();
    let mut seen: HashSet<&str> = HashSet::new();
    for cover in songs.iter().filter_map(|s| s.thumbnail.as_deref()) {
        if !seen.insert(cover) {
            continue;
        }
        let _ = scope.allow_file(cover);
        if let Ok(real) = Path::new(cover).canonicalize() {
            let _ = scope.allow_file(real);
        }
    }
}

/// Allow the watched folders and the covers directory at startup, before the window loads. Without
/// it, a restored queue whose current track is a local file asks for its artwork before the first
/// scan has finished and gets a 403 it never retries.
pub fn allow_music_paths(app: &tauri::AppHandle, db: &Db) {
    use tauri::Manager;
    let scope = app.asset_protocol_scope();
    for dir in folders(db)
        .into_iter()
        .chain([covers_dir(app).to_string_lossy().to_string()])
    {
        let _ = scope.allow_directory(&dir, true);
        if let Ok(real) = Path::new(&dir).canonicalize() {
            let _ = scope.allow_directory(real, true);
        }
    }
}

/// The covers directory, alongside the SQLite file (not inside the audio cache — "Clear caches"
/// must not wipe artwork that only a full re-tag would regenerate).
pub fn covers_dir(app: &tauri::AppHandle) -> PathBuf {
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("covers")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: the fields a grouping test cares about, the rest defaulted.
    fn track(path: &str, artist: &str, album: &str, key: &str) -> LocalTrack {
        LocalTrack {
            path: path.into(),
            title: path.into(),
            artist: artist.into(),
            album: album.into(),
            album_key: key.into(),
            track_no: 1,
            duration_secs: 10,
            cover: None,
            mtime: 1,
        }
    }

    #[test]
    fn album_key_is_stable_and_filename_safe() {
        let k = album_key("Daft Punk", "Discovery / Deluxe");
        assert_eq!(k, "daft-punk--discovery---deluxe");
        assert_eq!(
            album_key("Daft Punk", "Discovery / Deluxe"),
            k,
            "same album, same key"
        );
        assert!(!k.contains('/'), "the key doubles as the cover's filename");

        // Non-Latin titles have to stay distinct: mapping them all to `-` merged every Japanese
        // album into one, and truncating mid-codepoint used to be a panic waiting to happen.
        let a = album_key("米津玄師", "アルバムA");
        let b = album_key("米津玄師", "アルバムB");
        assert_ne!(a, b, "two Japanese albums are two albums");
        let long = album_key("x", &"あ".repeat(200));
        assert!(
            long.chars().count() <= KEY_CHARS + 16,
            "keys stay filename-sized"
        );
        // Two albums that only differ past the cut are still two albums.
        assert_ne!(
            album_key("x", &format!("{} one", "long ".repeat(30))),
            album_key("x", &format!("{} two", "long ".repeat(30)))
        );

        // Two folders called "Downloads" are two different albums.
        assert_ne!(
            folder_key(Some(Path::new("/a/Downloads")), "Downloads"),
            folder_key(Some(Path::new("/b/Downloads")), "Downloads")
        );
    }

    #[test]
    fn a_local_song_offers_no_album_to_go_to() {
        let s = to_song(&track("/m/x/a.mp3", "Drake", "Views", "drake--views"));
        assert_eq!(
            s.album.as_deref(),
            Some("Views"),
            "the name still travels (scrobbles, lyrics)"
        );
        assert_eq!(
            s.album_id, None,
            "but nothing for the ⋯ menu to navigate to"
        );
    }

    #[test]
    fn a_folder_of_loose_files_is_one_album_by_various_artists() {
        // No album tag → the folder groups them, whoever the per-file artist is. Splitting on
        // artist made "My music" show up once per singer, each with one track.
        let (_, key_a) = (0, folder_key(Some(Path::new("/m/My music")), "My music"));
        let tracks = vec![
            track("/m/My music/a.mp3", "Drake", "My music", &key_a),
            track("/m/My music/b.mp3", "PARTYNEXTDOOR", "My music", &key_a),
        ];
        let albums = albums_of(&tracks);
        assert_eq!(albums.len(), 1, "one folder, one album");
        assert_eq!(
            albums[0].subtitle.as_deref(),
            Some("Various artists • 2 songs")
        );

        let same = vec![track("/m/x/a.mp3", "Drake", "Views", "drake--views")];
        assert_eq!(
            albums_of(&same)[0].subtitle.as_deref(),
            Some("Drake • 1 song")
        );
    }

    #[test]
    fn a_collaboration_counts_under_the_first_artist() {
        // A guest list is not an artist: all four of these are Drake's, and the featured names get
        // no circle of their own. Commas stay put, so "Tyler, The Creator" survives whole.
        let tracks = vec![
            track("/m/a.mp3", "Drake", "Views", "drake--views"),
            track("/m/b.mp3", "Drake", "Views", "drake--views"),
            track("/m/c.mp3", "Drake; 21 Savage", "Views", "drake--views"),
            track("/m/d.mp3", "Drake Feat. Future", "Views", "drake--views"),
            track("/m/e.mp3", "Tyler, The Creator", "Igor", "tyler--igor"),
        ];
        let artists = artists_of(&tracks);
        assert_eq!(
            artists.len(),
            2,
            "Drake and Tyler — no 21 Savage, no Future"
        );
        assert_eq!(
            artists[0].kind, "artist",
            "which is what draws it as a circle"
        );
        assert_eq!(artists[0].title, "Drake");
        assert_eq!(artists[0].subtitle.as_deref(), Some("4 songs"));
        assert_eq!(artists[1].title, "Tyler, The Creator");

        // The id is the name, and it has to find its way back to exactly those files.
        let db = Db::open(Path::new(":memory:")).unwrap();
        db.put_local_tracks(&tracks);
        let page = artist_page(&db, artists[0].id.strip_prefix(ARTIST_PREFIX).unwrap());
        assert_eq!(page.title.as_deref(), Some("Drake"));
        assert_eq!(
            page.items.len(),
            4,
            "the page picks up the collaborations too"
        );
        assert_eq!(
            page.playlist_id, None,
            "nothing YouTube-shaped on a page built from disk"
        );
    }

    #[test]
    fn untagged_filenames_give_up_an_artist_and_title() {
        assert_eq!(
            split_filename("PARTYNEXTDOOR, Drake - CN TOWER"),
            (Some("PARTYNEXTDOOR, Drake".into()), "CN TOWER".into())
        );
        // Only the first separator splits — the rest belongs to the title.
        assert_eq!(
            split_filename("Artist - Song - Live"),
            (Some("Artist".into()), "Song - Live".into())
        );
        // Nothing claimed when there's no separator, or when either side would be empty.
        assert_eq!(split_filename("CN TOWER"), (None, "CN TOWER".into()));
        assert_eq!(split_filename(" - CN TOWER"), (None, " - CN TOWER".into()));
        // A hyphen that isn't a separator stays put.
        assert_eq!(split_filename("Jay-Z"), (None, "Jay-Z".into()));
    }

    #[test]
    fn a_file_with_no_album_never_borrows_artwork() {
        let dir = std::env::temp_dir().join("limusic-cover-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cover.jpg"), b"not really a jpeg").unwrap();
        let covers = dir.join("covers");

        // A real album takes the picture sitting next to its tracks.
        assert!(cover_for("band--album", None, Some(dir.as_path()), &covers).is_some());
        // A file with no album tag and no picture of its own gets nothing — not the folder's
        // cover.jpg, and not whatever a neighbouring download extracted earlier.
        assert_eq!(embedded_cover("file-deadbeef", None, &covers), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn durations_format_like_the_rest_of_the_app() {
        assert_eq!(fmt_duration(227), "3:47");
        assert_eq!(fmt_duration(4100), "1:08:20");
        assert_eq!(fmt_duration(5), "0:05");
    }

    #[test]
    fn folders_ignore_nesting_in_both_directions() {
        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        add_folder(&db, "/music".into());
        add_folder(&db, "/music/rock".into()); // already covered by the parent
        assert_eq!(folders(&db), vec!["/music".to_string()]);
        add_folder(&db, "/other/jazz".into());
        add_folder(&db, "/other".into()); // supersedes the child
        assert_eq!(
            folders(&db),
            vec!["/music".to_string(), "/other".to_string()]
        );
        remove_folder(&db, "/music");
        assert_eq!(folders(&db), vec!["/other".to_string()]);
    }

    #[test]
    fn a_scan_reports_what_left_the_disk() {
        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        let dir = std::env::temp_dir().join("limusic-local-scan-test");
        std::fs::create_dir_all(&dir).unwrap();
        add_folder(&db, dir.to_string_lossy().to_string());
        // Two tracks the library "knows" about; neither file exists any more.
        for (n, key) in [("a", "band--one"), ("b", "band--two")] {
            let path = dir.join(format!("{n}.mp3")).to_string_lossy().to_string();
            db.put_local_tracks(&[track(&path, "Band", n, key)]);
        }

        let lib = scan(&db, &dir.join("covers"));
        assert!(
            lib.songs.is_empty() && lib.albums.is_empty(),
            "the rows are gone from the library"
        );
        assert_eq!(
            lib.removed.len(),
            5,
            "two songs, two albums and the one artist are removed"
        );
        assert!(
            lib.removed.contains(&format!("{ARTIST_PREFIX}Band")),
            "the artist's circle too"
        );
        assert!(
            lib.removed.iter().any(|id| id.ends_with("a.mp3")),
            "the song id the UI knows"
        );
        assert!(
            lib.removed.contains(&format!("{ALBUM_PREFIX}band--one")),
            "and the album id, so its Shortcuts tile can go too"
        );
        assert!(
            scan(&db, &dir.join("covers")).removed.is_empty(),
            "a second scan reports nothing"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn forgetting_one_file_only_reports_the_album_when_it_empties() {
        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        db.put_local_tracks(&[
            track("/m/a.mp3", "Band", "Album", "band--album"),
            track("/m/b.mp3", "Band", "Album", "band--album"),
        ]);

        assert_eq!(
            forget_missing(&db, "/m/a.mp3"),
            vec!["LOCAL:/m/a.mp3".to_string()],
            "the album still has a track, so only the song id is reported"
        );
        assert_eq!(
            forget_missing(&db, "/m/b.mp3"),
            vec![
                "LOCAL:/m/b.mp3".to_string(),
                "LOCALALBUM:band--album".to_string(),
                "LOCALARTIST:Band".to_string()
            ],
            "the last track takes the album and the artist with it"
        );
        assert!(db.local_tracks(None).is_empty(), "and both rows are gone");
        assert!(
            forget_missing(&db, "/m/a.mp3").len() == 1,
            "forgetting twice is harmless"
        );
    }

    #[test]
    fn a_missing_file_is_not_playable() {
        assert!(playback_data("LOCAL:/nope/gone.mp3", "/nope/gone.mp3").is_err());
    }
}
