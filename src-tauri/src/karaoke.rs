//! Background karaoke engine for preparing instrumentals (current + next lookahead).
//!
//! Locking rules (see `karaoke::board`): playback only ever reads the lock-free enabled flag or
//! takes the board's mutex for a few field writes. The worker holds that mutex only to take a job
//! or publish a result; model load, inference, cache I/O and event emits all happen with it
//! released. A panic in a job fails that job; if the worker thread dies anyway, karaoke turns
//! itself off and reports unavailable, and playback carries on without it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::karaoke_model::ModelManager;
use karaoke::board::{isolate, JobBoard};
use karaoke::cache::{InstrumentalCache, PinSlot};
use karaoke::jobs::DesiredSet;
pub use karaoke::jobs::PrepareRequest;
use karaoke::mdx::MdxModel;
use karaoke::models::ModelId;
use karaoke::runtime::RuntimeReady;
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
    /// What the loaded model runs on, for Settings: 0 not loaded yet, 1 CPU, 2 GPU (DirectML).
    processor: Arc<AtomicU8>,
    model: Arc<Mutex<ModelId>>,
    loaded_model: Arc<Mutex<Option<ModelId>>>,
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
        let _ = self.ctx.app.emit("karaoke-status", &serde_json::json!({ "state": "error" }));
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
    /// Why the last attempt to start karaoke failed (an en.json `karaoke.unavailable.*` key),
    /// for [`Self::unavailable_reason`]. Cleared once karaoke starts.
    unavailable: Mutex<Option<&'static str>>,
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
        let model_id = app
            .try_state::<Arc<crate::state::AppState>>()
            .map(|st| crate::state::saved_karaoke_model(&st.db))
            .unwrap_or(ModelId::DEFAULT);
        // No I/O here: the directory is created on first enable (`spawn_worker`).
        let cache = Arc::new(InstrumentalCache::new(cache_dir, max_bytes));
        cache.set_model(model_id.key());
        let (ready_tx, _) = tokio::sync::broadcast::channel(64);
        let ctx = Ctx {
            app,
            models,
            cache,
            board: Arc::new(JobBoard::new()),
            ready_tx,
            processor: Arc::new(AtomicU8::new(0)),
            model: Arc::new(Mutex::new(model_id)),
            loaded_model: Arc::new(Mutex::new(None)),
        };
        Arc::new(Self {
            ctx,
            worker_started: Mutex::new(false),
            worker_alive: Arc::new(AtomicBool::new(false)),
            unavailable: Mutex::new(None),
        })
    }

    pub fn wake(&self) {
        self.ctx.board.notify();
    }

    pub fn model(&self) -> ModelId {
        *self.ctx.model.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn has_loaded(&self, id: ModelId) -> bool {
        *self.ctx.loaded_model.lock().unwrap_or_else(PoisonError::into_inner) == Some(id)
    }

    pub async fn unload_model_if_loaded(&self, id: ModelId) {
        if self.has_loaded(id) {
            self.ctx.board.notify();
            let start = Instant::now();
            while self.has_loaded(id) && start.elapsed() < Duration::from_secs(3) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    pub fn set_model(&self, id: ModelId) {
        let model_key = id.key();
        let enabled = self.is_enabled();
        tracing::trace!(target: "app_lib::karaoke", model_key, enabled, "karaoke engine set model");
        {
            let mut m = self.ctx.model.lock().unwrap_or_else(PoisonError::into_inner);
            *m = id;
        }
        self.ctx.cache.set_model(id.key());
        self.ctx.board.with(|s| {
            if let Some(r) = s.running.take() {
                r.cancel.store(true, Ordering::SeqCst);
            }
        });
        self.ctx.board.notify();
        self.ctx.refresh_in_background(true);
    }

    /// Prepares the cache directory and starts the worker thread, once. Called when karaoke is
    /// turned on; an error leaves karaoke unavailable (with a reason for the UI) and everything
    /// else untouched.
    fn ensure_worker(&self) -> Result<(), String> {
        let mut started = self.worker_started.lock().unwrap_or_else(PoisonError::into_inner);
        let failure = if !*started {
            match self.spawn_worker() {
                Ok(()) => {
                    *started = true;
                    None
                }
                Err(failure) => Some(failure),
            }
        } else if !self.worker_alive.load(Ordering::SeqCst) {
            Some(("stopped", "it stopped unexpectedly earlier".to_string()))
        } else {
            None
        };
        *self.unavailable.lock().unwrap_or_else(PoisonError::into_inner) =
            failure.as_ref().map(|(key, _)| *key);
        match failure {
            None => Ok(()),
            Some((key, detail)) => {
                let msg = karaoke::sanitize_error(&detail);
                tracing::warn!(target: "app_lib::karaoke", reason = key, error = %msg, "karaoke unavailable");
                Err(key.to_string())
            }
        }
    }

    /// Why karaoke could not start last time it was tried (an en.json `karaoke.unavailable.*`
    /// key), or `None`. Nothing is probed here: karaoke does no work until it is turned on.
    pub fn unavailable_reason(&self) -> Option<&'static str> {
        if *self.worker_started.lock().unwrap_or_else(PoisonError::into_inner)
            && !self.worker_alive.load(Ordering::SeqCst)
        {
            return Some("stopped");
        }
        *self.unavailable.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `"gpu"` or `"cpu"` once the model has loaded this session.
    pub fn processor(&self) -> Option<&'static str> {
        match self.ctx.processor.load(Ordering::SeqCst) {
            1 => Some("cpu"),
            2 => Some("gpu"),
            _ => None,
        }
    }

    /// [`Self::ensure_worker`] for callers on the async runtime, which should run it through
    /// `spawn_blocking`: the first time, it hashes and loads the Windows runtime (~36 MB).
    pub fn prepare(&self) -> Result<(), String> {
        self.ensure_worker()
    }

    /// `Err((reason key, detail))`.
    fn spawn_worker(&self) -> Result<(), (&'static str, String)> {
        // ONNX Runtime first: on Windows nothing may touch `ort` before it is loaded by full path.
        let rt =
            crate::karaoke_runtime::ensure(&self.ctx.app).map_err(|e| (e.key(), e.to_string()))?;
        self.ctx.cache.prepare().map_err(|e| ("load_failed", format!("cache directory: {e}")))?;
        let ctx = self.ctx.clone();
        let alive = self.worker_alive.clone();
        alive.store(true, Ordering::SeqCst);
        std::thread::Builder::new()
            .name("karaoke-worker".to_string())
            .spawn(move || {
                let _exit = WorkerExit { ctx: ctx.clone(), alive };
                worker_loop(&ctx, rt);
            })
            .map(|_| ())
            .map_err(|e| {
                self.worker_alive.store(false, Ordering::SeqCst);
                ("load_failed", format!("worker thread: {e}"))
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
        let enabled = self.ctx.board.is_enabled();
        let num_reqs = reqs.len();
        let first_id = reqs.first().map_or(String::new(), |r| r.video_id.clone());
        if !enabled {
            tracing::trace!(target: "app_lib::karaoke", enabled, num_reqs, first_id = %first_id, returned_early = true, "karaoke want");
            return;
        }
        let ids_of_last_reqs: Vec<String> = reqs.iter().map(|r| r.video_id.clone()).collect();
        // Tracks over the length cap are never prepared (memory, see `MAX_TRACK_SECS`): say so
        // for each, and leave them out of the queue so prefetch moves on to the next one.
        let (too_long, reqs_filtered_len): (Vec<_>, Vec<_>) =
            reqs.clone().into_iter().partition(|r| karaoke::jobs::too_long(r.expected_secs));
        for r in &too_long {
            let _ = self.ctx.app.emit(
                "karaoke-status",
                &serde_json::json!({ "videoId": &r.video_id, "state": "too-long" }),
            );
        }
        let (new_ids, deferred_cancel) = self.ctx.board.with(|s| {
            for r in &reqs {
                s.retries.refresh(&r.video_id, &r.url);
            }
            s.last_reqs = reqs;
            s.retries.retain_wanted(&ids_of_last_reqs);
            let reqs_for_desired: Vec<_> = reqs_filtered_len
                .into_iter()
                .filter(|r| !s.retries.is_blocked(&r.video_id))
                .collect();
            let mut new_ids = Vec::new();
            let mut seen = HashSet::new();
            for r in &reqs_for_desired {
                if seen.insert(r.video_id.clone()) {
                    new_ids.push(r.video_id.clone());
                    if new_ids.len() >= DesiredSet::MAX_DESIRED {
                        break;
                    }
                }
            }
            if s.desired.ids() == new_ids {
                return (new_ids, None);
            }
            let removed = s.desired.replace(reqs_for_desired);
            s.desired.clear_failed();
            (new_ids, Some(s.running.as_ref().filter(|r| removed.contains(&r.video_id)).cloned()))
        });
        if deferred_cancel.is_none() {
            tracing::trace!(target: "app_lib::karaoke", enabled, num_reqs, first_id = %first_id, returned_early = true, "karaoke want");
            return;
        }
        tracing::trace!(target: "app_lib::karaoke", enabled, num_reqs, first_id = %first_id, returned_early = false, "karaoke want");
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

    /// The player hands `video_id`'s instrumental to mpv in `slot`: its path, pinned so eviction
    /// and "Clear cache" leave it alone, or `None` (slot cleared) when it is not cached.
    pub fn pin(&self, slot: PinSlot, video_id: &str) -> Option<PathBuf> {
        self.ctx.cache.pin(slot, video_id)
    }

    /// `slot` now holds the original stream, not an instrumental.
    pub fn unpin(&self, slot: PinSlot) {
        self.ctx.cache.unpin(slot);
    }

    /// Gapless/crossfade advance: the queued next track is now the current one.
    pub fn promote_next_pin(&self) {
        self.ctx.cache.promote_next();
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
        let count = self.ctx.board.with(|s| {
            let count = s.desired.clear_failed();
            s.retries.reset();
            count
        });
        self.ctx.board.notify();
        self.ctx.refresh_in_background(false);
        count
    }
}

fn unload(model: &mut Option<MdxModel>, reason: &str, loaded_model: &Mutex<Option<ModelId>>) {
    if model.take().is_some() {
        *loaded_model.lock().unwrap_or_else(PoisonError::into_inner) = None;
        tracing::info!(target: "app_lib::karaoke", reason, "karaoke model unloaded");
    }
}

/// The worker. Holds the board's lock only inside `board.with` calls: never across model load,
/// a job, file access or an emit. `rt` proves ONNX Runtime was set up before any session.
fn worker_loop(ctx: &Ctx, rt: RuntimeReady) {
    let mut model: Option<MdxModel> = None;
    let mut loaded_model_id: Option<ModelId> = None;
    let mut notified_needs_model = false;

    let log_wake = |ctx: &Ctx, model: &Option<MdxModel>| {
        let enabled = ctx.board.is_enabled();
        let selected = { *ctx.model.lock().unwrap_or_else(PoisonError::into_inner) };
        let installed = ctx.models.is_installed(selected);
        let desired_count = ctx.board.with(|s| s.desired.ids().len());
        let branch = if !enabled {
            "disabled"
        } else if !installed {
            "needs-model"
        } else {
            let ready = ctx.announce_cached();
            if ctx.board.with(|s| s.desired.next_pending(|id| ready.contains(id))).is_none() {
                "idle"
            } else if model.is_none() {
                "load"
            } else {
                "job"
            }
        };
        tracing::trace!(target: "app_lib::karaoke", enabled, model_key = selected.key(), installed, desired_count, branch, "karaoke worker wakeup");
    };

    loop {
        let selected = {
            let m = ctx.model.lock().unwrap_or_else(PoisonError::into_inner);
            *m
        };

        if !ctx.board.is_enabled() {
            unload(&mut model, "disabled", &ctx.loaded_model);
            loaded_model_id = None;
            notified_needs_model = false;
            ctx.board.wait(None);
            log_wake(ctx, &model);
            continue;
        }

        if loaded_model_id != Some(selected) {
            unload(&mut model, "model-switch", &ctx.loaded_model);
            loaded_model_id = None;
            ctx.cache.set_model(selected.key());
        }

        if !ctx.models.is_installed(selected) {
            unload(&mut model, "error", &ctx.loaded_model);
            loaded_model_id = None;
            if !notified_needs_model {
                let _ = ctx.app.emit(
                    "karaoke-status",
                    &serde_json::json!({
                        "state": "needs-model",
                        "model": selected.key()
                    }),
                );
                notified_needs_model = true;
            }
            ctx.board.with(|s| s.desired.clear());
            ctx.emit_queue();
            ctx.board.wait(None);
            log_wake(ctx, &model);
            continue;
        }
        notified_needs_model = false;

        let due = ctx.board.with(|s| {
            let due = s.retries.take_due(Instant::now());
            if !due.is_empty() {
                let reqs: Vec<_> = s
                    .last_reqs
                    .iter()
                    .filter(|r| !s.retries.is_blocked(&r.video_id))
                    .cloned()
                    .collect();
                s.desired.replace(reqs);
            }
            due
        });
        if !due.is_empty() {
            tracing::debug!(target: "app_lib::karaoke", ?due, "karaoke retry due");
        }

        // What is cached is announced; the first wanted track that is not is the next job.
        let ready = ctx.announce_cached();
        let Some(req) = ctx.board.with(|s| s.desired.next_pending(|id| ready.contains(id))) else {
            ctx.emit_queue();
            let retry_wait = ctx.board.with(|s| s.retries.next_due(Instant::now()));
            let wait = retry_wait.map_or(IDLE_UNLOAD, |d| d.min(IDLE_UNLOAD));
            if ctx.board.wait(Some(wait)) && wait == IDLE_UNLOAD {
                unload(&mut model, "idle", &ctx.loaded_model);
                loaded_model_id = None;
            }
            log_wake(ctx, &model);
            continue;
        };

        if model.is_none() {
            let start_load = std::time::Instant::now();
            let path = ctx.models.model_path(selected);
            ctx.cache.set_model(selected.key());
            match isolate(|| MdxModel::load_auto(rt, &path, selected.spec().params)) {
                Ok((m, dev)) => {
                    let load_ms = start_load.elapsed().as_millis();
                    let model_key = selected.key();
                    tracing::info!(target: "app_lib::karaoke", model_key, load_ms, device = ?dev, "karaoke model loaded");
                    let processor = match dev {
                        karaoke::mdx::Device::Cpu => 1,
                        karaoke::mdx::Device::DirectMl { .. } => 2,
                    };
                    ctx.processor.store(processor, Ordering::SeqCst);
                    model = Some(m);
                    loaded_model_id = Some(selected);
                    *ctx.loaded_model.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(selected);
                }
                Err(e) => {
                    let msg = karaoke::sanitize_error(&e.to_string());
                    tracing::warn!(target: "app_lib::karaoke", error = %msg, "karaoke model load failed");
                    let _ = ctx.app.emit(
                        "karaoke-status",
                        &serde_json::json!({ "state": "error", "code": "failed" }),
                    );
                    ctx.board.with(|s| s.desired.clear());
                    ctx.emit_queue();
                    ctx.board.wait(None);
                    log_wake(ctx, &model);
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
                selected.spec().primary_stem,
                m,
                &mut |wav| {
                    // A few seconds past the cap: enough for the pipeline to see "too long".
                    let cap = Some(karaoke::jobs::MAX_TRACK_SECS + 5.0);
                    player::decode_to_wav(&req.url, wav, &req.headers, &cancel, cap)
                },
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
                ctx.board.with(|s| {
                    s.desired.complete(&req.video_id);
                    s.retries.on_success(&req.video_id);
                });
                ctx.evict();
                ctx.emit_queue();
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("panicked") {
                    // The session may be in any state after a panic mid-inference.
                    unload(&mut model, "error", &ctx.loaded_model);
                    loaded_model_id = None;
                }
                if err_str.contains("cancelled") {
                    tracing::warn!(target: "app_lib::karaoke", video_id = %req.video_id, "karaoke prepare cancelled");
                    ctx.emit_queue();
                } else if err_str == karaoke::jobs::TOO_LONG {
                    // Not a failure to retry: the track is longer than karaoke handles.
                    let _ = ctx.app.emit(
                        "karaoke-status",
                        &serde_json::json!({ "videoId": &req.video_id, "state": "too-long" }),
                    );
                    ctx.board.with(|s| {
                        s.desired.fail(&req.video_id);
                        s.desired.complete(&req.video_id);
                    });
                    ctx.emit_queue();
                } else {
                    let msg = karaoke::sanitize_error(&err_str);
                    let code = if err_str.contains("incomplete download") {
                        "incomplete_download"
                    } else {
                        "failed"
                    };
                    tracing::warn!(target: "app_lib::karaoke", video_id = %req.video_id, error = %msg, "karaoke prepare failed");
                    let _ = ctx.app.emit(
                        "karaoke-status",
                        &serde_json::json!({
                            "videoId": &req.video_id,
                            "state": "error",
                            "code": code
                        }),
                    );
                    ctx.board.with(|s| {
                        s.desired.fail(&req.video_id);
                        s.desired.complete(&req.video_id);
                        s.retries.on_failure(&req.video_id, &req.url, Instant::now());
                    });
                    ctx.emit_queue();
                }
            }
        }
    }
}
