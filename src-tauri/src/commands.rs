//! Tauri commands — the ONLY API the UI calls. context/11 UI contract. No YouTube shapes leak
//! past here; the UI never sees a stream URL.

use std::sync::Arc;

use innertube::SongItem;
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

#[tauri::command]
pub async fn next_track(state: St<'_>) -> Result<(), String> {
    let state = state.inner().clone();
    let idx = current_index(&state).await + 1;
    state.play_index(idx).await;
    Ok(())
}

#[tauri::command]
pub async fn prev_track(state: St<'_>) -> Result<(), String> {
    let state = state.inner().clone();
    let cur = current_index(&state).await;
    state.play_index(cur.saturating_sub(1)).await;
    Ok(())
}

#[tauri::command]
pub async fn toggle_pause(state: St<'_>) -> Result<(), String> {
    state.player.toggle().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn seek(state: St<'_>, position: f64) -> Result<(), String> {
    state.player.seek(position).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_volume(state: St<'_>, volume: i64) -> Result<(), String> {
    state.player.set_volume(volume).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_queue(state: St<'_>) -> Result<serde_json::Value, String> {
    Ok(state.queue_snapshot().await)
}

#[tauri::command]
pub async fn get_settings(state: St<'_>) -> Result<serde_json::Value, String> {
    let map: serde_json::Map<String, serde_json::Value> = state
        .db
        .all_settings()
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    Ok(serde_json::Value::Object(map))
}

#[tauri::command]
pub async fn set_setting(state: St<'_>, key: String, value: String) -> Result<(), String> {
    state.db.set_setting(&key, &value);
    Ok(())
}

async fn current_index(state: &Arc<AppState>) -> usize {
    state
        .queue_snapshot()
        .await
        .get("currentIndex")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize
}
