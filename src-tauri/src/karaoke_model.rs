//! Karaoke model download manager.
//!
//! Manages downloading, verifying (SHA-256), and checking the ONNX model file
//! required for the karaoke separation feature.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use karaoke::models::ModelId;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

/// Where the download, and GitHub's redirects for it, may go. `release-assets.githubusercontent.com`
/// is where release downloads land today (checked 2026-10-14); `objects.githubusercontent.com` is
/// the older asset host.
pub const MODEL_HOSTS: &[&str] =
    &["github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub model: &'static str,
    pub installed: bool,
    pub downloading: bool,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

pub struct ModelManager {
    dir: PathBuf,
    runtime_dir: PathBuf,
    busy: AtomicBool,
    cancel: karaoke::download::Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPlan {
    pub runtime: bool,
    pub model: bool,
    pub total: u64,
}

pub fn plan_download(
    model_present: bool,
    runtime_needed: bool,
    model_size: u64,
    runtime_size: u64,
) -> DownloadPlan {
    let model = !model_present;
    let runtime = runtime_needed;
    let total = (if model { model_size } else { 0 }) + (if runtime { runtime_size } else { 0 });
    DownloadPlan { runtime, model, total }
}

impl ModelManager {
    /// No I/O: the directory is only created when a download starts ([`Self::prepare_dir`]), so
    /// launching the app never touches it.
    pub fn new(data_dir: &Path) -> Self {
        let dir = data_dir.join("models");
        let runtime_dir = data_dir.join("runtime").join(karaoke::runtime_pkg::dir_name());
        Self {
            dir,
            runtime_dir,
            busy: AtomicBool::new(false),
            cancel: karaoke::download::Cancel::new(),
        }
    }

    #[allow(dead_code)]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    fn runtime_present(&self) -> bool {
        if let Some(pkg) = karaoke::runtime_pkg::current() {
            karaoke::runtime_pkg::present_cheap(pkg, &self.runtime_dir)
        } else {
            true
        }
    }

    /// Creates the models directory. A `.part` left by an interrupted download is kept: the next
    /// download resumes from it (and the final file is only ever the verified whole).
    fn prepare_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)
    }

    fn override_path(id: ModelId) -> Option<PathBuf> {
        if id == ModelId::DEFAULT {
            if let Ok(val) = std::env::var("LIMUSIC_KARAOKE_MODEL") {
                let p = PathBuf::from(val.trim());
                if p.exists() {
                    return Some(p);
                }
            }
        }
        None
    }

    pub fn model_path(&self, id: ModelId) -> PathBuf {
        if let Some(p) = Self::override_path(id) {
            return p;
        }
        let spec = id.spec();
        self.dir.join(spec.file_name)
    }

    fn model_present(&self, id: ModelId) -> bool {
        let spec = id.spec();
        let path = if let Some(p) = Self::override_path(id) {
            p
        } else {
            let default_path = self.dir.join(spec.file_name);
            if !default_path.exists() {
                return false;
            }
            if let Ok(meta) = std::fs::metadata(&default_path) {
                return meta.len() == spec.size;
            } else {
                return false;
            }
        };
        path.exists()
    }

    pub fn is_installed(&self, id: ModelId) -> bool {
        self.model_present(id) && self.runtime_present()
    }

    pub fn status(&self, id: ModelId) -> ModelStatus {
        let spec = id.spec();
        let installed = self.is_installed(id);
        let downloading = self.busy.load(Ordering::SeqCst);
        let path =
            if installed { Some(self.model_path(id).to_string_lossy().into_owned()) } else { None };
        let size_bytes = if installed {
            spec.size
        } else {
            let model_needed = !self.model_present(id);
            let runtime_needed =
                karaoke::runtime_pkg::current().is_some_and(|_| !self.runtime_present());
            (if model_needed { spec.size } else { 0 })
                + (if runtime_needed {
                    karaoke::runtime_pkg::current()
                        .map_or(0, karaoke::runtime_pkg::total_archive_size)
                } else {
                    0
                })
        };
        ModelStatus { model: spec.key, installed, downloading, size_bytes, path }
    }

    /// Stops a running download at once, even one stalled on the network. What arrived so far is
    /// kept, and the next download resumes from it.
    pub fn cancel_download(&self) {
        self.cancel.cancel();
    }

    /// Deletes an installed karaoke model and its leftover `.part` file.
    pub fn delete(&self, id: ModelId) -> Result<(), String> {
        if self.busy.load(Ordering::SeqCst) {
            return Err("in_use".to_string());
        }
        if Self::override_path(id).is_some() {
            return Err("override".to_string());
        }
        let final_path = self.dir.join(id.spec().file_name);
        let part_path = final_path.with_extension(
            final_path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!("{e}.part"))
                .unwrap_or_else(|| "part".to_string()),
        );

        for path in &[final_path, part_path] {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        return Err(karaoke::sanitize_error(&e.to_string()));
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn download(&self, app: &AppHandle, id: ModelId) -> Result<(), String> {
        let spec = id.spec();
        if self.is_installed(id) {
            let _ = app
                .emit("karaoke-model", serde_json::json!({ "model": spec.key, "state": "ready" }));
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
            tracing::warn!(target: "app_lib::karaoke", error = %msg, "karaoke prepare_dir failed");
            return Err(emit_error(app, spec.key, "disk"));
        }

        if let Some(parent) = self.runtime_dir.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                let msg = karaoke::sanitize_error(&e.to_string());
                tracing::warn!(target: "app_lib::karaoke", error = %msg, "karaoke create_dir_all failed");
                return Err(emit_error(app, spec.key, "disk"));
            }
        }

        let pkg = karaoke::runtime_pkg::current();
        let runtime_needed = pkg.is_some() && !self.runtime_present();
        let runtime_size = pkg.map_or(0, karaoke::runtime_pkg::total_archive_size);
        let plan = plan_download(self.model_present(id), runtime_needed, spec.size, runtime_size);

        let mut throttle = ProgressThrottle::new(Duration::from_millis(250));

        if plan.runtime {
            let pkg = pkg.unwrap();
            let mut finished_runtime_bytes = 0u64;

            for part in pkg.parts {
                let part_installed =
                    part.files.iter().filter(|f| !f.sha256.is_empty() && !f.optional).all(|f| {
                        let path = self.runtime_dir.join(f.dest);
                        path.exists() && path.metadata().map(|m| m.len() == f.size).unwrap_or(false)
                    });
                if part_installed {
                    finished_runtime_bytes += part.fetch_size;
                    continue;
                }

                let current_finished = finished_runtime_bytes;
                let emit_runtime_progress = |part_downloaded: u64| {
                    let downloaded = (current_finished + part_downloaded).min(plan.total);
                    let _ = app.emit(
                "karaoke-model",
                serde_json::json!({ "model": spec.key, "state": "downloading", "downloaded": downloaded, "total": plan.total }),
            );
                };

                match part.kind {
                    karaoke::runtime_pkg::ArchiveKind::Tgz
                    | karaoke::runtime_pkg::ArchiveKind::Zip => {
                        let ext = match part.kind {
                            karaoke::runtime_pkg::ArchiveKind::Tgz => "tgz",
                            karaoke::runtime_pkg::ArchiveKind::Zip => "zip",
                            _ => unreachable!(),
                        };
                        let runtime_archive = self.runtime_dir.parent().unwrap().join(format!(
                            "{}-{}.{}",
                            karaoke::runtime_pkg::dir_name(),
                            part.name,
                            ext
                        ));
                        let runtime_part = runtime_archive.with_extension(format!("{}.part", ext));

                        let source = karaoke::download::Source {
                            url: part.url,
                            size: part.archive_size,
                            sha256: part.archive_sha256,
                            hosts: part.hosts,
                            https_only: true,
                        };

                        emit_runtime_progress(0);
                        let result = karaoke::download::fetch(
                            crate::http::client(),
                            &source,
                            &runtime_part,
                            &runtime_archive,
                            karaoke::download::Timeouts::default(),
                            &self.cancel,
                            |downloaded| {
                                if throttle.should_emit() {
                                    emit_runtime_progress(downloaded);
                                }
                            },
                        )
                        .await;

                        match result {
                            Ok(()) => {
                                let part_clone = part;
                                let archive_path = runtime_archive.clone();
                                let dest_dir = self.runtime_dir.clone();
                                let extract_result =
                                    tauri::async_runtime::spawn_blocking(move || {
                                        karaoke::runtime_pkg::extract_part(
                                            part_clone,
                                            &archive_path,
                                            &dest_dir,
                                        )
                                    })
                                    .await;

                                let _ = std::fs::remove_file(&runtime_archive);

                                match extract_result {
                                    Ok(Ok(_)) => {
                                        tracing::info!(target: "app_lib::karaoke", part = part.name, "onnx runtime part downloaded and extracted");
                                    }
                                    Ok(Err(e)) => {
                                        let msg = karaoke::sanitize_error(&e.to_string());
                                        tracing::warn!(target: "app_lib::karaoke", error = %msg, part = part.name, "onnx runtime part extraction failed");
                                        return Err(emit_error(app, spec.key, "failed"));
                                    }
                                    Err(e) => {
                                        let msg = karaoke::sanitize_error(&e.to_string());
                                        tracing::warn!(target: "app_lib::karaoke", error = %msg, part = part.name, "onnx runtime part extraction task failed");
                                        return Err(emit_error(app, spec.key, "failed"));
                                    }
                                }
                            }
                            Err(e) => {
                                return Err(report_failure(app, spec.key, e));
                            }
                        }
                    }
                    karaoke::runtime_pkg::ArchiveKind::ZipRange => {
                        emit_runtime_progress(0);
                        let app_clone_tail = app.clone();
                        let model_key_tail = spec.key;
                        let finished_base_tail = finished_runtime_bytes;
                        let total_bytes_tail = plan.total;
                        let cancel_ref_tail = &self.cancel;
                        let part_url = part.url;
                        let part_hosts = part.hosts;

                        let tail_result = std::thread::scope(|s| {
                            s.spawn(move || {
                                let mut progress_fn = |n: u64| {
                                    let downloaded = (finished_base_tail + n).min(total_bytes_tail);
                                    let _ = app_clone_tail.emit(
                                "karaoke-model",
                                        serde_json::json!({ "model": model_key_tail, "state": "downloading", "downloaded": downloaded, "total": total_bytes_tail }),       
                                    );
                                };
                                tauri::async_runtime::block_on(karaoke::download::fetch_tail(
                                    crate::http::client(),
                                    part_url,
                                    part_hosts,
                                    true,
                                    karaoke::runtime_pkg::REMOTE_ZIP_TAIL,
                                    karaoke::download::Timeouts::default(),
                                    cancel_ref_tail,
                                    &mut progress_fn,
                                ))
                            })
                            .join()
                            .map_err(|e| karaoke::download::Error::Network(format!("thread panicked: {e:?}")))
                        });

                        let (total_len, tail) = match tail_result {
                            Ok(Ok(res)) => res,
                            Ok(Err(e)) => return Err(report_failure(app, spec.key, e)),
                            Err(e) => return Err(report_failure(app, spec.key, e)),
                        };

                        if self.cancel.is_cancelled() {
                            return Err(report_failure(
                                app,
                                spec.key,
                                karaoke::download::Error::Cancelled,
                            ));
                        }

                        let part_clone = part;
                        let tail_clone = tail;
                        let runtime_dir_clone = self.runtime_dir.clone();
                        let cancel_ref = &self.cancel;
                        let app_clone = app.clone();
                        let model_key = spec.key;
                        let finished_base = finished_runtime_bytes;
                        let total_bytes = plan.total;

                        let extract_result = std::thread::scope(|s| {
                            s.spawn(move || {
                                let mut downloaded_bytes = tail_clone.len() as u64;
                                let mut throttle = ProgressThrottle::new(Duration::from_millis(250));
                                let mut fetch_closure = |start: u64, end_inclusive: u64| -> Result<Vec<u8>, karaoke::runtime::Unavailable> {
                                    let res = tauri::async_runtime::block_on(karaoke::download::fetch_range(
                                        crate::http::client(),
                                        part_clone.url,
                                        part_clone.hosts,
                                        true,
                                        start,
                                        end_inclusive,
                                        karaoke::download::Timeouts::default(),
                                        cancel_ref,
                                        &mut |_| {},
                                    ));
                                    match res {
                                        Ok(bytes) => {
                                            downloaded_bytes += bytes.len() as u64;
                                            if throttle.should_emit() {
                                                let downloaded = (finished_base + downloaded_bytes).min(total_bytes);
                                                let _ = app_clone.emit(
                                                    "karaoke-model",
                                                    serde_json::json!({ "model": model_key, "state": "downloading", "downloaded": downloaded, "total": total_bytes }),     
                            );
                        }
                                            Ok(bytes)
                    }
                                        Err(e) => Err(karaoke::runtime::Unavailable::LoadFailed(karaoke::sanitize_error(&e.to_string()))),
                                    }
                                };
                                karaoke::runtime_pkg::extract_remote_zip(part_clone, total_len, &tail_clone, &runtime_dir_clone, &mut fetch_closure)
                            })
                            .join()
                            .map_err(|e| karaoke::runtime::Unavailable::LoadFailed(format!("thread panicked: {e:?}")))
                        });

                        if self.cancel.is_cancelled() {
                            return Err(report_failure(
                                app,
                                spec.key,
                                karaoke::download::Error::Cancelled,
                            ));
                        }

                        match extract_result {
                            Ok(Ok(_)) => {
                                tracing::info!(target: "app_lib::karaoke", part = part.name, "onnx runtime part downloaded and extracted");
                            }
                            Ok(Err(e)) => {
                                let msg = karaoke::sanitize_error(&e.to_string());
                                tracing::warn!(target: "app_lib::karaoke", error = %msg, part = part.name, "onnx runtime part extraction failed");
                                return Err(emit_error(app, spec.key, "failed"));
                            }
                            Err(e) => {
                                let msg = karaoke::sanitize_error(&e.to_string());
                                tracing::warn!(target: "app_lib::karaoke", error = %msg, part = part.name, "onnx runtime part extraction task failed");
                                return Err(emit_error(app, spec.key, "failed"));
                            }
                        }
                    }
                }

                finished_runtime_bytes += part.fetch_size;
            }
        }

        if plan.model {
            let final_path = self.model_path(id);
            let part_path = final_path.with_extension(
                final_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!("{e}.part"))
                    .unwrap_or_else(|| "part".to_string()),
            );

            let source = karaoke::download::Source {
                url: spec.url,
                size: spec.size,
                sha256: spec.sha256,
                hosts: MODEL_HOSTS,
                https_only: true,
            };

            let emit_model_progress = |model_downloaded: u64| {
                let downloaded = runtime_size + model_downloaded;
                let _ = app.emit(
                    "karaoke-model",
                    serde_json::json!({ "model": spec.key, "state": "downloading", "downloaded": downloaded, "total": plan.total }),
                );
            };
            emit_model_progress(0);
            let result = karaoke::download::fetch(
                crate::http::client(),
                &source,
                &part_path,
                &final_path,
                karaoke::download::Timeouts::default(),
                &self.cancel,
                |downloaded| {
                    if throttle.should_emit() {
                        emit_model_progress(downloaded);
                    }
                },
            )
            .await;

            match result {
                Ok(()) => {
                    emit_model_progress(spec.size);
                    tracing::info!(target: "app_lib::karaoke", model = spec.key, "karaoke model downloaded");
                }
                Err(e) => {
                    return Err(report_failure(app, spec.key, e));
                }
            }
        }

        let _ =
            app.emit("karaoke-model", serde_json::json!({ "model": spec.key, "state": "ready" }));
        Ok(())
    }
}

fn error_code(e: &karaoke::download::Error) -> &'static str {
    match e {
        karaoke::download::Error::Timeout
        | karaoke::download::Error::Network(_)
        | karaoke::download::Error::Status(_)
        | karaoke::download::Error::Incomplete { .. } => "network",
        karaoke::download::Error::HashMismatch | karaoke::download::Error::TooLarge { .. } => {
            "verify"
        }
        karaoke::download::Error::Io(_) => "disk",
        karaoke::download::Error::HostNotAllowed(_) => "failed",
        karaoke::download::Error::Cancelled => "cancelled",
    }
}

fn emit_error(app: &AppHandle, key: &str, code: &str) -> String {
    let _ = app
        .emit("karaoke-model", serde_json::json!({ "model": key, "state": "error", "code": code }));
    code.to_string()
}

fn report_failure(app: &AppHandle, key: &str, e: karaoke::download::Error) -> String {
    match e {
        karaoke::download::Error::Cancelled => {
            let _ = app
                .emit("karaoke-model", serde_json::json!({ "model": key, "state": "cancelled" }));
            "cancelled".to_string()
        }
        ref other => {
            let msg = karaoke::sanitize_error(&e.to_string());
            tracing::warn!(target: "app_lib::karaoke", model = key, error = %msg, "karaoke download failed");
            let code = error_code(other);
            let _ = app.emit(
                "karaoke-model",
                serde_json::json!({ "model": key, "state": "error", "code": code }),
            );
            code.to_string()
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

    fn fake_runtime(mgr: &ModelManager) {
        if let Some(pkg) = karaoke::runtime_pkg::current() {
            std::fs::create_dir_all(mgr.runtime_dir()).unwrap();
            for f in karaoke::runtime_pkg::pinned_files(pkg) {
                let path = mgr.runtime_dir().join(f.dest);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                let file = std::fs::File::create(&path).unwrap();
                file.set_len(f.size).unwrap();
            }
        }
    }

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
        fake_runtime(&manager);
        let id = ModelId::DEFAULT;
        assert!(!manager.is_installed(id));
        manager.prepare_dir().unwrap();

        let path = manager.model_path(id);
        std::fs::write(&path, b"short").unwrap();
        assert!(!manager.is_installed(id));

        let f = std::fs::File::create(&path).unwrap();
        f.set_len(id.spec().size).unwrap();
        drop(f);
        assert!(manager.is_installed(id));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_is_installed_runtime_dependency() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = temp_dir();
        let manager = ModelManager::new(&tmp);
        let id = ModelId::DEFAULT;
        manager.prepare_dir().unwrap();

        let path = manager.model_path(id);
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(id.spec().size).unwrap();
        drop(f);

        if karaoke::runtime_pkg::current().is_some() {
            assert!(!manager.is_installed(id));
        } else {
            assert!(manager.is_installed(id));
        }

        fake_runtime(&manager);
        assert!(manager.is_installed(id));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    /// An interrupted download's `.part` is what the next download resumes from.
    fn test_model_manager_keeps_part_for_resume() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = temp_dir();
        let models_dir = tmp.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let id = ModelId::DEFAULT;
        let part_file = models_dir.join(format!("{}.part", id.spec().file_name));
        std::fs::write(&part_file, b"first bytes").unwrap();

        let manager = ModelManager::new(&tmp);
        manager.prepare_dir().unwrap();
        assert!(part_file.exists(), "prepare_dir() must keep the partial download");
        assert!(!manager.is_installed(id));
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
        assert_eq!(manager.model_path(ModelId::DEFAULT), override_file);
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
        fake_runtime(&manager);
        let id = ModelId::DEFAULT;
        assert_eq!(manager.model_path(id), manager.dir.join(id.spec().file_name));
        manager.prepare_dir().unwrap();

        let path = manager.model_path(id);
        std::fs::write(&path, b"short").unwrap();
        assert!(!manager.is_installed(id));

        let f = std::fs::File::create(&path).unwrap();
        f.set_len(id.spec().size).unwrap();
        drop(f);
        assert!(manager.is_installed(id));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_model_delete_files() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = temp_dir();
        let manager = ModelManager::new(&tmp);
        let id = ModelId::DEFAULT;
        manager.prepare_dir().unwrap();

        let model_file = manager.model_path(id);
        let part_file = model_file.with_extension(
            model_file
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!("{e}.part"))
                .unwrap_or_else(|| "part".to_string()),
        );

        std::fs::write(&model_file, b"model data").unwrap();
        std::fs::write(&part_file, b"part data").unwrap();
        assert!(model_file.exists());
        assert!(part_file.exists());

        manager.delete(id).unwrap();
        assert!(!model_file.exists(), "model file must be removed");
        assert!(!part_file.exists(), "part file must be removed");

        // Missing file is Ok
        manager.delete(id).unwrap();

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_model_delete_override_refused() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = temp_dir();
        let override_file = tmp.join("custom_model.onnx");
        std::fs::write(&override_file, b"override").unwrap();

        std::env::set_var("LIMUSIC_KARAOKE_MODEL", override_file.to_str().unwrap());
        let _env_guard = EnvGuard;
        let manager = ModelManager::new(&tmp);

        let res = manager.delete(ModelId::DEFAULT);
        assert_eq!(res, Err("override".to_string()));
        assert!(override_file.exists(), "override file must not be deleted");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_plan_download() {
        let p1 = plan_download(false, true, 100, 50);
        assert_eq!(p1, DownloadPlan { runtime: true, model: true, total: 150 });

        let p2 = plan_download(true, true, 100, 50);
        assert_eq!(p2, DownloadPlan { runtime: true, model: false, total: 50 });

        let p3 = plan_download(false, false, 100, 50);
        assert_eq!(p3, DownloadPlan { runtime: false, model: true, total: 100 });

        let p4 = plan_download(true, false, 100, 50);
        assert_eq!(p4, DownloadPlan { runtime: false, model: false, total: 0 });
    }

    #[test]
    fn test_runtime_dir_and_present() {
        let tmp = temp_dir();
        let manager = ModelManager::new(&tmp);
        let rdir = manager.runtime_dir();
        assert!(
            rdir.ends_with(karaoke::runtime_pkg::dir_name()),
            "runtime_dir must end with dir_name()"
        );
        assert!(rdir.starts_with(tmp.join("runtime")), "runtime_dir must lie under tmp/runtime");

        if karaoke::runtime_pkg::current().is_none() {
            assert!(
                manager.runtime_present(),
                "when current() is None, runtime_present must be true"
            );
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
