//! WebGPU plugin execution provider (Linux): Vulkan on Intel, AMD and NVIDIA GPUs.
//! The plugin library is registered with ONNX Runtime by its absolute path, once per process.

use ort::environment::Environment;
use ort::memory::DeviceType;
use ort::session::builder::SessionBuilder;
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// Name under which the plugin reports its devices.
const EP_NAME: &str = "WebGpuExecutionProvider";

/// Result of the first registration attempt, kept for the whole process.
static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();

/// Registers the plugin library once per process. The returned handle is dropped on purpose: it
/// has no Drop impl, so the library stays registered until it is unregistered explicitly, which
/// we never do.
fn register(env: &Arc<Environment>, plugin: &Path) -> anyhow::Result<()> {
    REGISTERED
        .get_or_init(|| {
            env.register_ep_library("webgpu", plugin).map(|_| ()).map_err(|e| e.to_string())
        })
        .clone()
        .map_err(|e| anyhow::anyhow!("WebGPU plugin: {e}"))
}

/// Appends the WebGPU execution provider (a GPU through Vulkan) to `builder`.
pub fn attach(builder: SessionBuilder, plugin: &Path) -> anyhow::Result<SessionBuilder> {
    if !plugin.is_absolute() {
        anyhow::bail!("WebGPU plugin path must be absolute");
    }
    let env = Environment::current().map_err(|e| anyhow::anyhow!(e.to_string()))?;
    register(&env, plugin)?;

    let mut candidates = Vec::new();
    for device in env.devices() {
        let Ok(ep) = device.ep() else { continue };
        if !ep.eq_ignore_ascii_case(EP_NAME) {
            continue;
        }
        let hardware = device.hardware_device();
        let ty = hardware.ty();
        tracing::debug!(
            vendor = hardware.vendor().unwrap_or("unknown"),
            id = hardware.id(),
            "WebGPU device candidate"
        );
        if matches!(ty, DeviceType::CPU) {
            continue;
        }
        candidates.push((matches!(ty, DeviceType::GPU), device));
    }
    // There is no discrete/integrated flag in this API: take the first GPU, else the first device.
    let index = candidates.iter().position(|(is_gpu, _)| *is_gpu).unwrap_or(0);
    let Some((_, device)) = candidates.into_iter().nth(index) else {
        anyhow::bail!("no WebGPU device");
    };
    tracing::info!(
        target: "app_lib::karaoke",
        vendor = device.hardware_device().vendor().unwrap_or("unknown"),
        "karaoke: using a WebGPU (Vulkan) device"
    );
    builder.with_devices([device], None).map_err(|e| anyhow::anyhow!(e.to_string()))
}
