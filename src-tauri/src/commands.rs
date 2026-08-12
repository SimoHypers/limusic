//! Tauri commands — the ONLY API the UI calls. context/11 UI contract. No YouTube shapes leak
//! past here; the UI never sees a stream URL.

use std::sync::Arc;

use innertube::{
    AlbumPage, ArtistPage, BrowseItem, HomePage, PlaylistContinuation, PlaylistPage, SearchResults,
    SongItem,
};
use tauri::{Emitter, State};

use crate::state::{AppState, ON_REPEAT_ID, ON_REPEAT_LIMIT, ON_REPEAT_WINDOW_SECS};

type St<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub async fn search(state: St<'_>, query: String) -> Result<Vec<SongItem>, String> {
    let client = state
        .clients
        .get(innertube::METADATA_CLIENT)
        .ok_or("metadata client missing")?;
    let result = state
        .it
        .search_songs(client, &query)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.items)
}

/// Unfiltered search → categorized sections for the search page.
#[tauri::command]
pub async fn search_all(state: St<'_>, query: String) -> Result<SearchResults, String> {
    let client = metadata_client(&state)?;
    state
        .it
        .search_all(client, &query)
        .await
        .map_err(|e| e.to_string())
}

/// Filtered "Show more" search for one category (albums / artists / playlists).
#[tauri::command]
pub async fn search_cards(
    state: St<'_>,
    query: String,
    category: String,
) -> Result<Vec<BrowseItem>, String> {
    let client = metadata_client(&state)?;
    state
        .it
        .search_cards(client, &query, &category)
        .await
        .map_err(|e| e.to_string())
}

/// Play a track (from a search result). The UI passes the full item so we can seed the queue
/// with its metadata without another round-trip.
#[tauri::command]
pub async fn play(state: St<'_>, item: SongItem) -> Result<(), String> {
    let state = state.inner().clone();
    state.play_song(item).await;
    Ok(())
}

#[tauri::command]
pub async fn play_index(state: St<'_>, index: usize) -> Result<(), String> {
    let state = state.inner().clone();
    state.play_index(index).await;
    Ok(())
}

/// Remove an upcoming track from the queue (not the one playing). Guests are add-only — blocked
/// inside AppState.
#[tauri::command]
pub async fn remove_from_queue(state: St<'_>, index: usize) -> Result<(), String> {
    state.inner().clone().remove_from_queue(index).await;
    Ok(())
}

/// "Play next" from a ⋯ menu: one track or a whole album/playlist, inserted right after the
/// current song (behind any earlier manual adds). `from` is the album/playlist title, which heads
/// the block in the queue panel.
#[tauri::command]
pub async fn play_next(
    state: St<'_>,
    items: Vec<SongItem>,
    from: Option<String>,
) -> Result<(), String> {
    state.inner().clone().play_next(items, from).await;
    Ok(())
}

/// "Add to queue": the tracks go after everything the user picked, and ahead of anything the app
/// generated behind it (autoplay filler, or a radio's endless feed). `from` heads the block in the
/// queue panel; `continuation` is the source page's next-page token — the rest of a long playlist
/// is walked in in the background.
#[tauri::command]
pub async fn add_to_queue(
    state: St<'_>,
    items: Vec<SongItem>,
    from: Option<String>,
    continuation: Option<String>,
) -> Result<(), String> {
    state
        .inner()
        .clone()
        .add_to_queue(items, from, continuation)
        .await;
    Ok(())
}

/// Clear every upcoming manually-queued track (the queue panel's "Next in queue" section).
#[tauri::command]
pub async fn clear_queued(state: St<'_>) -> Result<(), String> {
    state.inner().clone().clear_queued().await;
    Ok(())
}

#[tauri::command]
pub async fn next_track(state: St<'_>) -> Result<(), String> {
    state.inner().clone().next_in_queue().await;
    Ok(())
}

#[tauri::command]
pub async fn prev_track(state: St<'_>) -> Result<(), String> {
    state.inner().clone().prev_in_queue().await;
    Ok(())
}

#[tauri::command]
pub async fn toggle_shuffle(state: St<'_>) -> Result<(), String> {
    state.inner().clone().toggle_shuffle().await;
    Ok(())
}

/// `mode` ∈ "off" | "all" | "one".
#[tauri::command]
pub async fn set_repeat(state: St<'_>, mode: String) -> Result<(), String> {
    let mode = match mode.as_str() {
        "off" => crate::state::RepeatMode::Off,
        "all" => crate::state::RepeatMode::All,
        "one" => crate::state::RepeatMode::One,
        other => return Err(format!("unknown repeat mode: {other}")),
    };
    state.inner().clone().set_repeat(mode).await;
    Ok(())
}

#[tauri::command]
pub async fn toggle_pause(state: St<'_>) -> Result<(), String> {
    let state = state.inner().clone();
    state.resume_or_toggle().await;
    Ok(())
}

#[tauri::command]
pub async fn seek(state: St<'_>, position: f64) -> Result<(), String> {
    // Routed through AppState so a Listen Together host broadcasts the seek and a guest is blocked.
    state.user_seek(position).await
}

#[tauri::command]
pub async fn set_volume(state: St<'_>, volume: i64) -> Result<(), String> {
    state.player.set_volume(volume).map_err(|e| e.to_string())?;
    // There is one volume and there can be two windows (the mini player). Without this the one
    // that didn't move the slider keeps showing the old level and lies about what you're hearing.
    let _ = state.app.emit("volume", volume);
    Ok(())
}

#[tauri::command]
pub async fn get_queue(state: St<'_>) -> Result<serde_json::Value, String> {
    Ok(state.queue_snapshot().await)
}

/// Settings the UI is allowed to read *and write*. Session/auth material (`session_cookie`,
/// `data_sync_id`, `account_json`, `visitor_data`) and internal blobs (`queue_json`,
/// `queue_position`) never cross into the webview — they'd otherwise ship the login credential to
/// the renderer on every open — and the webview can't overwrite them either.
const UI_SETTINGS: [&str; 12] = [
    "volume",
    "proxy",
    "quality",
    "enable_history",
    "disabled_stream_clients",
    "discord_rpc",
    "close_to_tray",
    "autostart",
    "autoplay",
    "hide_videos",
    "prevent_duplicates",
    "update_banner",
];

#[tauri::command]
pub async fn get_settings(state: St<'_>) -> Result<serde_json::Value, String> {
    let map: serde_json::Map<String, serde_json::Value> = state
        .db
        .all_settings()
        .into_iter()
        .filter(|(k, _)| UI_SETTINGS.contains(&k.as_str()))
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    Ok(serde_json::Value::Object(map))
}

#[tauri::command]
pub async fn set_setting(
    app: tauri::AppHandle,
    state: St<'_>,
    key: String,
    value: String,
) -> Result<(), String> {
    if !UI_SETTINGS.contains(&key.as_str()) {
        return Err(format!("unknown setting: {key}"));
    }
    state.db.set_setting(&key, &value);
    // Presence connects/clears the moment it's toggled — the user shouldn't have to skip a track
    // to see it take effect.
    if key == "discord_rpc" {
        state.set_discord_enabled(value == "true");
    }
    // Applies to what's fetched from here on: the live queue keeps whatever is already in it.
    if key == "hide_videos" {
        state.it.set_hide_videos(value == "true");
    }
    // Registers/removes the login autostart entry on toggle; the OS persists it from there.
    // ponytail: no startup re-sync against the OS state — add reconciliation only if drift is
    // ever reported.
    if key == "autostart" {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        let res = if value == "true" {
            al.enable()
        } else if al.is_enabled().unwrap_or(false) {
            al.disable()
        } else {
            Ok(())
        };
        res.map_err(|e| format!("autostart: {e}"))?;
    }
    Ok(())
}

/// The streamable client keys the orchestrator tries, for the "disabled clients" setting. Names
/// come from the innertube crate so the UI stays free of YouTube-shaped identity strings.
#[tauri::command]
pub async fn get_stream_clients() -> Result<Vec<String>, String> {
    let mut v = vec![innertube::MAIN_CLIENT.to_string()];
    v.extend(
        innertube::STREAM_FALLBACK_ORDER
            .iter()
            .map(|s| s.to_string()),
    );
    Ok(v)
}

/// Let the webview fetch one font file the user picked in the Themes tab, so a `@font-face` can
/// point at it.
///
/// Same runtime-scope trick as local artwork (`local::allow_covers`): the static asset scope stays
/// empty, and only the exact file gets a URL. The extension check keeps the command from being a
/// general "give the page a URL for any path on this machine" — today only the main window holds a
/// capability to call commands at all, and this stays safe if that ever widens.
#[tauri::command]
pub async fn allow_font_file(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri::Manager;
    const FONT_EXTS: [&str; 4] = ["ttf", "otf", "woff", "woff2"];
    let p = std::path::Path::new(&path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !FONT_EXTS.contains(&ext.as_str()) {
        return Err(format!("not a font file: {path}"));
    }
    // Scope grants succeed for paths that don't exist, so check here: this failing is how the UI
    // learns a loaded font was deleted or moved, and drops it instead of listing a dead entry.
    if !p.is_file() {
        return Err(format!("font file not found: {path}"));
    }
    let scope = app.asset_protocol_scope();
    scope.allow_file(&path).map_err(|e| e.to_string())?;
    // The scope check canonicalizes what it is asked about, so a font reached through a symlinked
    // folder needs the real path allowed too (see local::allow_covers).
    if let Ok(real) = p.canonicalize() {
        let _ = scope.allow_file(real);
    }
    Ok(())
}

/// Wipe both cache tiers (URL cache + mpv on-disk audio cache). context/14.
#[tauri::command]
pub async fn clear_caches(state: St<'_>) -> Result<(), String> {
    state.clear_caches();
    Ok(())
}

// --- auth (context/15) ---------------------------------------------------------------------

#[tauri::command]
pub async fn get_account(state: St<'_>) -> Result<serde_json::Value, String> {
    Ok(state.account_snapshot())
}

#[tauri::command]
pub async fn sign_out(state: St<'_>) -> Result<(), String> {
    let state = state.inner().clone();
    state.sign_out().await;
    Ok(())
}

/// Open the in-app Google sign-in webview (context/15 Path A). Completes asynchronously; the UI
/// hears back via `auth-changed` (success) or `login-error`.
#[tauri::command]
pub async fn login_webview(state: St<'_>) -> Result<(), String> {
    let state = state.inner().clone();
    let app = state.app.clone();
    crate::session::open_login(app, state);
    Ok(())
}

/// The current track, play state, position and duration in one shot. Events are the normal
/// channel; this is for a webview that started after them (the mini player, or the main window
/// on a cold start, where the queue is restored before the UI subscribes).
#[tauri::command]
pub async fn get_playback(state: St<'_>) -> Result<serde_json::Value, String> {
    Ok(state.playback_snapshot().await)
}

// --- mini player (mini.rs) ------------------------------------------------------------------

/// Swap the app for the floating widget: the main window hides to the tray behind it.
#[tauri::command]
pub async fn open_mini(app: tauri::AppHandle) -> Result<(), String> {
    // GTK wants window creation on the main thread, so hop and post the result back rather than
    // logging a failure the user would only see as a click that did nothing.
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(crate::mini::open(&handle));
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "the mini player never answered".to_string())?
}

/// Swap back. Same path as the tray, so the widget and the tray can't disagree about what
/// "show Limusic" means.
#[tauri::command]
pub async fn close_mini(app: tauri::AppHandle) -> Result<(), String> {
    crate::tray::show_main(&app);
    Ok(())
}

// --- browse / library (context/08) ---------------------------------------------------------

fn metadata_client(state: &Arc<AppState>) -> Result<&innertube::YouTubeClient, String> {
    state
        .clients
        .get(innertube::METADATA_CLIENT)
        .ok_or_else(|| "metadata client missing".into())
}

#[tauri::command]
pub async fn get_home(state: St<'_>, params: Option<String>) -> Result<HomePage, String> {
    let client = metadata_client(&state)?;
    state
        .it
        .home(client, params.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_home_more(state: St<'_>, token: String) -> Result<HomePage, String> {
    let client = metadata_client(&state)?;
    state
        .it
        .home_continuation(client, &token)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_library(state: St<'_>) -> Result<Vec<BrowseItem>, String> {
    let client = metadata_client(&state)?;
    let mut items = state
        .it
        .library_playlists(client)
        .await
        .map_err(|e| e.to_string())?;
    // On Repeat leads the library once there's anything in it. Hidden while empty rather than
    // shown as a dead tile on a fresh install.
    let songs = on_repeat_songs(&state);
    if !songs.is_empty() {
        items.insert(
            0,
            BrowseItem {
                kind: "playlist",
                id: ON_REPEAT_ID.into(),
                title: "On Repeat".into(),
                subtitle: Some(format!("{} songs", songs.len())),
                thumbnail: None, // the UI draws an icon cover for this one
                duration: None,
                artist_runs: Vec::new(),
                is_video: false,
            },
        );
    }
    Ok(items)
}

#[tauri::command]
pub async fn get_library_albums(state: St<'_>) -> Result<Vec<BrowseItem>, String> {
    let client = metadata_client(&state)?;
    state
        .it
        .library_albums(client)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_library_artists(state: St<'_>) -> Result<Vec<BrowseItem>, String> {
    let client = metadata_client(&state)?;
    state
        .it
        .library_artists(client)
        .await
        .map_err(|e| e.to_string())
}

/// A playlist or album page. `id` is the browseId (`VL…` / `MPRE…`); Liked Songs is `VLLM`, and
/// `LIMUSIC_ON_REPEAT` is the local auto-playlist rather than anything YouTube knows about.
#[tauri::command]
pub async fn get_playlist(state: St<'_>, id: String) -> Result<PlaylistPage, String> {
    if id == ON_REPEAT_ID {
        let items = on_repeat_songs(&state);
        return Ok(PlaylistPage {
            title: Some("On Repeat".into()),
            subtitle: Some(format!(
                "{} songs you've played most this month",
                items.len()
            )),
            thumbnail: None,
            items,
            continuation: None,
            owned: false, // nothing to rename or delete; it rebuilds itself from what you play
        });
    }
    let client = metadata_client(&state)?;
    state
        .it
        .playlist(client, &id)
        .await
        .map_err(|e| e.to_string())
}

/// The On Repeat track list: most-played first, over the trailing window. Rows whose stored JSON
/// no longer parses (a `SongItem` shape change) are dropped rather than failing the whole page.
fn on_repeat_songs(state: &Arc<AppState>) -> Vec<SongItem> {
    let since = now_secs() - ON_REPEAT_WINDOW_SECS;
    state
        .db
        .top_plays(since, ON_REPEAT_LIMIT)
        .into_iter()
        .filter_map(|(json, _plays)| serde_json::from_str(&json).ok())
        .map(shed_queue_context)
        .collect()
}

/// A play record is the whole `SongItem` as it sat in the queue, so it carries that slot's queue
/// metadata: `queued`/`queued_by` when the track was "added to queue" (in a Listen Together session,
/// stamped with who added it), `autoplay` when radio appended it, `set_video_id` from whatever
/// playlist it was played from. None of that describes the song, so On Repeat sheds it: otherwise
/// the row wears a session member's name forever, and playing On Repeat drops it into "Next in
/// queue" instead of the playlist. Strips on read so rows already stored this way are fixed too.
fn shed_queue_context(s: SongItem) -> SongItem {
    SongItem {
        queued: false,
        queued_end: false,
        queued_from: None,
        queued_by: None,
        autoplay: false,
        set_video_id: None,
        ..s
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub async fn get_playlist_more(
    state: St<'_>,
    token: String,
) -> Result<PlaylistContinuation, String> {
    let client = metadata_client(&state)?;
    state
        .it
        .playlist_continuation(client, &token)
        .await
        .map_err(|e| e.to_string())
}

/// An album page. `id` is the album browseId (`MPRE…`).
#[tauri::command]
pub async fn get_album(state: St<'_>, id: String) -> Result<AlbumPage, String> {
    // A local album is built from SQLite, so it opens the same page while offline (local.rs).
    if let Some(key) = id.strip_prefix(crate::local::ALBUM_PREFIX) {
        return Ok(crate::local::album_page(&state.db, key));
    }
    // A local artist rides this route too: same page shape, and none of the artist route's
    // YouTube furniture applies to files on disk (see `local::artist_page`).
    if let Some(name) = id.strip_prefix(crate::local::ARTIST_PREFIX) {
        return Ok(crate::local::artist_page(&state.db, name));
    }
    let client = metadata_client(&state)?;
    state.it.album(client, &id).await.map_err(|e| e.to_string())
}

/// An artist page. `id` is the channel browseId (`UC…`).
#[tauri::command]
pub async fn get_artist(state: St<'_>, id: String) -> Result<ArtistPage, String> {
    let client = metadata_client(&state)?;
    state
        .it
        .artist(client, &id)
        .await
        .map_err(|e| e.to_string())
}

/// A card grid reached from a carousel's "More" button (e.g. an artist's full albums list).
#[tauri::command]
pub async fn get_browse_grid(
    state: St<'_>,
    id: String,
    params: Option<String>,
) -> Result<Vec<BrowseItem>, String> {
    let client = metadata_client(&state)?;
    state
        .it
        .browse_grid(client, &id, params.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Play a playlist/album: the given items become the queue (no radio). `start` is the clicked
/// track index; `None`/omitted means "just play it" (random opener when shuffle is on).
/// `source_id` (the page's playlist/album playlist id) makes autoplay continue with that
/// context's radio when the queue runs out. `source_name` (the page title) feeds the queue
/// panel's "Next from" header; `shuffle: true` (page Shuffle buttons) turns shuffle on for
/// this queue — pass the items in their real order, the backend shuffles. `continuation` is the
/// page's next-page token when it has one: pass the tracks that are loaded and the backend walks
/// the rest into the queue in the background, so playback starts on page 1.
#[tauri::command]
pub async fn play_playlist(
    state: St<'_>,
    items: Vec<SongItem>,
    start: Option<usize>,
    source_id: Option<String>,
    source_name: Option<String>,
    shuffle: Option<bool>,
    continuation: Option<String>,
) -> Result<(), String> {
    let state = state.inner().clone();
    state
        .play_tracks(
            items,
            start,
            source_id,
            source_name,
            shuffle.unwrap_or(false),
            continuation,
        )
        .await;
    Ok(())
}

/// Start a radio seeded on a song, artist, album or playlist (context/08). `kind` is
/// `song` | `artist` | `album` | `playlist`; `id` is the videoId (song) or browseId/playlistId
/// (everything else) — the backend resolves it to a radio playlist. `name` titles the queue.
///
/// Starting a song radio on the track that's already playing keeps it playing and replaces only
/// what comes after it; every other case replaces the queue.
#[tauri::command]
pub async fn start_radio(
    state: St<'_>,
    kind: String,
    id: String,
    name: Option<String>,
) -> Result<(), String> {
    let state = state.inner().clone();
    state.start_radio(&kind, &id, name).await
}

// --- write actions (context/01 ✎, context/15) ----------------------------------------------

fn require_login(state: &Arc<AppState>) -> Result<&innertube::YouTubeClient, String> {
    if !state.it.is_logged_in() {
        return Err("Sign in first to use this.".into());
    }
    metadata_client(state)
}

#[tauri::command]
pub async fn like(state: St<'_>, video_id: String, liked: bool) -> Result<(), String> {
    let client = require_login(&state)?;
    state
        .it
        .like(client, &video_id, liked)
        .await
        .map_err(|e| e.to_string())
}

/// Save an album to the library, or remove it. `playlist_id` is the album's `OLAK5uy_…`
/// (`AlbumPage.playlistId`).
#[tauri::command]
pub async fn set_album_saved(
    state: St<'_>,
    playlist_id: String,
    saved: bool,
) -> Result<(), String> {
    let client = require_login(&state)?;
    state
        .it
        .like_playlist(client, &playlist_id, saved)
        .await
        .map_err(|e| e.to_string())
}

/// Login, plus the guard every playlist edit needs: On Repeat has no YouTube playlist behind it, so
/// its synthetic id must never reach `edit_playlist`, which answers 400 for an id it doesn't know.
fn editable_playlist<'a>(
    state: &'a Arc<AppState>,
    playlist_id: &str,
) -> Result<&'a innertube::YouTubeClient, String> {
    if playlist_id == ON_REPEAT_ID {
        return Err("On Repeat builds itself from what you play.".into());
    }
    require_login(state)
}

#[tauri::command]
pub async fn add_to_playlist(
    state: St<'_>,
    playlist_id: String,
    video_id: String,
) -> Result<(), String> {
    let client = editable_playlist(&state, &playlist_id)?;
    state
        .it
        .playlist_add(client, &playlist_id, &video_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_from_playlist(
    state: St<'_>,
    playlist_id: String,
    video_id: String,
    set_video_id: String,
) -> Result<(), String> {
    let client = editable_playlist(&state, &playlist_id)?;
    state
        .it
        .playlist_remove(client, &playlist_id, &video_id, &set_video_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_playlist(state: St<'_>, title: String) -> Result<String, String> {
    let client = require_login(&state)?;
    state
        .it
        .create_playlist(client, &title)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_playlist(
    state: St<'_>,
    playlist_id: String,
    name: String,
) -> Result<(), String> {
    let client = editable_playlist(&state, &playlist_id)?;
    state
        .it
        .playlist_rename(client, &playlist_id, &name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_playlist(state: St<'_>, playlist_id: String) -> Result<(), String> {
    let client = editable_playlist(&state, &playlist_id)?;
    state
        .it
        .delete_playlist(client, &playlist_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn subscribe(state: St<'_>, channel_id: String, subscribed: bool) -> Result<(), String> {
    let client = require_login(&state)?;
    state
        .it
        .subscribe(client, &channel_id, subscribed)
        .await
        .map_err(|e| e.to_string())
}

// --- local music (local.rs) ------------------------------------------------------------------

/// Rescan the watched folders and return the library. The scan is the deletion check too: its
/// `removed` list is every id that was on screen but is gone from disk, so the UI can drop those
/// tiles without waiting for anyone to click a dead one.
#[tauri::command]
pub async fn get_local_library(state: St<'_>) -> Result<crate::local::LocalLibrary, String> {
    scan_local(&state).await
}

#[tauri::command]
pub async fn add_local_folder(
    state: St<'_>,
    path: String,
) -> Result<crate::local::LocalLibrary, String> {
    crate::local::add_folder(&state.db, path);
    scan_local(&state).await
}

/// Stop watching a folder. Its tracks disappear from the library on the rescan that follows (they
/// come back untouched if the folder is added again — nothing on disk is modified).
#[tauri::command]
pub async fn remove_local_folder(
    state: St<'_>,
    path: String,
) -> Result<crate::local::LocalLibrary, String> {
    crate::local::remove_folder(&state.db, &path);
    scan_local(&state).await
}

/// Disk IO + tag parsing off the async runtime's worker threads.
async fn scan_local(state: &Arc<AppState>) -> Result<crate::local::LocalLibrary, String> {
    let app = state.app.clone();
    let state = state.clone();
    let covers = crate::local::covers_dir(&state.app);
    let lib = tauri::async_runtime::spawn_blocking(move || crate::local::scan(&state.db, &covers))
        .await
        .map_err(|e| e.to_string())?;
    // Artwork reaches the page over the asset protocol, which starts out allowing nothing.
    crate::local::allow_covers(&app, &lib.songs);
    Ok(lib)
}

// --- Listen Together (context/19) ----------------------------------------------------------

/// Current client-side LT state (status, role, room, participants, pending joins, suggestions).
#[tauri::command]
pub async fn lt_get_state(state: St<'_>) -> Result<serde_json::Value, String> {
    Ok(state.lt.snapshot().await)
}

/// Set + persist the sync server URL (e.g. the Tailscale Funnel `wss://…` address).
#[tauri::command]
pub async fn lt_set_server_url(state: St<'_>, url: String) -> Result<(), String> {
    let url = url.trim().to_string();
    state.db.set_setting("lt_server_url", &url);
    state.lt.set_server_url(url).await;
    Ok(())
}

#[tauri::command]
pub async fn lt_create_room(state: St<'_>, username: String) -> Result<(), String> {
    state.lt.create_room(username).await;
    Ok(())
}

#[tauri::command]
pub async fn lt_join_room(state: St<'_>, code: String, username: String) -> Result<(), String> {
    state.lt.join_room(code, username).await;
    Ok(())
}

#[tauri::command]
pub async fn lt_leave(state: St<'_>) -> Result<(), String> {
    state.lt.leave().await;
    Ok(())
}

#[tauri::command]
pub async fn lt_approve_join(state: St<'_>, user_id: String) -> Result<(), String> {
    state.lt.approve_join(user_id).await;
    Ok(())
}

#[tauri::command]
pub async fn lt_reject_join(state: St<'_>, user_id: String) -> Result<(), String> {
    state.lt.reject_join(user_id).await;
    Ok(())
}

#[tauri::command]
pub async fn lt_kick(state: St<'_>, user_id: String) -> Result<(), String> {
    state.lt.kick(user_id).await;
    Ok(())
}

#[tauri::command]
pub async fn lt_transfer_host(state: St<'_>, user_id: String) -> Result<(), String> {
    state.lt.transfer_host(user_id).await;
    Ok(())
}

/// Guest: send a track to the session queue (auto-approved by the host client, which stamps
/// who added it).
#[tauri::command]
pub async fn lt_suggest(state: St<'_>, item: SongItem) -> Result<(), String> {
    state.lt.suggest(crate::state::song_to_track(&item)).await;
    Ok(())
}

/// Host: approve a suggestion — add it to the real queue and notify the suggester. (Unused since
/// guest adds auto-approve, kept for a future "require approval" setting.)
#[tauri::command]
pub async fn lt_approve_suggestion(state: St<'_>, id: String) -> Result<(), String> {
    if let Some(track) = state.lt.approve_suggestion(id).await {
        state.inner().clone().lt_enqueue_track(track).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn lt_reject_suggestion(state: St<'_>, id: String) -> Result<(), String> {
    state.lt.reject_suggestion(id).await;
    Ok(())
}

/// Guest: force a re-sync with the room (drift correction).
#[tauri::command]
pub async fn lt_request_sync(state: St<'_>) -> Result<(), String> {
    state.lt.request_sync().await;
    Ok(())
}

// --- lyrics ---------------------------------------------------------------------------------

/// Lyrics for a track (cached). The UI passes the metadata it already has from `now-playing`;
/// `duration` is mpv's length in seconds. `None` = no lyrics found anywhere.
#[tauri::command]
pub async fn get_lyrics(
    state: St<'_>,
    video_id: String,
    title: String,
    artists: String,
    album: Option<String>,
    duration: Option<f64>,
) -> Result<Option<crate::lyrics::Lyrics>, String> {
    Ok(crate::lyrics::get_lyrics(
        state.inner(),
        crate::lyrics::LyricsRequest {
            video_id,
            title,
            artists,
            album,
            duration,
        },
    )
    .await)
}

// --- Last.fm scrobbling ---------------------------------------------------------------------

/// Start the browser auth flow. Returns once the authorize page is open; the outcome (session
/// stored, or an error) arrives via the `lastfm-state` event.
#[tauri::command]
pub async fn lastfm_connect(state: St<'_>) -> Result<(), String> {
    crate::lastfm::connect(state.inner().clone()).await
}

#[tauri::command]
pub async fn lastfm_disconnect(state: St<'_>) -> Result<(), String> {
    crate::lastfm::disconnect(&state);
    Ok(())
}

/// `{ connected, username }` from the persisted session — seeds the titlebar button on mount.
#[tauri::command]
pub async fn lastfm_status(state: St<'_>) -> Result<serde_json::Value, String> {
    Ok(crate::lastfm::status(&state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_repeat_rows_shed_the_queue_slot_they_were_played_from() {
        let played = SongItem {
            video_id: "abc".into(),
            title: "Grace".into(),
            queued: true,
            queued_by: Some("simohypers".into()),
            autoplay: true,
            set_video_id: Some("SVI".into()),
            ..Default::default()
        };
        let row = shed_queue_context(played.clone());
        assert_eq!(
            row,
            SongItem {
                video_id: "abc".into(),
                title: "Grace".into(),
                ..Default::default()
            }
        );
        assert_eq!(row.title, played.title, "the song itself survives");
    }
}
