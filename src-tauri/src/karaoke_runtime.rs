//! Makes ONNX Runtime available for karaoke. Called once, when karaoke is first turned on (from
//! `KaraokeEngine::ensure_worker`), never at launch.
//!
//! Linux/macOS link ONNX Runtime in, so it is always ready. On Windows nothing from ONNX Runtime is
//! linked into the exe; the bundled `onnxruntime.dll` has to be loaded by full path through
//! `karaoke::runtime::init_from` before any other `ort` call. Until that loader exists, Windows
//! reports karaoke as unavailable here instead of letting `ort` look the DLL up by name.

use karaoke::runtime::RuntimeReady;

#[cfg(not(windows))]
pub fn ensure() -> Result<RuntimeReady, String> {
    Ok(karaoke::runtime::linked())
}

#[cfg(windows)]
pub fn ensure() -> Result<RuntimeReady, String> {
    Err("the karaoke runtime is not included in this build".to_string())
}
