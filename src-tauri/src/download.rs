//! Download manager for songs, albums and playlists.
//!
//! Downloads audio directly from YouTube via the internal Innertube / Orchestrator resolver,
//! downloads streams in bounded HTTP chunks (bypassing googlevideo throttling), writes metadata
//! tags and front cover artwork using `lofty`, and reports real-time progress via Tauri events.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, Manager};

const CHUNK_SIZE: u64 = 2 * 1024 * 1024; // 2 MiB chunks
const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 3;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Tagging,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DownloadTask {
    pub id: String,
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub thumbnail: Option<String>,
    pub progress: f32, // 0.0 .. 100.0
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub status: DownloadStatus,
    pub error: Option<String>,
    pub file_path: Option<String>,
}

pub struct DownloadManager {
    tasks: Arc<Mutex<Vec<DownloadTask>>>,
    cancelled: Arc<Mutex<HashSet<String>>>,
    semaphore: Arc<tokio::sync::Semaphore>,
    current_concurrency: std::sync::atomic::AtomicUsize,
    pending_reductions: Arc<std::sync::atomic::AtomicUsize>,
    app: AppHandle,
    db: Arc<crate::db::Db>,
    orchestrator: Arc<crate::orchestrator::Orchestrator>,
}

impl DownloadManager {
    pub fn new(
        app: AppHandle,
        db: Arc<crate::db::Db>,
        orchestrator: Arc<crate::orchestrator::Orchestrator>,
    ) -> Self {
        let concurrency = db
            .get_setting("download_concurrency")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT_MAX_CONCURRENT_DOWNLOADS)
            .clamp(1, 5);
        let mgr = Self {
            tasks: Arc::new(Mutex::new(Vec::new())),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            semaphore: Arc::new(tokio::sync::Semaphore::new(concurrency)),
            current_concurrency: std::sync::atomic::AtomicUsize::new(concurrency),
            pending_reductions: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            app,
            db,
            orchestrator,
        };
        let history = mgr.load_history();
        if let Ok(mut tasks) = mgr.tasks.lock() {
            *tasks = history;
        }
        mgr
    }

    fn load_history(&self) -> Vec<DownloadTask> {
        let Ok(tracks) = self.db.get_all_downloaded_tracks() else { return Vec::new() };
        let mut list = Vec::new();
        for t in tracks {
            if Path::new(&t.file_path).exists() {
                list.push(DownloadTask {
                    id: format!("{}_db", t.video_id),
                    video_id: t.video_id,
                    title: t.title,
                    artist: t.artist,
                    album: t.album,
                    thumbnail: t.thumbnail,
                    progress: 100.0,
                    bytes_downloaded: 0,
                    total_bytes: 0,
                    status: DownloadStatus::Completed,
                    error: None,
                    file_path: Some(t.file_path),
                });
            } else {
                let _ = self.db.delete_download(&t.video_id);
            }
        }
        list
    }

    fn save_history(&self) {
        // SQLite is the single source of truth for completed downloads.
    }

    /// Default or configured download directory.
    pub fn download_dir(&self) -> PathBuf {
        if let Some(custom) = self.db.get_setting("download_dir") {
            let p = PathBuf::from(custom.trim());
            if !p.as_os_str().is_empty() {
                return p;
            }
        }
        self.app.path().download_dir().unwrap_or_else(|_| PathBuf::from(".")).join("Limusic")
    }

    pub fn set_download_dir(&self, dir: &str) {
        self.db.set_setting("download_dir", dir.trim());
    }

    pub fn get_tasks(&self) -> Vec<DownloadTask> {
        let Ok(tasks) = self.tasks.lock() else { return Vec::new() };
        tasks.clone()
    }

    pub fn cancel(&self, id: &str) {
        if let Ok(mut set) = self.cancelled.lock() {
            set.insert(id.to_string());
        }
        let mut cancelled_task = None;
        if let Ok(mut tasks) = self.tasks.lock() {
            if let Some(pos) = tasks.iter().position(|t| t.id == id) {
                let mut t = tasks.remove(pos);
                t.status = DownloadStatus::Cancelled;
                cancelled_task = Some(t);
            }
        }
        if let Some(t) = cancelled_task {
            let _ = self.app.emit("download-progress", t);
        }
        self.save_history();
    }

    pub fn cancel_all(&self) {
        let to_cancel: Vec<String> = {
            let Ok(tasks) = self.tasks.lock() else { return };
            tasks
                .iter()
                .filter(|t| {
                    t.status == DownloadStatus::Queued
                        || t.status == DownloadStatus::Downloading
                        || t.status == DownloadStatus::Tagging
                })
                .map(|t| t.id.clone())
                .collect()
        };
        for id in to_cancel {
            self.cancel(&id);
        }
    }

    pub fn find_existing_file(&self, title: &str, video_id: &str, subfolder: Option<&str>) -> Option<PathBuf> {
        let base_name = sanitize_filename(title);
        let exts = ["m4a", "opus"];
        let mut dirs_to_check = Vec::new();

        if let Some(sub) = subfolder {
            if !sub.trim().is_empty() {
                dirs_to_check.push(self.download_dir().join(sanitize_filename(sub)));
            }
        }
        dirs_to_check.push(self.download_dir());

        // Also check any direct subdirectories of download_dir (e.g. playlist folders)
        if let Ok(entries) = std::fs::read_dir(self.download_dir()) {
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        dirs_to_check.push(entry.path());
                    }
                }
            }
        }

        for dir in dirs_to_check {
            for ext in &exts {
                // 1. Direct match with unique video_id suffix: "Title [video_id].ext"
                let unique_candidate = dir.join(format!("{base_name} [{video_id}].{ext}"));
                if unique_candidate.exists() {
                    if let Ok(meta) = std::fs::metadata(&unique_candidate) {
                        if meta.len() > 100_000 {
                            return Some(unique_candidate);
                        }
                    }
                }

                // 2. Legacy or clean match: "Title.ext" - verify that comment tag matches video_id
                let legacy_candidate = dir.join(format!("{base_name}.{ext}"));
                if legacy_candidate.exists() {
                    if let Ok(meta) = std::fs::metadata(&legacy_candidate) {
                        if meta.len() > 100_000 {
                            use lofty::file::TaggedFileExt;
                            use lofty::tag::Accessor;
                            if let Ok(tagged) = lofty::probe::Probe::open(&legacy_candidate).and_then(|p| p.read()) {
                                if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                                    if tag.comment().as_deref() == Some(video_id) {
                                        return Some(legacy_candidate);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    pub fn is_downloaded(&self, video_id: &str) -> bool {
        if let Ok(Some(track)) = self.db.get_downloaded_track(video_id) {
            if Path::new(&track.file_path).exists() {
                return true;
            }
        }
        false
    }

    pub fn is_queued_or_downloaded(&self, video_id: &str) -> bool {
        if let Ok(tasks) = self.tasks.lock() {
            if tasks.iter().any(|t| {
                t.video_id == video_id
                    && (t.status == DownloadStatus::Queued
                        || t.status == DownloadStatus::Downloading
                        || t.status == DownloadStatus::Tagging)
            }) {
                return true;
            }
        }
        self.is_downloaded(video_id)
    }

    pub fn get_downloaded_path(&self, video_id: &str) -> Option<String> {
        if let Ok(Some(track)) = self.db.get_downloaded_track(video_id) {
            if Path::new(&track.file_path).exists() {
                return Some(track.file_path);
            }
        }
        None
    }

    pub fn get_downloaded_ids(&self) -> Vec<String> {
        if let Ok(tracks) = self.db.get_all_downloaded_tracks() {
            tracks
                .into_iter()
                .filter(|t| Path::new(&t.file_path).exists())
                .map(|t| t.video_id)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Retrieve the local cover image path for a downloaded track.
    /// If not already cached in `covers_dir`, attempts to extract the embedded cover
    /// artwork from the downloaded audio file using `lofty`, caches it, and returns the path.
    pub fn get_offline_cover(&self, video_id: &str) -> Option<String> {
        let covers_dir = crate::local::covers_dir(&self.app);
        for ext in ["jpg", "jpeg", "png"] {
            let cached = covers_dir.join(format!("dl-{video_id}.{ext}"));
            if cached.is_file() {
                return Some(cached.to_string_lossy().to_string());
            }
        }

        // If not found in cache, check if we have the downloaded file on disk
        let file_path = self.get_downloaded_path(video_id)?;
        let path = Path::new(&file_path);
        if !path.is_file() {
            return None;
        }

        use lofty::file::TaggedFileExt;
        let tagged = lofty::probe::Probe::open(path).ok()?.read().ok()?;
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
        let pic = tag.pictures().first()?;
        let ext = match pic.mime_type() {
            Some(lofty::picture::MimeType::Png) => "png",
            _ => "jpg",
        };
        let out = covers_dir.join(format!("dl-{video_id}.{ext}"));
        let _ = std::fs::create_dir_all(&covers_dir);
        std::fs::write(&out, pic.data()).ok()?;
        let out_str = out.to_string_lossy().to_string();

        // Update database record if present
        if let Ok(Some(mut track)) = self.db.get_downloaded_track(video_id) {
            track.thumbnail = Some(out_str.clone());
            let _ = self.db.record_download(&track);
        }

        Some(out_str)
    }

    pub fn clear_completed(&self) {
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.retain(|t| {
                t.status != DownloadStatus::Completed
                    && t.status != DownloadStatus::Failed
                    && t.status != DownloadStatus::Cancelled
            });
        }
    }

    /// Return the current concurrency limit (read from DB, default 3, max 5).
    pub fn get_concurrency(&self) -> usize {
        self.db
            .get_setting("download_concurrency")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_CONCURRENT_DOWNLOADS)
            .clamp(1, 5)
    }

    /// Persist a new concurrency limit to the DB and update the active semaphore.
    pub fn set_concurrency(&self, n: usize) {
        use std::sync::atomic::Ordering;
        let n = n.clamp(1, 5);
        let old = self.current_concurrency.swap(n, Ordering::SeqCst);
        self.db.set_setting("download_concurrency", &n.to_string());
        if n > old {
            let diff = n - old;
            // Offset against any pending reductions before adding fresh permits
            let to_add = self
                .pending_reductions
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |cur| {
                    if cur >= diff {
                        Some(cur - diff)
                    } else {
                        Some(0)
                    }
                })
                .map(|old_p| if diff > old_p { diff - old_p } else { 0 })
                .unwrap_or(diff);
            if to_add > 0 {
                self.semaphore.add_permits(to_add);
            }
        } else if old > n {
            let to_reduce = old - n;
            // Try to immediately reclaim available permits; track what we couldn't get as
            // pending reductions that will be consumed the next time a task acquires the semaphore.
            let actually_removed = self.semaphore.forget_permits(to_reduce);
            let still_needed = to_reduce.saturating_sub(actually_removed);
            if still_needed > 0 {
                self.pending_reductions.fetch_add(still_needed, Ordering::SeqCst);
            }
        }
    }

    /// Cancel all active downloads, remove every downloaded file from disk, and clear history.
    /// Returns the number of files deleted.
    pub async fn delete_all_downloads(&self) -> usize {
        // 1. Cancel in-flight tasks
        self.cancel_all();

        // 2. Collect file paths from SQLite and completed tasks
        let mut file_paths: HashSet<String> = HashSet::new();
        if let Ok(tracks) = self.db.get_all_downloaded_tracks() {
            for t in tracks {
                file_paths.insert(t.file_path);
            }
        }
        if let Ok(tasks) = self.tasks.lock() {
            for t in tasks.iter() {
                if let Some(fp) = &t.file_path {
                    file_paths.insert(fp.clone());
                }
            }
        }

        // 3. Delete each file
        let mut deleted = 0usize;
        for path in &file_paths {
            if tokio::fs::remove_file(path).await.is_ok() {
                deleted += 1;
            }
        }

        // 4. Also clean up any stray .part files in the download directory and subfolders
        let dl_dir = self.download_dir();
        let mut dirs = vec![dl_dir.clone()];
        if let Ok(entries) = std::fs::read_dir(&dl_dir) {
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        dirs.push(entry.path());
                    }
                }
            }
        }
        for dir in dirs {
            if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let p = entry.path();
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.ends_with("].m4a.part") || name.ends_with("].opus.part") {
                        let _ = tokio::fs::remove_file(&p).await;
                    }
                }
            }
        }

        // 5. Clear all tasks and SQLite records
        let _ = self.db.clear_all_downloads();
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.clear();
        }

        // 6. Clean up dl-* cached covers from covers_dir
        let covers_dir = crate::local::covers_dir(&self.app);
        if let Ok(entries) = std::fs::read_dir(&covers_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("dl-") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }

        deleted
    }

    /// Queue a single song for download.
    pub fn queue_song(
        self: &Arc<Self>,
        video_id: String,
        title: String,
        artist: String,
        album: Option<String>,
        thumbnail: Option<String>,
        is_upload: bool,
        subfolder: Option<String>,
    ) -> String {
        // 1. Check if already recorded in SQLite
        if let Ok(Some(existing)) = self.db.get_downloaded_track(&video_id) {
            if Path::new(&existing.file_path).exists() {
                let task = DownloadTask {
                    id: format!("{}_db", video_id),
                    video_id: video_id.clone(),
                    title: existing.title,
                    artist: existing.artist,
                    album: existing.album,
                    thumbnail: existing.thumbnail,
                    progress: 100.0,
                    bytes_downloaded: 0,
                    total_bytes: 0,
                    status: DownloadStatus::Completed,
                    error: None,
                    file_path: Some(existing.file_path),
                };
                let _ = self.app.emit("download-progress", task.clone());
                return task.id;
            }
        }

        // 2. Check if file already exists on disk (e.g. from existing library or prior session)
        if let Some(existing_file) = self.find_existing_file(&title, &video_id, subfolder.as_deref()) {
            let file_str = existing_file.to_string_lossy().to_string();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let record = crate::db::DownloadedTrack {
                video_id: video_id.clone(),
                title: title.clone(),
                artist: artist.clone(),
                album: album.clone(),
                thumbnail: thumbnail.clone(),
                file_path: file_str.clone(),
                downloaded_at: now,
            };
            let _ = self.db.record_download(&record);

            let task = DownloadTask {
                id: format!("{}_found", video_id),
                video_id: video_id.clone(),
                title,
                artist,
                album,
                thumbnail,
                progress: 100.0,
                bytes_downloaded: 0,
                total_bytes: 0,
                status: DownloadStatus::Completed,
                error: None,
                file_path: Some(file_str),
            };
            let _ = self.app.emit("download-progress", task.clone());
            return task.id;
        }

        let id = format!("{}_{}", video_id, rand::random::<u32>());
        let task = DownloadTask {
            id: id.clone(),
            video_id: video_id.clone(),
            title: title.clone(),
            artist: artist.clone(),
            album: album.clone(),
            thumbnail: thumbnail.clone(),
            progress: 0.0,
            bytes_downloaded: 0,
            total_bytes: 0,
            status: DownloadStatus::Queued,
            error: None,
            file_path: None,
        };

        if let Ok(mut tasks) = self.tasks.lock() {
            // 3. Check and insert under one lock so concurrent callers cannot both queue
            if let Some(existing) = tasks.iter().find(|t| {
                t.video_id == video_id
                    && (t.status == DownloadStatus::Queued
                        || t.status == DownloadStatus::Downloading
                        || t.status == DownloadStatus::Tagging)
            }) {
                return existing.id.clone();
            }
            tasks.push(task.clone());
        }
        let _ = self.app.emit("download-progress", task);

        let mgr = Arc::clone(self);
        let task_id = id.clone();
        tauri::async_runtime::spawn(async move {
            mgr.run_download(task_id, video_id, title, artist, album, thumbnail, is_upload, subfolder).await;
        });

        id
    }

    async fn run_download(
        &self,
        task_id: String,
        video_id: String,
        title: String,
        artist: String,
        album: Option<String>,
        thumbnail: Option<String>,
        is_upload: bool,
        subfolder: Option<String>,
    ) {
        use std::sync::atomic::Ordering;
        let _permit = loop {
            if self.is_cancelled(&task_id) {
                return;
            }
            let p = match self.semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };
            // Honor pending concurrency reductions: if the limit was lowered while permits were
            // in-flight, consume this slot to reduce semaphore capacity, then re-wait for a valid slot.
            if self.pending_reductions.load(Ordering::SeqCst) > 0 {
                if self
                    .pending_reductions
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                        if v > 0 { Some(v - 1) } else { None }
                    })
                    .is_ok()
                {
                    p.forget();
                    continue;
                }
            }
            break p;
        };

        if self.is_cancelled(&task_id) {
            return;
        }

        // Before starting any network download, check if file already exists on disk
        if let Some(existing_file) = self.find_existing_file(&title, &video_id, subfolder.as_deref()) {
            let file_str = existing_file.to_string_lossy().to_string();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let record = crate::db::DownloadedTrack {
                video_id: video_id.clone(),
                title: title.clone(),
                artist: artist.clone(),
                album: album.clone(),
                thumbnail: thumbnail.clone(),
                file_path: file_str.clone(),
                downloaded_at: now,
            };
            let _ = self.db.record_download(&record);

            self.update_task(&task_id, |t| {
                t.status = DownloadStatus::Completed;
                t.progress = 100.0;
                t.file_path = Some(file_str);
            });
            return;
        }

        self.update_task(&task_id, |t| {
            t.status = DownloadStatus::Downloading;
        });

        let target_dir = match subfolder {
            Some(sub) if !sub.trim().is_empty() => {
                self.download_dir().join(sanitize_filename(&sub))
            }
            _ => self.download_dir(),
        };

        if let Err(e) = tokio::fs::create_dir_all(&target_dir).await {
            self.fail_task(&task_id, format!("Cannot create folder: {e}"));
            return;
        }

        // 1. Resolve stream via orchestrator with respect to disabled stream clients
        let app_state = self.app.state::<crate::state::AppState>();
        let disabled_clients = app_state.disabled_clients();
        let playback_data = match self
            .orchestrator
            .resolve_download(&video_id, is_upload, &disabled_clients)
            .await
        {
            Ok(data) => data,
            Err(e) => {
                self.fail_task(&task_id, format!("Failed to resolve stream: {e:?}"));
                return;
            }
        };

        // 2. Determine file extension: Opus vs M4A with video_id suffix to prevent collisions
        let ext = if is_m4a_itag(playback_data.itag) { "m4a" } else { "opus" };
        let base_name = sanitize_filename(&title);
        let final_path = target_dir.join(format!("{base_name} [{video_id}].{ext}"));
        let part_path = target_dir.join(format!("{base_name} [{video_id}].{ext}.part"));

        // 3. Download stream chunks using shared HTTP client (respects user proxy)
        let client = crate::http::client();

        let mut pos: u64 = 0;
        let mut total_bytes: u64 = 0;

        // Open part file
        let mut file = match tokio::fs::File::create(&part_path).await {
            Ok(f) => f,
            Err(e) => {
                self.fail_task(&task_id, format!("Cannot create file: {e}"));
                return;
            }
        };

        use tokio::io::AsyncWriteExt;

        let mut last_emitted_pct = 0u32;
        let mut last_emitted_time = std::time::Instant::now();

        loop {
            if self.is_cancelled(&task_id) {
                let _ = tokio::fs::remove_file(&part_path).await;
                return;
            }

            let last = if total_bytes > 0 {
                (pos + CHUNK_SIZE - 1).min(total_bytes - 1)
            } else {
                pos + CHUNK_SIZE - 1
            };

            let mut req = client
                .get(&playback_data.stream_url)
                .header(reqwest::header::RANGE, format!("bytes={pos}-{last}"))
                .timeout(Duration::from_secs(30));

            for (k, v) in &playback_data.headers {
                if let (Ok(hk), Ok(hv)) = (
                    reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                    reqwest::header::HeaderValue::from_str(v),
                ) {
                    req = req.header(hk, hv);
                }
            }

            let resp = match req.send().await {
                Ok(r)
                    if r.status().is_success()
                        || r.status() == reqwest::StatusCode::PARTIAL_CONTENT =>
                {
                    r
                }
                Ok(r) => {
                    let _ = tokio::fs::remove_file(&part_path).await;
                    self.fail_task(&task_id, format!("HTTP error: {}", r.status()));
                    return;
                }
                Err(e) => {
                    let _ = tokio::fs::remove_file(&part_path).await;
                    self.fail_task(&task_id, format!("Download network error: {e}"));
                    return;
                }
            };

            // Extract total bytes from Content-Range
            if total_bytes == 0 {
                if let Some(cr) = resp.headers().get(reqwest::header::CONTENT_RANGE) {
                    if let Ok(s) = cr.to_str() {
                        if let Some(slash) = s.rfind('/') {
                            if let Ok(tot) = s[slash + 1..].trim().parse::<u64>() {
                                total_bytes = tot;
                            }
                        }
                    }
                }
                if total_bytes == 0 {
                    if let Some(cl) = resp.content_length() {
                        total_bytes = cl;
                    }
                }
            }

            let mut stream = resp.bytes_stream();
            let mut chunk_bytes: u64 = 0;
            while let Some(item) = stream.next().await {
                if self.is_cancelled(&task_id) {
                    let _ = tokio::fs::remove_file(&part_path).await;
                    return;
                }
                match item {
                    Ok(bytes) => {
                        let len = bytes.len() as u64;
                        if let Err(e) = file.write_all(&bytes).await {
                            let _ = tokio::fs::remove_file(&part_path).await;
                            self.fail_task(&task_id, format!("File write error: {e}"));
                            return;
                        }
                        chunk_bytes += len;
                        pos += len;

                        let new_pct = if total_bytes > 0 {
                            ((pos as f64 / total_bytes as f64) * 100.0) as u32
                        } else {
                            0
                        };
                        let now_inst = std::time::Instant::now();
                        if new_pct != last_emitted_pct || now_inst.duration_since(last_emitted_time) >= Duration::from_millis(250) {
                            last_emitted_pct = new_pct;
                            last_emitted_time = now_inst;
                            self.update_task(&task_id, |t| {
                                t.bytes_downloaded = pos;
                                t.total_bytes = total_bytes;
                                if total_bytes > 0 {
                                    t.progress = ((pos as f64 / total_bytes as f64) * 100.0) as f32;
                                }
                            });
                        }
                    }
                    Err(e) => {
                        let _ = tokio::fs::remove_file(&part_path).await;
                        self.fail_task(&task_id, format!("Stream read error: {e}"));
                        return;
                    }
                }
            }

            if total_bytes > 0 && pos >= total_bytes {
                break;
            }
            if chunk_bytes == 0 {
                if total_bytes > 0 {
                    let _ = tokio::fs::remove_file(&part_path).await;
                    self.fail_task(
                        &task_id,
                        format!("Download ended early at {pos} of {total_bytes} bytes"),
                    );
                    return;
                }
                break;
            }
        }

        if let Err(e) = file.flush().await {
            let _ = tokio::fs::remove_file(&part_path).await;
            self.fail_task(&task_id, format!("File flush error: {e}"));
            return;
        }
        drop(file);

        if self.is_cancelled(&task_id) {
            let _ = tokio::fs::remove_file(&part_path).await;
            return;
        }

        // Rename part to final path
        if let Err(e) = tokio::fs::rename(&part_path, &final_path).await {
            self.fail_task(&task_id, format!("File rename error: {e}"));
            return;
        }

        if self.is_cancelled(&task_id) {
            let _ = tokio::fs::remove_file(&final_path).await;
            return;
        }

        // 4. Tagging
        self.update_task(&task_id, |t| {
            t.status = DownloadStatus::Tagging;
            t.progress = 100.0;
        });

        // Download thumbnail bytes if present (request high-res 1200x1200 master, falling back if needed)
        let raw_thumb = thumbnail.clone().or_else(|| playback_data.thumbnail.clone()).unwrap_or_default();
        let high_res = high_res_cover_url(&raw_thumb);
        let cover_bytes = if !high_res.is_empty() {
            match client.get(&high_res).timeout(Duration::from_secs(15)).send().await {
                Ok(r) if r.status().is_success() => r.bytes().await.ok().map(|b| b.to_vec()),
                _ => {
                    let fallback_url = force_jpeg_url(&raw_thumb);
                    if !fallback_url.is_empty() && fallback_url != high_res {
                        match client.get(&fallback_url).timeout(Duration::from_secs(15)).send().await {
                            Ok(r) if r.status().is_success() => r.bytes().await.ok().map(|b| b.to_vec()),
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
            }
        } else {
            None
        };

        // Cache the high-res cover image to covers_dir for offline UI and MPRIS playback
        let local_cover_path = if let Some(bytes) = &cover_bytes {
            let covers_dir = crate::local::covers_dir(&self.app);
            let _ = std::fs::create_dir_all(&covers_dir);
            let cover_file = covers_dir.join(format!("dl-{video_id}.jpg"));
            if std::fs::write(&cover_file, bytes).is_ok() {
                Some(cover_file.to_string_lossy().to_string())
            } else {
                None
            }
        } else {
            None
        };

        // Fetch lyrics for offline playback and embed in tags
        let lreq = crate::lyrics::LyricsRequest {
            video_id: video_id.clone(),
            title: title.clone(),
            artists: artist.clone(),
            album: album.clone(),
            duration: playback_data.duration.as_deref().and_then(crate::lyrics::duration_str_secs),
            searched: false,
        };
        let lrc_text = match crate::lyrics::get_lyrics(app_state.inner(), lreq, None).await {
            Ok(Some(l)) => {
                tracing::info!(title = %title, source = %l.source, synced = l.synced, "Fetched lyrics for download");
                lyrics_to_lrc(&l)
            }
            Ok(None) => {
                tracing::debug!(title = %title, "No lyrics found for download");
                None
            }
            Err(e) => {
                tracing::warn!(title = %title, error = %e, "Failed to fetch lyrics for download");
                None
            }
        };

        if self.is_cancelled(&task_id) {
            let _ = tokio::fs::remove_file(&final_path).await;
            return;
        }

        let final_path_clone = final_path.clone();
        let title_clone = title.clone();
        let video_id_clone = video_id.clone();
        let artist_clone = artist.clone();
        let album_clone = album.clone();
        let lrc_text_clone = lrc_text.clone();
        let tag_res = tauri::async_runtime::spawn_blocking(move || {
            apply_tags(
                &final_path_clone,
                &video_id_clone,
                &title_clone,
                &artist_clone,
                album_clone.as_deref(),
                cover_bytes.as_deref(),
                lrc_text_clone.as_deref(),
            )
        })
        .await;

        match tag_res {
            Ok(Ok(())) => tracing::info!(title = %title, "Tagged audio file successfully"),
            Ok(Err(e)) => tracing::warn!(title = %title, error = %e, "Tagging audio file failed"),
            Err(e) => tracing::warn!(title = %title, error = ?e, "Failed to join tagging task"),
        }

        if self.is_cancelled(&task_id) {
            let _ = tokio::fs::remove_file(&final_path).await;
            return;
        }

        let final_path_str = final_path.to_string_lossy().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let record = crate::db::DownloadedTrack {
            video_id: video_id.clone(),
            title: title.clone(),
            artist: artist.clone(),
            album: album.clone(),
            thumbnail: local_cover_path.or_else(|| thumbnail.clone()),
            file_path: final_path_str.clone(),
            downloaded_at: now,
        };
        let _ = self.db.record_download(&record);

        self.update_task(&task_id, |t| {
            t.status = DownloadStatus::Completed;
            t.progress = 100.0;
            t.file_path = Some(final_path_str);
        });
    }

    fn is_cancelled(&self, task_id: &str) -> bool {
        if let Ok(set) = self.cancelled.lock() {
            set.contains(task_id)
        } else {
            false
        }
    }

    fn update_task<F>(&self, task_id: &str, f: F)
    where
        F: FnOnce(&mut DownloadTask),
    {
        let mut task_opt = None;
        if let Ok(mut tasks) = self.tasks.lock() {
            if let Some(t) = tasks.iter_mut().find(|t| t.id == task_id) {
                f(t);
                task_opt = Some(t.clone());
            }
        }
        if let Some(t) = task_opt {
            let _ = self.app.emit("download-progress", t);
        }
    }

    fn fail_task(&self, task_id: &str, error: String) {
        tracing::error!(task_id, %error, "Download failed");
        self.update_task(task_id, |t| {
            t.status = DownloadStatus::Failed;
            t.error = Some(error);
        });
    }
}

fn is_m4a_itag(itag: i64) -> bool {
    matches!(itag, 140 | 141 | 139 | 256 | 258)
}

pub fn sanitize_filename(name: &str) -> String {
    let invalid = ['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0'];
    let s: String = name.chars().map(|c| if invalid.contains(&c) { '_' } else { c }).collect();
    let trimmed = s.trim().trim_matches('.');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn high_res_cover_url(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    if url.contains("googleusercontent.com") {
        if let Some(idx) = url.find('=') {
            format!("{}=w1200-h1200-l90-rj", &url[..idx])
        } else {
            format!("{url}=w1200-h1200-l90-rj")
        }
    } else if url.contains("ytimg.com/vi/") {
        if let Some(slash) = url.rfind('/') {
            format!("{}/maxresdefault.jpg", &url[..slash])
        } else {
            url.to_string()
        }
    } else {
        url.to_string()
    }
}

pub fn force_jpeg_url(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    if url.contains("lh3.googleusercontent.com") || url.contains("yt3.googleusercontent.com") {
        if url.ends_with("-rj") {
            url.to_string()
        } else if url.contains('=') {
            format!("{url}-rj")
        } else {
            format!("{url}=s0-rj")
        }
    } else {
        url.to_string()
    }
}

fn lyrics_to_lrc(l: &crate::lyrics::Lyrics) -> Option<String> {
    if l.instrumental || l.lines.is_empty() {
        return None;
    }
    if l.synced {
        let lrc = l
            .lines
            .iter()
            .filter_map(|line| {
                let ms = line.time_ms?;
                let min = ms / 60_000;
                let sec = (ms % 60_000) / 1_000;
                let centis = (ms % 1_000) / 10;
                Some(format!("[{:02}:{:02}.{:02}] {}", min, sec, centis, line.text))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if lrc.trim().is_empty() {
            None
        } else {
            Some(lrc)
        }
    } else {
        let text = l.lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>().join("\n");
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

fn apply_tags(
    path: &Path,
    video_id: &str,
    title: &str,
    artist: &str,
    album: Option<&str>,
    cover_bytes: Option<&[u8]>,
    lrc_text: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use lofty::file::TaggedFileExt;
    use lofty::probe::Probe;
    use lofty::tag::{Accessor, ItemKey, TagExt};

    let mut tagged_file = Probe::open(path)?.read()?;
    let tag = match tagged_file.primary_tag_mut() {
        Some(t) => t,
        None => {
            if let Some(first) = tagged_file.first_tag_mut() {
                first
            } else {
                let tag_type = tagged_file.primary_tag_type();
                tagged_file.insert_tag(lofty::tag::Tag::new(tag_type));
                tagged_file.primary_tag_mut().ok_or_else(|| "cannot create tag")?
            }
        }
    };

    tag.set_title(title.to_string());
    tag.set_artist(artist.to_string());
    tag.set_comment(video_id.to_string());
    if let Some(alb) = album {
        tag.set_album(alb.to_string());
    }

    if let Some(lrc) = lrc_text {
        tag.insert_text(ItemKey::Lyrics, lrc.to_string());
    }

    if let Some(bytes) = cover_bytes {
        let mime = if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            Some(lofty::picture::MimeType::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(lofty::picture::MimeType::Jpeg)
        } else {
            None
        };

        if let Some(m) = mime {
            let pic = lofty::picture::Picture::new_unchecked(
                lofty::picture::PictureType::CoverFront,
                Some(m),
                None,
                bytes.to_vec(),
            );
            tag.push_picture(pic);
        } else {
            tracing::warn!(title, "Cover bytes are neither JPEG nor PNG; skipping artwork tag");
        }
    }

    tag.save_to_path(path, lofty::config::WriteOptions::default())?;
    Ok(())
}

pub fn open_folder(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(path).spawn().ok();
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn().ok();
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(path).spawn().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("AC/DC: Back in Black?"), "AC_DC_ Back in Black_");
        assert_eq!(sanitize_filename("..."), "untitled");
        assert_eq!(sanitize_filename("Clean Name"), "Clean Name");
    }

    #[test]
    fn test_is_m4a_itag() {
        assert!(is_m4a_itag(140));
        assert!(is_m4a_itag(139));
        assert!(!is_m4a_itag(251));
    }

    #[test]
    fn test_high_res_cover_url() {
        assert_eq!(
            high_res_cover_url("https://lh3.googleusercontent.com/abc=w544-h544"),
            "https://lh3.googleusercontent.com/abc=w1200-h1200-l90-rj"
        );
        assert_eq!(
            high_res_cover_url("https://lh3.googleusercontent.com/abc=s120"),
            "https://lh3.googleusercontent.com/abc=w1200-h1200-l90-rj"
        );
        assert_eq!(
            high_res_cover_url("https://i.ytimg.com/vi/abc/hqdefault.jpg"),
            "https://i.ytimg.com/vi/abc/maxresdefault.jpg"
        );
        assert_eq!(high_res_cover_url(""), "");
    }

    #[test]
    fn test_force_jpeg_url() {
        assert_eq!(
            force_jpeg_url("https://lh3.googleusercontent.com/abc=w544-h544"),
            "https://lh3.googleusercontent.com/abc=w544-h544-rj"
        );
        assert_eq!(
            force_jpeg_url("https://lh3.googleusercontent.com/abc=w544-h544-rj"),
            "https://lh3.googleusercontent.com/abc=w544-h544-rj"
        );
        assert_eq!(
            force_jpeg_url("https://lh3.googleusercontent.com/abc"),
            "https://lh3.googleusercontent.com/abc=s0-rj"
        );
        assert_eq!(
            force_jpeg_url("https://i.ytimg.com/vi/abc/hqdefault.jpg"),
            "https://i.ytimg.com/vi/abc/hqdefault.jpg"
        );
        assert_eq!(force_jpeg_url(""), "");
    }
}
