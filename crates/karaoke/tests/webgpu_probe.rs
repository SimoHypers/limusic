//! Run this test with:
//! cargo test -p karaoke --test webgpu_probe -- --ignored --nocapture

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

mod webgpu_common;

use karaoke::models::ModelId;
use std::path::PathBuf;
use std::time::Instant;

/// `ort::Error<SessionBuilder>` holds the builder, which is not `Send`, so `?` cannot turn it into
/// an `anyhow::Error` directly. The crate does the same conversion in `MdxModel::load`.
fn ort_err(e: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

struct ScratchGuard(PathBuf);
impl Drop for ScratchGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "downloads the runtime and a model; run it by hand or in CI"]
async fn where_do_the_nodes_run_webgpu() -> anyhow::Result<()> {
    let (dest, rt) = webgpu_common::install_runtime().await;
    let scratch_dir = dest.parent().unwrap().to_path_buf();
    let _guard = ScratchGuard(scratch_dir.clone());
    let _rt = rt.expect("runtime must be available on Linux x86_64");

    let model_path =
        webgpu_common::download_model(ModelId::Kara, &scratch_dir.join("models")).await;
    let spec = ModelId::Kara.spec();

    // Load a plain session just to print the inputs.
    let temp_session = ort::session::Session::builder()
        .map_err(ort_err)
        .expect("failed to create session builder")
        .commit_from_file(&model_path)
        .map_err(ort_err)
        .expect("failed to commit model for input inspection");

    for input in temp_session.inputs() {
        println!("WEBGPU_PROBE input name = {}, dtype = {:?}", input.name(), input.dtype());
    }

    let input_name = temp_session.inputs()[0].name().to_string();
    drop(temp_session);

    let plugin_path = karaoke::runtime::webgpu_plugin_path()
        .expect("WebGPU plugin is not available in this runtime");
    let profile_prefix = scratch_dir.join("profile_webgpu");

    let load_start = Instant::now();
    let mut builder = ort::session::Session::builder().map_err(ort_err)?;
    builder = builder
        .with_memory_pattern(false)
        .map_err(ort_err)?
        .with_parallel_execution(false)
        .map_err(ort_err)?
        .with_profiling(&profile_prefix)
        .map_err(ort_err)?;

    let mut builder = karaoke::webgpu::attach(builder, plugin_path)?;

    let mut session = builder.commit_from_file(&model_path).map_err(ort_err)?;
    let load_ms = load_start.elapsed().as_millis();

    let tensor = ort::value::Tensor::from_array((
        vec![1usize, 4, spec.params.dim_f, spec.params.dim_t],
        vec![0.0f32; 4 * spec.params.dim_f * spec.params.dim_t],
    ))
    .map_err(ort_err)?;

    // Run three times; the first run includes any lazy compilation.
    let mut run_durations = Vec::new();
    for _ in 0..3 {
        let run_start = Instant::now();
        let _outputs =
            session.run(ort::inputs! { input_name.as_str() => tensor.view() }).map_err(ort_err)?;
        run_durations.push(run_start.elapsed().as_millis());
    }
    let run_ms_avg = run_durations.iter().sum::<u128>() / run_durations.len() as u128;

    let profile_file = session.end_profiling().map_err(ort_err)?;
    let json_content = std::fs::read_to_string(&profile_file)?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&json_content)?;

    let mut provider_counts = std::collections::BTreeMap::new();
    for event in events {
        if event.get("cat").and_then(|v| v.as_str()) == Some("Node") {
            if let Some(args) = event.get("args") {
                let provider = args.get("provider").and_then(|v| v.as_str()).unwrap_or("Unknown");
                *provider_counts.entry(provider.to_string()).or_insert(0) += 1;
            }
        }
    }

    for (provider, count) in &provider_counts {
        println!("WEBGPU_PROBE provider={} nodes={}", provider, count);
    }
    println!("WEBGPU_PROBE load_ms={} run_ms={}", load_ms, run_ms_avg);

    let webgpu_nodes = provider_counts.get("WebGpuExecutionProvider").copied().unwrap_or(0);
    assert!(webgpu_nodes > 0, "WebGPU took no node of the model, everything would run on the CPU");
    Ok(())
}
