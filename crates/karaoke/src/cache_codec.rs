use std::fs::{self, File};
use std::path::Path;
use anyhow::{Context, Result, anyhow};
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheFormat {
    Flac,
    Opus,
}

impl CacheFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::Opus => "opus",
        }
    }
}

pub struct Pcm {
    pub sample_rate: u32,
    pub channels: [Vec<f32>; 2],
}

pub fn write_cached(path: &Path, pcm: &Pcm, format: CacheFormat) -> Result<()> {
    match format {
        CacheFormat::Opus => {
            return Err(anyhow!("Opus cache format is not available in this build"));
        }
        CacheFormat::Flac => {
            let channels = 2;
            let bits_per_sample = 16;
            let sample_rate = pcm.sample_rate as usize;
            let len = pcm.channels[0].len().min(pcm.channels[1].len());
            let mut interleaved = Vec::with_capacity(len * channels);
            for i in 0..len {
                let l = pcm.channels[0][i].clamp(-1.0, 1.0);
                let r = pcm.channels[1][i].clamp(-1.0, 1.0);
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
            fs::rename(&tmp_path, path).context("Failed to rename temporary cache file to target")?;
        }
    }
    Ok(())
}

pub fn read_cached(path: &Path) -> Result<Pcm> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ext != "flac" {
        return Err(anyhow!("Unsupported cache format extension: {}", ext));
    }

    let file = File::open(path).context("Failed to open cached FLAC file")?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("flac");

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("Failed to probe FLAC format")?;

    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| anyhow!("No audio track found in FLAC cache"))?;
    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.ok_or_else(|| anyhow!("Sample rate not found"))?;
    let n_frames = track.codec_params.n_frames.map(|n| n as usize);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("Failed to create FLAC decoder")?;

    let mut left = Vec::new();
    let mut right = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::ResetRequired) => continue,
            Err(e) => return Err(e.into()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let frames = decoded.frames() as u64;
                let mut sample_buf = SampleBuffer::<f32>::new(frames, spec);
                sample_buf.copy_interleaved_ref(decoded);
                let samples = sample_buf.samples();
                let channels = spec.channels.count();

                if channels == 1 {
                    for &s in samples {
                        left.push(s);
                        right.push(s);
                    }
                } else if channels >= 2 {
                    for chunk in samples.chunks(channels) {
                        left.push(chunk[0]);
                        right.push(chunk[1]);
                    }
                }
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }

    if let Some(n) = n_frames {
        if left.len() > n {
            left.truncate(n);
        }
        if right.len() > n {
            right.truncate(n);
        }
    }

    Ok(Pcm { sample_rate, channels: [left, right] })
}

#[cfg(test)]
mod tests {
    use super::*;
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

        let pcm_in = Pcm {
            sample_rate,
            channels: [left, right],
        };

        let tmp_dir = std::env::temp_dir();
        let flac_path = tmp_dir.join("limusic_test_cache.flac");
        let tmp_path = tmp_dir.join("limusic_test_cache.flac.tmp");

        if flac_path.exists() {
            let _ = fs::remove_file(&flac_path);
        }
        if tmp_path.exists() {
            let _ = fs::remove_file(&tmp_path);
        }

        write_cached(&flac_path, &pcm_in, CacheFormat::Flac)?;

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
    fn test_cache_format_opus() {
        let tmp_dir = std::env::temp_dir();
        let dummy_path = tmp_dir.join("limusic_test_cache.opus");
        let pcm = Pcm {
            sample_rate: 44100,
            channels: [vec![0.0; 100], vec![0.0; 100]],
        };
        let res = write_cached(&dummy_path, &pcm, CacheFormat::Opus);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Opus cache format is not available"));
    }
}
