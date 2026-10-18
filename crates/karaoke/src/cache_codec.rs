use anyhow::{anyhow, Context, Result};
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use std::fs;
use std::path::Path;

pub const CACHE_EXT: &str = "flac";

pub struct Pcm {
    pub sample_rate: u32,
    pub channels: [Vec<f32>; 2],
}

pub fn write_cached(path: &Path, pcm: &Pcm) -> Result<()> {
    let channels = 2;
    let bits_per_sample = 16;
    let sample_rate = pcm.sample_rate as usize;
    let len = pcm.channels[0].len().min(pcm.channels[1].len());

    let mut peak = 0.0f32;
    for i in 0..len {
        let l = pcm.channels[0][i].abs();
        if l > peak {
            peak = l;
        }
        let r = pcm.channels[1][i].abs();
        if r > peak {
            peak = r;
        }
    }
    let scale = if peak > 1.0 { 1.0 / peak } else { 1.0 };

    let mut interleaved = Vec::with_capacity(len * channels);
    for i in 0..len {
        let l = (pcm.channels[0][i] * scale).clamp(-1.0, 1.0);
        let r = (pcm.channels[1][i] * scale).clamp(-1.0, 1.0);
        interleaved.push((l * 32767.0).round() as i32);
        interleaved.push((r * 32767.0).round() as i32);
    }

    let source = flacenc::source::MemSource::from_samples(
        &interleaved,
        channels,
        bits_per_sample,
        sample_rate,
    );
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| anyhow!("flacenc config error: {:?}", e))?;
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| anyhow!("flacenc encode error: {:?}", e))?;

    let mut bv = flacenc::bitsink::ByteSink::with_capacity(stream.count_bits());
    stream.write(&mut bv).map_err(|e| anyhow!("flacenc write error: {:?}", e))?;

    if let Some(p) = path.parent() {
        if !p.as_os_str().is_empty() {
            fs::create_dir_all(p)?;
        }
    }

    let tmp_path = match path.file_name() {
        Some(name) => {
            let mut tmp_name = name.to_os_string();
            tmp_name.push(".tmp");
            path.with_file_name(tmp_name)
        }
        None => path.with_extension("tmp"),
    };

    fs::write(&tmp_path, bv.as_slice()).context("Failed to write temporary cache file")?;
    fs::rename(&tmp_path, path)
        .context("Failed to rename temporary cache file to target")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_cached(path: &Path) -> Result<Pcm> {
        let (sample_rate, channels) = crate::flac_read::read_flac(path)?;
        Ok(Pcm { sample_rate, channels })
    }
    use std::f32::consts::PI;

    #[test]
    fn test_cache_codec_roundtrip() -> Result<()> {
        let sample_rate = 44100;
        let duration_secs = 2.0;
        let num_samples = (sample_rate as f32 * duration_secs) as usize;

        let mut left = Vec::with_capacity(num_samples);
        let mut right = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            left.push(0.5 * (2.0 * PI * 440.0 * t).sin());
            right.push(0.5 * (2.0 * PI * 660.0 * t).sin());
        }

        let pcm_in = Pcm { sample_rate, channels: [left, right] };

        let tmp_dir = std::env::temp_dir();
        let flac_path = tmp_dir.join("limusic_test_cache.flac");
        let tmp_path = tmp_dir.join("limusic_test_cache.flac.tmp");

        if flac_path.exists() {
            let _ = fs::remove_file(&flac_path);
        }
        if tmp_path.exists() {
            let _ = fs::remove_file(&tmp_path);
        }

        write_cached(&flac_path, &pcm_in)?;

        assert!(!tmp_path.exists(), "Temporary file should not remain after successful write");
        assert!(flac_path.exists(), "FLAC cache file should exist");

        let pcm_out = read_cached(&flac_path)?;

        assert_eq!(pcm_out.sample_rate, pcm_in.sample_rate);
        assert_eq!(pcm_out.channels[0].len(), pcm_in.channels[0].len());
        assert_eq!(pcm_out.channels[1].len(), pcm_in.channels[1].len());

        let mut max_err_l = 0.0f32;
        let mut max_err_r = 0.0f32;

        for i in 0..num_samples {
            let err_l = (pcm_out.channels[0][i] - pcm_in.channels[0][i]).abs();
            let err_r = (pcm_out.channels[1][i] - pcm_in.channels[1][i]).abs();
            if err_l > max_err_l {
                max_err_l = err_l;
            }
            if err_r > max_err_r {
                max_err_r = err_r;
            }
        }

        assert!(max_err_l < 1.0 / 16384.0, "Left max error {} exceeds threshold", max_err_l);
        assert!(max_err_r < 1.0 / 16384.0, "Right max error {} exceeds threshold", max_err_r);

        let _ = fs::remove_file(&flac_path);
        Ok(())
    }

    #[test]
    fn test_cache_codec_peak_normalization() -> Result<()> {
        let sample_rate = 44100;
        let num_samples = 1000;
        let mut left = Vec::with_capacity(num_samples);
        let mut right = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            left.push(1.2 * (2.0 * PI * 440.0 * t).sin());
            right.push(-1.2 * (2.0 * PI * 660.0 * t).sin());
        }

        let pcm_in = Pcm { sample_rate, channels: [left, right] };

        let tmp_dir = std::env::temp_dir();
        let flac_path = tmp_dir.join("limusic_peak_test.flac");
        if flac_path.exists() {
            let _ = fs::remove_file(&flac_path);
        }

        write_cached(&flac_path, &pcm_in)?;
        let pcm_out = read_cached(&flac_path)?;

        let mut max_err_l = 0.0f32;
        let mut max_err_r = 0.0f32;

        for i in 0..num_samples {
            let expected_l = pcm_in.channels[0][i] / 1.2;
            let expected_r = pcm_in.channels[1][i] / 1.2;
            let err_l = (pcm_out.channels[0][i] - expected_l).abs();
            let err_r = (pcm_out.channels[1][i] - expected_r).abs();
            if err_l > max_err_l {
                max_err_l = err_l;
            }
            if err_r > max_err_r {
                max_err_r = err_r;
            }
        }

        assert!(
            max_err_l < 1.0 / 16384.0,
            "Left peak norm max error {} exceeds threshold",
            max_err_l
        );
        assert!(
            max_err_r < 1.0 / 16384.0,
            "Right peak norm max error {} exceeds threshold",
            max_err_r
        );

        let _ = fs::remove_file(&flac_path);
        Ok(())
    }
}
