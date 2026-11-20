//! Run this test with:
//! cargo test -p karaoke --test webgpu_parity -- --ignored --nocapture
//! Requires a Vulkan driver (on CI the software lavapipe one).

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

mod webgpu_common;

use karaoke::mdx::{Device, LoadOptions, MdxModel};
use karaoke::models::ModelId;
use std::path::PathBuf;
use std::time::Instant;

struct SimpleLcg {
    state: u64,
}

impl SimpleLcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_f32(&mut self) -> f32 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let val = (self.state >> 33) as i32 as f32;
        val / (i32::MAX as f32)
    }
}

struct ScratchGuard(PathBuf);
impl Drop for ScratchGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pearson_correlation(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len();
    if n == 0 {
        return 1.0;
    }
    let mean_a: f32 = a.iter().sum::<f32>() / n as f32;
    let mean_b: f32 = b.iter().sum::<f32>() / n as f32;
    let mut num = 0.0;
    let mut den_a = 0.0;
    let mut den_b = 0.0;
    for (&ai, &bi) in a.iter().zip(b.iter()) {
        let da = ai - mean_a;
        let db = bi - mean_b;
        num += da * db;
        den_a += da * da;
        den_b += db * db;
    }
    let den = (den_a * den_b).sqrt();
    if den == 0.0 {
        1.0
    } else {
        num / den
    }
}

fn relative_l2_error(reference: &[f32], test: &[f32]) -> f32 {
    let mut num_sq = 0.0;
    let mut ref_sq = 0.0;
    for (&r, &t) in reference.iter().zip(test.iter()) {
        let diff = r - t;
        num_sq += diff * diff;
        ref_sq += r * r;
    }
    if ref_sq == 0.0 {
        if num_sq == 0.0 {
            0.0
        } else {
            f32::INFINITY
        }
    } else {
        num_sq.sqrt() / ref_sq.sqrt()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "downloads the runtime and a model; run it by hand or in CI"]
async fn webgpu_matches_the_cpu() {
    let (dest, rt) = webgpu_common::install_runtime().await;
    let _guard = ScratchGuard(dest.parent().unwrap().to_path_buf());
    let rt = rt.expect("runtime must be available on Linux x86_64");

    let model_path =
        webgpu_common::download_model(ModelId::Light, &dest.parent().unwrap().join("models")).await;
    let spec = ModelId::Light.spec();

    let num_samples = 15 * 44100;
    let mut mix_l = vec![0.0f32; num_samples];
    let mut mix_r = vec![0.0f32; num_samples];
    let mut lcg = SimpleLcg::new(1337);

    for (i, (l, r)) in mix_l.iter_mut().zip(mix_r.iter_mut()).enumerate() {
        let t = i as f32 / 44100.0;
        let s1 = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.2;
        let s2 = (2.0 * std::f32::consts::PI * 880.0 * t).sin() * 0.15;
        let s3 = (2.0 * std::f32::consts::PI * 330.0 * t).sin() * 0.25;
        let noise_l = lcg.next_f32() * 0.05;
        let noise_r = lcg.next_f32() * 0.05;

        *l = s1 + s2 + noise_l;
        *r = s1 + s3 + noise_r;
    }

    // Load CPU model directly without load_with_pref to avoid silent fallback
    let start = Instant::now();
    let mut cpu_model = MdxModel::load(
        rt,
        &model_path,
        spec.params,
        LoadOptions { device: Device::Cpu, intra_threads: None },
    )
    .expect("failed to load CPU model");
    let cpu_load_ms = start.elapsed().as_millis();
    println!("cpu_load_ms = {}", cpu_load_ms);

    // Load WebGPU model directly without load_with_pref to avoid silent fallback
    let start = Instant::now();
    let mut webgpu_model = MdxModel::load(
        rt,
        &model_path,
        spec.params,
        LoadOptions { device: Device::WebGpu, intra_threads: None },
    )
    .expect("failed to load WebGPU model");
    let webgpu_load_ms = start.elapsed().as_millis();
    println!("webgpu_load_ms = {}", webgpu_load_ms);

    let audio_seconds = num_samples as f32 / 44100.0;

    // Run separation on CPU
    let start = Instant::now();
    let cpu_sep = cpu_model
        .separate([&mix_l, &mix_r], 0.25, false, &mut |_, _| {})
        .expect("CPU separation failed");
    let cpu_separate_ms = start.elapsed().as_millis();
    let cpu_rtf = cpu_separate_ms as f32 / 1000.0 / audio_seconds;
    println!(
        "cpu_separate_ms = {}, cpu_rtf = {:.4}, audio_seconds = {}",
        cpu_separate_ms, cpu_rtf, audio_seconds
    );

    // Run separation on WebGPU
    let start = Instant::now();
    let webgpu_sep = webgpu_model
        .separate([&mix_l, &mix_r], 0.25, false, &mut |_, _| {})
        .expect("WebGPU separation failed");
    let webgpu_separate_ms = start.elapsed().as_millis();
    let webgpu_rtf = webgpu_separate_ms as f32 / 1000.0 / audio_seconds;
    println!(
        "webgpu_separate_ms = {}, webgpu_rtf = {:.4}, audio_seconds = {}",
        webgpu_separate_ms, webgpu_rtf, audio_seconds
    );

    let corr_l = pearson_correlation(&cpu_sep[0], &webgpu_sep[0]);
    let corr_r = pearson_correlation(&cpu_sep[1], &webgpu_sep[1]);
    let err_l = relative_l2_error(&cpu_sep[0], &webgpu_sep[0]);
    let err_r = relative_l2_error(&cpu_sep[1], &webgpu_sep[1]);

    println!(
        "corr_l = {:.6}, corr_r = {:.6}, err_l = {:.6}, err_r = {:.6}",
        corr_l, corr_r, err_l, err_r
    );

    assert!(corr_l >= 0.99, "Left channel correlation {} < 0.99", corr_l);
    assert!(corr_r >= 0.99, "Right channel correlation {} < 0.99", corr_r);
    assert!(corr_l.is_finite(), "corr_l is not finite");
    assert!(corr_r.is_finite(), "corr_r is not finite");
    assert!(err_l.is_finite(), "err_l is not finite");
    assert!(err_r.is_finite(), "err_r is not finite");

    let speedup = cpu_separate_ms as f64 / webgpu_separate_ms as f64;
    println!(
        "WEBGPU_SUMMARY cpu_load_ms={} webgpu_load_ms={} cpu_rtf={:.4} webgpu_rtf={:.4} speedup={:.2} corr_l={:.4} corr_r={:.4}",
        cpu_load_ms, webgpu_load_ms, cpu_rtf, webgpu_rtf, speedup, corr_l, corr_r
    );
}
