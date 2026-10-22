//! Makes ONNX Runtime available for karaoke. Called when karaoke is first turned on (from
//! `KaraokeEngine::ensure_worker`, off the async runtime), never at launch.
//!
//! Linux/macOS link ONNX Runtime in, so it is always ready. On Windows nothing from ONNX Runtime
//! is linked into the exe: the runtime bundled in `<install dir>\karaoke-runtime\` is checked and
//! loaded by full path (`karaoke::runtime::load`), and only then may anything touch `ort`.

use karaoke::runtime::{RuntimeReady, Unavailable};
use tauri::AppHandle;

#[cfg(not(windows))]
pub fn ensure(_app: &AppHandle) -> Result<RuntimeReady, Unavailable> {
    Ok(karaoke::runtime::linked())
}

#[cfg(windows)]
pub fn ensure(app: &AppHandle) -> Result<RuntimeReady, Unavailable> {
    use std::sync::OnceLock;
    use tauri::Manager;

    // Once loaded, ONNX Runtime stays for the life of the process; a failure is retried on the
    // next enable (the user may have installed the VC++ redistributable meanwhile).
    static LOADED: OnceLock<RuntimeReady> = OnceLock::new();
    if let Some(rt) = LOADED.get() {
        return Ok(*rt);
    }
    let dir = app
        .path()
        .resource_dir()
        .map_err(|e| Unavailable::NotInstalled(format!("resource folder ({e})")))?
        .join(karaoke::runtime::RUNTIME_DIR);
    let started = std::time::Instant::now();
    let rt = karaoke::runtime::load(&dir)?;
    tracing::info!(
        target: "app_lib::karaoke",
        directml = rt.directml(),
        load_ms = started.elapsed().as_millis() as u64,
        "karaoke runtime loaded"
    );
    Ok(*LOADED.get_or_init(|| rt))
}
