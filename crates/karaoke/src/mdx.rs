use crate::device::DevicePref;
use crate::runtime::RuntimeReady;
use crate::stft::Stft;
use realfft::num_complex::Complex32;
use std::cmp::min;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct MdxParams {
    pub n_fft: usize,
    pub hop: usize,
    pub dim_f: usize,
    pub dim_t: usize,
    pub compensate: f32,
}

impl Default for MdxParams {
    fn default() -> Self {
        Self { n_fft: 6144, hop: 1024, dim_f: 3072, dim_t: 256, compensate: 1.035 }
    }
}

impl MdxParams {
    /// UVR's default overlap: consecutive chunks share one FFT window (`step = chunk_size - n_fft`
    /// in its `demix`), about 2.4%. The 25% used before processed a quarter more chunks for no
    /// measurable gain: on a 60 s chorus Light left 4.13% of the vocals at 25% and 4.10% at this,
    /// in 21% less time (Fedora, i5-8400). Inference is 95% of a job, so chunks are the cost.
    pub fn uvr_overlap(&self) -> f32 {
        self.n_fft as f32 / (self.hop * (self.dim_t - 1)) as f32
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Device {
    Cpu,
    DirectMl {
        device_id: i32,
    },
    /// CoreML (ML Program, CPU+GPU, non-static shapes)
    CoreMl,
}

#[derive(Clone, Copy, Debug)]
pub struct LoadOptions {
    pub device: Device,
    pub intra_threads: Option<usize>,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self { device: Device::Cpu, intra_threads: None }
    }
}

impl LoadOptions {
    pub fn auto(directml: bool) -> LoadOptions {
        Self::auto_with_cpu_percent(crate::device::DEFAULT_CPU_PERCENT, directml)
    }

    /// DirectML on a discrete GPU with enough VRAM when `directml` is allowed
    /// ([`RuntimeReady::directml`]), else the CPU with `percent` of the cores.
    pub fn auto_with_cpu_percent(percent: u32, directml: bool) -> LoadOptions {
        let adapters = if directml { crate::device::list_adapters() } else { Vec::new() };
        Self::choose(DevicePref::Auto, percent, directml, false, &adapters)
    }

    pub fn choose(
        pref: DevicePref,
        percent: u32,
        directml: bool,
        coreml: bool,
        adapters: &[crate::device::GpuAdapter],
    ) -> LoadOptions {
        let cpu = LoadOptions {
            device: Device::Cpu,
            intra_threads: Some(crate::device::cpu_threads_for_percent(percent)),
        };
        if directml {
            let candidate = match pref {
                DevicePref::Cpu => None,
                DevicePref::Auto => {
                    crate::device::pick_gpu(adapters, crate::device::MIN_DISCRETE_VRAM_MB)
                }
                DevicePref::Gpu => crate::device::pick_gpu_any(adapters),
            };
            if let Some(a) = candidate {
                tracing::info!("{}", crate::device::describe(a));
                return LoadOptions {
                    device: Device::DirectMl { device_id: a.index },
                    intra_threads: None,
                };
            }
        }
        if coreml && matches!(pref, DevicePref::Gpu) {
            tracing::info!("CoreML (CPU and GPU)");
            return LoadOptions { device: Device::CoreMl, intra_threads: None };
        }
        cpu
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
/// The real model has a dynamic batch_size axis, so CoreML must not be told to require static input shapes: with that option it takes none of the nodes and everything silently runs on the CPU.
pub const COREML_STATIC_SHAPES: bool = false;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub fn coreml_provider(
    units: ort::ep::coreml::ComputeUnits,
    format: ort::ep::coreml::ModelFormat,
    static_shapes: bool,
) -> ort::ep::ExecutionProviderDispatch {
    ort::ep::CoreML::default()
        .with_compute_units(units)
        .with_model_format(format)
        .with_static_input_shapes(static_shapes)
        .build()
        .error_on_failure()
}

pub struct MdxModel {
    session: ort::session::Session,
    params: MdxParams,
    stft: Stft,
}

impl MdxModel {
    /// `RuntimeReady` proves ONNX Runtime was set up first (see [`crate::runtime`]).
    pub fn load(
        rt: RuntimeReady,
        model_path: &Path,
        params: MdxParams,
        opts: LoadOptions,
    ) -> anyhow::Result<Self> {
        let mut builder =
            ort::session::Session::builder().map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if let Some(n) = opts.intra_threads {
            builder = builder.with_intra_threads(n).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        match opts.device {
            Device::Cpu => {}
            Device::DirectMl { device_id } => {
                #[cfg(not(windows))]
                {
                    let _ = device_id;
                    anyhow::bail!("DirectML is only available on Windows");
                }
                #[cfg(windows)]
                {
                    if !rt.directml() {
                        anyhow::bail!("DirectML is not available in this runtime");
                    }
                    builder = builder
                        .with_memory_pattern(false)
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?
                        .with_parallel_execution(false)
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?
                        .with_execution_providers([ort::ep::DirectML::default()
                            .with_device_id(device_id)
                            .build()])
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                }
            }
            Device::CoreMl => {
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    anyhow::bail!("CoreML is only available on Apple Silicon");
                }
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    if !rt.coreml() {
                        anyhow::bail!("CoreML is not available in this runtime");
                    }
                    builder = builder
                        .with_memory_pattern(false)
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?
                        .with_parallel_execution(false)
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?
                        .with_execution_providers([coreml_provider(
                            ort::ep::coreml::ComputeUnits::CPUAndGPU,
                            ort::ep::coreml::ModelFormat::MLProgram,
                            COREML_STATIC_SHAPES,
                        )])
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                }
            }
        }
        let session =
            builder.commit_from_file(model_path).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        // `onnxruntime.dll` resolves DirectML.dll by name when the session starts using it. If
        // that is not the downloaded copy the runtime pre-loaded, drop the session: the caller falls
        // back to the CPU rather than run on an unknown DirectML build.
        #[cfg(windows)]
        if matches!(opts.device, Device::DirectMl { .. }) {
            crate::runtime::check_directml_module().map_err(|e| anyhow::anyhow!(e))?;
        }
        #[cfg(not(windows))]
        let _ = rt;
        let stft = Stft::new(params.n_fft, params.hop);
        Ok(Self { session, params, stft })
    }

    pub fn load_with_pref(
        rt: RuntimeReady,
        model_path: &Path,
        params: MdxParams,
        pref: DevicePref,
        percent: u32,
    ) -> anyhow::Result<(Self, Device)> {
        let directml = rt.directml();
        let coreml = rt.coreml();
        let adapters = if pref != DevicePref::Cpu && directml {
            crate::device::list_adapters()
        } else {
            Vec::new()
        };
        let opts = LoadOptions::choose(pref, percent, directml, coreml, &adapters);
        let device = opts.device;
        match Self::load(rt, model_path, params, opts) {
            Ok(model) => Ok((model, device)),
            Err(e) => match device {
                Device::DirectMl { .. } => {
                    tracing::warn!(error = %crate::sanitize_error(&e.to_string()), "DirectML model load failed, falling back to CPU");
                    let cpu_opts = LoadOptions {
                        device: Device::Cpu,
                        intra_threads: Some(crate::device::cpu_threads_for_percent(percent)),
                    };
                    let model = Self::load(rt, model_path, params, cpu_opts)?;
                    Ok((model, Device::Cpu))
                }
                Device::CoreMl => {
                    tracing::warn!(error = %crate::sanitize_error(&e.to_string()), "CoreML model load failed, falling back to CPU");
                    let cpu_opts = LoadOptions {
                        device: Device::Cpu,
                        intra_threads: Some(crate::device::cpu_threads_for_percent(percent)),
                    };
                    let model = Self::load(rt, model_path, params, cpu_opts)?;
                    Ok((model, Device::Cpu))
                }
                Device::Cpu => Err(e),
            },
        }
    }

    pub fn load_auto(
        rt: RuntimeReady,
        model_path: &Path,
        params: MdxParams,
    ) -> anyhow::Result<(Self, Device)> {
        Self::load_auto_with_cpu_percent(rt, model_path, params, crate::device::DEFAULT_CPU_PERCENT)
    }

    pub fn load_auto_with_cpu_percent(
        rt: RuntimeReady,
        model_path: &Path,
        params: MdxParams,
        percent: u32,
    ) -> anyhow::Result<(Self, Device)> {
        Self::load_with_pref(rt, model_path, params, DevicePref::Auto, percent)
    }

    pub fn params(&self) -> MdxParams {
        self.params
    }

    /// mix: two channels (left, right), 44100 Hz. Returns the PRIMARY stem (for Inst_HQ_3 this is the instrumental), two channels, same length as the input.
    pub fn separate(
        &mut self,
        mix: [&[f32]; 2],
        overlap: f32,
        denoise: bool,
        progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<[Vec<f32>; 2]> {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        self.separate_cancellable(mix, overlap, denoise, 0, &cancel, progress, None)
    }

    /// Separates `mix`, starting with the chunk under `first_frame` and running to the end before
    /// going back for the start: the song from where it is playing comes out first. `emit`, when
    /// given, gets each stretch of output as soon as it is final (no chunk left to add to it),
    /// as `(first frame, left, right)` after `compensate`. Every frame is emitted exactly once, and
    /// the result is the same as a plain front-to-back pass.
    #[allow(clippy::too_many_arguments)]
    pub fn separate_cancellable(
        &mut self,
        mix: [&[f32]; 2],
        overlap: f32,
        denoise: bool,
        first_frame: usize,
        cancel: &std::sync::atomic::AtomicBool,
        mut progress: impl FnMut(usize, usize),
        mut emit: Option<&mut dyn FnMut(usize, &[f32], &[f32])>,
    ) -> anyhow::Result<[Vec<f32>; 2]> {
        let n_fft = self.params.n_fft;
        let hop = self.params.hop;
        let dim_f = self.params.dim_f;
        let dim_t = self.params.dim_t;
        let compensate = self.params.compensate;

        let l = mix[0].len();
        if mix[1].len() != l {
            anyhow::bail!("Left and right channels must have the same length");
        }

        let chunk_size = hop * (dim_t - 1);
        let trim = n_fft / 2;
        let gen_size = chunk_size - 2 * trim;
        let pad = gen_size + trim - (l % gen_size);
        let mixture_len = trim + l + pad;

        let mut mixture_l = vec![0.0f32; mixture_len];
        let mut mixture_r = vec![0.0f32; mixture_len];
        mixture_l[trim..trim + l].copy_from_slice(mix[0]);
        mixture_r[trim..trim + l].copy_from_slice(mix[1]);

        let step = ((1.0 - overlap) * chunk_size as f32) as usize;

        let mut result_l = vec![0.0f32; mixture_len];
        let mut result_r = vec![0.0f32; mixture_len];
        let mut divider = vec![0.0f32; mixture_len];

        let mut chunks = Vec::new();
        let mut i = 0;
        while i < mixture_len {
            let start = i;
            let end = min(i + chunk_size, mixture_len);
            chunks.push((start, end));
            i += step;
        }
        let total_chunks = chunks.len();
        // A frame is covered by at most two chunks only while chunks overlap by half or less,
        // which `newly_final` relies on.
        debug_assert!(step * 2 >= chunk_size);
        let first = chunks.iter().rposition(|&(s, _)| s <= first_frame + trim).unwrap_or(0);
        let mut done = vec![false; total_chunks];

        let input_name = self.session.inputs()[0].name().to_string();
        let n_bins = self.stft.n_bins();

        for (pass, chunk_idx) in (first..total_chunks).chain(0..first).enumerate() {
            let (start, end) = chunks[chunk_idx];
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                anyhow::bail!("cancelled");
            }
            let n = end - start;
            let mut chunk_l = vec![0.0f32; chunk_size];
            let mut chunk_r = vec![0.0f32; chunk_size];
            chunk_l[..n].copy_from_slice(&mixture_l[start..start + n]);
            chunk_r[..n].copy_from_slice(&mixture_r[start..start + n]);

            let mut w = vec![0.0f32; n];
            for (k, item) in w.iter_mut().enumerate() {
                *item = if overlap > 0.0 {
                    if n == 1 {
                        1.0
                    } else {
                        0.5 - 0.5
                            * (2.0 * std::f32::consts::PI * (k as f32) / ((n - 1) as f32)).cos()
                    }
                } else {
                    1.0
                };
            }

            let spec_l = self.stft.forward(&chunk_l);
            let spec_r = self.stft.forward(&chunk_r);

            let mut input_data = vec![0.0f32; 4 * dim_f * dim_t];
            for c in 0..4 {
                for f in 0..dim_f {
                    for t in 0..dim_t {
                        let cplx =
                            if t < self.stft.frames(chunk_size) && t * n_bins + f < spec_l.len() {
                                match c {
                                    0 | 1 => spec_l[t * n_bins + f],
                                    2 | 3 => spec_r[t * n_bins + f],
                                    _ => Complex32::new(0.0, 0.0),
                                }
                            } else {
                                Complex32::new(0.0, 0.0)
                            };
                        let val = if f < 3 {
                            0.0
                        } else {
                            match c {
                                0 | 2 => cplx.re,
                                1 | 3 => cplx.im,
                                _ => 0.0,
                            }
                        };
                        input_data[((c * dim_f + f) * dim_t) + t] = val;
                    }
                }
            }

            let tensor = ort::value::Tensor::from_array((
                vec![1usize, 4, dim_f, dim_t],
                input_data.clone(),
            ))?;
            let outputs = self.session.run(ort::inputs! { input_name.as_str() => tensor })?;

            let pred = if denoise {
                let data = {
                    let (_, d) = outputs[0].try_extract_tensor::<f32>()?;
                    d.to_vec()
                };
                drop(outputs);
                let neg_input_data: Vec<f32> = input_data.iter().map(|&x| -x).collect();
                let neg_tensor = ort::value::Tensor::from_array((
                    vec![1usize, 4, dim_f, dim_t],
                    neg_input_data,
                ))?;
                let outputs_neg =
                    self.session.run(ort::inputs! { input_name.as_str() => neg_tensor })?;
                let data_neg = {
                    let (_, d_neg) = outputs_neg[0].try_extract_tensor::<f32>()?;
                    d_neg.to_vec()
                };
                let mut p = vec![0.0f32; data.len()];
                for idx in 0..data.len() {
                    p[idx] = 0.5 * data[idx] - 0.5 * data_neg[idx];
                }
                p
            } else {
                let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
                data.to_vec()
            };

            let mut spec_out_l = vec![Complex32::new(0.0, 0.0); dim_t * n_bins];
            let mut spec_out_r = vec![Complex32::new(0.0, 0.0); dim_t * n_bins];

            for t in 0..dim_t {
                for b in 0..n_bins {
                    let (re_l, im_l, re_r, im_r) = if b < dim_f {
                        // Channel layout: re_L, im_L, re_R, im_R
                        let idx_l_re = (b * dim_t) + t;
                        let idx_l_im = ((dim_f + b) * dim_t) + t;
                        let idx_r_re = ((2 * dim_f + b) * dim_t) + t;
                        let idx_r_im = ((3 * dim_f + b) * dim_t) + t;
                        (pred[idx_l_re], pred[idx_l_im], pred[idx_r_re], pred[idx_r_im])
                    } else {
                        (0.0, 0.0, 0.0, 0.0)
                    };
                    spec_out_l[t * n_bins + b] = Complex32::new(re_l, im_l);
                    spec_out_r[t * n_bins + b] = Complex32::new(re_r, im_r);
                }
            }

            let out_l = self.stft.inverse(&spec_out_l, dim_t);
            let out_r = self.stft.inverse(&spec_out_r, dim_t);

            for k in 0..n {
                result_l[start + k] += out_l[k] * w[k];
                result_r[start + k] += out_r[k] * w[k];
                divider[start + k] += w[k];
            }

            done[chunk_idx] = true;
            if let Some(emit) = emit.as_mut() {
                for (a, b) in newly_final(&chunks, &done, chunk_idx) {
                    // Mixture frames to output frames: the first `trim` are padding.
                    let lo = a.max(trim) - trim;
                    let hi = b.min(trim + l).saturating_sub(trim);
                    if hi <= lo {
                        continue;
                    }
                    let fin = |res: &[f32], i: usize| {
                        let d = divider[trim + i];
                        if d >= 1e-6 {
                            (res[trim + i] / d) * compensate
                        } else {
                            0.0
                        }
                    };
                    let el: Vec<f32> = (lo..hi).map(|i| fin(&result_l, i)).collect();
                    let er: Vec<f32> = (lo..hi).map(|i| fin(&result_r, i)).collect();
                    emit(lo, &el, &er);
                }
            }

            progress(pass + 1, total_chunks);
        }

        let mut final_l = vec![0.0f32; l];
        let mut final_r = vec![0.0f32; l];
        for idx in 0..l {
            let d = divider[trim + idx];
            if d >= 1e-6 {
                final_l[idx] = (result_l[trim + idx] / d) * compensate;
                final_r[idx] = (result_r[trim + idx] / d) * compensate;
            } else {
                final_l[idx] = 0.0;
                final_r[idx] = 0.0;
            }
        }

        Ok([final_l, final_r])
    }
}

/// The mixture frames that became final when chunk `k` finished: the ones no unfinished chunk
/// still covers. Chunks overlap their neighbours only, so a frame has at most two coverers.
fn newly_final(chunks: &[(usize, usize)], done: &[bool], k: usize) -> Vec<(usize, usize)> {
    let (s, e) = chunks[k];
    let prev_end = if k > 0 { chunks[k - 1].1.min(e) } else { s };
    let next_start = chunks.get(k + 1).map_or(e, |c| c.0.max(s));
    let mut out = Vec::new();
    let mut push = |a: usize, b: usize| {
        if b > a {
            match out.last_mut() {
                Some((_, end)) if *end == a => *end = b,
                _ => out.push((a, b)),
            }
        }
    };
    if k == 0 || done[k - 1] {
        push(s, prev_end.max(s));
    }
    push(prev_end.max(s), next_start.min(e));
    if k + 1 == chunks.len() || done[k + 1] {
        push(next_start.min(e), e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever chunk the stream starts in, every output frame is emitted exactly once, and only
    /// once nothing else covers it.
    #[test]
    fn every_frame_is_final_once_in_any_start_order() {
        for overlap in [0.25f32, 6144.0 / 261120.0] {
            let chunk = 1024 * 255;
            let step = ((1.0 - overlap) * chunk as f32) as usize;
            let len = 44100 * 70;
            let mut chunks = Vec::new();
            let mut i = 0;
            while i < len {
                chunks.push((i, (i + chunk).min(len)));
                i += step;
            }
            for first in [0, 1, chunks.len() / 2, chunks.len() - 1] {
                let mut done = vec![false; chunks.len()];
                let mut seen = vec![0u8; len];
                for k in (first..chunks.len()).chain(0..first) {
                    done[k] = true;
                    for (a, b) in newly_final(&chunks, &done, k) {
                        for f in a..b {
                            seen[f] += 1;
                            if f % 97 == 0 {
                                let covered_by_unfinished = chunks
                                    .iter()
                                    .enumerate()
                                    .any(|(j, &(cs, ce))| !done[j] && cs <= f && f < ce);
                                assert!(!covered_by_unfinished, "frame {f} emitted early");
                            }
                        }
                    }
                }
                assert!(seen.iter().all(|&n| n == 1), "first chunk {first}, overlap {overlap}");
            }
        }
    }

    fn test_chunk_bookkeeping(
        l: usize,
        n_fft: usize,
        hop: usize,
        dim_t: usize,
        overlap: f32,
    ) -> (usize, bool) {
        let chunk_size = hop * (dim_t - 1);
        let trim = n_fft / 2;
        let gen_size = chunk_size - 2 * trim;
        let pad = gen_size + trim - (l % gen_size);
        let mixture_len = trim + l + pad;
        let step = ((1.0 - overlap) * chunk_size as f32) as usize;

        let mut divider = vec![0.0f32; mixture_len];
        let mut chunks = Vec::new();
        let mut i = 0;
        while i < mixture_len {
            let start = i;
            let end = min(i + chunk_size, mixture_len);
            chunks.push((start, end));
            i += step;
        }

        for &(start, end) in &chunks {
            let n = end - start;
            let mut w = vec![0.0f32; n];
            for (k, item) in w.iter_mut().enumerate() {
                *item = if overlap > 0.0 {
                    if n == 1 {
                        1.0
                    } else {
                        0.5 - 0.5
                            * (2.0 * std::f32::consts::PI * (k as f32) / ((n - 1) as f32)).cos()
                    }
                } else {
                    1.0
                };
            }
            for (k, &wk) in w.iter().enumerate() {
                divider[start + k] += wk;
            }
        }

        let total_chunks = chunks.len();
        let all_covered = divider[trim..(trim + l)].iter().all(|&v| v >= 1e-6);

        (total_chunks, all_covered)
    }

    #[test]
    fn test_load_options_choose() {
        use crate::device::{DevicePref, GpuAdapter, GpuKind, MIN_DISCRETE_VRAM_MB};

        let adapters_full = vec![
            GpuAdapter {
                index: 0,
                name: "Integrated".into(),
                kind: GpuKind::Integrated,
                dedicated_vram_mb: 512,
                shared_system_mb: 0,
            },
            GpuAdapter {
                index: 1,
                name: "Discrete 8GB".into(),
                kind: GpuKind::Discrete,
                dedicated_vram_mb: 8000,
                shared_system_mb: 0,
            },
            GpuAdapter {
                index: 2,
                name: "Software".into(),
                kind: GpuKind::Software,
                dedicated_vram_mb: 16384,
                shared_system_mb: 0,
            },
        ];

        let adapters_only_integrated = vec![GpuAdapter {
            index: 0,
            name: "Integrated".into(),
            kind: GpuKind::Integrated,
            dedicated_vram_mb: 512,
            shared_system_mb: 0,
        }];

        let adapters_low_discrete = vec![GpuAdapter {
            index: 1,
            name: "Discrete 2GB".into(),
            kind: GpuKind::Discrete,
            dedicated_vram_mb: MIN_DISCRETE_VRAM_MB - 1,
            shared_system_mb: 0,
        }];

        let empty_adapters = vec![];

        // 1. Cpu pref always gives Device::Cpu
        for &dm in &[true, false] {
            for &cm in &[true, false] {
                for adapters in &[&adapters_full, &adapters_only_integrated, &empty_adapters] {
                    let opts = LoadOptions::choose(DevicePref::Cpu, 35, dm, cm, adapters);
                    assert!(matches!(opts.device, Device::Cpu));
                }
            }
        }

        // 2. directml=false and coreml=false always gives Device::Cpu
        for &pref in &[DevicePref::Auto, DevicePref::Cpu, DevicePref::Gpu] {
            for adapters in &[&adapters_full, &adapters_only_integrated, &empty_adapters] {
                let opts = LoadOptions::choose(pref, 35, false, false, adapters);
                assert!(matches!(opts.device, Device::Cpu));
            }
        }

        // 3. Auto with directml=true:
        // - full adapter list picks discrete (index 1) >= MIN_DISCRETE_VRAM_MB
        let opts = LoadOptions::choose(DevicePref::Auto, 35, true, false, &adapters_full);
        assert!(matches!(opts.device, Device::DirectMl { device_id: 1 }));

        // - only integrated gives Cpu
        let opts =
            LoadOptions::choose(DevicePref::Auto, 35, true, false, &adapters_only_integrated);
        assert!(matches!(opts.device, Device::Cpu));

        // - low discrete gives Cpu
        let opts = LoadOptions::choose(DevicePref::Auto, 35, true, false, &adapters_low_discrete);
        assert!(matches!(opts.device, Device::Cpu));

        // - empty gives Cpu
        let opts = LoadOptions::choose(DevicePref::Auto, 35, true, false, &empty_adapters);
        assert!(matches!(opts.device, Device::Cpu));

        // 4. Gpu with directml=true:
        // - full adapter list picks discrete (index 1)
        let opts = LoadOptions::choose(DevicePref::Gpu, 35, true, false, &adapters_full);
        assert!(matches!(opts.device, Device::DirectMl { device_id: 1 }));

        // - only integrated picks integrated (index 0)
        let opts = LoadOptions::choose(DevicePref::Gpu, 35, true, false, &adapters_only_integrated);
        assert!(matches!(opts.device, Device::DirectMl { device_id: 0 }));

        // - empty falls back to Cpu
        let opts = LoadOptions::choose(DevicePref::Gpu, 35, true, false, &empty_adapters);
        assert!(matches!(opts.device, Device::Cpu));

        // 5. CoreML tests:
        // - coreml=true with Gpu gives CoreMl
        let opts = LoadOptions::choose(DevicePref::Gpu, 35, false, true, &empty_adapters);
        assert!(matches!(opts.device, Device::CoreMl));

        // - coreml=true with Auto gives Cpu
        let opts = LoadOptions::choose(DevicePref::Auto, 35, false, true, &empty_adapters);
        assert!(matches!(opts.device, Device::Cpu));

        // - coreml=true with Cpu gives Cpu
        let opts = LoadOptions::choose(DevicePref::Cpu, 35, false, true, &empty_adapters);
        assert!(matches!(opts.device, Device::Cpu));

        // - directml=true and coreml=true prefers DirectML
        let opts = LoadOptions::choose(DevicePref::Gpu, 35, true, true, &adapters_full);
        assert!(matches!(opts.device, Device::DirectMl { device_id: 1 }));

        // - both false gives Cpu
        let opts = LoadOptions::choose(DevicePref::Gpu, 35, false, false, &empty_adapters);
        assert!(matches!(opts.device, Device::Cpu));
    }

    #[test]
    fn test_bookkeeping_coverage() {
        let hop = 1024;
        let dim_t = 256;
        for n_fft in [5120, 6144] {
            let uvr = MdxParams { n_fft, hop, dim_f: 2048, dim_t, compensate: 1.0 }.uvr_overlap();
            for overlap in [0.25, uvr] {
                for l in [44100 * 5, 44100 * 61 + 17] {
                    let (total_chunks, all_covered) =
                        test_chunk_bookkeeping(l, n_fft, hop, dim_t, overlap);
                    assert!(total_chunks > 0);
                    assert!(all_covered, "gap at n_fft {n_fft} overlap {overlap} len {l}");
                }
            }
        }
    }

    #[test]
    fn uvr_overlap_is_one_fft_window_per_chunk() {
        let light = MdxParams { n_fft: 6144, hop: 1024, dim_f: 2048, dim_t: 256, compensate: 1.0 };
        assert!((light.uvr_overlap() - 6144.0 / 261120.0).abs() < 1e-6);
        // A 60 s clip: 15 chunks at 25%, 12 at UVR's.
        let l = 44100 * 60;
        assert_eq!(test_chunk_bookkeeping(l, 6144, 1024, 256, 0.25).0, 15);
        assert_eq!(test_chunk_bookkeeping(l, 6144, 1024, 256, light.uvr_overlap()).0, 12);
    }
}
