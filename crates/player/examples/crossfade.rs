//! Audible manual-crossfade check: `cargo run -p player --example crossfade`
//!
//! Plays a low tone, skips to a high one the way Next does, and reports the overlap. `--no-manual`
//! is the same skip with the setting off, which should cut. Needs libmpv on the linker's path.

use std::collections::HashMap;
use std::time::Duration;

use player::{Player, PlayerEvent};

/// Long enough that the skip lands well inside the first tone.
const RATE: usize = 44_100;
const TONE_SECS: f64 = 12.0;

fn wav(path: &std::path::Path, hz: f64) {
    let frames = (TONE_SECS * RATE as f64).round() as u32;
    let bytes = frames * 4;
    let mut data = Vec::with_capacity(bytes as usize + 44);
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(bytes + 36).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x02\0");
    data.extend_from_slice(&(RATE as u32).to_le_bytes());
    data.extend_from_slice(&(RATE as u32 * 4).to_le_bytes());
    data.extend_from_slice(b"\x04\0\x10\0data");
    data.extend_from_slice(&bytes.to_le_bytes());
    for frame in 0..frames {
        let t = frame as f64 / RATE as f64;
        // Ramped over 50 ms at each end, so a hard cut is audible as a hard cut.
        let edge = (t / 0.05).min((TONE_SECS - t) / 0.05).clamp(0.0, 1.0);
        let value = (std::f64::consts::TAU * hz * t).sin() * 0.25 * edge * i16::MAX as f64;
        data.extend_from_slice(&(value as i16).to_le_bytes());
        data.extend_from_slice(&(value as i16).to_le_bytes());
    }
    std::fs::write(path, data).expect("write wav");
}

#[tokio::main]
async fn main() {
    let manual = !std::env::args().skip(1).any(|a| a == "--no-manual");

    let dir = std::env::temp_dir().join("limusic-crossfade-example");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let first = dir.join("outgoing.wav");
    let second = dir.join("incoming.wav");
    wav(&first, 220.0);
    wav(&second, 660.0);

    let mut p = Player::new(dir.to_str().unwrap()).expect("player");
    let mut events = p.take_events().unwrap();
    p.set_volume(70).expect("volume");
    p.set_crossfade(Some(3.0));
    p.set_manual_crossfade(manual);
    println!(
        "crossfade 3 s, manual crossfade {}",
        if manual { "ON (the tones should overlap)" } else { "OFF (a hard cut)" }
    );

    p.load(first.to_str().unwrap(), &HashMap::new(), None, None).expect("load A");
    p.play().expect("play");

    // Let the first tone establish itself, then do exactly what Next does: load the new track.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    println!("--- skip ---");
    p.load(second.to_str().unwrap(), &HashMap::new(), None, None).expect("load B");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut overlap_ms = 0u64;
    let mut ended = 0;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), events.recv()).await {
            Ok(Some(PlayerEvent::TrackEnded)) => {
                ended += 1;
                println!("TrackEnded — the app's queue would advance here");
            }
            Ok(Some(PlayerEvent::TrackFailed(e))) => {
                eprintln!("track failed: {e}");
                std::process::exit(1);
            }
            Ok(None) => break,
            _ => {}
        }
        if p.is_crossfading() {
            overlap_ms += 50;
        } else if overlap_ms > 0 {
            break; // the fade finished
        }
    }
    println!("\noverlap: ~{overlap_ms} ms");
    let ok = if manual { overlap_ms >= 2000 && ended == 0 } else { overlap_ms == 0 };
    println!(
        "{}",
        if ok {
            if manual {
                "OK: the skip overlapped, and no TrackEnded was sent for it"
            } else {
                "OK: no overlap with the setting off (hard cut, as before)"
            }
        } else {
            "UNEXPECTED: what happened does not match the setting"
        }
    );
    p.stop().ok();
    if !ok {
        std::process::exit(1);
    }
}
