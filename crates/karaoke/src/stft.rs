use realfft::{RealFftPlanner, num_complex::Complex32};

pub struct Stft {
    n_fft: usize,
    hop: usize,
    window: Vec<f32>,
}

fn reflect_index(i: isize, len: usize) -> usize {
    let mut idx = i;
    let l = len as isize;
    if l <= 1 {
        return 0;
    }
    while idx < 0 || idx >= l {
        if idx < 0 {
            idx = -idx;
        }
        if idx >= l {
            idx = 2 * l - 2 - idx;
        }
    }
    idx as usize
}

impl Stft {
    pub fn new(n_fft: usize, hop: usize) -> Self {
        let mut window = Vec::with_capacity(n_fft);
        let two_pi = 2.0 * std::f32::consts::PI;
        for n in 0..n_fft {
            let val = 0.5 - 0.5 * (two_pi * (n as f32) / (n_fft as f32)).cos();
            window.push(val);
        }
        Self { n_fft, hop, window }
    }

    pub fn n_bins(&self) -> usize {
        self.n_fft / 2 + 1
    }

    pub fn frames(&self, len: usize) -> usize {
        1 + len / self.hop
    }

    /// one channel -> Vec<Complex32> laid out [frame][bin], length frames*n_bins
    pub fn forward(&self, x: &[f32]) -> Vec<Complex32> {
        let len = x.len();
        let pad = self.n_fft / 2;
        let total_len = len + 2 * pad;
        let mut padded = Vec::with_capacity(total_len);
        for i in 0..total_len {
            let sig_idx = (i as isize) - (pad as isize);
            let idx = reflect_index(sig_idx, len);
            padded.push(x[idx]);
        }

        let num_frames = self.frames(len);
        let n_bins = self.n_bins();
        let mut spec = Vec::with_capacity(num_frames * n_bins);

        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(self.n_fft);
        let mut input_buf = r2c.make_input_vector();
        let mut output_buf = r2c.make_output_vector();

        for t in 0..num_frames {
            let start = t * self.hop;
            for n in 0..self.n_fft {
                input_buf[n] = padded[start + n] * self.window[n];
            }
            r2c.process(&mut input_buf, &mut output_buf).unwrap();
            spec.extend_from_slice(&output_buf);
        }

        spec
    }

    /// inverse of forward. Returns hop*(frames-1) samples.
    pub fn inverse(&self, spec: &[Complex32], frames: usize) -> Vec<f32> {
        let n_bins = self.n_bins();
        let out_len = (frames - 1) * self.hop + self.n_fft;
        let mut overlap_add = vec![0.0f32; out_len];
        let mut window_sum = vec![0.0f32; out_len];

        let mut planner = RealFftPlanner::<f32>::new();
        let c2r = planner.plan_fft_inverse(self.n_fft);
        let mut c_in = c2r.make_input_vector();
        let mut r_out = c2r.make_output_vector();

        let nyquist = self.n_fft / 2;

        for t in 0..frames {
            let spec_slice = &spec[t * n_bins .. (t + 1) * n_bins];
            c_in.copy_from_slice(spec_slice);

            c_in[0].im = 0.0;
            if self.n_fft % 2 == 0 {
                c_in[nyquist].im = 0.0;
            }

            c2r.process(&mut c_in, &mut r_out).unwrap();

            let start = t * self.hop;
            let scale = 1.0 / (self.n_fft as f32);
            for n in 0..self.n_fft {
                let val = (r_out[n] * scale) * self.window[n];
                overlap_add[start + n] += val;
                window_sum[start + n] += self.window[n] * self.window[n];
            }
        }

        for i in 0..out_len {
            if window_sum[i] >= 1e-8 {
                overlap_add[i] /= window_sum[i];
            }
        }

        let pad = self.n_fft / 2;
        let start_idx = pad;
        let end_idx = out_len - pad;
        overlap_add[start_idx..end_idx].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_trip() {
        let n_fft = 6144;
        let hop = 1024;
        let len = 261120;
        let stft = Stft::new(n_fft, hop);

        let mut state = 12345u32;
        let mut x = Vec::with_capacity(len);
        for _ in 0..len {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            let val = ((state >> 8) as f32 / 16777216.0) * 2.0 - 1.0;
            x.push(val);
        }

        let frames = stft.frames(len);
        let spec = stft.forward(&x);
        let rec = stft.inverse(&spec, frames);

        assert_eq!(rec.len(), len);

        let mut max_err = 0.0f32;
        for (a, b) in x.iter().zip(rec.iter()) {
            let err = (a - b).abs();
            if err > max_err {
                max_err = err;
            }
        }

        assert!(max_err < 1e-4, "max abs error {} >= 1e-4", max_err);
    }

    #[test]
    fn test_frames_count() {
        let stft = Stft::new(6144, 1024);
        assert_eq!(stft.frames(261120), 256);
    }

    #[test]
    fn test_sine_peak_bin() {
        let n_fft = 6144;
        let hop = 1024;
        let fs = 44100.0f32;
        let freq = 1000.0f32;
        let stft = Stft::new(n_fft, hop);

        let duration_samples = 44100 * 2;
        let mut x = Vec::with_capacity(duration_samples);
        let two_pi = 2.0 * std::f32::consts::PI;
        for i in 0..duration_samples {
            let t = i as f32 / fs;
            x.push((two_pi * freq * t).sin());
        }

        let spec = stft.forward(&x);
        let frames = stft.frames(x.len());
        let n_bins = stft.n_bins();

        let mut bin_magnitudes = vec![0.0f32; n_bins];
        for f in 0..frames {
            for b in 0..n_bins {
                let c = spec[f * n_bins + b];
                bin_magnitudes[b] += c.norm();
            }
        }

        let peak_bin = bin_magnitudes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;

        let expected_bin = ((freq * (n_fft as f32)) / fs).round() as usize;
        assert_eq!(peak_bin, expected_bin);
    }
}
