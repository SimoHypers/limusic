pub mod parse;
pub mod score;
pub mod spotify;

pub use spotify::SpotifyPlaylist;

/// Fetches a Spotify playlist by URL or ID and parses its embed page.
#[tauri::command]
pub async fn import_fetch_spotify(url: String) -> Result<SpotifyPlaylist, String> {
    spotify::fetch_spotify(&url).await
}

use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use tauri::{Emitter, State};

type St<'a> = State<'a, Arc<AppState>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportTrack {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_secs: Option<u32>,
    pub isrc: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchStatus {
    Matched,
    Review,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub song: innertube::SongItem,
    pub score: f32,
    pub duration_diff_secs: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub index: usize,
    pub status: MatchStatus,
    pub candidates: Vec<Candidate>,
}

static CANCELLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn cancelled_jobs() -> &'static Mutex<HashSet<String>> {
    CANCELLED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn is_cancelled(job_id: &str) -> bool {
    cancelled_jobs().lock().unwrap().contains(job_id)
}

fn remove_cancelled(job_id: &str) {
    cancelled_jobs().lock().unwrap().remove(job_id);
}

struct JobCleanup<'a>(&'a str);
impl<'a> Drop for JobCleanup<'a> {
    fn drop(&mut self) {
        remove_cancelled(self.0);
    }
}

fn strip_brackets(title: &str) -> String {
    let mut depth = 0u32;
    let mut out = String::new();
    for c in title.chars() {
        match c {
            '(' | '[' | '{' | '<' | '【' | '（' => depth += 1,
            ')' | ']' | '}' | '>' | '】' | '）' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads a CSV import file from disk (after verifying size <= 5 MB) and parses it into import tracks.
#[tauri::command]
pub async fn import_load_file(path: String) -> Result<Vec<ImportTrack>, String> {
    let metadata = tokio::fs::metadata(&path).await.map_err(|e| e.to_string())?;
    if metadata.len() > 5 * 1024 * 1024 {
        return Err("File too large (>5 MB)".to_string());
    }
    let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
    let content = String::from_utf8_lossy(&bytes);
    parse::parse_csv(&content)
}

/// Parses raw CSV content string into import tracks.
#[tauri::command]
pub async fn import_parse_csv(content: String) -> Result<Vec<ImportTrack>, String> {
    parse::parse_csv(&content)
}

/// Matches import tracks against YouTube Music candidates concurrently, emitting progress events and returning when finished or cancelled.
#[tauri::command]
pub async fn import_match(
    state: St<'_>,
    job_id: String,
    tracks: Vec<ImportTrack>,
) -> Result<(), String> {
    let _cleanup = JobCleanup(&job_id);

    if tracks.is_empty() {
        let _ = state.app.emit(
            "import-done",
            serde_json::json!({
                "job_id": job_id,
                "cancelled": false,
            }),
        );
        return Ok(());
    }

    let total = tracks.len();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
    let done_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut join_set = tokio::task::JoinSet::new();

    let state_arc = state.inner().clone();
    let job_id_clone = job_id.clone();

    for (index, track) in tracks.into_iter().enumerate() {
        if is_cancelled(&job_id_clone) {
            break;
        }

        let semaphore = semaphore.clone();
        let done_count = done_count.clone();
        let state_arc = state_arc.clone();
        let job_id_clone = job_id_clone.clone();

        join_set.spawn(async move {
            if is_cancelled(&job_id_clone) {
                return;
            }

            let _permit = match semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };

            if is_cancelled(&job_id_clone) {
                return;
            }

            let client = match state_arc.clients.get(innertube::METADATA_CLIENT) {
                Some(c) => c,
                None => {
                    tracing::warn!("metadata client missing during import match");
                    let result =
                        MatchResult { index, status: MatchStatus::NotFound, candidates: vec![] };
                    let done = done_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    let _ = state_arc.app.emit(
                        "import-match",
                        serde_json::json!({
                            "job_id": job_id_clone,
                            "done": done,
                            "total": total,
                            "result": result,
                        }),
                    );
                    return;
                }
            };

            let stripped_title = {
                let s = strip_brackets(&track.title);
                if s.is_empty() {
                    track.title.clone()
                } else {
                    s
                }
            };

            let query1 = match track.artists.first() {
                Some(artist) if !artist.trim().is_empty() => {
                    format!("{} {}", stripped_title, artist)
                }
                _ => stripped_title.clone(),
            };

            let mut candidates = match state_arc.it.search_songs(client, &query1, false).await {
                Ok(res) => score::rank(&track, res.items),
                Err(e) => {
                    tracing::warn!(?e, "import search failed for track {}", track.title);
                    vec![]
                }
            };

            let needs_retry = candidates.first().map_or(true, |c| c.score < 0.45);
            if needs_retry
                && track.artists.first().map_or(false, |a| !a.trim().is_empty())
                && stripped_title != query1
            {
                if is_cancelled(&job_id_clone) {
                    return;
                }
                match state_arc.it.search_songs(client, &stripped_title, false).await {
                    Ok(res) => {
                        let retry_candidates = score::rank(&track, res.items);
                        if retry_candidates.first().map_or(0.0, |c| c.score)
                            > candidates.first().map_or(0.0, |c| c.score)
                        {
                            candidates = retry_candidates;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            ?e,
                            "import retry search failed for track {}",
                            stripped_title
                        );
                    }
                }
            }

            let status = score::classify(&candidates, &track);
            let result = MatchResult { index, status, candidates };

            let done = done_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            let _ = state_arc.app.emit(
                "import-match",
                serde_json::json!({
                    "job_id": job_id_clone,
                    "done": done,
                    "total": total,
                    "result": result,
                }),
            );
        });
    }

    let mut cancelled = false;
    while let Some(res) = join_set.join_next().await {
        if let Err(e) = res {
            tracing::warn!(?e, "import task panicked or failed");
        }
        if is_cancelled(&job_id) {
            cancelled = true;
            join_set.abort_all();
            break;
        }
    }

    if is_cancelled(&job_id) {
        cancelled = true;
    }

    let _ = state.app.emit(
        "import-done",
        serde_json::json!({
            "job_id": job_id,
            "cancelled": cancelled,
        }),
    );

    Ok(())
}

/// Cancels an ongoing import matching job by its job_id.
#[tauri::command]
pub async fn import_cancel(job_id: String) -> Result<(), String> {
    cancelled_jobs().lock().unwrap().insert(job_id);
    Ok(())
}
