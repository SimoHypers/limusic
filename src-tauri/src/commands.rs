//! Tauri commands — the ONLY API the UI calls. context/11 UI contract. No YouTube shapes leak
//! past here; the UI never sees a stream URL.

use std::sync::Arc;

use innertube::{
    AlbumPage, ArtistPage, BrowseItem, HomePage, PlaylistContinuation, PlaylistPage, SearchResults,
    SongItem,
};
use tauri::State;

use crate::state::AppState;

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
    state.it.search_all(client, &query).await.map_err(|e| e.to_string())
}

/// Filtered "Show more" search for one category (albums / artists / playlists).
#[tauri::command]
pub async fn search_cards(
    state: St<'_>,
    query: String,
    category: String,
) -> Result<Vec<BrowseItem>, String> {
    let client = metadata_client(&state)?;
    state.it.search_cards(client, &query, &category).await.map_err(|e| e.to_string())
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

/// "Add to queue" from a track's ⋯ menu. Solo: end of the queue; in a session (host or guest):
/// the session boundary, right after the current song.
#[tauri::command]
pub async fn add_to_queue(state: St<'_>, item: SongItem) -> Result<(), String> {
    state.inner().clone().add_to_queue(item).await;
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
    state.player.set_volume(volume).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_queue(state: St<'_>) -> Result<serde_json::Value, String> {
    Ok(state.queue_snapshot().await)
}

/// Settings the UI is allowed to read *and write*. Session/auth material (`session_cookie`,
/// `data_sync_id`, `account_json`, `visitor_data`) and internal blobs (`queue_json`,
/// `queue_position`) never cross into the webview — they'd otherwise ship the login credential to
/// the renderer on every open — and the webview can't overwrite them either.
const UI_SETTINGS: [&str; 8] = [
    "proxy",
    "quality",
    "enable_history",
    "disabled_stream_clients",
    "discord_rpc",
    "close_to_tray",
    "autostart",
    "autoplay",
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
    v.extend(innertube::STREAM_FALLBACK_ORDER.iter().map(|s| s.to_string()));
    Ok(v)
}

/// Wipe both cache tiers (URL cache + mpv on-disk audio cache). context/14.
#[tauri::command]
pub async fn clear_caches(state: St<'_>) -> Result<(), String> {
    state.clear_caches();
    Ok(())
}

// --- auth (context/15) ---------------------------------------------------------------------

/// Sign in by pasting a Cookie header (context/15 Path B). Returns the account for the UI.
#[tauri::command]
pub async fn set_cookie(state: St<'_>, cookie: String) -> Result<serde_json::Value, String> {
    let state = state.inner().clone();
    state.sign_in(cookie).await
}

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

// --- browse / library (context/08) ---------------------------------------------------------

fn metadata_client(state: &Arc<AppState>) -> Result<&innertube::YouTubeClient, String> {
    state.clients.get(innertube::METADATA_CLIENT).ok_or_else(|| "metadata client missing".into())
}

#[tauri::command]
pub async fn get_home(state: St<'_>, params: Option<String>) -> Result<HomePage, String> {
    let client = metadata_client(&state)?;
    state.it.home(client, params.as_deref()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_home_more(state: St<'_>, token: String) -> Result<HomePage, String> {
    let client = metadata_client(&state)?;
    state.it.home_continuation(client, &token).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_library(state: St<'_>) -> Result<Vec<BrowseItem>, String> {
    let client = metadata_client(&state)?;
    state.it.library_playlists(client).await.map_err(|e| e.to_string())
}

/// A playlist or album page. `id` is the browseId (`VL…` / `MPRE…`); Liked Songs is `VLLM`.
#[tauri::command]
pub async fn get_playlist(state: St<'_>, id: String) -> Result<PlaylistPage, String> {
    let client = metadata_client(&state)?;
    state.it.playlist(client, &id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_playlist_more(
    state: St<'_>,
    token: String,
) -> Result<PlaylistContinuation, String> {
    let client = metadata_client(&state)?;
    state.it.playlist_continuation(client, &token).await.map_err(|e| e.to_string())
}

/// An album page. `id` is the album browseId (`MPRE…`).
#[tauri::command]
pub async fn get_album(state: St<'_>, id: String) -> Result<AlbumPage, String> {
    let client = metadata_client(&state)?;
    state.it.album(client, &id).await.map_err(|e| e.to_string())
}

/// An artist page. `id` is the channel browseId (`UC…`).
#[tauri::command]
pub async fn get_artist(state: St<'_>, id: String) -> Result<ArtistPage, String> {
    let client = metadata_client(&state)?;
    state.it.artist(client, &id).await.map_err(|e| e.to_string())
}

/// A card grid reached from a carousel's "More" button (e.g. an artist's full albums list).
#[tauri::command]
pub async fn get_browse_grid(
    state: St<'_>,
    id: String,
    params: Option<String>,
) -> Result<Vec<BrowseItem>, String> {
    let client = metadata_client(&state)?;
    state.it.browse_grid(client, &id, params.as_deref()).await.map_err(|e| e.to_string())
}

/// Play a playlist/album: the given items become the queue (no radio). `start` is the clicked
/// track index; `None`/omitted means "just play it" (random opener when shuffle is on).
/// `source_id` (the page's playlist/album playlist id) makes autoplay continue with that
/// context's radio when the queue runs out.
#[tauri::command]
pub async fn play_playlist(
    state: St<'_>,
    items: Vec<SongItem>,
    start: Option<usize>,
    source_id: Option<String>,
) -> Result<(), String> {
    let state = state.inner().clone();
    state.play_tracks(items, start, source_id).await;
    Ok(())
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
    state.it.like(client, &video_id, liked).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_to_playlist(
    state: St<'_>,
    playlist_id: String,
    video_id: String,
) -> Result<(), String> {
    let client = require_login(&state)?;
    state.it.playlist_add(client, &playlist_id, &video_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_from_playlist(
    state: St<'_>,
    playlist_id: String,
    video_id: String,
    set_video_id: String,
) -> Result<(), String> {
    let client = require_login(&state)?;
    state
        .it
        .playlist_remove(client, &playlist_id, &video_id, &set_video_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_playlist(state: St<'_>, title: String) -> Result<String, String> {
    let client = require_login(&state)?;
    state.it.create_playlist(client, &title).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_playlist(state: St<'_>, playlist_id: String, name: String) -> Result<(), String> {
    let client = require_login(&state)?;
    state.it.playlist_rename(client, &playlist_id, &name).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_playlist(state: St<'_>, playlist_id: String) -> Result<(), String> {
    let client = require_login(&state)?;
    state.it.delete_playlist(client, &playlist_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn subscribe(state: St<'_>, channel_id: String, subscribed: bool) -> Result<(), String> {
    let client = require_login(&state)?;
    state.it.subscribe(client, &channel_id, subscribed).await.map_err(|e| e.to_string())
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
        crate::lyrics::LyricsRequest { video_id, title, artists, album, duration },
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
