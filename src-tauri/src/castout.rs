use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use cast::{watch, CastDevice, CastEvent, CastSession, DiscoveryEvent, LoadRequest, PlayerState};

/// Manages Chromecast discovery, connection state, and session synchronization.
pub struct CastManager {
    app: AppHandle,
    devices: Arc<Mutex<HashMap<String, CastDevice>>>,
    inner: Arc<CastInner>,
    watcher_started: AtomicBool,
}

struct CastInner {
    active: Mutex<Option<ActiveCast>>,
    last_pos_info: Mutex<Option<(Instant, f64, PlayerState)>>,
    loaded_at: Mutex<Option<Instant>>,
    last_correction: Mutex<Option<Instant>>,
    last_status: Mutex<Option<(PlayerState, Option<cast::session::IdleReason>)>>,
    has_played_or_buffered: AtomicBool,
    state: Mutex<Option<Arc<crate::state::AppState>>>,
    notice_sent_for_load: Mutex<Option<Instant>>,
    desired_playing: AtomicBool,
    last_reconcile: Mutex<Option<Instant>>,
    loaded_video_id: Mutex<Option<String>>,
    sync_pending: AtomicBool,
    last_playing_pos: Mutex<Option<f64>>,
}

struct ActiveCast {
    device: CastDevice,
    session: CastSession,
}

impl CastManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            devices: Arc::new(Mutex::new(HashMap::new())),
            inner: Arc::new(CastInner {
                active: Mutex::new(None),
                last_pos_info: Mutex::new(None),
                loaded_at: Mutex::new(None),
                last_correction: Mutex::new(None),
                last_status: Mutex::new(None),
                has_played_or_buffered: AtomicBool::new(false),
                state: Mutex::new(None),
                notice_sent_for_load: Mutex::new(None),
                desired_playing: AtomicBool::new(false),
                last_reconcile: Mutex::new(None),
                loaded_video_id: Mutex::new(None),
                sync_pending: AtomicBool::new(false),
                last_playing_pos: Mutex::new(None),
            }),
            watcher_started: AtomicBool::new(false),
        }
    }

    /// Start background mDNS discovery watcher lazily.
    pub fn ensure_discovery(&self) {
        if self.watcher_started.load(Ordering::SeqCst) {
            return;
        }
        if self.watcher_started.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            let devices = self.devices.clone();
            let app = self.app.clone();
            std::thread::spawn(move || {
                let (rx, _handle) = watch();
                while let Ok(event) = rx.recv() {
                    let mut map = match devices.lock() {
                        Ok(m) => m,
                        Err(_) => break,
                    };
                    match event {
                        DiscoveryEvent::Found(device) => {
                            map.insert(device.id.clone(), device);
                        }
                        DiscoveryEvent::Lost(id) => {
                            map.remove(&id);
                        }
                    }
                    let list: Vec<CastDevice> = map.values().cloned().collect();
                    let _ = app.emit("cast-devices", &list);
                }
            });
        }
    }

    pub fn list_devices(&self) -> Vec<CastDevice> {
        self.ensure_discovery();
        let is_empty = if let Ok(map) = self.devices.lock() {
            map.is_empty()
        } else {
            false
        };

        if is_empty {
            if let Ok(browsed) = cast::browse(std::time::Duration::from_secs(1)) {
                if let Ok(mut map) = self.devices.lock() {
                    for d in browsed {
                        map.insert(d.id.clone(), d);
                    }
                    return map.values().cloned().collect();
                }
            }
        }

        if let Ok(map) = self.devices.lock() {
            map.values().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn status(&self) -> Option<CastDevice> {
        self.inner.active.lock().ok()?.as_ref().map(|a| a.device.clone())
    }

    pub fn is_connected(&self) -> bool {
        self.inner.active.lock().ok().and_then(|a| a.as_ref().map(|_| true)).unwrap_or(false)
    }

    pub fn set_loaded_video_id(&self, video_id: String) {
        if let Ok(mut guard) = self.inner.loaded_video_id.lock() {
            *guard = Some(video_id);
        }
    }

    pub async fn connect(&self, device_id: &str, state: &Arc<crate::state::AppState>) -> Result<CastDevice, String> {
        let device = {
            let map = self.devices.lock().map_err(|e| e.to_string())?;
            map.get(device_id).cloned().ok_or_else(|| format!("device not found: {device_id}"))?
        };

        let _ = self.disconnect(state).await;

        if let Ok(mut g) = self.inner.state.lock() {
            *g = Some(state.clone());
        }

        let session = CastSession::connect(device.ip, device.port).map_err(|e| e.to_string())?;
        
        self.inner.desired_playing.store(state.is_playing.load(Ordering::Relaxed), Ordering::SeqCst);

        crate::audioproxy::start_lan();
        let _ = state.player.set_muted(true);
        let _ = session.set_volume(state.player.volume() as f32 / 100.0);

        let mut events = session.events();
        let inner = self.inner.clone();
        let state_clone = state.clone();

        tauri::async_runtime::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    CastEvent::Status { state, position_secs, duration_secs: _, idle_reason, volume: _ } => {
                        if matches!(state, PlayerState::Playing | PlayerState::Buffering) {
                            inner.has_played_or_buffered.store(true, Ordering::SeqCst);
                        }
                        if let Ok(mut guard) = inner.last_pos_info.lock() {
                            *guard = Some((Instant::now(), position_secs, state));
                        }
                        if let Ok(guard) = inner.loaded_at.lock() {
                            if let Some(t) = *guard {
                                if t.elapsed().as_secs_f64() > 20.0 {
                                    inner.sync_pending.store(false, Ordering::SeqCst);
                                }
                                if t.elapsed().as_secs_f64() < 15.0 {
                                    let desired = inner.desired_playing.load(Ordering::SeqCst);
                                    let reconcile = if desired && state == PlayerState::Paused {
                                        Some(true)
                                    } else if !desired && state == PlayerState::Playing {
                                        Some(false)
                                    } else {
                                        None
                                    };
                                    if let Some(to_play) = reconcile {
                                        let mut ok_to_reconcile = false;
                                        if let Ok(mut rec_guard) = inner.last_reconcile.lock() {
                                            if rec_guard.map_or(true, |last| last.elapsed() >= Duration::from_secs(2)) {
                                                *rec_guard = Some(Instant::now());
                                                ok_to_reconcile = true;
                                            }
                                        }
                                        if ok_to_reconcile {
                                            tracing::info!("cast: reconcile to playing/paused");
                                            if to_play {
                                                let _ = state_clone.cast.play();
                                            } else {
                                                let _ = state_clone.cast.pause();
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Align mpv back to the TV once after startup once real progress is observed,
                        // because the TV takes 1-4 s to start and stays ~3 s behind mpv.
                        if state == PlayerState::Playing {
                            if inner.sync_pending.load(Ordering::SeqCst) && state_clone.is_playing.load(Ordering::Relaxed) {
                                let mut performed_sync = false;
                                if let Ok(guard) = inner.last_playing_pos.lock() {
                                    if let Some(prev) = *guard {
                                        if position_secs > prev + 0.3 {
                                            let target = position_secs + 0.4;
                                            let mpv_pos = state_clone.current_position();
                                            if mpv_pos - target > 0.7 {
                                                let _ = state_clone.player.seek(target);
                                                tracing::info!(diff = mpv_pos - target, "cast: aligned mpv back to the TV");
                                            }
                                            performed_sync = true;
                                        }
                                    }
                                }
                                if performed_sync {
                                    inner.sync_pending.store(false, Ordering::SeqCst);
                                }
                            }
                            if let Ok(mut guard) = inner.last_playing_pos.lock() {
                                *guard = Some(position_secs);
                            }
                        }

                        let mut changed = false;
                        if let Ok(mut status_guard) = inner.last_status.lock() {
                            let current = (state, idle_reason.clone());
                            if *status_guard != Some(current.clone()) {
                                changed = true;
                                *status_guard = Some(current);
                            }
                        }
                        if changed {
                            tracing::debug!(?state, position_secs, ?idle_reason, "cast status");
                        }
                        if matches!(idle_reason, Some(cast::session::IdleReason::Error)) {
                            tracing::error!("Chromecast error idle reason encountered");
                            let _ = state_clone.cast_disconnect().await;
                            state_clone.emit_notice("Chromecast playback error");
                            break;
                        }
                    }
                    CastEvent::Disconnected(reason) => {
                        tracing::warn!(reason, "Chromecast session disconnected");
                        let _ = state_clone.cast_disconnect().await;
                        state_clone.emit_notice(&format!("Chromecast disconnected: {reason}"));
                        break;
                    }
                }
            }
        });

        {
            let mut active = self.inner.active.lock().map_err(|e| e.to_string())?;
            *active = Some(ActiveCast {
                device: device.clone(),
                session,
            });
        }

        let _ = self.app.emit("cast-state", Some(&device));

        if let Some(_item) = state.current_item().await {
            let pos = state.current_position();
            let _ = state.cast_follow_current(pos).await;
        }

        Ok(device)
    }

    pub async fn disconnect(&self, state: &Arc<crate::state::AppState>) -> Result<(), String> {
        let old_active = {
            let mut active = self.inner.active.lock().map_err(|e| e.to_string())?;
            active.take()
        };

        if let Some(active) = old_active {
            let session = active.session;
            let _ = tauri::async_runtime::spawn_blocking(move || session.shutdown(std::time::Duration::from_secs(2))).await;
            tracing::info!("cast: stopped on shutdown");
        }

        crate::audioproxy::stop_lan();
        let _ = state.player.set_muted(false);
        let _ = self.app.emit("cast-state", None::<CastDevice>);
        if let Ok(mut guard) = self.inner.last_pos_info.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.inner.loaded_at.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.inner.last_reconcile.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.inner.last_correction.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.inner.last_status.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.inner.notice_sent_for_load.lock() {
            *guard = None;
        }
        self.inner.has_played_or_buffered.store(false, Ordering::SeqCst);
        self.inner.sync_pending.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = self.inner.last_playing_pos.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = self.inner.state.lock() {
            *guard = None;
        }
        Ok(())
    }

    pub fn set_volume(&self, level: f32) -> Result<(), String> {
        if let Some(active) = self.inner.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.set_volume(level).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn play(&self) -> Result<(), String> {
        self.inner.desired_playing.store(true, Ordering::SeqCst);
        tracing::debug!("cast: mirror play");
        if let Some(active) = self.inner.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.play().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<(), String> {
        self.inner.desired_playing.store(false, Ordering::SeqCst);
        tracing::debug!("cast: mirror pause");
        if let Some(active) = self.inner.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.pause().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn seek(&self, secs: f64) -> Result<(), String> {
        if let Some(active) = self.inner.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.seek(secs).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn session_load(&self, req: LoadRequest) -> Result<(), String> {
        if let Ok(mut guard) = self.inner.last_pos_info.lock() {
            *guard = None;
        }
        let load_time = Instant::now();
        if let Ok(mut guard) = self.inner.loaded_at.lock() {
            *guard = Some(load_time);
        }
        if let Ok(mut guard) = self.inner.last_reconcile.lock() {
            *guard = None;
        }
        self.inner.has_played_or_buffered.store(false, Ordering::SeqCst);
        self.inner.sync_pending.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.inner.last_playing_pos.lock() {
            *guard = None;
        }
        let mut req = req;
        req.autoplay = self.inner.desired_playing.load(Ordering::SeqCst);

        let inner = self.inner.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let current_load = inner.loaded_at.lock().ok().and_then(|g| *g);
            if current_load == Some(load_time) && current_load.is_some() {
                if !inner.has_played_or_buffered.load(Ordering::SeqCst) {
                    let mut sent_guard = inner.notice_sent_for_load.lock().unwrap();
                    if *sent_guard != current_load {
                        *sent_guard = current_load;
                        if let Some(state) = inner.state.lock().ok().and_then(|g| g.clone()) {
                            state.emit_notice("Chromecast can't reach Limusic. Check that the firewall allows Limusic on private networks.");
                        }
                    }
                }
            }
        });

        if let Some(active) = self.inner.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.load(req).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn check_drift(&self, mpv_pos: f64, is_playing: bool) {
        if !is_playing {
            return;
        }
        if let Ok(guard) = self.inner.loaded_video_id.lock() {
            if let Some(loaded_vid) = &*guard {
                if let Ok(state_guard) = self.inner.state.lock() {
                    if let Some(state) = state_guard.as_ref() {
                        if let Some(curr_vid) = state.current_video_id() {
                            if curr_vid != *loaded_vid {
                                return;
                            }
                        }
                    }
                }
            }
        }
        if let Ok(guard) = self.inner.loaded_at.lock() {
            if let Some(t) = *guard {
                if t.elapsed().as_secs_f64() < 6.0 {
                    return;
                }
            }
        }
        if let Ok(guard) = self.inner.last_correction.lock() {
            if let Some(t) = *guard {
                if t.elapsed().as_secs_f64() < 5.0 {
                    return;
                }
            }
        }
        if let Ok(mut guard) = self.inner.last_pos_info.lock() {
            if let Some((instant, cast_pos, player_state)) = *guard {
                if !matches!(player_state, PlayerState::Playing) {
                    return;
                }
                if instant.elapsed().as_secs_f64() > 3.0 {
                    return;
                }
                let elapsed = instant.elapsed().as_secs_f64();
                let estimated_cast_pos = cast_pos + elapsed;
                if (mpv_pos - estimated_cast_pos).abs() > 3.0 {
                    tracing::debug!(mpv_pos, estimated_cast_pos, "Chromecast drift correction triggered");
                    let _ = self.seek(mpv_pos);
                    *guard = Some((Instant::now(), mpv_pos, player_state));
                    if let Ok(mut corr) = self.inner.last_correction.lock() {
                        *corr = Some(Instant::now());
                    }
                }
            }
        }
    }
}
