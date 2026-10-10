use std::path::Path;
use std::cmp::min;
use num_complex::Complex32;
use crate::stft::Stft;

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
        Self {
            n_fft: 6144,
            hop: 1024,
            dim_f: 3072,
            dim_t: 256,
            compensate: 1.035,
        }
    }
}

pub struct MdxModel {
    session: ort::session::Session,
    params: MdxParams,
    stft: Stft,
}

impl MdxModel {
    pub fn load(model_path: &Path, params: MdxParams) -> anyhow::Result<Self> {
        let session = ort::session::Session::builder()?
            .commit_from_file(model_path)?;
        let stft = Stft::new(params.n_fft, params.hop);
        Ok(Self { session, params, stft })
    }

    /// mix: two channels (left, right), 44100 Hz. Returns the PRIMARY stem (for Inst_HQ_3 this is the instrumental), two channels, same length as the input.
    pub fn separate(
        &mut self,
        mix: [&[f32]; 2],
        overlap: f32,
        denoise: bool,
        progress: &mut dyn FnMut(usize, usize),
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

        let input_name = self.session.inputs()[0].name().to_string();
        let n_bins = self.stft.n_bins();

        for (chunk_idx, &(start, end)) in chunks.iter().enumerate() {
            let n = end - start;
            let mut chunk_l = vec![0.0f32; chunk_size];
            let mut chunk_r = vec![0.0f32; chunk_size];
            for k in 0..n {
                chunk_l[k] = mixture_l[start + k];
                chunk_r[k] = mixture_r[start + k];
            }

            let mut w = vec![0.0f32; n];
            for k in 0..n {
                w[k] = if overlap > 0.0 {
                    if n == 1 {
                        1.0
                    } else {
                        0.5 - 0.5 * (2.0 * std::f32::consts::PI * (k as f32) / ((n - 1) as f32)).cos()
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
                        let cplx = if t < self.stft.frames(chunk_size) && t * n_bins + f < spec_l.len() {
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

            let tensor = ort::value::Tensor::from_array((vec![1usize, 4, dim_f, dim_t], input_data.clone()))?;
            let outputs = self.session.run(ort::inputs! { input_name.as_str() => tensor })?;

            let pred = if denoise {
                let neg_input_data: Vec<f32> = input_data.iter().map(|&x| -x).collect();
                let neg_tensor = ort::value::Tensor::from_array((vec![1usize, 4, dim_f, dim_t], neg_input_data))?;
                let outputs_neg = self.session.run(ort::inputs! { input_name.as_str() => neg_tensor })?;
                let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
                let (_, data_neg) = outputs_neg[0].try_extract_tensor::<f32>()?;
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
                        let idx_l_re = ((0 * dim_f + b) * dim_t) + t;
                        let idx_l_im = ((1 * dim_f + b) * dim_t) + t;
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

            progress(chunk_idx + 1, total_chunks);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chunk_bookkeeping(l: usize, n_fft: usize, hop: usize, dim_t: usize, overlap: f32) -> (usize, bool) {
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
            for k in 0..n {
                w[k] = if overlap > 0.0 {
                    if n == 1 {
                        1.0
                    } else {
                        0.5 - 0.5 * (2.0 * std::f32::consts::PI * (k as f32) / ((n - 1) as f32)).cos()
                    }
                } else {
                    1.0
                };
            }
            for k in 0..n {
                divider[start + k] += w[k];
            }
        }

        let total_chunks = chunks.len();
        let mut all_covered = true;
        for i in trim..(trim + l) {
            if divider[i] < 1e-6 {
                all_covered = false;
                break;
            }
        }

        (total_chunks, all_covered)
    }

    #[test]
    fn test_bookkeeping_coverage() {
        let n_fft = 6144;
        let hop = 1024;
        let dim_t = 256;
        let l = 44100 * 5;
        let overlap = 0.25;

        let (total_chunks, all_covered) = test_chunk_bookkeeping(l, n_fft, hop, dim_t, overlap);
        assert!(total_chunks > 0);
        assert!(all_covered, "Not all samples in trim..trim+L are covered by divider > 0");
    }
}
