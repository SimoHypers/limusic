//! Decodes a cached FLAC back to PCM. Only tests and the `cache_test` example need it, so it
//! is compiled into the library for tests only and pulled into the example with `#[path]`;
//! that keeps symphonia a dev-dependency.

use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub fn read_flac(path: &Path) -> Result<(u32, [Vec<f32>; 2])> {
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
    let track =
        format.default_track().ok_or_else(|| anyhow!("No audio track found in FLAC cache"))?;
    let track_id = track.id;
    let sample_rate =
        track.codec_params.sample_rate.ok_or_else(|| anyhow!("Sample rate not found"))?;
    let n_frames = track.codec_params.n_frames.map(|n| n as usize);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("Failed to create FLAC decoder")?;

    let mut left = Vec::new();
    let mut right = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
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

    Ok((sample_rate, [left, right]))
}
