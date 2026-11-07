//! The ONNX Runtime that `ort` talks to, and the proof that it is ready.
//!
//! No platform links ONNX Runtime anymore; built with `load-dynamic`.
//! - Windows: loads the downloaded runtime folder via `load` (bundled VC++ runtime or system fallback).
//! - Linux/macOS: will download the runtime on first use (see `karaoke_runtime.rs`).
//!
//! [`RuntimeReady`] can only be obtained from those functions, and every entry point that creates
//! a session ([`crate::mdx::MdxModel::load`] and friends) takes one, so the compiler enforces the
//! order.

use std::fmt;

/// Proof that ONNX Runtime is available to `ort` in this process.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeReady {
    directml: bool,
}

impl RuntimeReady {
    /// Whether the DirectML execution provider may be used. In the app, only when *our*
    /// `DirectML.dll` was pre-loaded: `onnxruntime.dll` delay-loads it by name, and without that
    /// pre-load Windows would hand it whatever copy the search order finds first (System32's).
    pub fn directml(&self) -> bool {
        self.directml
    }
}

/// The VC++ runtime `onnxruntime.dll` imports. Not hash-pinned (they change with each
/// Visual Studio update); taken from the bundled app folder first, system folder as fallback, each must be at least [`VC_RUNTIME_MIN`].
pub const VC_RUNTIME_DLLS: [&str; 4] =
    ["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "msvcp140_1.dll"];
/// `onnxruntime.dll` 1.24.4 was linked with MSVC 14.44, and the VC++ runtime it runs against must
/// be the same or later (Microsoft Learn, "Latest supported Visual C++ Redistributable downloads").
pub const VC_RUNTIME_MIN: (u16, u16) = (14, 44);
/// Where a user gets a current VC++ runtime (Microsoft's permalink for the latest x64 package).
pub const VC_REDIST_URL: &str = "https://aka.ms/vc14/vc_redist.x64.exe";

/// Why karaoke cannot run on this machine. [`Unavailable::key`] names the reason for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// The runtime has not been downloaded yet, or its folder was damaged. Carries the file name.
    NotInstalled(String),
    /// A runtime file does not match its pinned hash or is not a loadable DLL. Carries the name.
    Corrupt(String),
    /// The system VC++ runtime, or one already loaded in the process, is missing or too old. Carries the detail.
    VcRuntime(String),
    /// Anything else that stopped ONNX Runtime from loading.
    LoadFailed(String),
}

impl Unavailable {
    /// Stable reason key (en.json `karaoke.unavailable.<key>`).
    pub fn key(&self) -> &'static str {
        match self {
            Self::NotInstalled(_) => "not_installed",
            Self::Corrupt(_) => "corrupt",
            Self::VcRuntime(_) => "vc_runtime",
            Self::LoadFailed(_) => "load_failed",
        }
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled(file) => write!(f, "the karaoke runtime is not installed ({file} is missing)"),
            Self::Corrupt(file) => write!(f, "{file} is damaged and will be downloaded again"),
            Self::VcRuntime(detail) => write!(
                f,
                "needs the Microsoft Visual C++ Redistributable {}.{} or newer ({VC_REDIST_URL}): {detail}",
                VC_RUNTIME_MIN.0, VC_RUNTIME_MIN.1
            ),
            Self::LoadFailed(detail) => write!(f, "ONNX Runtime could not be loaded: {detail}"),
        }
    }
}

impl std::error::Error for Unavailable {}

/// A `(major, minor, build, revision)` file version is at least `min` (major, minor).
pub fn version_at_least(v: (u16, u16, u16, u16), min: (u16, u16)) -> bool {
    (v.0, v.1) >= min
}

/// What a failed `LoadLibraryExW("…\onnxruntime.dll")` means, from its Win32 error code.
pub fn load_error(code: u32) -> Unavailable {
    match code {
        // ERROR_MOD_NOT_FOUND / ERROR_PROC_NOT_FOUND: a DLL it imports is missing or too old. The
        // VC++ runtime is the only non-system dependency.
        126 | 127 => Unavailable::VcRuntime(format!(
            "a DLL onnxruntime.dll needs is missing or too old (Windows error {code})"
        )),
        // ERROR_BAD_EXE_FORMAT: not a valid x64 DLL.
        193 => Unavailable::Corrupt("onnxruntime.dll".into()),
        _ => Unavailable::LoadFailed(format!("Windows error {code}")),
    }
}

/// Hands ONNX Runtime at `path` (absolute) to `ort`. Must be the first `ort` call in the
/// process; calling it again later is harmless (the first library stays loaded). `directml` says
/// whether the DirectML provider may be used (see [`RuntimeReady::directml`]). The app goes
/// through [`load`] on Windows, which does the checks and pre-loads first.
pub fn init_from(path: &std::path::Path, directml: bool) -> anyhow::Result<RuntimeReady> {
    if !path.is_absolute() {
        anyhow::bail!("ONNX Runtime path must be absolute");
    }
    let builder = ort::init_from(path).map_err(|e| anyhow::anyhow!("{e}"))?;
    // `false` only means an environment was committed before; the library is the same one.
    let _ = builder.with_name("limusic-karaoke").commit();
    Ok(RuntimeReady { directml })
}

/// For the examples and other tools: loads the shared library named by `ORT_DYLIB_PATH` (an
/// absolute path to onnxruntime.dll / libonnxruntime.so / libonnxruntime.dylib).
pub fn from_env() -> anyhow::Result<RuntimeReady> {
    let path = std::env::var_os("ORT_DYLIB_PATH").ok_or_else(|| {
        anyhow::anyhow!(
            "set ORT_DYLIB_PATH to the absolute path of the ONNX Runtime shared library"
        )
    })?;
    init_from(std::path::Path::new(&path), cfg!(windows))
}

#[cfg(windows)]
pub use win::{check_directml_module, file_version, load, pinned_sha};

#[cfg(windows)]
mod win {
    use super::{
        init_from, load_error, version_at_least, RuntimeReady, Unavailable, VC_RUNTIME_DLLS,
        VC_RUNTIME_MIN,
    };
    use core::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use windows::core::{s, w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleW, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        LOAD_LIBRARY_SEARCH_SYSTEM32,
    };

    /// The `DirectML.dll` [`load`] pre-loaded, so [`check_directml_module`] can tell whether the
    /// one in use is ours.
    static OUR_DIRECTML: OnceLock<PathBuf> = OnceLock::new();

    pub fn pinned_sha(dest: &str) -> &'static str {
        crate::runtime_pkg::pinned_files(&crate::runtime_pkg::WINDOWS_X86_64)
            .find(|f| f.dest == dest)
            .map(|f| f.sha256)
            .unwrap_or_else(|| panic!("pinned sha for {dest} not found"))
    }

    /// Loads the downloaded runtime from `dir` and makes it `ort`'s. Every failure is a reason, never
    /// a panic, and leaves the app as it was (at worst with an unused DLL mapped).
    ///
    /// 1. `onnxruntime.dll` present and matches its pinned SHA-256; `DirectML.dll` optional, and if present matches its pinned SHA-256.
    /// 2. The VC++ DLLs come from `vc_dir` when present there, otherwise from System32; each must be at least [`VC_RUNTIME_MIN`].
    /// 3. `DirectML.dll`, then `onnxruntime.dll`, loaded by full path with the search limited to
    ///    the DLL's own folder and System32: dependencies never come from the current directory or
    ///    `PATH`. DirectML first, because `onnxruntime.dll` delay-loads it *by name* and must find
    ///    ours already loaded. Handles are never freed.
    /// 4. The VC++ modules now actually in use are checked again (path and version).
    /// 5. [`init_from`].
    pub fn load(dir: &Path, vc_dir: Option<&Path>) -> Result<RuntimeReady, Unavailable> {
        let dir = plain(dir);
        let ort = dir.join("onnxruntime.dll");
        let dml = dir.join("DirectML.dll");

        if !ort.is_file() {
            return Err(Unavailable::NotInstalled("onnxruntime.dll".into()));
        }
        if sha256(&ort).as_deref() != Some(pinned_sha("onnxruntime.dll")) {
            return Err(Unavailable::Corrupt("onnxruntime.dll".into()));
        }
        // DirectML is optional: without it karaoke runs on the CPU.
        let dml_present = dml.is_file();
        if dml_present && sha256(&dml).as_deref() != Some(pinned_sha("DirectML.dll")) {
            return Err(Unavailable::Corrupt("DirectML.dll".into()));
        }

        let system_root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let system32 = system_root.join("System32");

        let flags = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32;

        for name in VC_RUNTIME_DLLS {
            let bundled = vc_dir.map(|d| plain(d).join(name)).filter(|p| p.is_file());
            if let Some(path) = bundled {
                check_vc(name, &path)?; // bundled copy must also be >= VC_RUNTIME_MIN
                                        // pre-load by full path, same flags as the other DLLs (DLL_LOAD_DIR | SYSTEM32), so a later
                                        // import by name from onnxruntime.dll reuses OUR copy; handle is never freed
                if let Err(e) =
                    unsafe { LoadLibraryExW(&HSTRING::from(path.as_path()), None, flags) }
                {
                    return Err(Unavailable::VcRuntime(format!(
                        "{name} from the app folder did not load (Windows error {})",
                        (e.code().0 as u32) & 0xFFFF
                    )));
                }
                tracing::debug!(target: "karaoke::runtime", module = name, "using the bundled VC++ module");
            } else {
                let path = system32.join(name);
                if !path.is_file() {
                    return Err(Unavailable::VcRuntime(format!("{name} is not installed")));
                }
                check_vc(name, &path)?;
            }
        }
        check_loaded_vc()?;

        let directml = dml_present
            && unsafe { LoadLibraryExW(&HSTRING::from(dml.as_path()), None, flags) }.is_ok();
        if directml {
            let _ = OUR_DIRECTML.set(dml.clone());
        } else if dml_present {
            tracing::warn!(target: "karaoke::runtime", "DirectML.dll did not load; karaoke will use the CPU");
        }
        if let Err(e) = unsafe { LoadLibraryExW(&HSTRING::from(ort.as_path()), None, flags) } {
            return Err(load_error((e.code().0 as u32) & 0xFFFF));
        }

        check_loaded_vc()?;
        init_from(&ort, directml).map_err(|e| Unavailable::LoadFailed(e.to_string()))
    }

    /// After a DirectML session is created: is the `DirectML.dll` in use the one [`load`]
    /// pre-loaded? Nothing to check when nothing was pre-loaded (tools using [`init_from`]).
    pub fn check_directml_module() -> Result<(), String> {
        let Some(ours) = OUR_DIRECTML.get() else { return Ok(()) };
        match loaded_module("DirectML.dll") {
            Some(p) if same_path(&p, ours) => Ok(()),
            Some(p) => {
                Err(format!("DirectML.dll in use is {}, not the downloaded one", p.display()))
            }
            None => Err("DirectML.dll is not loaded".into()),
        }
    }

    /// `(major, minor, build, revision)` from the file's version resource. `version.dll` is looked
    /// up at run time, like dxgi/d3d12 in `device.rs`, so it never becomes an import of the exe.
    pub fn file_version(path: &Path) -> Option<(u16, u16, u16, u16)> {
        #[repr(C)]
        struct FixedFileInfo {
            signature: u32,
            struc_version: u32,
            file_ms: u32,
            file_ls: u32,
            rest: [u32; 9],
        }
        type SizeFn = unsafe extern "system" fn(PCWSTR, *mut u32) -> u32;
        type InfoFn = unsafe extern "system" fn(PCWSTR, u32, u32, *mut c_void) -> i32;
        type QueryFn =
            unsafe extern "system" fn(*const c_void, PCWSTR, *mut *mut c_void, *mut u32) -> i32;
        use crate::device::sys::proc;
        let size_fn: SizeFn = unsafe { proc(w!("version.dll"), s!("GetFileVersionInfoSizeW")) }?;
        let info_fn: InfoFn = unsafe { proc(w!("version.dll"), s!("GetFileVersionInfoW")) }?;
        let query_fn: QueryFn = unsafe { proc(w!("version.dll"), s!("VerQueryValueW")) }?;

        let wide = HSTRING::from(path);
        let mut handle = 0u32;
        let len = unsafe { size_fn(PCWSTR(wide.as_ptr()), &mut handle) };
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        if unsafe { info_fn(PCWSTR(wide.as_ptr()), 0, len, buf.as_mut_ptr().cast()) } == 0 {
            return None;
        }
        let mut info: *mut c_void = std::ptr::null_mut();
        let mut info_len = 0u32;
        let ok = unsafe { query_fn(buf.as_ptr().cast(), w!("\\"), &mut info, &mut info_len) };
        if ok == 0 || info.is_null() || (info_len as usize) < std::mem::size_of::<FixedFileInfo>() {
            return None;
        }
        // VerQueryValueW returns a pointer into `buf`, which is still alive here.
        let fi = unsafe { std::ptr::read_unaligned(info as *const FixedFileInfo) };
        if fi.signature != 0xFEEF_04BD {
            return None;
        }
        Some((
            (fi.file_ms >> 16) as u16,
            fi.file_ms as u16,
            (fi.file_ls >> 16) as u16,
            fi.file_ls as u16,
        ))
    }

    fn check_vc(name: &str, path: &Path) -> Result<(), Unavailable> {
        match file_version(path) {
            Some(v) if version_at_least(v, VC_RUNTIME_MIN) => Ok(()),
            Some(v) => Err(Unavailable::VcRuntime(format!(
                "{name} is {}.{}.{} ({})",
                v.0,
                v.1,
                v.2,
                path.display()
            ))),
            None => Err(Unavailable::VcRuntime(format!(
                "{name} has no version information ({})",
                path.display()
            ))),
        }
    }

    /// Every VC++ module already loaded in this process, whatever folder it came from.
    fn check_loaded_vc() -> Result<(), Unavailable> {
        for name in VC_RUNTIME_DLLS {
            if let Some(path) = loaded_module(name) {
                check_vc(name, &path)?;
                tracing::debug!(target: "karaoke::runtime", module = name, path = %path.display(), "VC++ module in use");
            }
        }
        Ok(())
    }

    fn loaded_module(name: &str) -> Option<PathBuf> {
        let module = unsafe { GetModuleHandleW(&HSTRING::from(name)) }.ok()?;
        module_path(module)
    }

    fn module_path(module: HMODULE) -> Option<PathBuf> {
        let mut buf = vec![0u16; 2048];
        let n = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
        (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
    }

    /// Without a `\\?\` prefix, which some APIs hand out and which would defeat path comparison.
    fn plain(path: &Path) -> PathBuf {
        let s = path.to_string_lossy();
        match s.strip_prefix(r"\\?\") {
            Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
            _ => path.to_path_buf(),
        }
    }

    fn same_path(a: &Path, b: &Path) -> bool {
        plain(a).to_string_lossy().to_lowercase() == plain(b).to_string_lossy().to_lowercase()
    }

    fn sha256(path: &Path) -> Option<String> {
        use sha2::{Digest, Sha256};
        let mut file = std::fs::File::open(path).ok()?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).ok()?;
        Some(format!("{:x}", hasher.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_floor_compares_major_then_minor() {
        assert!(version_at_least((14, 44, 0, 0), (14, 44)));
        assert!(version_at_least((14, 51, 36247, 0), (14, 44)));
        assert!(version_at_least((15, 0, 0, 0), (14, 44)));
        assert!(!version_at_least((14, 40, 33816, 0), (14, 44)));
        assert!(!version_at_least((14, 34, 31931, 0), (14, 44)));
    }

    #[test]
    fn load_errors_map_to_reasons() {
        assert_eq!(load_error(126).key(), "vc_runtime");
        assert_eq!(load_error(127).key(), "vc_runtime");
        assert_eq!(load_error(193), Unavailable::Corrupt("onnxruntime.dll".into()));
        assert_eq!(load_error(5).key(), "load_failed");
        assert!(load_error(126).to_string().contains(VC_REDIST_URL));
    }

    #[test]
    fn a_relative_path_is_refused_before_ort_is_touched() {
        let err = init_from(std::path::Path::new("onnxruntime.dll"), false).unwrap_err();
        assert!(err.to_string().contains("absolute"), "{err}");
    }

    #[cfg(windows)]
    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("limusic_runtime_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(windows)]
    #[test]
    fn a_missing_runtime_is_not_installed() {
        let dir = scratch_dir("missing");
        assert_eq!(
            load(&dir, None).unwrap_err(),
            Unavailable::NotInstalled("onnxruntime.dll".into())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_tampered_runtime_is_refused_before_anything_loads() {
        let dir = scratch_dir("tampered");
        std::fs::write(dir.join("onnxruntime.dll"), b"not the pinned file").unwrap();
        assert_eq!(load(&dir, None).unwrap_err(), Unavailable::Corrupt("onnxruntime.dll".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn pinned_hashes_are_found() {
        let s1 = pinned_sha("onnxruntime.dll");
        let s2 = pinned_sha("DirectML.dll");
        assert_eq!(s1.len(), 64);
        assert_eq!(s2.len(), 64);
        assert_eq!(s1, s1.to_ascii_lowercase());
        assert_eq!(s2, s2.to_ascii_lowercase());
    }

    #[cfg(windows)]
    #[test]
    fn the_fetched_runtime_loads() {
        let Some(dir_str) = std::env::var_os("LIMUSIC_TEST_RUNTIME_DIR") else {
            eprintln!("skipped: set LIMUSIC_TEST_RUNTIME_DIR");
            return;
        };
        let dir = std::path::Path::new(&dir_str);
        if !dir.join("onnxruntime.dll").is_file() {
            eprintln!("skipped: set LIMUSIC_TEST_RUNTIME_DIR");
            return;
        }
        let dir = std::fs::canonicalize(dir).unwrap();
        match load(&dir, None) {
            Ok(rt) => assert!(rt.directml(), "the downloaded DirectML.dll should pre-load"),
            Err(e @ Unavailable::VcRuntime(_)) => eprintln!("refused as expected here: {e}"),
            Err(e) => panic!("{e}"),
        }
    }
}
