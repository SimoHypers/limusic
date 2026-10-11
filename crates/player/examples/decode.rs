//! Decode any audio file or URL to WAV:
//! `cargo run --release -p player --example decode -- <input path or URL> <out.wav>`

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().unwrap_or_else(|| {
        eprintln!("Usage: cargo run --release -p player --example decode -- <input> <out.wav>");
        std::process::exit(1);
    });
    let out_wav = PathBuf::from(args.next().unwrap_or_else(|| "output.wav".into()));

    println!("Decoding {} -> {} ...", input, out_wav.display());
    let cancel = AtomicBool::new(false);
    let start = Instant::now();

    match player::decode_to_wav(&input, &out_wav, &cancel) {
        Ok(()) => {
            let elapsed = start.elapsed();
            let metadata = std::fs::metadata(&out_wav).unwrap();
            println!("Successfully decoded in {:.2?}", elapsed);
            println!("Output file size: {} bytes", metadata.len());

            if let Ok(reader) = hound::WavReader::open(&out_wav) {
                let spec = reader.spec();
                let duration = reader.duration() as f64 / spec.sample_rate as f64;
                println!("Sample rate: {} Hz", spec.sample_rate);
                println!("Channels: {}", spec.channels);
                println!("Sample format: {:?}", spec.sample_format);
                println!("Bits per sample: {}", spec.bits_per_sample);
                println!("Duration: {:.2} seconds", duration);
            }
        }
        Err(e) => {
            eprintln!("Decode failed: {e}");
            std::process::exit(1);
        }
    }
}
