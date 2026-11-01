//! Makes ONNX Runtime available for karaoke. Called when karaoke is first turned on (from
//! `KaraokeEngine::ensure_worker`, off the async runtime), never at launch.
//!
//! On every platform the runtime is downloaded on first use together with the model (see
//! karaoke_model.rs), verified (SHA-256) and loaded by full path; nothing is bundled.

use karaoke::runtime::{RuntimeReady, Unavailable};
use tauri::AppHandle;

/// Verifies the installed runtime package in the given directory.
/// If the verification reveals corruption (`Unavailable::Corrupt(_)`), the entire directory
/// is deleted so that `ModelManager::is_installed` becomes false and the app's existing
/// "needs-model" flow offers the download again. Any other error is returned unchanged.
fn verify_or_discard(
    pkg: &karaoke::runtime_pkg::RuntimePackage,
    dir: &std::path::Path,
) -> Result<std::path::PathBuf, Unavailable> {
    match karaoke::runtime_pkg::verify_installed(pkg, dir) {
        Ok(path) => Ok(path),
        Err(err @ Unavailable::Corrupt(_)) => {
            let _ = std::fs::remove_dir_all(dir);
            Err(err)
        }
        Err(err) => Err(err),
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn load_unix_runtime(
    pkg: &karaoke::runtime_pkg::RuntimePackage,
    dir: &std::path::Path,
) -> Result<RuntimeReady, Unavailable> {
    let lib_path = verify_or_discard(pkg, dir)?;
    karaoke::runtime::init_from(&lib_path, false)
        .map_err(|e| Unavailable::LoadFailed(e.to_string()))
}

#[cfg(windows)]
fn load_windows_runtime(
    dir: &std::path::Path,
    vc_dir: Option<&std::path::Path>,
) -> Result<RuntimeReady, Unavailable> {
    match karaoke::runtime::load(dir, vc_dir) {
        Ok(rt) => Ok(rt),
        Err(err @ Unavailable::Corrupt(_)) => {
            let _ = std::fs::remove_dir_all(dir);
            Err(err)
        }
        Err(err) => Err(err),
    }
}

#[cfg(not(windows))]
fn load_windows_runtime(
    _dir: &std::path::Path,
    _vc_dir: Option<&std::path::Path>,
) -> Result<RuntimeReady, Unavailable> {
    Err(Unavailable::LoadFailed("windows runtime on a non-windows target".into()))
}

pub fn ensure(app: &AppHandle) -> Result<RuntimeReady, Unavailable> {
    use std::sync::{Arc, OnceLock};
    use tauri::Manager;

    static LOADED: OnceLock<RuntimeReady> = OnceLock::new();
    if let Some(rt) = LOADED.get() {
        return Ok(*rt);
    }

    let pkg = karaoke::runtime_pkg::current()
        .ok_or_else(|| Unavailable::NotInstalled("ONNX Runtime for this platform".into()))?;

    let dir = app.state::<Arc<crate::karaoke_model::ModelManager>>().runtime_dir().to_path_buf();

    let started = std::time::Instant::now();
    let vc_dir = {
        use tauri::Manager;
        app.path().resource_dir().ok().map(|d| d.join("vcruntime"))
    };
    let rt = if cfg!(windows) {
        load_windows_runtime(&dir, vc_dir.as_deref())?
    } else {
        load_unix_runtime(pkg, &dir)?
    };

    tracing::info!(
        target: "app_lib::karaoke",
        directml = rt.directml(),
        load_ms = started.elapsed().as_millis() as u64,
        "karaoke runtime loaded"
    );

    Ok(*LOADED.get_or_init(|| rt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn make_fake_pkg(lib_file: &'static str, bytes: &[u8]) -> karaoke::runtime_pkg::RuntimePackage {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let sha = format!("{:x}", hasher.finalize());
        let part_file = karaoke::runtime_pkg::PartFile {
            entry: "x",
            dest: lib_file,
            size: bytes.len() as u64,
            sha256: Box::leak(sha.into_boxed_str()),
            optional: false,
        };
        let files = Box::leak(Box::new([part_file]));
        let part = karaoke::runtime_pkg::Part {
            name: "fake",
            url: "https://example.com/fake.tgz",
            archive_size: 100,
            archive_sha256: "",
            fetch_size: 100,
            kind: karaoke::runtime_pkg::ArchiveKind::Tgz,
            hosts: &[],
            files,
        };
        let parts = Box::leak(Box::new([part]));
        karaoke::runtime_pkg::RuntimePackage { platform: "fake", parts }
    }

    #[test]
    fn test_verify_or_discard_tampered() {
        let tmp =
            std::env::temp_dir().join(format!("limusic-karaoke-test-{}", rand::random::<u64>()));
        let _ = std::fs::create_dir_all(&tmp);

        let fake_bytes = b"fake runtime bytes";
        let pkg = make_fake_pkg("libfake.so", fake_bytes);

        // Write different bytes of the same length
        let lib_path = tmp.join(karaoke::runtime_pkg::main_lib(&pkg).dest);
        std::fs::write(&lib_path, b"fake runtime bytez").unwrap();
        assert!(tmp.exists());

        let res = verify_or_discard(&pkg, &tmp);
        assert!(matches!(res, Err(Unavailable::Corrupt(_))));
        assert!(!tmp.exists());
    }

    #[test]
    fn test_verify_or_discard_missing() {
        let tmp =
            std::env::temp_dir().join(format!("limusic-karaoke-test-{}", rand::random::<u64>()));
        let _ = std::fs::create_dir_all(&tmp);

        let fake_bytes = b"fake runtime bytes";
        let pkg = make_fake_pkg("libfake.so", fake_bytes);

        // Directory exists but file is missing
        assert!(tmp.exists());

        let res = verify_or_discard(&pkg, &tmp);
        assert!(matches!(res, Err(Unavailable::NotInstalled(_))));
        assert!(tmp.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn a_corrupt_windows_runtime_is_discarded() {
        let tmp = std::env::temp_dir()
            .join(format!("limusic-karaoke-test-win-{}", rand::random::<u64>()));
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("onnxruntime.dll"), b"garbage bytes").unwrap();
        assert!(tmp.exists());

        let res = load_windows_runtime(&tmp, None);
        assert!(matches!(res, Err(Unavailable::Corrupt(_))));
        assert!(!tmp.exists());
    }
}
