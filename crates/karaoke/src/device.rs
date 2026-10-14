#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuKind {
    Discrete,
    Integrated,
    Software,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct GpuAdapter {
    pub index: i32,
    pub name: String,
    pub kind: GpuKind,
    pub dedicated_vram_mb: u64,
    pub shared_system_mb: u64,
}

/// `dxgi.dll` and `d3d12.dll` are looked up at run time, from System32 only, instead of being
/// imported: the exe's import table has to stay exactly upstream's, so that a machine without
/// them still starts and nothing GPU-related is loaded unless karaoke runs (this is only called
/// when the model loads). The COM calls afterwards go through vtables and import nothing.
#[cfg(windows)]
pub(crate) mod sys {
    use core::ffi::c_void;
    use windows::core::{GUID, HRESULT, PCSTR, PCWSTR};
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL;
    use windows::Win32::System::LibraryLoader::{
        GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };

    /// `CreateDXGIFactory1(REFIID, void**)`.
    pub type CreateDxgiFactory1 =
        unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
    /// `D3D12CreateDevice(IUnknown*, D3D_FEATURE_LEVEL, REFIID, void**)`.
    pub type D3d12CreateDevice = unsafe extern "system" fn(
        *mut c_void,
        D3D_FEATURE_LEVEL,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT;

    /// `name` from the System32 copy of `dll`. The module is never freed: objects created from it
    /// outlive this call, and it is a system DLL.
    ///
    /// # Safety
    /// `T` must be the function pointer type that matches the export's real signature.
    pub unsafe fn proc<T: Copy>(dll: PCWSTR, name: PCSTR) -> Option<T> {
        let module = unsafe { LoadLibraryExW(dll, None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
        let f = unsafe { GetProcAddress(module, name) }?;
        debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of_val(&f));
        Some(unsafe { std::mem::transmute_copy(&f) })
    }
}

#[cfg(windows)]
pub fn list_adapters() -> Vec<GpuAdapter> {
    use windows::core::{s, w, Interface};
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
    use windows::Win32::Graphics::Direct3D12::{
        ID3D12Device, D3D12_FEATURE_ARCHITECTURE1, D3D12_FEATURE_DATA_ARCHITECTURE1,
    };
    use windows::Win32::Graphics::Dxgi::{
        IDXGIAdapter1, IDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_ERROR_NOT_FOUND,
    };

    let mut adapters = Vec::new();

    let Some(create_factory) =
        (unsafe { sys::proc::<sys::CreateDxgiFactory1>(w!("dxgi.dll"), s!("CreateDXGIFactory1")) })
    else {
        return adapters;
    };
    // Missing on a machine without D3D12: adapters are still listed, just of unknown kind.
    let create_device =
        unsafe { sys::proc::<sys::D3d12CreateDevice>(w!("d3d12.dll"), s!("D3D12CreateDevice")) };

    let factory: IDXGIFactory1 = {
        let mut raw = std::ptr::null_mut();
        let hr = unsafe { create_factory(&IDXGIFactory1::IID, &mut raw) };
        if hr.is_err() || raw.is_null() {
            return adapters;
        }
        unsafe { IDXGIFactory1::from_raw(raw) }
    };

    let mut i = 0;
    loop {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(i) } {
            Ok(a) => a,
            Err(e) => {
                if e.code() == DXGI_ERROR_NOT_FOUND {
                    break;
                }
                break;
            }
        };

        let desc: DXGI_ADAPTER_DESC1 = match unsafe { adapter.GetDesc1() } {
            Ok(d) => d,
            Err(_) => {
                i += 1;
                continue;
            }
        };

        let name_len =
            desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..name_len]);

        let dedicated_vram_mb = desc.DedicatedVideoMemory as u64 / (1024 * 1024);
        let shared_system_mb = desc.SharedSystemMemory as u64 / (1024 * 1024);

        let kind = if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
            GpuKind::Software
        } else {
            let unknown = match adapter.cast::<windows::core::IUnknown>() {
                Ok(u) => u,
                Err(_) => {
                    i += 1;
                    continue;
                }
            };
            let device: Option<ID3D12Device> = create_device.and_then(|create| {
                let mut raw = std::ptr::null_mut();
                let hr = unsafe {
                    create(unknown.as_raw(), D3D_FEATURE_LEVEL_11_0, &ID3D12Device::IID, &mut raw)
                };
                (hr.is_ok() && !raw.is_null()).then(|| unsafe { ID3D12Device::from_raw(raw) })
            });
            if let Some(dev) = device {
                let mut arch =
                    D3D12_FEATURE_DATA_ARCHITECTURE1 { NodeIndex: 0, ..Default::default() };
                let support_result = unsafe {
                    dev.CheckFeatureSupport(
                        D3D12_FEATURE_ARCHITECTURE1,
                        &mut arch as *mut _ as *mut _,
                        std::mem::size_of::<D3D12_FEATURE_DATA_ARCHITECTURE1>() as u32,
                    )
                };
                if support_result.is_ok() {
                    if arch.UMA.as_bool() {
                        GpuKind::Integrated
                    } else {
                        GpuKind::Discrete
                    }
                } else {
                    GpuKind::Unknown
                }
            } else {
                GpuKind::Unknown
            }
        };

        adapters.push(GpuAdapter {
            index: i as i32,
            name,
            kind,
            dedicated_vram_mb,
            shared_system_mb,
        });

        i += 1;
    }

    adapters
}

#[cfg(not(windows))]
pub fn list_adapters() -> Vec<GpuAdapter> {
    Vec::new()
}

pub const MIN_DISCRETE_VRAM_MB: u64 = 3000;

pub fn pick_gpu(adapters: &[GpuAdapter], min_discrete_vram_mb: u64) -> Option<&GpuAdapter> {
    let mut best_discrete: Option<&GpuAdapter> = None;
    for a in adapters {
        if a.kind == GpuKind::Discrete && a.dedicated_vram_mb >= min_discrete_vram_mb {
            match best_discrete {
                None => best_discrete = Some(a),
                Some(curr) => {
                    if a.dedicated_vram_mb > curr.dedicated_vram_mb
                        || (a.dedicated_vram_mb == curr.dedicated_vram_mb && a.index < curr.index)
                    {
                        best_discrete = Some(a);
                    }
                }
            }
        }
    }
    best_discrete
}

pub fn describe(a: &GpuAdapter) -> String {
    let kind_str = match a.kind {
        GpuKind::Discrete => "Discrete",
        GpuKind::Integrated => "Integrated",
        GpuKind::Software => "Software",
        GpuKind::Unknown => "Unknown",
    };
    format!("{} [{}, {} MB VRAM, index {}]", a.name, kind_str, a.dedicated_vram_mb, a.index)
}

pub const DEFAULT_CPU_PERCENT: u32 = 35;

pub fn cpu_threads_for_percent(percent: u32) -> usize {
    let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let p = percent.clamp(1, 100);
    let threads = (logical as f64 * p as f64 / 100.0).round() as usize;
    std::cmp::max(1, std::cmp::min(threads, logical))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pick_gpu() {
        let adapters = vec![
            GpuAdapter {
                index: 0,
                name: "Integrated GPU".into(),
                kind: GpuKind::Integrated,
                dedicated_vram_mb: 8192,
                shared_system_mb: 0,
            },
            GpuAdapter {
                index: 1,
                name: "Discrete GPU 4GB".into(),
                kind: GpuKind::Discrete,
                dedicated_vram_mb: 4096,
                shared_system_mb: 0,
            },
        ];

        // 1. integrated 8192 MB + discrete 4096 MB -> picks the discrete one
        let picked = pick_gpu(&adapters, 3000);
        assert!(picked.is_some());
        assert_eq!(picked.unwrap().index, 1);

        // 2. discrete below the threshold gives None
        let adapters_low_discrete = vec![GpuAdapter {
            index: 0,
            name: "Discrete GPU 2GB".into(),
            kind: GpuKind::Discrete,
            dedicated_vram_mb: 2048,
            shared_system_mb: 0,
        }];
        let picked = pick_gpu(&adapters_low_discrete, 3000);
        assert!(picked.is_none());

        // 3. integrated GPU with large memory is NOT picked, even when it is the only adapter (result None)
        let adapters_only_integrated = vec![GpuAdapter {
            index: 0,
            name: "Integrated GPU".into(),
            kind: GpuKind::Integrated,
            dedicated_vram_mb: 8192,
            shared_system_mb: 0,
        }];
        let picked = pick_gpu(&adapters_only_integrated, 3000);
        assert!(picked.is_none());

        // 4. software never picked
        let adapters_software = vec![GpuAdapter {
            index: 0,
            name: "Software GPU".into(),
            kind: GpuKind::Software,
            dedicated_vram_mb: 16384,
            shared_system_mb: 0,
        }];
        let picked = pick_gpu(&adapters_software, 3000);
        assert!(picked.is_none());

        // 5. two discrete 4096 and 8192 -> picks 8192
        let adapters_two_discrete = vec![
            GpuAdapter {
                index: 0,
                name: "Discrete 4GB".into(),
                kind: GpuKind::Discrete,
                dedicated_vram_mb: 4096,
                shared_system_mb: 0,
            },
            GpuAdapter {
                index: 1,
                name: "Discrete 8GB".into(),
                kind: GpuKind::Discrete,
                dedicated_vram_mb: 8192,
                shared_system_mb: 0,
            },
        ];
        let picked = pick_gpu(&adapters_two_discrete, 3000);
        assert!(picked.is_some());
        assert_eq!(picked.unwrap().index, 1);

        // 6. discrete with exactly min_discrete_vram_mb passes, one MB less fails
        let adapters_exact = vec![
            GpuAdapter {
                index: 0,
                name: "Discrete Exact".into(),
                kind: GpuKind::Discrete,
                dedicated_vram_mb: 3000,
                shared_system_mb: 0,
            },
            GpuAdapter {
                index: 1,
                name: "Discrete Less".into(),
                kind: GpuKind::Discrete,
                dedicated_vram_mb: 2999,
                shared_system_mb: 0,
            },
        ];
        let picked = pick_gpu(&adapters_exact, 3000);
        assert!(picked.is_some());
        assert_eq!(picked.unwrap().index, 0);
    }

    #[test]
    fn test_cpu_threads_for_percent() {
        let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);

        let t100 = cpu_threads_for_percent(100);
        assert_eq!(t100, logical);

        let t1 = cpu_threads_for_percent(1);
        assert!(t1 >= 1);

        let t0 = cpu_threads_for_percent(0);
        assert!(t0 >= 1);

        let t500 = cpu_threads_for_percent(500);
        assert_eq!(t500, logical);
    }

    /// Runs the real run-time DXGI/D3D12 lookup. Whatever the machine has (CI runners have only
    /// the software adapter), it must return without panicking, and indices must be in order.
    #[cfg(windows)]
    #[test]
    fn listing_adapters_through_runtime_lookup_does_not_panic() {
        let adapters = list_adapters();
        for (i, a) in adapters.iter().enumerate() {
            assert!(a.index >= i as i32);
        }
    }
}
