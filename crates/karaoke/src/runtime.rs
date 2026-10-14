//! The ONNX Runtime that `ort` talks to, and the proof that it is ready.
//!
//! - Linux/macOS: linked in statically, so it is always ready ([`linked`]).
//! - Windows: built with ort's `load-dynamic`, so nothing from ONNX Runtime is in the exe. The DLL
//!   is loaded by full path through [`init_from`], and only that: if any other `ort` call came
//!   first, ort would load `onnxruntime.dll` *by name*, which can pick up the old copy Windows
//!   ships in System32 (and ort panics when that one is too old).
//!
//! [`RuntimeReady`] can only be obtained from those functions, and every entry point that creates
//! a session ([`crate::mdx::MdxModel::load`] and friends) takes one, so the compiler enforces the
//! order.

/// Proof that ONNX Runtime is available to `ort` in this process.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeReady(());

/// Linux/macOS: ONNX Runtime is linked into the binary.
#[cfg(not(windows))]
pub fn linked() -> RuntimeReady {
    RuntimeReady(())
}

/// Windows: loads ONNX Runtime from `path`, which must be absolute. Must be the first `ort` call
/// in the process; calling it again later is harmless (the first library stays loaded).
#[cfg(windows)]
pub fn init_from(path: &std::path::Path) -> anyhow::Result<RuntimeReady> {
    if !path.is_absolute() {
        anyhow::bail!("ONNX Runtime path must be absolute");
    }
    let builder = ort::init_from(path).map_err(|e| anyhow::anyhow!("{e}"))?;
    // `false` only means an environment was committed before; the library is the same one.
    let _ = builder.with_name("limusic-karaoke").commit();
    Ok(RuntimeReady(()))
}

/// For the examples and other tools: on Windows, loads the DLL named by `ORT_DYLIB_PATH` (an
/// absolute path); elsewhere, the linked runtime.
pub fn from_env() -> anyhow::Result<RuntimeReady> {
    #[cfg(windows)]
    {
        let path = std::env::var_os("ORT_DYLIB_PATH").ok_or_else(|| {
            anyhow::anyhow!(
                "set ORT_DYLIB_PATH to the absolute path of onnxruntime.dll (DirectML build 1.24.x)"
            )
        })?;
        init_from(std::path::Path::new(&path))
    }
    #[cfg(not(windows))]
    {
        Ok(linked())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn a_relative_path_is_refused_before_ort_is_touched() {
        let err = super::init_from(std::path::Path::new("onnxruntime.dll")).unwrap_err();
        assert!(err.to_string().contains("absolute"), "{err}");
    }

    #[cfg(not(windows))]
    #[test]
    fn the_linked_runtime_is_always_ready() {
        let _ = super::linked();
        assert!(super::from_env().is_ok());
    }
}
