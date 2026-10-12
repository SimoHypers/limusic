//! Background karaoke engine for preparing instrumentals (current + next lookahead).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use karaoke::cache::InstrumentalCache;
use karaoke::cache_codec::CacheFormat;
use karaoke::jobs::DesiredSet;
pub use karaoke::jobs::PrepareRequest;
use karaoke::mdx::MdxParams;
use crate::karaoke_model::ModelManager;
use tauri::{AppHandle, Emitter};

struct Shared {
    enabled: bool,
    desired: DesiredSet,
    running: Option<(String, Arc<AtomicBool>)>,
}

#[allow(dead_code)]
pub struct KaraokeEngine {
    app: AppHandle,
    models: Arc<ModelManager>,
    cache: Arc<InstrumentalCache>,
    shared: Arc<(Mutex<Shared>, Condvar)>,
    ready_tx: tokio::sync::broadcast::Sender<String>,
}

fn sanitize_error(err: &str) -> String {
    let mut sanitized = err.to_string();
    while let Some(start) = sanitized.find("http://").or_else(|| sanitized.find("https://")) {
        let end = sanitized[start..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ')')
            .map(|e| start + e)
            .unwrap_or(sanitized.len());
        sanitized.replace_range(start..end, "[url]");
    }
    sanitized
}

fn evict_desired(cache: &InstrumentalCache, desired: &DesiredSet) {
    let keep: Vec<PathBuf> = desired.ids()
        .iter()
        .filter_map(|id| cache.get(id))
        .collect();
    let _ = cache.evict(&keep);
}

#[allow(dead_code)]
impl KaraokeEngine {
    pub fn new(app: AppHandle, data_dir: &Path, models: Arc<ModelManager>) -> Arc<Self> {
        let cache_dir = data_dir.join("karaoke-cache");
        let max_bytes = 2 * 1024 * 1024 * 1024_u64; // 2 GiB
        let cache = match InstrumentalCache::new(cache_dir, CacheFormat::Flac, max_bytes) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!(error = %e, "failed to create karaoke cache, falling back to temp dir");
                let fallback = std::env::temp_dir().join("limusic-karaoke-cache");
                Arc::new(InstrumentalCache::new(fallback, CacheFormat::Flac, max_bytes).unwrap())
            }
        };

        let (ready_tx, _) = tokio::sync::broadcast::channel(64);
        let shared = Arc::new((
            Mutex::new(Shared {
                enabled: false,
                desired: DesiredSet::default(),
                running: None,
            }),
            Condvar::new(),
        ));

        let engine = Arc::new(Self {
            app,
            models,
            cache,
            shared: shared.clone(),
            ready_tx,
        });

        let worker_app = engine.app.clone();
        let worker_models = engine.models.clone();
        let worker_cache = engine.cache.clone();
        let worker_shared = shared.clone();
        let worker_ready_tx = engine.ready_tx.clone();

        std::thread::Builder::new()
            .name("karaoke-worker".to_string())
            .spawn(move || {
                let mut model: Option<karaoke::mdx::MdxModel> = None;
                let mut notified_needs_model = false;

                let (lock, condvar) = &*worker_shared;
                let mut shared_guard = lock.lock().unwrap();

                loop {
                    if !shared_guard.enabled {
                        if model.is_some() {
                            tracing::info!(target: "app_lib::karaoke", reason = "disabled", "karaoke model unloaded");
                            model = None;
                        }
                        notified_needs_model = false;
                        shared_guard = condvar.wait(shared_guard).unwrap();
                        continue;
                    }

                    if !worker_models.is_installed() {
                        if model.is_some() {
                            tracing::info!(target: "app_lib::karaoke", reason = "error", "karaoke model unloaded");
                            model = None;
                        }
                        if !notified_needs_model {
                            let _ = worker_app.emit("karaoke-status", &serde_json::json!({"state": "needs-model"}));
                            notified_needs_model = true;
                        }
                        shared_guard.desired.clear();
                        shared_guard = condvar.wait(shared_guard).unwrap();
                        continue;
                    } else {
                        notified_needs_model = false;
                    }

                    if let Some(req) = shared_guard.desired.next_pending(|id| worker_cache.get(id).is_some()) {
                        if let Some(_path) = worker_cache.get(&req.video_id) {
                            tracing::info!(target: "app_lib::karaoke", video_id = %req.video_id, "karaoke cache hit");
                            let _ = worker_app.emit("karaoke-status", &serde_json::json!({
                                "videoId": &req.video_id,
                                "state": "ready"
                            }));
                            let _ = worker_ready_tx.send(req.video_id.clone());
                            shared_guard.desired.complete(&req.video_id);
                            evict_desired(&worker_cache, &shared_guard.desired);
                            continue;
                        }

                        if model.is_none() {
                            let start_load = std::time::Instant::now();
                            match karaoke::mdx::MdxModel::load_auto(&worker_models.model_path(), MdxParams::default()) {
                                Ok((m, dev)) => {
                                    let load_ms = start_load.elapsed().as_millis();
                                    tracing::info!(target: "app_lib::karaoke", load_ms, device = ?dev, "karaoke model loaded");
                                    model = Some(m);
                                }
                                Err(e) => {
                                    if model.is_some() {
                                        tracing::info!(target: "app_lib::karaoke", reason = "error", "karaoke model unloaded");
                                        model = None;
                                    }
                                    let err_str = e.to_string();
                                    let msg = sanitize_error(&err_str);
                                    tracing::warn!(target: "app_lib::karaoke", error = %err_str, "karaoke model load failed");
                                    let _ = worker_app.emit("karaoke-status", &serde_json::json!({
                                        "state": "error",
                                        "message": msg
                                    }));
                                    shared_guard.desired.clear();
                                    shared_guard = condvar.wait(shared_guard).unwrap();
                                    continue;
                                }
                            }
                        }

                        let cancel = Arc::new(AtomicBool::new(false));
                        shared_guard.running = Some((req.video_id.clone(), cancel.clone()));
                        drop(shared_guard);

                        tracing::info!(target: "app_lib::karaoke", video_id = %req.video_id, "karaoke prepare start");

                        let mut throttle = crate::karaoke_model::ProgressThrottle::new(Duration::from_millis(250));
                        let video_id_clone = req.video_id.clone();
                        let app_clone = worker_app.clone();

                        let res = karaoke::pipeline::prepare_instrumental(
                            &worker_cache,
                            &req.video_id,
                            model.as_mut().unwrap(),
                            &mut |wav| player::decode_to_wav(&req.url, wav, &req.headers, &cancel),
                            &cancel,
                            &mut |i, n| {
                                if throttle.should_emit() {
                                    let progress = if n > 0 { i as f64 / n as f64 } else { 0.0 };
                                    let _ = app_clone.emit("karaoke-status", &serde_json::json!({
                                        "videoId": &video_id_clone,
                                        "state": "preparing",
                                        "progress": progress
                                    }));
                                }
                            },
                        );

                        shared_guard = lock.lock().unwrap();
                        shared_guard.running = None;

                        match res {
                            Ok(_) => {
                                let _ = worker_app.emit("karaoke-status", &serde_json::json!({
                                    "videoId": &req.video_id,
                                    "state": "ready"
                                }));
                                let _ = worker_ready_tx.send(req.video_id.clone());
                                shared_guard.desired.complete(&req.video_id);
                                evict_desired(&worker_cache, &shared_guard.desired);
                            }
                            Err(e) => {
                                let err_str = e.to_string();
                                if err_str.contains("cancelled") {
                                    tracing::warn!(target: "app_lib::karaoke", video_id = %req.video_id, "karaoke prepare cancelled");
                                    // Cancelled, do nothing
                                } else {
                                    tracing::warn!(target: "app_lib::karaoke", video_id = %req.video_id, error = %err_str, "karaoke prepare failed");
                                    let msg = sanitize_error(&err_str);
                                    let _ = worker_app.emit("karaoke-status", &serde_json::json!({
                                        "videoId": &req.video_id,
                                        "state": "error",
                                        "message": msg
                                    }));
                                    shared_guard.desired.complete(&req.video_id);
                                }
                            }
                        }
                    } else {
                        let (guard, timeout_result) = condvar.wait_timeout(shared_guard, Duration::from_secs(30)).unwrap();
                        shared_guard = guard;
                        if timeout_result.timed_out() {
                            if model.is_some() {
                                tracing::info!(target: "app_lib::karaoke", reason = "idle", "karaoke model unloaded");
                                model = None;
                            }
                        }
                    }
                }
            })
            .unwrap();

        engine
    }

    pub fn set_enabled(&self, on: bool) {
        let (lock, condvar) = &*self.shared;
        let mut shared = lock.lock().unwrap();
        shared.enabled = on;
        if !on {
            shared.desired.clear();
            if let Some((_, cancel)) = shared.running.take() {
                cancel.store(true, Ordering::SeqCst);
            }
            let _ = self.app.emit("karaoke-status", &serde_json::json!({"state": "off"}));
        }
        condvar.notify_one();
    }

    pub fn is_enabled(&self) -> bool {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap().enabled
    }

    pub fn want(&self, reqs: Vec<PrepareRequest>) {
        let (lock, condvar) = &*self.shared;
        let mut shared = lock.lock().unwrap();
        if !shared.enabled {
            return;
        }
        let current_ids = shared.desired.ids();
        let mut new_ids = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for r in &reqs {
            if seen.insert(r.video_id.clone()) {
                new_ids.push(r.video_id.clone());
                if new_ids.len() >= DesiredSet::MAX_DESIRED {
                    break;
                }
            }
        }
        if current_ids == new_ids {
            return;
        }
        let ids: Vec<&str> = new_ids.iter().map(|s| s.as_str()).collect();
        tracing::info!(target: "app_lib::karaoke", ?ids, "karaoke wants updated");
        let removed = shared.desired.replace(reqs);
        if let Some((ref running_id, ref cancel)) = shared.running {
            if removed.contains(running_id) {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        evict_desired(&self.cache, &shared.desired);
        condvar.notify_one();
    }

    pub fn ready_path(&self, video_id: &str) -> Option<PathBuf> {
        self.cache.get(video_id)
    }

    pub fn subscribe_ready(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.ready_tx.subscribe()
    }

    pub fn cache_bytes(&self) -> u64 {
        self.cache.total_size()
    }

    pub fn clear_cache(&self) {
        let (lock, condvar) = &*self.shared;
        let mut shared = lock.lock().unwrap();
        if let Some((_, cancel)) = shared.running.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        shared.desired.clear();
        drop(shared);
        let _ = self.cache.clear();
        condvar.notify_one();
    }
}
