use anyhow::{Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};
use karaoke::mdx::{MdxModel, MdxParams};
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use std::env;
use std::fs::File;
use std::path::Path;
use std::time::Instant;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 4 {
        eprintln!(
            "Usage: cargo run -p karaoke --example separate -- <input_audio> <model.onnx> <output.wav> [--overlap <f32>] [--denoise]"
        );
        std::process::exit(1);
    }

    let input_path = &args[1];
    let model_path = &args[2];
    let output_path = &args[3];

    let mut overlap = 0.25f32;
    let mut denoise = false;

    let mut i = 4;
    while i < args.len() {
        match args[i].as_str() {
            "--overlap" => {
                if i + 1 < args.len() {
                    overlap = args[i + 1].parse().context("Invalid overlap value")?;
                    i += 2;
                } else {
                    anyhow::bail!("--overlap requires a value");
                }
            }
            "--denoise" => {
                denoise = true;
                i += 1;
            }
            other => {
                anyhow::bail!("Unknown argument: {}", other);
            }
        }
    }

    // Decode input audio file using symphonia (supports mp3, flac, wav, aac/m4a, ogg)
    eprintln!("Decoding audio: {}", input_path);
    let file = File::open(input_path).context("Failed to open input audio file")?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = Path::new(input_path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("Failed to probe audio format")?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow::anyhow!("No audio track found"))?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| anyhow::anyhow!("Sample rate not found in audio track"))?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("Failed to create audio decoder")?;

    let mut channels_l = Vec::new();
    let mut channels_r = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
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
                        channels_l.push(s);
                        channels_r.push(s);
                    }
                } else if channels >= 2 {
                    for chunk in samples.chunks(channels) {
                        channels_l.push(chunk[0]);
                        channels_r.push(chunk[1]);
                    }
                } else {
                    anyhow::bail!("Unsupported channel count: {}", channels);
                }
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }

    let original_duration = channels_l.len() as f64 / sample_rate as f64;
    eprintln!(
        "Decoded {} Hz, stereo, duration: {:.2}s",
        sample_rate, original_duration
    );

    // Resample to 44100 Hz with rubato if sample rate differs
    let (resampled_l, resampled_r) = if sample_rate != 44100 {
        eprintln!("Resampling from {} Hz to 44100 Hz with rubato...", sample_rate);
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Cubic,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris,
        };
        let mut resampler = SincFixedIn::<f32>::new(
            44100.0 / sample_rate as f64,
            2.0,
            params,
            1024,
            2,
        )
        .context("Failed to create resampler")?;

        let chunk_size = resampler.input_frames_next();
        let mut out_l = Vec::new();
        let mut out_r = Vec::new();

        let mut pos = 0;
        let total_in = channels_l.len();
        while pos < total_in {
            let end = std::cmp::min(pos + chunk_size, total_in);
            let mut chunk_l = channels_l[pos..end].to_vec();
            let mut chunk_r = channels_r[pos..end].to_vec();
            if chunk_l.len() < chunk_size {
                chunk_l.resize(chunk_size, 0.0);
                chunk_r.resize(chunk_size, 0.0);
            }
            let waves_in = vec![chunk_l, chunk_r];
            let waves_out = resampler.process(&waves_in, None)?;
            out_l.extend(&waves_out[0]);
            out_r.extend(&waves_out[1]);
            pos += chunk_size;
        }

        let target_len = (total_in as f64 * 44100.0 / sample_rate as f64) as usize;
        if out_l.len() > target_len {
            out_l.truncate(target_len);
            out_r.truncate(target_len);
        } else if out_l.len() < target_len {
            out_l.resize(target_len, 0.0);
            out_r.resize(target_len, 0.0);
        }

        (out_l, out_r)
    } else {
        (channels_l, channels_r)
    };

    let audio_duration = resampled_l.len() as f64 / 44100.0;

    // Load MDX model with default parameters
    eprintln!("Loading MDX model from: {}", model_path);
    let mut model = MdxModel::load(Path::new(model_path), MdxParams::default())
        .context("Failed to load MDX model")?;

    // Measure wall-clock time of separation
    eprintln!("Running separation...");
    let start_time = Instant::now();

    let separated = model
        .separate(
            [&resampled_l, &resampled_r],
            overlap,
            denoise,
            &mut |curr, total| {
                eprintln!("Progress: chunk {} / {}", curr, total);
            },
        )
        .context("Separation failed")?;

    let processing_time = start_time.elapsed().as_secs_f64();
    let real_time_factor = processing_time / audio_duration;

    // Print timing metrics
    println!("Audio duration: {:.2} s", audio_duration);
    println!("Processing time: {:.2} s", processing_time);
    println!("Real-time factor: {:.4}", real_time_factor);

    // Write output as 32-bit float stereo 44100 Hz WAV with hound
    eprintln!("Writing output WAV to: {}", output_path);
    let spec = WavSpec {
        channels: 2,
        sample_rate: 44100,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(output_path, spec).context("Failed to create WAV writer")?;

    let [sep_l, sep_r] = separated;
    for (l, r) in sep_l.into_iter().zip(sep_r.into_iter()) {
        writer.write_sample(l)?;
        writer.write_sample(r)?;
    }
    writer.finalize().context("Failed to finalize WAV writer")?;

    eprintln!("Done!");
    Ok(())
}
