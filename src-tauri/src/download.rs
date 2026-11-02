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
const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 5;

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
            .clamp(1, 10);
        let mgr = Self {
            tasks: Arc::new(Mutex::new(Vec::new())),
            cancelled: Arc::new(Mutex::new(HashSet::new())),
            semaphore: Arc::new(tokio::sync::Semaphore::new(concurrency)),
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

    fn history_path(&self) -> PathBuf {
        self.app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from(".")).join("downloads.json")
    }

    fn load_history(&self) -> Vec<DownloadTask> {
        let path = self.history_path();
        if !path.exists() {
            return Vec::new();
        }
        let Ok(file) = std::fs::File::open(&path) else { return Vec::new() };
        let mut list: Vec<DownloadTask> = serde_json::from_reader(file).unwrap_or_default();
        list.retain(|t| {
            if t.status == DownloadStatus::Completed {
                if let Some(fp) = &t.file_path {
                    return Path::new(fp).exists();
                }
            }
            false
        });
        list
    }

    fn save_history(&self) {
        let path = self.history_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tasks = self.get_tasks();
        let completed: Vec<_> = tasks
            .into_iter()
            .filter(|t| t.status == DownloadStatus::Completed)
            .collect();
        if let Ok(file) = std::fs::File::create(&path) {
            let _ = serde_json::to_writer_pretty(file, &completed);
        }
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

    #[allow(dead_code)]
    pub fn is_downloaded(&self, video_id: &str) -> bool {
        let Ok(tasks) = self.tasks.lock() else { return false };
        tasks.iter().any(|t| {
            t.video_id == video_id
                && t.status == DownloadStatus::Completed
                && t.file_path.as_ref().map_or(false, |p| Path::new(p).exists())
        })
    }

    pub fn is_queued_or_downloaded(&self, video_id: &str) -> bool {
        let Ok(tasks) = self.tasks.lock() else { return false };
        tasks.iter().any(|t| {
            t.video_id == video_id && (
                t.status == DownloadStatus::Queued
                || t.status == DownloadStatus::Downloading
                || t.status == DownloadStatus::Tagging
                || (t.status == DownloadStatus::Completed && t.file_path.as_ref().map_or(false, |p| Path::new(p).exists()))
            )
        })
    }

    pub fn get_downloaded_path(&self, video_id: &str) -> Option<String> {
        let Ok(tasks) = self.tasks.lock() else { return None };
        tasks.iter().find_map(|t| {
            if t.video_id == video_id && t.status == DownloadStatus::Completed {
                if let Some(fp) = &t.file_path {
                    if Path::new(fp).exists() {
                        return Some(fp.clone());
                    }
                }
            }
            None
        })
    }

    pub fn clear_completed(&self) {
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.retain(|t| {
                t.status != DownloadStatus::Completed
                    && t.status != DownloadStatus::Failed
                    && t.status != DownloadStatus::Cancelled
            });
        }
        self.save_history();
    }

    /// Return the current concurrency limit (read from DB, default 5).
    pub fn get_concurrency(&self) -> usize {
        self.db
            .get_setting("download_concurrency")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_CONCURRENT_DOWNLOADS)
            .clamp(1, 10)
    }

    /// Persist a new concurrency limit to the DB (applied on next app launch).
    pub fn set_concurrency(&self, n: usize) {
        let n = n.clamp(1, 10);
        self.db.set_setting("download_concurrency", &n.to_string());
    }

    /// Cancel all active downloads, remove every downloaded file from disk, and clear history.
    /// Returns the number of files deleted.
    pub async fn delete_all_downloads(&self) -> usize {
        // 1. Cancel in-flight tasks
        self.cancel_all();

        // 2. Collect file paths of completed tasks
        let file_paths: Vec<String> = {
            let Ok(tasks) = self.tasks.lock() else { return 0 };
            tasks
                .iter()
                .filter_map(|t| t.file_path.clone())
                .collect()
        };

        // 3. Delete each file
        let mut deleted = 0usize;
        for path in &file_paths {
            if tokio::fs::remove_file(path).await.is_ok() {
                deleted += 1;
            }
        }

        // 4. Also clean up any stray .part files in the download directory
        let dl_dir = self.download_dir();
        if let Ok(mut entries) = tokio::fs::read_dir(&dl_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("part") {
                    let _ = tokio::fs::remove_file(&p).await;
                }
            }
        }

        // 5. Clear all tasks and history
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.clear();
        }
        self.save_history();

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
        subfolder: Option<String>,
    ) -> String {
        if let Some(existing) = {
            let Ok(tasks) = self.tasks.lock() else { return String::new() };
            tasks.iter().find(|t| t.video_id == video_id && t.status == DownloadStatus::Completed).cloned()
        } {
            if let Some(fp) = &existing.file_path {
                if Path::new(fp).exists() {
                    let _ = self.app.emit("download-progress", existing.clone());
                    return existing.id;
                }
            }
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
            tasks.push(task.clone());
        }
        let _ = self.app.emit("download-progress", task);

        let mgr = Arc::clone(self);
        let task_id = id.clone();
        tauri::async_runtime::spawn(async move {
            mgr.run_download(task_id, video_id, title, artist, album, thumbnail, subfolder).await;
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
        subfolder: Option<String>,
    ) {
        let _permit = match self.semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => return,
        };

        if self.is_cancelled(&task_id) {
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

        // 1. Resolve stream via orchestrator
        let playback_data = match self
            .orchestrator
            .resolve_download(&video_id)
            .await
        {
            Ok(data) => data,
            Err(e) => {
                self.fail_task(&task_id, format!("Failed to resolve stream: {e:?}"));
                return;
            }
        };

        // 2. Determine file extension: Opus vs M4A
        let ext = if is_m4a_itag(playback_data.itag) { "m4a" } else { "opus" };
        let base_name = sanitize_filename(&title);
        let final_path = target_dir.join(format!("{base_name}.{ext}"));
        let part_path = target_dir.join(format!("{base_name}.{ext}.part"));

        // 3. Download stream chunks
        let client =
            reqwest::Client::builder().timeout(Duration::from_secs(30)).build().unwrap_or_default();

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
                .header(reqwest::header::RANGE, format!("bytes={pos}-{last}"));

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

                        self.update_task(&task_id, |t| {
                            t.bytes_downloaded = pos;
                            t.total_bytes = total_bytes;
                            if total_bytes > 0 {
                                t.progress = ((pos as f64 / total_bytes as f64) * 100.0) as f32;
                            }
                        });
                    }
                    Err(e) => {
                        let _ = tokio::fs::remove_file(&part_path).await;
                        self.fail_task(&task_id, format!("Stream read error: {e}"));
                        return;
                    }
                }
            }

            if chunk_bytes == 0 || (total_bytes > 0 && pos >= total_bytes) {
                break;
            }
        }

        if let Err(e) = file.flush().await {
            let _ = tokio::fs::remove_file(&part_path).await;
            self.fail_task(&task_id, format!("File flush error: {e}"));
            return;
        }
        drop(file);

        // Rename part to final path
        if let Err(e) = tokio::fs::rename(&part_path, &final_path).await {
            self.fail_task(&task_id, format!("File rename error: {e}"));
            return;
        }

        // 4. Tagging
        self.update_task(&task_id, |t| {
            t.status = DownloadStatus::Tagging;
            t.progress = 100.0;
        });

        // Download thumbnail bytes if present (request high-res 1200x1200 master, falling back if needed)
        let raw_thumb = thumbnail.or_else(|| playback_data.thumbnail.clone()).unwrap_or_default();
        let high_res = high_res_cover_url(&raw_thumb);
        let cover_bytes = if !high_res.is_empty() {
            match client.get(&high_res).send().await {
                Ok(r) if r.status().is_success() => r.bytes().await.ok().map(|b| b.to_vec()),
                _ => {
                    let fallback_url = force_jpeg_url(&raw_thumb);
                    if !fallback_url.is_empty() && fallback_url != high_res {
                        match client.get(&fallback_url).send().await {
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

        let final_path_clone = final_path.clone();
        let title_clone = title.clone();
        let tag_res = tauri::async_runtime::spawn_blocking(move || {
            apply_tags(&final_path_clone, &title_clone, &artist, album.as_deref(), cover_bytes.as_deref())
        })
        .await;

        match tag_res {
            Ok(Ok(())) => tracing::info!(title = %title, "Tagged audio file successfully"),
            Ok(Err(e)) => tracing::warn!(title = %title, error = %e, "Tagging audio file failed"),
            Err(e) => tracing::warn!(title = %title, error = ?e, "Failed to join tagging task"),
        }

        let final_path_str = final_path.to_string_lossy().to_string();
        self.update_task(&task_id, |t| {
            t.status = DownloadStatus::Completed;
            t.progress = 100.0;
            t.file_path = Some(final_path_str);
        });
        self.save_history();
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

fn apply_tags(
    path: &Path,
    title: &str,
    artist: &str,
    album: Option<&str>,
    cover_bytes: Option<&[u8]>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use lofty::file::TaggedFileExt;
    use lofty::probe::Probe;
    use lofty::tag::{Accessor, TagExt};

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
    if let Some(alb) = album {
        tag.set_album(alb.to_string());
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
