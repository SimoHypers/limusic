//! Karaoke model download manager.
//!
//! Manages downloading, verifying (SHA-256), and checking the ONNX model file
//! required for the karaoke separation feature.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

pub const MODEL_FILE_NAME: &str = "UVR-MDX-NET-Inst_HQ_3.onnx";
pub const MODEL_URL: &str =
    "https://github.com/TRvlvr/model_repo/releases/download/all_public_uvr_models/UVR-MDX-NET-Inst_HQ_3.onnx";
pub const MODEL_SIZE: u64 = 66_759_214;
pub const MODEL_SHA256: &str = "317554b07fe1ea5279a77f2b1520a41ea4b93432560c4ffd08792c30fddf9adc";
/// Where the download, and GitHub's redirects for it, may go. `release-assets.githubusercontent.com`
/// is where release downloads land today (checked 2026-10-14); `objects.githubusercontent.com` is
/// the older asset host.
pub const MODEL_HOSTS: &[&str] =
    &["github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub installed: bool,
    pub downloading: bool,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

pub struct ModelManager {
    dir: PathBuf,
    busy: AtomicBool,
    cancel: karaoke::download::Cancel,
}

impl ModelManager {
    /// No I/O: the directory is only created when a download starts ([`Self::prepare_dir`]), so
    /// launching the app never touches it.
    pub fn new(data_dir: &Path) -> Self {
        let dir = data_dir.join("models");
        Self { dir, busy: AtomicBool::new(false), cancel: karaoke::download::Cancel::new() }
    }

    /// Creates the models directory. A `.part` left by an interrupted download is kept: the next
    /// download resumes from it (and the final file is only ever the verified whole).
    fn prepare_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)
    }

    fn override_path() -> Option<PathBuf> {
        if let Ok(val) = std::env::var("LIMUSIC_KARAOKE_MODEL") {
            let p = PathBuf::from(val.trim());
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    pub fn model_path(&self) -> PathBuf {
        if let Some(p) = Self::override_path() {
            return p;
        }
        self.dir.join(MODEL_FILE_NAME)
    }

    pub fn is_installed(&self) -> bool {
        let path = if let Some(p) = Self::override_path() {
            p
        } else {
            let default_path = self.dir.join(MODEL_FILE_NAME);
            if !default_path.exists() {
                return false;
            }
            if let Ok(meta) = std::fs::metadata(&default_path) {
                return meta.len() == MODEL_SIZE;
            } else {
                return false;
            }
        };
        path.exists()
    }

    pub fn status(&self) -> ModelStatus {
        let installed = self.is_installed();
        let downloading = self.busy.load(Ordering::SeqCst);
        let path =
            if installed { Some(self.model_path().to_string_lossy().into_owned()) } else { None };
        ModelStatus { installed, downloading, size_bytes: MODEL_SIZE, path }
    }

    /// Stops a running download at once, even one stalled on the network. What arrived so far is
    /// kept, and the next download resumes from it.
    pub fn cancel_download(&self) {
        self.cancel.cancel();
    }

    pub async fn download(&self, app: &AppHandle) -> Result<(), String> {
        if self.is_installed() {
            return Ok(());
        }

        if self.busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return Err("already downloading".to_string());
        }
        self.cancel.reset();

        // Guard to ensure busy is cleared on exit
        struct BusyGuard<'a>(&'a AtomicBool);
        impl<'a> Drop for BusyGuard<'a> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _guard = BusyGuard(&self.busy);

        if let Err(e) = self.prepare_dir() {
            let msg = karaoke::sanitize_error(&e.to_string());
            let _ =
                app.emit("karaoke-model", serde_json::json!({ "state": "error", "message": msg }));
            return Err(msg);
        }

        let final_path = self.model_path();
        let part_path = final_path.with_extension(
            final_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!("{e}.part"))
                .unwrap_or_else(|| "part".to_string()),
        );

        let source = karaoke::download::Source {
            url: MODEL_URL,
            size: MODEL_SIZE,
            sha256: MODEL_SHA256,
            hosts: MODEL_HOSTS,
            https_only: true,
        };
        let mut throttle = ProgressThrottle::new(Duration::from_millis(250));
        let emit_progress = |downloaded: u64| {
            let _ = app.emit(
                "karaoke-model",
                serde_json::json!({ "state": "downloading", "downloaded": downloaded, "total": MODEL_SIZE }),
            );
        };
        emit_progress(0);
        let result = karaoke::download::fetch(
            crate::http::client(),
            &source,
            &part_path,
            &final_path,
            karaoke::download::Timeouts::default(),
            &self.cancel,
            |downloaded| {
                if throttle.should_emit() {
                    emit_progress(downloaded);
                }
            },
        )
        .await;

        match result {
            Ok(()) => {
                emit_progress(MODEL_SIZE);
                tracing::info!(target: "app_lib::karaoke", "karaoke model downloaded");
                let _ = app.emit("karaoke-model", serde_json::json!({ "state": "ready" }));
                Ok(())
            }
            // The `.part` stays, so the next attempt resumes instead of starting over.
            Err(karaoke::download::Error::Cancelled) => {
                let _ = app.emit("karaoke-model", serde_json::json!({ "state": "cancelled" }));
                Err("cancelled".to_string())
            }
            Err(e) => {
                let msg = karaoke::sanitize_error(&e.to_string());
                tracing::warn!(target: "app_lib::karaoke", error = %msg, "karaoke model download failed");
                let _ = app
                    .emit("karaoke-model", serde_json::json!({ "state": "error", "message": msg }));
                Err(msg)
            }
        }
    }
}

/// Pure helper for progress throttling.
pub struct ProgressThrottle {
    interval: Duration,
    last_emitted: Instant,
}

impl ProgressThrottle {
    pub fn new(interval: Duration) -> Self {
        Self { interval, last_emitted: Instant::now() - interval - Duration::from_secs(1) }
    }

    pub fn should_emit(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_emitted) >= self.interval {
            self.last_emitted = now;
            true
        } else {
            false
        }
    }
}

#[allow(dead_code)]
pub fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard;
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("LIMUSIC_KARAOKE_MODEL");
        }
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("limusic_test_{}", rand::random::<u64>()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_sha256_abc() {
        let digest = compute_sha256(b"abc");
        assert_eq!(digest, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn test_progress_throttle() {
        let mut throttle = ProgressThrottle::new(Duration::from_millis(250));
        assert!(throttle.should_emit(), "first call should emit");
        assert!(!throttle.should_emit(), "immediate second call should be suppressed");
    }

    #[test]
    fn test_is_installed_size_check() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = temp_dir();
        let manager = ModelManager::new(&tmp);
        assert!(!manager.is_installed());
        manager.prepare_dir().unwrap();

        let path = manager.model_path();
        std::fs::write(&path, b"short").unwrap();
        assert!(!manager.is_installed());

        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MODEL_SIZE).unwrap();
        drop(f);
        assert!(manager.is_installed());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    /// An interrupted download's `.part` is what the next download resumes from.
    fn test_model_manager_keeps_part_for_resume() {
        let tmp = temp_dir();
        let models_dir = tmp.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let part_file = models_dir.join(format!("{MODEL_FILE_NAME}.part"));
        std::fs::write(&part_file, b"first bytes").unwrap();

        let manager = ModelManager::new(&tmp);
        manager.prepare_dir().unwrap();
        assert!(part_file.exists(), "prepare_dir() must keep the partial download");
        assert!(!manager.is_installed());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_model_manager_new_creates_nothing() {
        let tmp = std::env::temp_dir().join(format!("limusic_test_{}", rand::random::<u64>()));
        let _manager = ModelManager::new(&tmp);
        assert!(!tmp.exists(), "constructing the manager at launch must not create directories");
    }

    #[test]
    fn test_model_path_override() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = temp_dir();
        let override_file = tmp.join("custom_model.onnx");
        std::fs::write(&override_file, b"override").unwrap();

        std::env::set_var("LIMUSIC_KARAOKE_MODEL", override_file.to_str().unwrap());
        let _env_guard = EnvGuard;
        let manager = ModelManager::new(&tmp);
        assert_eq!(manager.model_path(), override_file);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_model_override_nonexistent() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = temp_dir();
        let nonexistent = tmp.join("nonexistent.onnx");
        std::env::set_var("LIMUSIC_KARAOKE_MODEL", nonexistent.to_str().unwrap());
        let _env_guard = EnvGuard;

        let manager = ModelManager::new(&tmp);
        assert_eq!(manager.model_path(), manager.dir.join(MODEL_FILE_NAME));
        manager.prepare_dir().unwrap();

        let path = manager.model_path();
        std::fs::write(&path, b"short").unwrap();
        assert!(!manager.is_installed());

        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MODEL_SIZE).unwrap();
        drop(f);
        assert!(manager.is_installed());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
