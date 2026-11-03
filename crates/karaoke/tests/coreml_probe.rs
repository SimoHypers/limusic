//! Run this test with:
//! cargo test -p karaoke --test coreml_probe -- --ignored --nocapture

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

mod common;

use karaoke::mdx::coreml_provider;
use karaoke::models::ModelId;
use std::path::PathBuf;
use std::time::Instant;

struct ScratchGuard(PathBuf);
impl Drop for ScratchGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "downloads the runtime and a model; run it by hand or in CI"]
async fn where_do_the_nodes_run() {
    let (dest, rt) = common::install_runtime().await;
    let scratch_dir = dest.parent().unwrap().to_path_buf();
    let _guard = ScratchGuard(scratch_dir.clone());
    let rt = rt.expect("runtime must be available on Apple Silicon");

    let model_path = common::download_model(ModelId::Kara, &scratch_dir.join("models")).await;
    let spec = ModelId::Kara.spec();

    // Load a temporary session just to print inputs
    let temp_session = ort::session::Session::builder()
        .map_err(|e| anyhow::anyhow!(e.to_string()))
        .expect("failed to create session builder")
        .commit_from_file(&model_path)
        .map_err(|e| anyhow::anyhow!(e.to_string()))
        .expect("failed to commit model for input inspection");

    for input in temp_session.inputs() {
        println!("COREML_PROBE input name = {}, dtype = {:?}", input.name(), input.dtype());
    }

    let input_name = temp_session.inputs()[0].name().to_string();
    drop(temp_session);

    let variants = vec![
        (
            "mlprog_gpu_static",
            true,
            ort::ep::coreml::ModelFormat::MLProgram,
            ort::ep::coreml::ComputeUnits::CPUAndGPU,
        ),
        (
            "mlprog_gpu_dyn",
            false,
            ort::ep::coreml::ModelFormat::MLProgram,
            ort::ep::coreml::ComputeUnits::CPUAndGPU,
        ),
        (
            "nn_gpu_static",
            true,
            ort::ep::coreml::ModelFormat::NeuralNetwork,
            ort::ep::coreml::ComputeUnits::CPUAndGPU,
        ),
        (
            "nn_gpu_dyn",
            false,
            ort::ep::coreml::ModelFormat::NeuralNetwork,
            ort::ep::coreml::ComputeUnits::CPUAndGPU,
        ),
        (
            "mlprog_all_dyn",
            false,
            ort::ep::coreml::ModelFormat::MLProgram,
            ort::ep::coreml::ComputeUnits::All,
        ),
    ];

    for (name, static_shapes, format, units) in variants {
        let profile_prefix = scratch_dir.join(format!("profile_{}", name));

        let run_test = || -> Result<(u128, u128, usize, usize, Vec<String>), anyhow::Error> {
            let load_start = Instant::now();
            let mut builder =
                ort::session::Session::builder().map_err(|e| anyhow::anyhow!(e.to_string()))?;
            builder = builder
                .with_memory_pattern(false)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .with_parallel_execution(false)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .with_profiling(&profile_prefix)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .with_execution_providers([coreml_provider(units, format, static_shapes)]);

            let mut session = builder
                .commit_from_file(&model_path)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            let load_ms = load_start.elapsed().as_millis();

            let tensor = ort::value::Tensor::from_array((
                vec![1usize, 4, spec.params.dim_f, spec.params.dim_t],
                vec![0.0f32; 4 * spec.params.dim_f * spec.params.dim_t],
            ))?;

            // Warmup / run 3 times
            let mut run_durations = Vec::new();
            for _ in 0..3 {
                let run_start = Instant::now();
                let _outputs =
                    session.run(ort::inputs! { input_name.as_str() => tensor.view() })?;
                run_durations.push(run_start.elapsed().as_millis());
            }
            let run_ms_avg = run_durations.iter().sum::<u128>() / run_durations.len() as u128;

            let profile_file = session.end_profiling()?;
            let json_content = std::fs::read_to_string(&profile_file)?;
            let events: Vec<serde_json::Value> = serde_json::from_str(&json_content)?;

            let mut coreml_events = 0;
            let mut cpu_events = 0;
            let mut cpu_ops = std::collections::BTreeSet::new();

            for event in events {
                if event.get("cat").and_then(|v| v.as_str()) == Some("Node") {
                    if let Some(args) = event.get("args") {
                        let provider = args.get("provider").and_then(|v| v.as_str()).unwrap_or("");
                        let op_name = args.get("op_name").and_then(|v| v.as_str()).unwrap_or("");
                        if provider.contains("CoreML") {
                            coreml_events += 1;
                        } else {
                            cpu_events += 1;
                            if !op_name.is_empty() {
                                cpu_ops.insert(op_name.to_string());
                            }
                        }
                    }
                }
            }

            let cpu_ops_vec: Vec<String> = cpu_ops.into_iter().take(15).collect();
            Ok((load_ms, run_ms_avg, coreml_events, cpu_events, cpu_ops_vec))
        };

        match run_test() {
            Ok((load_ms, run_ms_avg, coreml_events, cpu_events, cpu_ops)) => {
                let cpu_ops_str = cpu_ops.join(",");
                println!(
                    "COREML_PROBE variant={} load_ms={} run_ms_avg={} coreml_events={} cpu_events={} cpu_ops={}",
                    name, load_ms, run_ms_avg, coreml_events, cpu_events, cpu_ops_str
                );
            }
            Err(e) => {
                let err_msg = karaoke::sanitize_error(&e.to_string());
                println!("COREML_PROBE variant={} error={}", name, err_msg);
            }
        }
    }
}
