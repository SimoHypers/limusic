//! Tauri commands — the ONLY API the UI calls. context/11 UI contract. No YouTube shapes leak
//! past here; the UI never sees a stream URL.

use std::sync::Arc;

use innertube::{
    AlbumPage, ArtistPage, BrowseItem, HistoryGroup, HomePage, PlaylistContinuation, PlaylistPage,
    PlaylistSort, Rating, SearchResults, SongItem,
};
use tauri::{Emitter, State};

use crate::blocked::BlockedArtist;
use crate::state::{AppState, ON_REPEAT_ID, ON_REPEAT_LIMIT, ON_REPEAT_WINDOW_SECS};

type St<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub async fn search(state: St<'_>, query: String) -> Result<Vec<SongItem>, String> {
    let client = state.clients.get(innertube::METADATA_CLIENT).ok_or("metadata client missing")?;
    let result = state.it.search_songs(client, &query).await.map_err(|e| e.to_string())?;
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

/// Drag-to-reorder in the queue panel: move the upcoming track at `from` to `to`. Out-of-range or
/// already-played indices are ignored.
#[tauri::command]
pub async fn move_in_queue(state: St<'_>, from: usize, to: usize) -> Result<(), String> {
    state.inner().clone().move_in_queue(from, to).await;
    Ok(())
}

/// "Add to queue": the tracks go at the back of the "Next in queue" block, so they play after
/// everything already queued by hand and ahead of the playing context (and its radio/filler).
/// `from` heads the block in the queue panel; `continuation` is the source page's next-page token —
/// the rest of a long playlist is walked in in the background.
#[tauri::command]
pub async fn add_to_queue(
    state: St<'_>,
    items: Vec<SongItem>,
    from: Option<String>,
    continuation: Option<String>,
) -> Result<(), String> {
    state.inner().clone().add_to_queue(items, from, continuation).await;
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

/// Tempo (0.25–2.0) and pitch (−12..=12 semitones), the "Advanced" dialog. Volatile by design:
/// both reset to 1.0 / 0 on restart, so nobody wonders next week why everything sounds wrong.
#[tauri::command]
pub async fn set_playback_params(state: St<'_>, speed: f64, semitones: i32) -> Result<(), String> {
    // Pitch first: it's the one that can fail (no librubberband), and it rolls itself back, so a
    // failure leaves nothing applied and the UI can revert both steppers together.
    state.player.set_pitch(semitones).map_err(|e| e.to_string())?;
    state.player.set_speed(speed).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_queue(state: St<'_>) -> Result<serde_json::Value, String> {
    Ok(state.queue_snapshot().await)
}

/// Settings the UI is allowed to read *and write*. Session/auth material (`session_cookie`,
/// `selected_identity_json`, `data_sync_id`, `account_json`, `account_selection_pending`,
/// `visitor_data`) and internal blobs (`queue_json`, `queue_index`, `queue_position`) never cross
/// into the webview: they'd otherwise ship the login credential to the renderer on every open, and
/// the webview can't overwrite them either.
const UI_SETTINGS: [&str; 15] = [
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
    "lyrics_boidu",
    "music_videos",
    "sticky_shuffle",
];

/// Resolve the music video for `video_id` and hand back a `limusicvideo://` URL the player view
/// can put in a `<video src>`. `None` when YouTube has no usable video stream for it, which is the
/// ordinary answer for a song and leaves the artwork in place. The real googlevideo URL never
/// leaves Rust (context/11).
#[tauri::command]
pub async fn video_stream(
    state: St<'_>,
    video_id: String,
    max_height: i32,
) -> Result<Option<String>, String> {
    if crate::local::is_local_song(&video_id) {
        return Ok(None);
    }
    // Already resolved and still live: the loopback URL is a pure function of the videoId, so
    // there is nothing left to do. Re-resolving costs up to two `/player` round trips, and the
    // player view pays them on every reopen otherwise.
    if state.video_url(&video_id).is_some() {
        return Ok(crate::videoproxy::url_for(&video_id));
    }
    // The webview picks the height from its own box, so clamp it here rather than trusting it.
    let max_height = max_height.clamp(144, 1080);
    match state.orchestrator.resolve_video(&video_id, max_height).await {
        Some(url) => {
            state.put_video_url(&video_id, url);
            Ok(crate::videoproxy::url_for(&video_id))
        }
        None => Ok(None),
    }
}

/// Forget a resolved music-video URL, so the next `video_stream` for this id resolves a fresh one.
/// The player view calls this when the `<video>` element fails to load, which is what an expired
/// or revoked googlevideo link looks like from the webview.
#[tauri::command]
pub async fn forget_video_stream(state: St<'_>, video_id: String) -> Result<(), String> {
    state.forget_video_url(&video_id);
    Ok(())
}

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
    // Cached lyrics outlive the setting that produced them, so a track fetched while Boidu was on
    // would keep its word timings (and one fetched while off would never gain them) forever.
    if key == "lyrics_boidu" {
        state.db.clear_lyrics_cache();
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
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
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

// --- app icon (#173) -------------------------------------------------------------------------

/// Set the app icon to a PNG the user picked, or clear it back to the bundled one with `None`.
/// The file is copied rather than referenced (see appicon.rs).
#[tauri::command]
pub async fn set_app_icon(app: tauri::AppHandle, path: Option<String>) -> Result<(), String> {
    let dest = crate::appicon::path(&app).ok_or("no app data directory")?;
    match path {
        Some(src) => {
            // `from_path` reads and decodes the whole file, so bound it first: a compressed
            // 16 MB of uniform 8192x8192 still unpacks into 256 MB before anything can reject it.
            // The signature and IHDR are the first 24 bytes of any PNG.
            let len =
                std::fs::metadata(&src).map_err(|e| format!("couldn't read that PNG: {e}"))?.len();
            if len > 16 * 1024 * 1024 {
                return Err("that file is too big; use a PNG under 16 MB".into());
            }
            let mut head = [0u8; 24];
            {
                use std::io::Read;
                std::fs::File::open(&src)
                    .and_then(|mut f| f.read_exact(&mut head))
                    .map_err(|e| format!("couldn't read that PNG: {e}"))?;
            }
            if head[..8] != *b"\x89PNG\r\n\x1a\n" {
                return Err("that file isn't a PNG".into());
            }
            let width = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
            let height = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
            // Nothing draws an icon above 256px, and every launch would pay to decode whatever
            // was picked: a 5000px photo is a 100 MB buffer for a 16px titlebar logo.
            if width > 1024 || height > 1024 {
                return Err(format!(
                    "that image is {width}x{height}; use one no larger than 1024x1024"
                ));
            }
            // Decoding it here is the validation, now bounded to 1024x1024. Storing a file the
            // icon loader can't read would fail silently at every launch instead, with the old
            // icon still showing.
            tauri::image::Image::from_path(&src)
                .map_err(|e| format!("couldn't read that PNG: {e}"))?;
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("couldn't save the icon: {e}"))?;
            }
            // Copy beside the destination and rename over it. `fs::copy` truncates the old file
            // before writing, so a failure part way through would leave a half-written PNG that
            // every later launch still treats as the custom icon.
            let tmp = dest.with_extension("png.tmp");
            std::fs::copy(&src, &tmp).and_then(|_| std::fs::rename(&tmp, &dest)).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("couldn't save the icon: {e}")
            })?;
        }
        None => match std::fs::remove_file(&dest) {
            Ok(()) => {}
            // Already gone is the state the caller asked for; anything else means the custom icon
            // is still on disk and `apply` below would just put it back.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("couldn't remove the icon: {e}")),
        },
    }
    crate::appicon::apply(&app);
    Ok(())
}

/// The custom icon's path, granted to the asset protocol so the in-app logo can render it. `None`
/// when the bundled icon is in use.
#[tauri::command]
pub async fn app_icon_path(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri::Manager;
    let Some(p) = crate::appicon::custom_path(&app) else { return Ok(None) };
    let scope = app.asset_protocol_scope();
    scope.allow_file(&p).map_err(|e| e.to_string())?;
    // Same symlink dance as the font grant: the scope check canonicalizes what it is asked about.
    if let Ok(real) = p.canonicalize() {
        let _ = scope.allow_file(real);
    }
    Ok(Some(p.to_string_lossy().into_owned()))
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
pub async fn get_account_identities(state: St<'_>) -> Result<Vec<serde_json::Value>, String> {
    state.account_identities().await
}

#[tauri::command]
pub async fn switch_account(
    state: St<'_>,
    selection_key: String,
) -> Result<serde_json::Value, String> {
    state.switch_account(&selection_key).await
}

#[tauri::command]
pub async fn sign_out(state: St<'_>) -> Result<(), String> {
    let state = state.inner().clone();
    state.sign_out().await;
    Ok(())
}

// --- saved Google accounts (multi-account, context/15) ---------------------------------------

/// Saved Google accounts for the account menu. Display fields only — cookies, delegated ids and
/// visitorData never cross the Tauri boundary.
#[tauri::command]
pub async fn get_google_accounts(state: St<'_>) -> Result<Vec<serde_json::Value>, String> {
    Ok(state.google_accounts())
}

/// Activate a saved Google account without a Google re-login. Fails if the stored cookie no
/// longer authenticates, in which case the caller should re-sign-in with that account.
#[tauri::command]
pub async fn switch_google_account(state: St<'_>, id: String) -> Result<serde_json::Value, String> {
    state.switch_google_account(&id).await
}

/// Delete a saved account. Removing the active one signs out; the others stay listed.
#[tauri::command]
pub async fn remove_google_account(state: St<'_>, id: String) -> Result<(), String> {
    state.remove_google_account(&id).await;
    Ok(())
}

/// Open the in-app Google sign-in webview (context/15 Path A). Completes asynchronously; the UI
/// hears back via `auth-changed` (success) or `login-error`. With `add_account`, the webview
/// opens Google's AddSession screen so a second account can be added even when the webview
/// already holds a Google session.
#[tauri::command]
pub async fn login_webview(state: St<'_>, add_account: Option<bool>) -> Result<(), String> {
    let state = state.inner().clone();
    let app = state.app.clone();
    crate::session::open_login(app, state, add_account.unwrap_or(false));
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
    rx.await.map_err(|_| "the mini player never answered".to_string())?
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
    // Signed out there is no YouTube library to ask for (the browse would come back as a sign-in
    // shell), but On Repeat is built from this machine's play history and is still real.
    let mut items = if state.it.is_logged_in() {
        state.it.library_playlists(client).await.map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
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
                play_count: None,
                is_video: false,
                is_upload: false,
                explicit: false,
            },
        );
    }
    // A card has nowhere to put two images, so a custom cover simply is the artwork here.
    for item in &mut items {
        if let Some(cover) = custom_cover(&state, &item.id) {
            item.thumbnail = Some(cover);
        }
    }
    Ok(items)
}

/// Empty rather than an error when signed out: the Library page merges the user's local saves into
/// these grids, so "nothing of yours on YouTube" is an answer, not a failure.
#[tauri::command]
pub async fn get_library_albums(state: St<'_>) -> Result<Vec<BrowseItem>, String> {
    if !state.it.is_logged_in() {
        return Ok(Vec::new());
    }
    let client = metadata_client(&state)?;
    state.it.library_albums(client).await.map_err(|e| e.to_string())
}

/// The user's own uploaded albums (Library ▸ Uploads ▸ Albums).
#[tauri::command]
pub async fn get_upload_albums(state: St<'_>) -> Result<Vec<BrowseItem>, String> {
    if !state.it.is_logged_in() {
        return Ok(Vec::new());
    }
    let client = metadata_client(&state)?;
    state.it.upload_albums(client).await.map_err(|e| e.to_string())
}

/// The account's YouTube Music play history, grouped by day. Empty when signed out, same as the
/// library grids: history lives on the account, and there is nothing to fail about not having one.
#[tauri::command]
pub async fn get_history(state: St<'_>) -> Result<Vec<HistoryGroup>, String> {
    if !state.it.is_logged_in() {
        return Ok(Vec::new());
    }
    let client = metadata_client(&state)?;
    state.it.history(client).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_library_artists(state: St<'_>) -> Result<Vec<BrowseItem>, String> {
    if !state.it.is_logged_in() {
        return Ok(Vec::new());
    }
    let client = metadata_client(&state)?;
    state.it.library_artists(client).await.map_err(|e| e.to_string())
}

/// A playlist or album page. `id` is the browseId (`VL…` / `MPRE…`); Liked Songs is `VLLM`, and
/// `LIMUSIC_ON_REPEAT` is the local auto-playlist rather than anything YouTube knows about.
///
/// `sort` asks YouTube for the tracks in a given order; `None` gets whatever order the account
/// already has the list in, which is what a fresh visit wants (it matches YouTube Music).
#[tauri::command]
pub async fn get_playlist(
    state: St<'_>,
    id: String,
    sort: Option<PlaylistSort>,
    desc: Option<bool>,
) -> Result<PlaylistPage, String> {
    if id == ON_REPEAT_ID {
        let items = on_repeat_songs(&state);
        return Ok(PlaylistPage {
            title: Some("On Repeat".into()),
            subtitle: Some(format!("{} songs you've played most this month", items.len())),
            thumbnail: None,
            description: None,
            privacy: None,
            cover: None,
            items,
            continuation: None,
            owned: false, // nothing to rename or delete; it rebuilds itself from what you play
            collaborative: false,
            sort_menu: None, // built from local history, so YouTube has no order to give
        });
    }
    let client = metadata_client(&state)?;
    let sort = sort.map(|s| (s, desc.unwrap_or(false)));
    let mut page = state.it.playlist(client, &id, sort).await.map_err(|e| e.to_string())?;
    // Alongside YouTube's own thumbnail, not over it: the dialog offers to drop the custom one.
    page.cover = custom_cover(&state, &id);
    Ok(page)
}

/// Store a sort order on a playlist, so YouTube Music and every other client show it the same way.
///
/// Only for a playlist whose `sortMenu.editable` said the options are writes. Everywhere else the
/// order is view-only and this would 400.
#[tauri::command]
pub async fn set_playlist_sort(
    state: St<'_>,
    playlist_id: String,
    sort: PlaylistSort,
) -> Result<(), String> {
    let client = editable_playlist(&state, &playlist_id)?;
    state.it.playlist_set_sort(client, &playlist_id, sort).await.map_err(|e| e.to_string())
}

/// videoId → how many times it was played, over the same trailing window On Repeat is built from
/// (the history table is pruned to it, so there is no older data to offer). Feeds the playlist
/// page's "Most played" sort; a track the map doesn't mention has not been played this month.
#[tauri::command]
pub fn play_counts(state: St<'_>) -> std::collections::HashMap<String, i64> {
    state.db.play_counts(now_secs() - ON_REPEAT_WINDOW_SECS).into_iter().collect()
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
        added_by: None,
        added_by_avatar: None,
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
    state.it.playlist_continuation(client, &token).await.map_err(|e| e.to_string())
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
        .play_tracks(items, start, source_id, source_name, shuffle.unwrap_or(false), continuation)
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

/// Like, dislike, or clear a track's rating. One command for all three: YouTube's states are
/// mutually exclusive, so a dislike un-likes in the same call and the UI never has to send two.
#[tauri::command]
pub async fn rate(state: St<'_>, video_id: String, rating: Rating) -> Result<(), String> {
    let client = require_login(&state)?;
    // Before the write, not after: a `refresh_rating` round trip already in flight was asked
    // before this rating existed, so its answer is stale from here on either way (issue #93).
    state.rate_epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    state.it.rate(client, &video_id, rating).await.map_err(|e| e.to_string())
}

/// Add a song to the library (Library ▸ Songs), or take it out. `token` comes from the row's own
/// menu (`SongItem.library`), which is the only handle YouTube gives on this: it is a feedback
/// action, not a rating, so it leaves Liked Music alone.
#[tauri::command]
pub async fn set_song_saved(state: St<'_>, token: String) -> Result<(), String> {
    let client = require_login(&state)?;
    state.it.feedback(client, &token).await.map_err(|e| e.to_string())
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
    state.it.like_playlist(client, &playlist_id, saved).await.map_err(|e| e.to_string())
}

/// Login, plus the guard every playlist edit needs. Two ids never reach `edit_playlist`: On Repeat
/// has no YouTube playlist behind it, and Liked Music is an auto-playlist YouTube edits through the
/// rating endpoint instead. Both answer 400 there.
fn editable_playlist<'a>(
    state: &'a Arc<AppState>,
    playlist_id: &str,
) -> Result<&'a innertube::YouTubeClient, String> {
    if playlist_id == ON_REPEAT_ID {
        return Err("On Repeat builds itself from what you play.".into());
    }
    if playlist_id == LIKED_MUSIC_ID {
        return Err("Liked Music follows your likes; like the song instead.".into());
    }
    require_login(state)
}

/// Liked Music. It is indexed like the rest, because a search row is the one place YouTube tells
/// us nothing: `search` responses carry no `likeStatus` at all (live-checked 2026-08-28), so
/// membership of this list is the only way a result can draw its heart filled. Not shown in the
/// "saved in" chip, though: the thumbs-up already says it (the UI filters it out there).
const LIKED_MUSIC_ID: &str = "VLLM";
/// How long the membership index is trusted before a re-crawl. Adds and removes made in this app
/// patch it as they happen, so this window only ever covers edits made somewhere else.
const PLAYLIST_INDEX_TTL_SECS: i64 = 6 * 3600;
/// Continuation pages per playlist. YouTube hands back 100 tracks a page, so this covers 5000 of
/// them. ponytail: a hard stop, not paging state. A playlist past it marks its first 5000 tracks
/// and no more, which beats one pathological list turning a sync into hundreds of requests.
const PLAYLIST_INDEX_MAX_PAGES: usize = 50;

/// videoId → the ids of your playlists holding it, straight from SQLite with no network at all,
/// so a track list can draw the "saved" mark on its first paint. Empty until the first sync.
#[tauri::command]
pub fn playlist_index(state: St<'_>) -> std::collections::HashMap<String, Vec<String>> {
    state.db.playlist_memberships()
}

/// Rebuild that index by walking the playlists you own, then answer with it.
///
/// Nothing else knows playlist membership: the library browse gives cards, a playlist browse gives
/// one list's tracks, and InnerTube's per-video add-to-playlist dialog would be a request per row.
/// So the crawl is the price, and it is paid at most once every `PLAYLIST_INDEX_TTL_SECS`, on a
/// launch or a sign-in. Playlists you merely saved are skipped: they are someone else's, so "you
/// saved this song to it" would be a lie, and their long mixes would double the walk.
#[tauri::command]
pub async fn sync_playlist_index(
    state: St<'_>,
) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    if !state.it.is_logged_in() {
        state.db.clear_playlist_index();
        return Ok(std::collections::HashMap::new());
    }
    let fresh_until = state
        .db
        // `_v2`: the key changed when Liked Music joined the index, so an install with a fresh
        // stamp re-crawls once instead of showing hearts empty for another six hours.
        .get_setting("playlist_index_synced_at_v2")
        .and_then(|at| at.parse::<i64>().ok())
        .map(|at| at + PLAYLIST_INDEX_TTL_SECS);
    if fresh_until.is_some_and(|until| now_secs() < until) {
        return Ok(state.db.playlist_memberships());
    }
    let client = metadata_client(&state)?;
    let library = state.it.library_playlists(client).await.map_err(|e| e.to_string())?;
    // A degraded response that parses as an empty library would otherwise wipe every mark and
    // then call the wipe fresh for six hours. Nothing to index is nothing to trust: keep what is
    // stored and try again on the next launch.
    if library.is_empty() {
        return Ok(state.db.playlist_memberships());
    }
    let mut indexed: Vec<String> = Vec::new();
    for item in library {
        if item.id == ON_REPEAT_ID {
            continue;
        }
        // One playlist failing (a deleted id, a hiccup) must not abandon the rest of the crawl,
        // and must not drop what is already indexed for it either: leaving it out of `indexed`
        // would have `retain_playlists` forget the tracks we do know about.
        let Ok(page) = state.it.playlist(client, &item.id, None).await else {
            indexed.push(item.id);
            continue;
        };
        // A collaborative playlist reads `owned: false` (YouTube drops the editable header on it)
        // but is one you add to and remove from, so the membership index has to cover it too.
        // Liked Music is exempt: YouTube sends it without the editable header, so it reads
        // `owned: false` even though it is yours.
        if item.id != LIKED_MUSIC_ID && !page.owned && !page.collaborative {
            continue;
        }
        let mut video_ids: Vec<String> = page.items.into_iter().map(|song| song.video_id).collect();
        let mut token = page.continuation;
        for _ in 0..PLAYLIST_INDEX_MAX_PAGES {
            let Some(next) = token.take() else { break };
            let Ok(more) = state.it.playlist_continuation(client, &next).await else { break };
            video_ids.extend(more.items.into_iter().map(|song| song.video_id));
            token = more.continuation;
        }
        state.db.set_playlist_tracks(&item.id, &video_ids);
        indexed.push(item.id);
    }
    state.db.retain_playlists(&indexed);
    state.db.set_setting("playlist_index_synced_at_v2", &now_secs().to_string());
    Ok(state.db.playlist_memberships())
}

/// `false` means the playlist already had the track and YouTube added nothing — not an error, but
/// the UI must not draw an optimistic row for it (there is no real row to remove later).
#[tauri::command]
pub async fn add_to_playlist(
    state: St<'_>,
    playlist_id: String,
    video_id: String,
) -> Result<bool, String> {
    let client = editable_playlist(&state, &playlist_id)?;
    let added =
        state.it.playlist_add(client, &playlist_id, &video_id).await.map_err(|e| e.to_string())?;
    // Also on `false`: YouTube refusing a duplicate means the playlist holds the track, which is
    // exactly what the index should say. A stale index is how it got asked in the first place.
    state.db.add_playlist_track(&playlist_id, &video_id);
    Ok(added)
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
        .map_err(|e| e.to_string())?;
    state.db.remove_playlist_track(&playlist_id, &video_id);
    Ok(())
}

#[tauri::command]
pub async fn create_playlist(state: St<'_>, title: String) -> Result<String, String> {
    let client = require_login(&state)?;
    state.it.create_playlist(client, &title).await.map_err(|e| e.to_string())
}

/// Edit a playlist you own, from the "Edit playlist" dialog: name, description, visibility.
///
/// Each field is `None` when the user left it alone, and only what changed is sent: an edit of
/// the name must not blank a description we failed to read back off the page.
#[tauri::command]
pub async fn edit_playlist_details(
    state: St<'_>,
    playlist_id: String,
    name: Option<String>,
    description: Option<String>,
    public: Option<bool>,
) -> Result<(), String> {
    let client = editable_playlist(&state, &playlist_id)?;
    // The switch is two-state; YouTube's third value (UNLISTED) is only ever left as it was.
    let privacy = public.map(|p| if p { "PUBLIC" } else { "PRIVATE" });
    state
        .it
        .playlist_edit_details(
            client,
            &playlist_id,
            name.as_deref(),
            description.as_deref(),
            privacy,
        )
        .await
        .map_err(|e| e.to_string())
}

/// Custom playlist artwork, in both places it lives.
///
/// Setting one is local-first: the picked image is copied in beside the local-music covers and
/// answered straight back, then pushed to YouTube Music in the background (`sync_cover`), because
/// the upload is three round trips and nobody should watch a spinner for their own file.
///
/// Dropping one waits, and that is deliberate. Once a cover has been up there, YouTube's own
/// thumbnail *is* that cover, so a local-first removal would fall back to the very image being
/// removed and only reach the rebuilt collage a beat later: two swaps, the first of them pointless.
/// The clear is a single small call, so it answers with the thumbnail YouTube rebuilt and the UI
/// changes once.
#[tauri::command]
pub async fn set_playlist_cover(
    app: tauri::AppHandle,
    state: St<'_>,
    playlist_id: String,
    path: Option<String>,
) -> Result<CoverResult, String> {
    use tauri::Manager;
    // What YouTube's uploader will take. WebP is not on the list: it answers 415 for one, and a
    // cover that only works on this machine is worse than one the picker never offered.
    const IMAGE_EXTS: [&str; 3] = ["jpg", "jpeg", "png"];

    let key = cover_key(&playlist_id);
    let stored = state.db.get_setting(&key);
    let Some(src) = path else {
        // YouTube first, so the local copy is still on screen while it answers. Its refusal is
        // never fatal though: dropping the cover from this machine is what the user clicked, and
        // an account that was not allowed to set one up there has nothing to clear anyway.
        let thumbnail = match clear_cover_on_youtube(&state, &playlist_id).await {
            Ok(t) => {
                state.db.delete_setting(&synced_key(&playlist_id));
                t
            }
            Err(e) => {
                tracing::warn!(playlist_id, error = %e, "custom cover not cleared on YouTube Music");
                // Only worth saying when a cover of ours actually reached the account: otherwise
                // there was nothing up there to keep, and the warning would be a lie.
                if state.db.get_setting(&synced_key(&playlist_id)).is_some() {
                    let _ = state.app.emit(
                        "cover-error",
                        serde_json::json!({
                            "message": "Removed here, but YouTube Music kept its copy.",
                        }),
                    );
                }
                None
            }
        };
        state.db.delete_setting(&key);
        if let Some(old) = stored {
            let _ = std::fs::remove_file(old);
        }
        return Ok(CoverResult { cover: None, thumbnail });
    };
    let src = std::path::Path::new(&src);
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return Err("Pick a JPEG or PNG image: YouTube Music won't take anything else.".into());
    }
    // ponytail: a flat size cap instead of downscaling. It keeps a 40px sidebar thumb from
    // decoding a camera raw in the webview and the upload from swallowing one; reach for the
    // `image` crate and a real resize only if 8 MB turns out to bother anyone.
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    if src.metadata().map(|m| m.len()).unwrap_or(0) > MAX_BYTES {
        return Err("That image is over 8 MB. Pick a smaller one.".into());
    }
    let dir = crate::local::covers_dir(&app).join("playlists");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Timestamped, so replacing a cover can't be served out of the webview's cache under the name
    // it already has. The id is filtered to filename characters rather than trusted: it arrives
    // from the UI, and a `..` in it would write outside this directory.
    let stem: String = playlist_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let dest = dir.join(format!("{stem}-{}.{ext}", crate::db::now_secs()));
    std::fs::copy(src, &dest).map_err(|e| e.to_string())?;
    // Only now is the cover it replaces safe to unlink. Dropping it any earlier means a picked
    // file this command goes on to refuse (wrong format, too big, unreadable) takes the artwork
    // already on screen down with it, and the toast talks about the new file while the old one is
    // the thing that just disappeared.
    if let Some(old) = stored {
        let _ = std::fs::remove_file(old);
    }
    let dest = dest.to_string_lossy().to_string();
    // The covers directory is allowed recursively at startup, but the first cover on a fresh
    // install is written after that ran, so name this file explicitly too.
    let _ = app.asset_protocol_scope().allow_file(&dest);
    state.db.set_setting(&key, &dest);
    sync_cover(&state, &playlist_id, dest.clone());
    Ok(CoverResult { cover: Some(dest), thumbnail: None })
}

/// What the UI needs to draw after a cover changed: where the local copy is, and (on a removal)
/// the thumbnail YouTube rebuilt in its place.
#[derive(serde::Serialize)]
pub struct CoverResult {
    cover: Option<String>,
    thumbnail: Option<String>,
}

/// Send the cover on to YouTube Music behind the picker's back: the local copy is already on
/// screen, and the upload is a three-call round trip nobody should wait through.
///
/// A failure is a toast, not a rollback: the artwork is still right here, and it is still this
/// playlist's cover on this machine. Signed out (or On Repeat, which YouTube has never heard of),
/// there is nothing to sync and local is all there ever was.
fn sync_cover(state: &Arc<AppState>, playlist_id: &str, path: String) {
    if playlist_id == ON_REPEAT_ID || !state.it.is_logged_in() {
        return;
    }
    let state = Arc::clone(state);
    let playlist_id = playlist_id.to_owned();
    tauri::async_runtime::spawn(async move {
        let Some(client) = state.clients.get(innertube::METADATA_CLIENT) else {
            return;
        };
        // Read here, not on the command's thread: the file was just written and the caller has its
        // answer already.
        let result = match std::fs::read(&path) {
            Ok(image) => state.it.playlist_set_cover(client, &playlist_id, image).await,
            Err(e) => Err(innertube::Error::Other(e.to_string())),
        };
        match result {
            // Remembered so a later removal knows whether YouTube has anything of ours to drop.
            Ok(()) => state.db.set_setting(&synced_key(&playlist_id), "1"),
            Err(e) => {
                tracing::warn!(playlist_id, error = %e, "playlist cover didn't reach YouTube Music");
                let message = match e {
                    // The one refusal with a known cause and no fix inside this app. Say it once,
                    // plainly, and leave the cover where it already is: on this machine.
                    innertube::Error::CoverRefused => format!("Artwork saved on this device. {e}"),
                    e => format!("Artwork saved here, but the upload to YouTube Music failed: {e}"),
                };
                let _ = state.app.emit("cover-error", serde_json::json!({ "message": message }));
            }
        }
    });
}

/// Drop the custom thumbnail from the account, answering the one YouTube rebuilt from the tracks.
/// Nothing to do (and nothing to answer with) when there is no account behind the playlist.
async fn clear_cover_on_youtube(
    state: &Arc<AppState>,
    playlist_id: &str,
) -> Result<Option<String>, String> {
    if playlist_id == ON_REPEAT_ID || !state.it.is_logged_in() {
        return Ok(None);
    }
    let client = metadata_client(state)?;
    state.it.playlist_clear_cover(client, playlist_id).await.map_err(|e| e.to_string())
}

fn cover_key(playlist_id: &str) -> String {
    // Browse ids arrive `VL`-prefixed and playlist ids don't; one playlist, one key either way.
    format!("playlist_cover:{}", playlist_id.strip_prefix("VL").unwrap_or(playlist_id))
}

/// Set once a cover of ours has actually landed on the account, so a removal knows whether there
/// is anything up there to warn about failing to clear.
fn synced_key(playlist_id: &str) -> String {
    format!("{}:synced", cover_key(playlist_id))
}

/// The custom artwork stored for a playlist, if the file is still there. The user owns that
/// directory and can empty it, and a dead path renders as a broken image.
fn custom_cover(state: &Arc<AppState>, playlist_id: &str) -> Option<String> {
    let path = state.db.get_setting(&cover_key(playlist_id))?;
    std::path::Path::new(&path).is_file().then_some(path)
}

#[tauri::command]
pub async fn delete_playlist(state: St<'_>, playlist_id: String) -> Result<(), String> {
    let client = editable_playlist(&state, &playlist_id)?;
    state.it.delete_playlist(client, &playlist_id).await.map_err(|e| e.to_string())?;
    state.db.forget_playlist(&playlist_id);
    Ok(())
}

#[tauri::command]
pub async fn subscribe(state: St<'_>, channel_id: String, subscribed: bool) -> Result<(), String> {
    let client = require_login(&state)?;
    state.it.subscribe(client, &channel_id, subscribed).await.map_err(|e| e.to_string())
}

// --- blocked artists (blocked.rs, plan 046) ----------------------------------------------------

/// The blocked-artist list, for the settings pane.
#[tauri::command]
pub async fn get_blocked_artists(state: St<'_>) -> Result<Vec<BlockedArtist>, String> {
    Ok(crate::blocked::list(&state.db))
}

/// Block an artist: persist, re-arm the fetch filter, and drop them out of the live queue. Returns
/// the new list. `id` is the channel browseId when the caller has one (a track row often does not).
#[tauri::command]
pub async fn block_artist(
    state: St<'_>,
    id: Option<String>,
    name: String,
) -> Result<Vec<BlockedArtist>, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("an artist needs a name to be blocked".into());
    }
    let state = state.inner().clone();
    let list =
        crate::blocked::block(&state.db, BlockedArtist { id: id.filter(|i| !i.is_empty()), name });
    let predicate = crate::blocked::block_list(&state.db);
    state.it.set_blocked(predicate.clone());
    // The fetch filter only covers what is fetched from here on, so the queue already on screen
    // has to be cleaned out separately or the block reads as having done nothing.
    state.purge_blocked(&predicate).await;
    Ok(list)
}

#[tauri::command]
pub async fn unblock_artist(state: St<'_>, key: String) -> Result<Vec<BlockedArtist>, String> {
    let list = crate::blocked::unblock(&state.db, &key);
    state.it.set_blocked(crate::blocked::block_list(&state.db));
    Ok(list)
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
        crate::lyrics::LyricsRequest { video_id, title, artists, album, duration },
    )
    .await)
}

// --- Changelog ------------------------------------------------------------------------------

#[derive(Clone, serde::Serialize)]
pub struct ReleaseNote {
    version: String,
    /// `YYYY-MM-DD`, or empty for an unpublished tag.
    date: String,
    /// The release description, verbatim markdown. The About tab renders it.
    body: String,
}

/// What's new, read straight from the GitHub releases API so the release description is the only
/// place the changelog is written. Cached for the process: the list only changes when a release
/// is cut, and unauthenticated GitHub allows 60 requests an hour.
#[tauri::command]
pub async fn release_notes() -> Result<Vec<ReleaseNote>, String> {
    static CACHE: std::sync::OnceLock<Vec<ReleaseNote>> = std::sync::OnceLock::new();
    if let Some(cached) = CACHE.get() {
        return Ok(cached.clone());
    }
    #[derive(serde::Deserialize)]
    struct GhRelease {
        tag_name: String,
        published_at: Option<String>,
        body: Option<String>,
        draft: bool,
        prerelease: bool,
    }
    let releases: Vec<GhRelease> = crate::http::client()
        .get("https://api.github.com/repos/SimoHypers/limusic/releases?per_page=20")
        .header("User-Agent", concat!("Limusic/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let notes: Vec<ReleaseNote> = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .map(|r| ReleaseNote {
            version: r.tag_name.trim_start_matches('v').to_string(),
            date: r
                .published_at
                .and_then(|d| d.split('T').next().map(str::to_string))
                .unwrap_or_default(),
            body: r.body.unwrap_or_default(),
        })
        .collect();
    Ok(CACHE.get_or_init(|| notes).clone())
}

/// Whether this build can install an update itself, or only point the user at the download.
///
/// Tauri's Linux updater knows one trick: rewrite an AppImage in place. It takes the path from
/// `Env::appimage` and, when that is unset, falls back to `current_exe()` and writes the downloaded
/// AppImage bytes over whatever it finds there. On the `.rpm` and on distro packages (the AUR's
/// `limusic-bin`) that is a package-manager-owned `/usr/bin/limusic-app`: it fails on permissions
/// rather than doing damage, but offering the button at all is a lie. Those users update through
/// their package manager, so the UI shows them a download link instead.
///
/// Reads the same `Env::appimage` the updater plugin decides on, so the two cannot disagree.
#[tauri::command]
pub fn can_self_update(app: tauri::AppHandle) -> bool {
    #[cfg(target_os = "linux")]
    {
        use tauri::Manager;
        app.env().appimage.is_some()
    }
    // Windows runs the NSIS installer and macOS swaps the .app bundle; both work however the app
    // was installed.
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        true
    }
}

/// Open a link from the UI in the real browser. An `<a href>` inside the webview would navigate
/// the app itself off the SPA, with no way back.
#[tauri::command]
pub async fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("only http(s) links".into());
    }
    crate::lastfm::open_browser(&url)
}

// --- Diagnostics ----------------------------------------------------------------------------

/// The bug-report blob for Settings ▸ About: environment header plus the redacted tail of
/// `limusic.log`. See `crate::diagnostics`.
#[tauri::command]
pub fn diagnostics(app: tauri::AppHandle, state: St<'_>) -> String {
    crate::diagnostics::report(&app, &state.db)
}

/// The environment block on its own, for prefilling the bug form's `system` field. GitHub's query
/// parameters only reach `input` and `textarea` fields, so this is how the app tells us what it is
/// running on without the user typing it.
#[tauri::command]
pub fn diagnostics_summary(app: tauri::AppHandle, state: St<'_>) -> String {
    crate::diagnostics::summary(&app, &state.db)
}

/// The same text written to a path the user picked in a save dialog, for attaching to an issue
/// when it is too long to paste comfortably.
#[tauri::command]
pub fn save_diagnostics(app: tauri::AppHandle, state: St<'_>, path: String) -> Result<(), String> {
    std::fs::write(&path, crate::diagnostics::report(&app, &state.db)).map_err(|e| e.to_string())
}

/// The webview's own errors, into the same log file as everything else. Without this a blank
/// screen or a rejected `invoke` leaves no trace at all in the log a user hands over.
#[tauri::command]
pub fn log_ui(level: String, message: String) {
    // Bounded: a throwing `$effect` re-fires every frame, and the file has no size cap.
    let message: String = message.chars().take(2000).collect();
    match level.as_str() {
        "info" => tracing::info!(target: "ui", "{message}"),
        "warn" => tracing::warn!(target: "ui", "{message}"),
        _ => tracing::error!(target: "ui", "{message}"),
    }
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

/// Theater mode's fullscreen toggle (#139).
///
/// `setFullscreen` on its own is not enough on Windows. tao decides the client area in
/// WM_NCCALCSIZE: while the real Win32 placement says maximized it clamps the client to the
/// monitor's *work* area, so the "fullscreen" window sits under the taskbar with a frame-thick
/// border around it, and while the window is undecorated-with-shadow it insets the client by the
/// frame thickness. Both are decided before the fullscreen flag is, and tao's own `is_maximized`
/// reads a cached flag that can disagree with the placement, which is why unmaximizing from the
/// UI only fixed it some of the time.
///
/// So: restore from the real placement, go fullscreen, then put the window on the monitor rect
/// with SWP_FRAMECHANGED to force one recalculation with the fullscreen flag set. On the main
/// thread, where the window messages run inline and the order is guaranteed.
#[tauri::command]
pub fn theater_fullscreen(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        window.set_fullscreen(on).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "windows")]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        use windows::Win32::UI::WindowsAndMessaging::{
            IsZoomed, SetWindowPos, ShowWindow, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE,
            SWP_NOZORDER, SW_MAXIMIZE, SW_RESTORE,
        };

        /// Restoring the window is what makes fullscreen work, so theater has to put the
        /// maximized state back itself on the way out.
        static WAS_MAXIMIZED: AtomicBool = AtomicBool::new(false);

        let w = window.clone();
        window
            .run_on_main_thread(move || {
                let Ok(hwnd) = w.hwnd() else { return };
                if on {
                    let zoomed = unsafe { IsZoomed(hwnd).as_bool() };
                    WAS_MAXIMIZED.store(zoomed, Ordering::Relaxed);
                    if zoomed {
                        unsafe {
                            let _ = ShowWindow(hwnd, SW_RESTORE);
                        }
                    }
                    let _ = w.set_fullscreen(true);
                    if let Ok(Some(m)) = w.current_monitor() {
                        let (p, s) = (m.position(), m.size());
                        unsafe {
                            let _ = SetWindowPos(
                                hwnd,
                                None,
                                p.x,
                                p.y,
                                s.width as i32,
                                s.height as i32,
                                SWP_FRAMECHANGED | SWP_NOZORDER,
                            );
                        }
                    }
                } else {
                    let _ = w.set_fullscreen(false);
                    if WAS_MAXIMIZED.swap(false, Ordering::Relaxed) {
                        unsafe {
                            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                        }
                    }
                    unsafe {
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            0,
                            0,
                            0,
                            0,
                            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
                        );
                    }
                }
            })
            .map_err(|e| e.to_string())
    }
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
            SongItem { video_id: "abc".into(), title: "Grace".into(), ..Default::default() }
        );
        assert_eq!(row.title, played.title, "the song itself survives");
    }
}
