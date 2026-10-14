//! Background karaoke engine for preparing instrumentals (current + next lookahead).
//!
//! Locking rules (see `karaoke::board`): playback only ever reads the lock-free enabled flag or
//! takes the board's mutex for a few field writes. The worker holds that mutex only to take a job
//! or publish a result; model load, inference, cache I/O and event emits all happen with it
//! released. A panic in a job fails that job; if the worker thread dies anyway, karaoke turns
//! itself off and reports unavailable, and playback carries on without it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::karaoke_model::ModelManager;
use karaoke::board::{isolate, JobBoard};
use karaoke::cache::InstrumentalCache;
use karaoke::cache_codec::CacheFormat;
use karaoke::jobs::DesiredSet;
pub use karaoke::jobs::PrepareRequest;
use karaoke::mdx::{MdxModel, MdxParams};
use tauri::{AppHandle, Emitter, Manager};

const IDLE_UNLOAD: Duration = Duration::from_secs(30);

/// Everything the worker and the off-thread helpers need, cheap to clone.
#[derive(Clone)]
struct Ctx {
    app: AppHandle,
    models: Arc<ModelManager>,
    cache: Arc<InstrumentalCache>,
    board: Arc<JobBoard>,
    ready_tx: tokio::sync::broadcast::Sender<String>,
}

impl Ctx {
    /// Emits the queue dots. Snapshot under the lock, file checks after.
    fn emit_queue(&self) {
        if !self.board.is_enabled() {
            let _ = self.app.emit("karaoke-queue", &serde_json::json!({ "slots": [] }));
            return;
        }
        let (ids, running, failed) = self.board.with(|s| {
            (
                s.desired.ids(),
                s.running.as_ref().map(|r| (r.video_id.clone(), s.last_progress)),
                s.desired.failed_ids().clone(),
            )
        });
        let running = running.as_ref().map(|(id, p)| (id.as_str(), *p));
        let slots =
            karaoke::jobs::build_slots(&ids, running, &failed, |id| self.cache.contains(id));
        let _ = self.app.emit("karaoke-queue", &serde_json::json!({ "slots": slots }));
    }

    /// Announces `video_id` as ready once per enable.
    fn announce_ready(&self, video_id: &str) {
        if self.board.with(|s| s.announced_ready.insert(video_id.to_string())) {
            let _ = self.app.emit(
                "karaoke-status",
                &serde_json::json!({ "videoId": video_id, "state": "ready" }),
            );
            let _ = self.ready_tx.send(video_id.to_string());
        }
    }

    /// Announces every wanted track that is already cached. File checks run unlocked.
    fn announce_cached(&self) -> HashSet<String> {
        let ids = self.board.with(|s| s.desired.ids());
        let ready: HashSet<String> = ids.into_iter().filter(|id| self.cache.contains(id)).collect();
        for id in &ready {
            self.announce_ready(id);
        }
        ready
    }

    /// Evicts over the limit, keeping what is wanted. All file work unlocked.
    fn evict(&self) {
        let ids = self.board.with(|s| s.desired.ids());
        let keep: Vec<PathBuf> = ids.iter().filter_map(|id| self.cache.get(id)).collect();
        let _ = self.cache.evict(&keep);
        let announced = self.board.with(|s| s.announced_ready.clone());
        let gone: HashSet<String> =
            announced.into_iter().filter(|id| !self.cache.contains(id)).collect();
        if !gone.is_empty() {
            self.board.with(|s| s.announced_ready.retain(|id| !gone.contains(id)));
        }
    }

    /// Eviction (when the wanted set changed) + announce + dots on a blocking thread, for callers
    /// that must not touch the disk (playback, commands).
    fn refresh_in_background(&self, evict: bool) {
        let ctx = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if evict {
                ctx.evict();
            }
            if ctx.board.is_enabled() {
                ctx.announce_cached();
            }
            ctx.emit_queue();
        });
    }
}

/// Dropped when the worker thread ends, which it only does by panicking outside a job: karaoke
/// turns itself off and reports unavailable; playback never depended on it.
struct WorkerExit {
    ctx: Ctx,
    alive: Arc<AtomicBool>,
}

impl Drop for WorkerExit {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.ctx.board.set_enabled(false);
        tracing::error!(target: "app_lib::karaoke", "karaoke worker stopped; karaoke unavailable");
        let _ = self.ctx.app.emit("karaoke", false);
        let _ = self.ctx.app.emit(
            "karaoke-status",
            &serde_json::json!({ "state": "error", "message": "Karaoke stopped unexpectedly" }),
        );
        let _ = self.ctx.app.emit("karaoke-queue", &serde_json::json!({ "slots": [] }));
    }
}

#[allow(dead_code)]
pub struct KaraokeEngine {
    ctx: Ctx,
    /// Whether the worker thread was started. It is started, and the cache directory prepared,
    /// the first time karaoke is turned on, never at launch: a failure there must only make
    /// karaoke unavailable.
    worker_started: Mutex<bool>,
    /// False once the worker thread has died (see [`WorkerExit`]).
    worker_alive: Arc<AtomicBool>,
}

#[allow(dead_code)]
impl KaraokeEngine {
    pub fn new(app: AppHandle, data_dir: &Path, models: Arc<ModelManager>) -> Arc<Self> {
        let cache_dir = data_dir.join("karaoke-cache");
        let limit_mb = app
            .try_state::<Arc<crate::state::AppState>>()
            .map(|st| crate::state::saved_karaoke_cache_limit_mb(&st.db))
            .unwrap_or(2048)
            .clamp(512, 20480);
        let max_bytes = limit_mb * 1024 * 1024;
        // No I/O here: the directory is created on first enable (`spawn_worker`).
        let cache = Arc::new(InstrumentalCache::new(cache_dir, CacheFormat::Flac, max_bytes));
        let (ready_tx, _) = tokio::sync::broadcast::channel(64);
        let ctx = Ctx { app, models, cache, board: Arc::new(JobBoard::new()), ready_tx };
        Arc::new(Self {
            ctx,
            worker_started: Mutex::new(false),
            worker_alive: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Prepares the cache directory and starts the worker thread, once. Called when karaoke is
    /// turned on; an error leaves karaoke unavailable and everything else untouched.
    fn ensure_worker(&self) -> Result<(), String> {
        let mut started = self.worker_started.lock().unwrap_or_else(PoisonError::into_inner);
        if !*started {
            self.spawn_worker().map_err(|e| {
                let msg = karaoke::sanitize_error(&e);
                tracing::warn!(target: "app_lib::karaoke", error = %msg, "karaoke unavailable");
                format!("Karaoke is unavailable: {msg}")
            })?;
            *started = true;
        } else if !self.worker_alive.load(Ordering::SeqCst) {
            return Err("Karaoke is unavailable: it stopped unexpectedly earlier".to_string());
        }
        Ok(())
    }

    fn spawn_worker(&self) -> Result<(), String> {
        self.ctx.cache.prepare().map_err(|e| format!("cache directory: {e}"))?;
        let ctx = self.ctx.clone();
        let alive = self.worker_alive.clone();
        alive.store(true, Ordering::SeqCst);
        std::thread::Builder::new()
            .name("karaoke-worker".to_string())
            .spawn(move || {
                let _exit = WorkerExit { ctx: ctx.clone(), alive };
                worker_loop(&ctx);
            })
            .map(|_| ())
            .map_err(|e| {
                self.worker_alive.store(false, Ordering::SeqCst);
                format!("worker thread: {e}")
            })
    }

    /// Never waits for the worker: on sets a flag and wakes it; off also cancels the running job,
    /// and the worker unloads the model when its current step ends. Turning on starts the worker
    /// on first use (see [`Self::ensure_worker`]); if that fails, karaoke stays off and the error
    /// is returned for the caller to report.
    pub fn set_enabled(&self, on: bool) -> Result<(), String> {
        if on {
            self.ensure_worker()?;
        }
        self.ctx.board.set_enabled(on);
        if on {
            self.ctx.refresh_in_background(false);
        } else {
            let _ = self.ctx.app.emit("karaoke-status", &serde_json::json!({"state": "off"}));
            let _ = self.ctx.app.emit("karaoke-queue", &serde_json::json!({ "slots": [] }));
        }
        Ok(())
    }

    /// Lock-free; called on every track start.
    pub fn is_enabled(&self) -> bool {
        self.ctx.board.is_enabled()
    }

    /// Called by playback with the queue lock held: bookkeeping only, no disk, no waiting.
    pub fn want(&self, reqs: Vec<PrepareRequest>) {
        if !self.ctx.board.is_enabled() {
            return;
        }
        let mut new_ids = Vec::new();
        let mut seen = HashSet::new();
        for r in &reqs {
            if seen.insert(r.video_id.clone()) {
                new_ids.push(r.video_id.clone());
                if new_ids.len() >= DesiredSet::MAX_DESIRED {
                    break;
                }
            }
        }
        let deferred_cancel = self.ctx.board.with(|s| {
            s.last_reqs = reqs.clone();
            if s.desired.ids() == new_ids {
                return None;
            }
            let removed = s.desired.replace(reqs);
            s.desired.clear_failed();
            s.running.as_ref().filter(|r| removed.contains(&r.video_id)).cloned().map(Some)
        });
        let Some(deferred_cancel) = deferred_cancel else { return };
        let ids: Vec<&str> = new_ids.iter().map(|s| s.as_str()).collect();
        tracing::info!(target: "app_lib::karaoke", ?ids, "karaoke wants updated");

        if let Some(running) = deferred_cancel {
            // Give a quick skip-and-back a moment before throwing away work.
            let ctx = self.ctx.clone();
            tracing::debug!(target: "app_lib::karaoke", video_id = %running.video_id, "karaoke cancel deferred");
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let cancel = ctx.board.with(|s| {
                    s.running.as_ref().is_some_and(|r| r.seq == running.seq)
                        && !s.desired.contains(&running.video_id)
                });
                if cancel {
                    running.cancel.store(true, Ordering::SeqCst);
                    tracing::debug!(target: "app_lib::karaoke", video_id = %running.video_id, "karaoke cancel executed");
                } else {
                    tracing::debug!(target: "app_lib::karaoke", video_id = %running.video_id, "karaoke cancel skipped (id wanted again)");
                }
            });
        }
        self.ctx.board.notify();
        self.ctx.refresh_in_background(true);
    }

    pub fn ready_path(&self, video_id: &str) -> Option<PathBuf> {
        self.ctx.cache.get(video_id)
    }

    pub fn subscribe_ready(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.ctx.ready_tx.subscribe()
    }

    /// Reads the cache directory: call off the main thread.
    pub fn cache_bytes(&self) -> u64 {
        self.ctx.cache.total_size()
    }

    /// Evicts down to the new limit: call off the main thread.
    pub fn set_cache_limit_mb(&self, mb: u64) -> u64 {
        let limit_mb = mb.clamp(512, 20480);
        self.ctx.cache.set_max_bytes(limit_mb * 1024 * 1024);
        self.ctx.evict();
        let total_bytes = self.cache_bytes();
        tracing::info!(target: "app_lib::karaoke", limit_mb, total_bytes, "karaoke cache limit changed");
        limit_mb
    }

    /// Deletes the cache: call off the main thread. Cancels the running job first, under the
    /// lock only long enough to take it; the files go with the lock released.
    pub fn clear_cache(&self) {
        let initial_size = self.ctx.cache.total_size();
        let enabled = self.ctx.board.with(|s| {
            if let Some(r) = s.running.take() {
                r.cancel.store(true, Ordering::SeqCst);
            }
            s.announced_ready.clear();
            s.desired.clear_failed();
            s.last_progress = 0.0;
            let enabled = self.ctx.board.is_enabled();
            if enabled {
                let last = s.last_reqs.clone();
                s.desired.replace(last);
            } else {
                s.desired.clear();
                s.last_reqs.clear();
            }
            enabled
        });
        let remaining_bytes =
            self.ctx.cache.clear().unwrap_or_else(|_| self.ctx.cache.total_size());
        let freed_bytes = initial_size.saturating_sub(remaining_bytes);
        if enabled {
            self.ctx.announce_cached();
            self.ctx.emit_queue();
        } else {
            let _ = self.ctx.app.emit("karaoke-queue", &serde_json::json!({ "slots": [] }));
        }
        self.ctx.board.notify();
        tracing::info!(target: "app_lib::karaoke", freed_bytes, remaining_bytes, "karaoke cache cleared");
    }

    pub fn clear_failed(&self) -> usize {
        let count = self.ctx.board.with(|s| s.desired.clear_failed());
        self.ctx.board.notify();
        self.ctx.refresh_in_background(false);
        count
    }
}

fn unload(model: &mut Option<MdxModel>, reason: &str) {
    if model.take().is_some() {
        tracing::info!(target: "app_lib::karaoke", reason, "karaoke model unloaded");
    }
}

/// The worker. Holds the board's lock only inside `board.with` calls: never across model load,
/// a job, file access or an emit.
fn worker_loop(ctx: &Ctx) {
    let mut model: Option<MdxModel> = None;
    let mut notified_needs_model = false;

    loop {
        if !ctx.board.is_enabled() {
            unload(&mut model, "disabled");
            notified_needs_model = false;
            ctx.board.wait(None);
            continue;
        }

        if !ctx.models.is_installed() {
            unload(&mut model, "error");
            if !notified_needs_model {
                let _ =
                    ctx.app.emit("karaoke-status", &serde_json::json!({"state": "needs-model"}));
                notified_needs_model = true;
            }
            ctx.board.with(|s| s.desired.clear());
            ctx.emit_queue();
            ctx.board.wait(None);
            continue;
        }
        notified_needs_model = false;

        // What is cached is announced; the first wanted track that is not is the next job.
        let ready = ctx.announce_cached();
        let Some(req) = ctx.board.with(|s| s.desired.next_pending(|id| ready.contains(id))) else {
            ctx.emit_queue();
            if ctx.board.wait(Some(IDLE_UNLOAD)) {
                unload(&mut model, "idle");
            }
            continue;
        };

        if model.is_none() {
            let start_load = std::time::Instant::now();
            let path = ctx.models.model_path();
            match isolate(|| MdxModel::load_auto(&path, MdxParams::default())) {
                Ok((m, dev)) => {
                    let load_ms = start_load.elapsed().as_millis();
                    tracing::info!(target: "app_lib::karaoke", load_ms, device = ?dev, "karaoke model loaded");
                    model = Some(m);
                }
                Err(e) => {
                    let msg = karaoke::sanitize_error(&e.to_string());
                    tracing::warn!(target: "app_lib::karaoke", error = %msg, "karaoke model load failed");
                    let _ = ctx.app.emit(
                        "karaoke-status",
                        &serde_json::json!({ "state": "error", "message": msg }),
                    );
                    ctx.board.with(|s| s.desired.clear());
                    ctx.emit_queue();
                    ctx.board.wait(None);
                    continue;
                }
            }
            // Turned off while loading: the top of the loop unloads.
            continue;
        }

        // `None`: turned off since the check above.
        let Some(running) = ctx.board.begin(&req.video_id) else { continue };
        ctx.emit_queue();
        tracing::info!(target: "app_lib::karaoke", video_id = %req.video_id, "karaoke prepare start");

        let mut throttle = crate::karaoke_model::ProgressThrottle::new(Duration::from_millis(250));
        let cancel = running.cancel.clone();
        let Some(m) = model.as_mut() else { continue };
        let res = isolate(|| {
            karaoke::pipeline::prepare_instrumental(
                &ctx.cache,
                &req.video_id,
                m,
                &mut |wav| player::decode_to_wav(&req.url, wav, &req.headers, &cancel),
                req.expected_secs,
                &cancel,
                &mut |i, n| {
                    if throttle.should_emit() {
                        let progress = if n > 0 { i as f64 / n as f64 } else { 0.0 };
                        ctx.board.with(|s| s.last_progress = progress);
                        let _ = ctx.app.emit(
                            "karaoke-status",
                            &serde_json::json!({
                                "videoId": &req.video_id,
                                "state": "preparing",
                                "progress": progress
                            }),
                        );
                        ctx.emit_queue();
                    }
                },
            )
        });
        ctx.board.finish(running.seq);

        match res {
            Ok(_) => {
                ctx.announce_ready(&req.video_id);
                ctx.board.with(|s| s.desired.complete(&req.video_id));
                ctx.evict();
                ctx.emit_queue();
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("panicked") {
                    // The session may be in any state after a panic mid-inference.
                    unload(&mut model, "error");
                }
                if err_str.contains("cancelled") {
                    tracing::warn!(target: "app_lib::karaoke", video_id = %req.video_id, "karaoke prepare cancelled");
                    ctx.emit_queue();
                } else {
                    let msg = if err_str.contains("incomplete download") {
                        "Download was incomplete, will retry".to_string()
                    } else {
                        karaoke::sanitize_error(&err_str)
                    };
                    tracing::warn!(target: "app_lib::karaoke", video_id = %req.video_id, error = %msg, "karaoke prepare failed");
                    let _ = ctx.app.emit(
                        "karaoke-status",
                        &serde_json::json!({
                            "videoId": &req.video_id,
                            "state": "error",
                            "message": msg
                        }),
                    );
                    ctx.board.with(|s| {
                        s.desired.fail(&req.video_id);
                        s.desired.complete(&req.video_id);
                    });
                    ctx.emit_queue();
                }
            }
        }
    }
}
