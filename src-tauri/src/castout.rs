use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use cast::{watch, CastDevice, CastEvent, CastSession, DiscoveryEvent, LoadRequest};
use crate::orchestrator::PlaybackData;

/// Manages Chromecast discovery, connection state, and session synchronization.
pub struct CastManager {
    app: AppHandle,
    devices: Arc<Mutex<HashMap<String, CastDevice>>>,
    active: Mutex<Option<ActiveCast>>,
    watcher_started: AtomicBool,
    last_pos_info: Mutex<Option<(Instant, f64)>>,
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
            active: Mutex::new(None),
            watcher_started: AtomicBool::new(false),
            last_pos_info: Mutex::new(None),
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
        if let Ok(map) = self.devices.lock() {
            if map.is_empty() {
                if let Ok(browsed) = cast::browse(std::time::Duration::from_secs(1)) {
                    drop(map);
                    if let Ok(mut map) = self.devices.lock() {
                        for d in browsed {
                            map.insert(d.id.clone(), d);
                        }
                        return map.values().cloned().collect();
                    }
                }
            }
            map.values().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn status(&self) -> Option<CastDevice> {
        self.active.lock().ok()?.as_ref().map(|a| a.device.clone())
    }

    pub fn is_connected(&self) -> bool {
        self.active.lock().ok().and_then(|a| a.as_ref().map(|_| true)).unwrap_or(false)
    }

    pub async fn connect(&self, device_id: &str, state: &Arc<crate::state::AppState>) -> Result<CastDevice, String> {
        let device = {
            let map = self.devices.lock().map_err(|e| e.to_string())?;
            map.get(device_id).cloned().ok_or_else(|| format!("device not found: {device_id}"))?
        };

        let _ = self.disconnect(state).await;

        let session = CastSession::connect(device.ip, device.port).map_err(|e| e.to_string())?;
        
        crate::audioproxy::start_lan();
        let _ = state.player.set_muted(true);

        let mut events = session.events();
        let state_clone = state.clone();

        tauri::async_runtime::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    CastEvent::Status { state: _, position_secs, duration_secs: _, idle_reason, volume: _ } => {
                        if let Ok(mut guard) = state_clone.cast.last_pos_info.lock() {
                            *guard = Some((Instant::now(), position_secs));
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
            let mut active = self.active.lock().map_err(|e| e.to_string())?;
            *active = Some(ActiveCast {
                device: device.clone(),
                session,
            });
        }

        let _ = self.app.emit("cast-state", Some(&device));

        if let Some(item) = state.current_item().await {
            let pos = state.current_position();
            let _ = state.cast_follow_current(pos).await;
        }

        Ok(device)
    }

    pub async fn disconnect(&self, state: &Arc<crate::state::AppState>) -> Result<(), String> {
        let old_active = {
            let mut active = self.active.lock().map_err(|e| e.to_string())?;
            active.take()
        };

        if let Some(active) = old_active {
            let _ = active.session.quit();
        }

        crate::audioproxy::stop_lan();
        let _ = state.player.set_muted(false);
        let _ = self.app.emit("cast-state", None::<CastDevice>);
        if let Ok(mut guard) = self.last_pos_info.lock() {
            *guard = None;
        }
        Ok(())
    }

    pub fn set_volume(&self, level: f32) -> Result<(), String> {
        if let Some(active) = self.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.set_volume(level).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn play(&self) -> Result<(), String> {
        if let Some(active) = self.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.play().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn pause(&self) -> Result<(), String> {
        if let Some(active) = self.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.pause().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn seek(&self, secs: f64) -> Result<(), String> {
        if let Some(active) = self.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.seek(secs).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn session_load(&self, req: LoadRequest) -> Result<(), String> {
        if let Some(active) = self.active.lock().ok().and_then(|a| a.as_ref().map(|a| a.session.clone())) {
            active.load(req).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn check_drift(&self, mpv_pos: f64, is_playing: bool) {
        if !is_playing {
            return;
        }
        if let Ok(mut guard) = self.last_pos_info.lock() {
            if let Some((instant, cast_pos)) = *guard {
                let elapsed = instant.elapsed().as_secs_f64();
                let estimated_cast_pos = cast_pos + elapsed;
                if (mpv_pos - estimated_cast_pos).abs() > 2.5 {
                    tracing::debug!(mpv_pos, estimated_cast_pos, "Chromecast drift correction triggered");
                    let _ = self.seek(mpv_pos);
                    *guard = Some((Instant::now(), mpv_pos));
                }
            }
        }
    }
}
