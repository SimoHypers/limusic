use anyhow::{Context, Result};
use karaoke::cache_codec::{write_cached, CacheFormat, Pcm};
use std::env;
use std::path::PathBuf;
use std::time::Instant;

#[path = "../src/flac_read.rs"]
mod flac_read;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "Usage: cargo run --release -p karaoke --example cache_test -- <in.wav> <out.flac>"
        );
        std::process::exit(1);
    }

    let in_wav_path = &args[1];
    let out_flac_path = PathBuf::from(&args[2]);

    println!("Reading input WAV: {}", in_wav_path);
    let mut reader = hound::WavReader::open(in_wav_path).context("Failed to open input WAV")?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as usize;

    let mut left = Vec::new();
    let mut right = Vec::new();

    match spec.sample_format {
        hound::SampleFormat::Float => {
            let samples: Result<Vec<f32>, _> = reader.samples::<f32>().collect();
            let samples = samples?;
            if channels == 1 {
                for &s in &samples {
                    left.push(s);
                    right.push(s);
                }
            } else {
                for chunk in samples.chunks(channels) {
                    left.push(chunk[0]);
                    right.push(chunk.get(1).copied().unwrap_or(chunk[0]));
                }
            }
        }
        hound::SampleFormat::Int => {
            let max_val = ((1i64 << (spec.bits_per_sample - 1)) - 1).max(1) as f32;
            let samples: Result<Vec<i32>, _> = reader.samples::<i32>().collect();
            let samples = samples?;
            if channels == 1 {
                for &s in &samples {
                    let f = (s as f32) / max_val;
                    left.push(f);
                    right.push(f);
                }
            } else {
                for chunk in samples.chunks(channels) {
                    let l = (chunk[0] as f32) / max_val;
                    let r = (chunk.get(1).copied().unwrap_or(chunk[0]) as f32) / max_val;
                    left.push(l);
                    right.push(r);
                }
            }
        }
    }

    let pcm = Pcm { sample_rate, channels: [left, right] };

    let input_size =
        std::fs::metadata(in_wav_path).context("Failed to get input WAV metadata")?.len();

    println!("Encoding to FLAC: {}", out_flac_path.display());
    let encode_start = Instant::now();
    write_cached(&out_flac_path, &pcm, CacheFormat::Flac)?;
    let encode_time = encode_start.elapsed();

    let output_size =
        std::fs::metadata(&out_flac_path).context("Failed to get output FLAC metadata")?.len();
    let ratio = output_size as f64 / input_size as f64;

    println!("Input size: {} bytes", input_size);
    println!("Output size: {} bytes", output_size);
    println!("Compression ratio: {:.4} ({:.2}%)", ratio, ratio * 100.0);
    println!("Encode time: {:.2?}", encode_time);

    println!("Decoding cached FLAC: {}", out_flac_path.display());
    let decode_start = Instant::now();
    let (sample_rate, channels) = flac_read::read_flac(&out_flac_path)?;
    let pcm_decoded = Pcm { sample_rate, channels };
    let decode_time = decode_start.elapsed();

    println!("Decode time: {:.2?}", decode_time);

    let len = pcm.channels[0].len().min(pcm_decoded.channels[0].len());
    let mut max_diff = 0.0f32;
    for i in 0..len {
        let diff_l = (pcm.channels[0][i] - pcm_decoded.channels[0][i]).abs();
        let diff_r = (pcm.channels[1][i] - pcm_decoded.channels[1][i]).abs();
        if diff_l > max_diff {
            max_diff = diff_l;
        }
        if diff_r > max_diff {
            max_diff = diff_r;
        }
    }

    println!("Max absolute difference vs input: {:.6}", max_diff);

    Ok(())
}
