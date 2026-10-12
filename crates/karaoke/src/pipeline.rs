use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use anyhow::{Context, Result};
use crate::cache::InstrumentalCache;
use crate::cache_codec::{Pcm, write_cached};
use crate::mdx::MdxModel;

pub trait Separator {
    fn separate_pcm(
        &mut self,
        mix: [&[f32]; 2],
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<[Vec<f32>; 2]>;
}

impl Separator for MdxModel {
    fn separate_pcm(
        &mut self,
        mix: [&[f32]; 2],
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<[Vec<f32>; 2]> {
        self.separate_cancellable(mix, 0.25, false, cancel, progress)
    }
}

struct TmpWavGuard(PathBuf);
impl Drop for TmpWavGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn prepare_instrumental(
    cache: &InstrumentalCache,
    track_id: &str,
    separator: &mut dyn Separator,
    decode_to_wav: &mut dyn FnMut(&Path) -> Result<()>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PathBuf> {
    if let Some(cached_path) = cache.get(track_id) {
        tracing::info!(target: "karaoke::pipeline", video_id = %track_id, "karaoke cache hit video_id={}", track_id);
        return Ok(cached_path);
    }

    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        anyhow::bail!("cancelled");
    }

    let start_total = std::time::Instant::now();

    let wav_path = cache.tmp_wav_path(track_id);
    if let Some(p) = wav_path.parent() {
        fs::create_dir_all(p)?;
    }
    let _guard = TmpWavGuard(wav_path.clone());

    let start_decode = std::time::Instant::now();
    decode_to_wav(&wav_path)?;
    let decode_ms = start_decode.elapsed().as_millis();
    tracing::debug!(target: "karaoke::pipeline", video_id = %track_id, decode_ms, "karaoke stage decode decode_ms={}", decode_ms);

    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        anyhow::bail!("cancelled");
    }

    let mut reader = hound::WavReader::open(&wav_path).context("Failed to open temporary WAV file")?;
    let spec = reader.spec();
    if spec.sample_rate != 44100 {
        anyhow::bail!("Required sample rate 44100 Hz, got {} Hz", spec.sample_rate);
    }
    let channels = spec.channels;
    if channels < 1 {
        anyhow::bail!("Invalid channel count: {}", channels);
    }

    let mut left = Vec::new();
    let mut right = Vec::new();

    match spec.sample_format {
        hound::SampleFormat::Float => {
            let samples: Result<Vec<f32>, _> = reader.samples::<f32>().collect();
            let samples = samples.context("Failed to read float samples from WAV")?;
            if channels == 1 {
                for &s in &samples {
                    left.push(s);
                    right.push(s);
                }
            } else {
                for chunk in samples.chunks(channels as usize) {
                    left.push(chunk[0]);
                    right.push(chunk[1]);
                }
            }
        }
        hound::SampleFormat::Int => {
            let max_val = (1u32 << (spec.bits_per_sample - 1)) as f32;
            let samples: Result<Vec<i32>, _> = reader.samples::<i32>().collect();
            let samples = samples.context("Failed to read integer samples from WAV")?;
            if channels == 1 {
                for &s in &samples {
                    let f = s as f32 / max_val;
                    left.push(f);
                    right.push(f);
                }
            } else {
                for chunk in samples.chunks(channels as usize) {
                    let l = chunk[0] as f32 / max_val;
                    let r = chunk[1] as f32 / max_val;
                    left.push(l);
                    right.push(r);
                }
            }
        }
    }

    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        anyhow::bail!("cancelled");
    }

    let audio_seconds = left.len() as f64 / 44100.0;

    let start_separate = std::time::Instant::now();
    let [sep_l, sep_r] = separator.separate_pcm([&left, &right], cancel, progress)?;
    let separate_ms = start_separate.elapsed().as_millis();
    tracing::debug!(target: "karaoke::pipeline", video_id = %track_id, separate_ms, "karaoke stage separate separate_ms={}", separate_ms);

    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        anyhow::bail!("cancelled");
    }

    let start_encode = std::time::Instant::now();
    let result_path = cache.path_for(track_id);
    let write_res = write_cached(
        &result_path,
        &Pcm {
            sample_rate: 44100,
            channels: [sep_l, sep_r],
        },
        cache.format(),
    );
    let encode_ms = start_encode.elapsed().as_millis();
    tracing::debug!(target: "karaoke::pipeline", video_id = %track_id, encode_ms, "karaoke stage encode encode_ms={}", encode_ms);

    if let Err(e) = write_res {
        let tmp_res_path = match result_path.file_name() {
            Some(name) => {
                let mut t = name.to_os_string();
                t.push(".tmp");
                result_path.with_file_name(t)
            }
            None => result_path.with_extension("tmp"),
        };
        let _ = fs::remove_file(&tmp_res_path);
        let _ = fs::remove_file(&result_path);
        return Err(e);
    }

    let _ = cache.evict(Some(&result_path));

    let total_ms = start_total.elapsed().as_millis();
    let rtf = if audio_seconds > 0.0 { (total_ms as f64 / 1000.0) / audio_seconds } else { 0.0 };
    let bytes = std::fs::metadata(&result_path).map(|m| m.len()).unwrap_or(0);

    tracing::info!(
        target: "karaoke::pipeline",
        video_id = %track_id,
        decode_ms,
        separate_ms,
        encode_ms,
        total_ms,
        audio_seconds,
        rtf,
        bytes,
        "karaoke prepare done video_id={0} decode_ms={1} separate_ms={2} encode_ms={3} total_ms={4} audio_seconds={5:.2} rtf={6:.3} bytes={7}",
        track_id, decode_ms, separate_ms, encode_ms, total_ms, audio_seconds, rtf, bytes
    );

    Ok(result_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::InstrumentalCache;
    use crate::cache_codec::CacheFormat;
    use hound::{WavSpec, WavWriter, SampleFormat};
    use std::cell::Cell;

    struct FakeSeparator {
        calls: usize,
    }

    impl Separator for FakeSeparator {
        fn separate_pcm(
            &mut self,
            mix: [&[f32]; 2],
            _cancel: &AtomicBool,
            progress: &mut dyn FnMut(usize, usize),
        ) -> Result<[Vec<f32>; 2]> {
            self.calls += 1;
            progress(1, 1);
            Ok([mix[0].to_vec(), mix[1].to_vec()])
        }
    }

    #[test]
    fn test_pipeline_workflow() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_pipeline_test");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), CacheFormat::Flac, 1024 * 1024)?;
        let track_id = "test_track_1";
        let cancel = AtomicBool::new(false);
        let mut separator = FakeSeparator { calls: 0 };
        let decode_calls = Cell::new(0);

        let mut decode_to_wav = |wav_path: &Path| -> Result<()> {
            decode_calls.set(decode_calls.get() + 1);
            let spec = WavSpec {
                channels: 2,
                sample_rate: 44100,
                bits_per_sample: 32,
                sample_format: SampleFormat::Float,
            };
            let mut writer = WavWriter::create(wav_path, spec)?;
            for i in 0..44100 {
                let t = i as f32 / 44100.0;
                writer.write_sample(t.sin())?;
                writer.write_sample(-t.sin())?;
            }
            writer.finalize()?;
            Ok(())
        };

        let mut progress_calls = 0;
        let mut progress = |_, _| {
            progress_calls += 1;
        };

        let p1 = prepare_instrumental(&cache, track_id, &mut separator, &mut decode_to_wav, &cancel, &mut progress)?;
        assert!(p1.exists());
        assert_eq!(decode_calls.get(), 1);
        assert_eq!(separator.calls, 1);
        assert!(!cache.tmp_wav_path(track_id).exists());

        let p2 = prepare_instrumental(&cache, track_id, &mut separator, &mut decode_to_wav, &cancel, &mut progress)?;
        assert_eq!(p1, p2);
        assert_eq!(decode_calls.get(), 1);
        assert_eq!(separator.calls, 1);

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn test_pipeline_cancellation() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_pipeline_cancel");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), CacheFormat::Flac, 1024 * 1024)?;
        let track_id = "test_cancel";
        let cancel = AtomicBool::new(true);
        let mut separator = FakeSeparator { calls: 0 };
        let mut decode_to_wav = |_wav_path: &Path| -> Result<()> {
            Ok(())
        };
        let mut progress = |_, _| {};

        let res = prepare_instrumental(&cache, track_id, &mut separator, &mut decode_to_wav, &cancel, &mut progress);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("cancelled"));

        assert!(!cache.path_for(track_id).exists());
        let tmp_dir = tmp.join("tmp");
        if tmp_dir.exists() {
            let entries: Vec<_> = fs::read_dir(&tmp_dir)?.flatten().collect();
            assert!(entries.is_empty(), "tmp dir must be empty after cancellation");
        }

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn test_pipeline_decode_error() -> Result<()> {
        let tmp = std::env::temp_dir().join("limusic_pipeline_err");
        if tmp.exists() {
            let _ = fs::remove_dir_all(&tmp);
        }

        let cache = InstrumentalCache::new(tmp.clone(), CacheFormat::Flac, 1024 * 1024)?;
        let track_id = "test_err";
        let cancel = AtomicBool::new(false);
        let mut separator = FakeSeparator { calls: 0 };
        let mut decode_to_wav = |_wav_path: &Path| -> Result<()> {
            anyhow::bail!("decode error");
        };
        let mut progress = |_, _| {};

        let res = prepare_instrumental(&cache, track_id, &mut separator, &mut decode_to_wav, &cancel, &mut progress);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("decode error"));

        assert!(!cache.path_for(track_id).exists());
        let tmp_dir = tmp.join("tmp");
        if tmp_dir.exists() {
            let entries: Vec<_> = fs::read_dir(&tmp_dir)?.flatten().collect();
            assert!(entries.is_empty(), "tmp dir must be empty after error");
        }

        let _ = fs::remove_dir_all(&tmp);
        Ok(())
    }
}
