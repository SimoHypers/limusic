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
use tauri::{AppHandle, Emitter, Manager};

struct Shared {
    enabled: bool,
    desired: DesiredSet,
    last_reqs: Vec<PrepareRequest>,
    running: Option<(String, Arc<AtomicBool>, u64)>, // (video_id, cancel, job_seq)
    job_seq: u64,
    last_progress: f64,
    failed_ids: std::collections::HashSet<String>,
    announced_ready: std::collections::HashSet<String>,
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

fn emit_queue(app: &AppHandle, shared: &Shared, cache: &InstrumentalCache) {
    if !shared.enabled {
        let _ = app.emit("karaoke-queue", &serde_json::json!({ "slots": [] }));
        return;
    }
    let desired_ids = shared.desired.ids();
    let running = shared.running.as_ref().map(|(id, _, _)| (id.as_str(), shared.last_progress));
    let slots = karaoke::jobs::build_slots(&desired_ids, running, &shared.failed_ids, |id| cache.contains(id));
    let _ = app.emit("karaoke-queue", &serde_json::json!({ "slots": slots }));
}

fn evict_desired(cache: &InstrumentalCache, desired_ids: &[String], announced_ready: &mut std::collections::HashSet<String>) {
    let keep: Vec<PathBuf> = desired_ids
        .iter()
        .filter_map(|id| cache.get(id))
        .collect();
    let _ = cache.evict(&keep);
    announced_ready.retain(|id| cache.contains(id));
}

#[allow(dead_code)]
impl KaraokeEngine {
    pub fn new(app: AppHandle, data_dir: &Path, models: Arc<ModelManager>) -> Arc<Self> {
        let cache_dir = data_dir.join("karaoke-cache");
        let limit_mb = app.try_state::<Arc<crate::state::AppState>>()
            .map(|st| crate::state::saved_karaoke_cache_limit_mb(&st.db))
            .unwrap_or(2048)
            .clamp(512, 20480);
        let max_bytes = limit_mb * 1024 * 1024;
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
                last_reqs: Vec::new(),
                running: None,
                job_seq: 0,
                last_progress: 0.0,
                failed_ids: std::collections::HashSet::new(),
                announced_ready: std::collections::HashSet::new(),
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
                        emit_queue(&worker_app, &shared_guard, &worker_cache);
                        shared_guard = condvar.wait(shared_guard).unwrap();
                        continue;
                    } else {
                        notified_needs_model = false;
                    }

                    if let Some(req) = shared_guard.desired.next_pending(|id| worker_cache.contains(id)) {
                        if worker_cache.contains(&req.video_id) {
                            tracing::info!(target: "app_lib::karaoke", video_id = %req.video_id, "karaoke cache hit");
                            if !shared_guard.announced_ready.contains(&req.video_id) {
                                let _ = worker_app.emit("karaoke-status", &serde_json::json!({
                                    "videoId": &req.video_id,
                                    "state": "ready"
                                }));
                                let _ = worker_ready_tx.send(req.video_id.clone());
                                shared_guard.announced_ready.insert(req.video_id.clone());
                            }
                            shared_guard.desired.complete(&req.video_id);
                            emit_queue(&worker_app, &shared_guard, &worker_cache);
                            let desired_ids = shared_guard.desired.ids();
                            evict_desired(&worker_cache, &desired_ids, &mut shared_guard.announced_ready);
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
                                    emit_queue(&worker_app, &shared_guard, &worker_cache);
                                    shared_guard = condvar.wait(shared_guard).unwrap();
                                    continue;
                                }
                            }
                        }

                        let cancel = Arc::new(AtomicBool::new(false));
                        shared_guard.job_seq = shared_guard.job_seq.wrapping_add(1);
                        let current_job_seq = shared_guard.job_seq;
                        shared_guard.running = Some((req.video_id.clone(), cancel.clone(), current_job_seq));
                        shared_guard.last_progress = 0.0;
                        emit_queue(&worker_app, &shared_guard, &worker_cache);
                        drop(shared_guard);

                        tracing::info!(target: "app_lib::karaoke", video_id = %req.video_id, "karaoke prepare start");

                        let mut throttle = crate::karaoke_model::ProgressThrottle::new(Duration::from_millis(250));
                        let video_id_clone = req.video_id.clone();
                        let app_clone = worker_app.clone();
                        let shared_ref = worker_shared.clone();
                        let worker_cache_clone = worker_cache.clone();

                        let res = karaoke::pipeline::prepare_instrumental(
                            &worker_cache,
                            &req.video_id,
                            model.as_mut().unwrap(),
                            &mut |wav| player::decode_to_wav(&req.url, wav, &req.headers, &cancel),
                            &cancel,
                            &mut |i, n| {
                                if throttle.should_emit() {
                                    let progress = if n > 0 { i as f64 / n as f64 } else { 0.0 };
                                    let (lock, _) = &*shared_ref;
                                    let mut guard = lock.lock().unwrap();
                                    guard.last_progress = progress;
                                    let _ = app_clone.emit("karaoke-status", &serde_json::json!({
                                        "videoId": &video_id_clone,
                                        "state": "preparing",
                                        "progress": progress
                                    }));
                                    emit_queue(&app_clone, &guard, &worker_cache_clone);
                                }
                            },
                        );

                        shared_guard = lock.lock().unwrap();
                        shared_guard.running = None;
                        shared_guard.last_progress = 0.0;

                        match res {
                            Ok(_) => {
                                if !shared_guard.announced_ready.contains(&req.video_id) {
                                    let _ = worker_app.emit("karaoke-status", &serde_json::json!({
                                        "videoId": &req.video_id,
                                        "state": "ready"
                                    }));
                                    let _ = worker_ready_tx.send(req.video_id.clone());
                                    shared_guard.announced_ready.insert(req.video_id.clone());
                                }
                                shared_guard.desired.complete(&req.video_id);
                                emit_queue(&worker_app, &shared_guard, &worker_cache);
                                let desired_ids = shared_guard.desired.ids();
                                evict_desired(&worker_cache, &desired_ids, &mut shared_guard.announced_ready);
                            }
                            Err(e) => {
                                let err_str = e.to_string();
                                if err_str.contains("cancelled") {
                                    tracing::warn!(target: "app_lib::karaoke", video_id = %req.video_id, "karaoke prepare cancelled");
                                    emit_queue(&worker_app, &shared_guard, &worker_cache);
                                } else {
                                    tracing::warn!(target: "app_lib::karaoke", video_id = %req.video_id, error = %err_str, "karaoke prepare failed");
                                    let msg = sanitize_error(&err_str);
                                    let _ = worker_app.emit("karaoke-status", &serde_json::json!({
                                        "videoId": &req.video_id,
                                        "state": "error",
                                        "message": msg
                                    }));
                                    shared_guard.failed_ids.insert(req.video_id.clone());
                                    shared_guard.desired.complete(&req.video_id);
                                    emit_queue(&worker_app, &shared_guard, &worker_cache);
                                }
                            }
                        }
                    } else {
                        emit_queue(&worker_app, &shared_guard, &worker_cache);
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
            shared.last_reqs.clear();
            shared.failed_ids.clear();
            shared.announced_ready.clear();
            shared.last_progress = 0.0;
            if let Some((_, cancel, _)) = shared.running.take() {
                cancel.store(true, Ordering::SeqCst);
            }
            let _ = self.app.emit("karaoke-status", &serde_json::json!({"state": "off"}));
            let _ = self.app.emit("karaoke-queue", &serde_json::json!({ "slots": [] }));
        } else {
            emit_queue(&self.app, &shared, &self.cache);
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
        shared.last_reqs = reqs.clone();
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
        shared.failed_ids.clear();

        for id in &new_ids {
            if self.cache.contains(id) {
                if shared.announced_ready.insert(id.clone()) {
                    let _ = self.app.emit("karaoke-status", &serde_json::json!({
                        "videoId": id,
                        "state": "ready"
                    }));
                    let _ = self.ready_tx.send(id.clone());
                }
            }
        }

        if let Some((ref running_id, ref cancel, running_seq)) = shared.running {
            if removed.contains(running_id) {
                let running_id_clone = running_id.clone();
                let _cancel_clone = cancel.clone();
                let shared_ref = self.shared.clone();
                let app_handle = self.app.clone();
                let cache_clone = self.cache.clone();
                tracing::debug!(target: "app_lib::karaoke", video_id = %running_id_clone, "karaoke cancel deferred");
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(1500));
                    let (lock, _condvar) = &*shared_ref;
                    let guard = lock.lock().unwrap();
                    if let Some((ref r_id, ref c, seq)) = guard.running {
                        if seq == running_seq && r_id == &running_id_clone {
                            if !guard.desired.contains(&running_id_clone) {
                                c.store(true, Ordering::SeqCst);
                                tracing::debug!(target: "app_lib::karaoke", video_id = %running_id_clone, "karaoke cancel executed");
                                emit_queue(&app_handle, &guard, &cache_clone);
                            } else {
                                tracing::debug!(target: "app_lib::karaoke", video_id = %running_id_clone, "karaoke cancel skipped (id wanted again)");
                            }
                        } else {
                            tracing::debug!(target: "app_lib::karaoke", video_id = %running_id_clone, "karaoke cancel skipped (id wanted again)");
                        }
                    }
                });
            }
        }
        let desired_ids = shared.desired.ids();
        evict_desired(&self.cache, &desired_ids, &mut shared.announced_ready);
        emit_queue(&self.app, &shared, &self.cache);
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

    pub fn set_cache_limit_mb(&self, mb: u64) -> u64 {
        let limit_mb = mb.clamp(512, 20480);
        let max_bytes = limit_mb * 1024 * 1024;
        self.cache.set_max_bytes(max_bytes);

        let (lock, _) = &*self.shared;
        let mut shared = lock.lock().unwrap();
        let desired_ids = shared.desired.ids();
        evict_desired(&self.cache, &desired_ids, &mut shared.announced_ready);
        drop(shared);

        let total_bytes = self.cache_bytes();
        tracing::info!(target: "app_lib::karaoke", limit_mb, total_bytes, "karaoke cache limit changed");
        limit_mb
    }

    pub fn clear_cache(&self) {
        let initial_size = self.cache.total_size();
        let (lock, condvar) = &*self.shared;
        let mut shared = lock.lock().unwrap();
        if let Some((_, cancel, _)) = shared.running.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        let remaining_bytes = self.cache.clear().unwrap_or_else(|_| self.cache.total_size());
        let freed_bytes = initial_size.saturating_sub(remaining_bytes);

        shared.announced_ready.clear();
        shared.failed_ids.clear();
        shared.last_progress = 0.0;

        let enabled = shared.enabled;
        let last_reqs = shared.last_reqs.clone();

        if enabled {
            shared.desired.replace(last_reqs);
            for id in shared.desired.ids() {
                if self.cache.contains(&id) {
                    if shared.announced_ready.insert(id.clone()) {
                        let _ = self.app.emit("karaoke-status", &serde_json::json!({
                            "videoId": &id,
                            "state": "ready"
                        }));
                        let _ = self.ready_tx.send(id.clone());
                    }
                }
            }
            emit_queue(&self.app, &shared, &self.cache);
        } else {
            shared.desired.clear();
            shared.last_reqs.clear();
            let _ = self.app.emit("karaoke-queue", &serde_json::json!({ "slots": [] }));
        }

        drop(shared);
        condvar.notify_one();

        tracing::info!(target: "app_lib::karaoke", freed_bytes, remaining_bytes, "karaoke cache cleared");
    }
}
