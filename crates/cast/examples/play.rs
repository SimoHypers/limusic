use cast::{CastEvent, CastSession, LoadRequest};
use std::env;
use std::io::{self, Write};
use std::net::IpAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: cargo run --example play -- <ip> <media_url>");
        std::process::exit(1);
    }

    let ip: IpAddr = args[1].parse()?;
    let media_url = args[2].clone();

    println!("Connecting to Chromecast at {}...", ip);
    let session = CastSession::connect(ip, 8009)?;
    let mut events = session.events();

    // Spawn task to print events from the session
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                CastEvent::Status {
                    state,
                    position_secs,
                    duration_secs,
                    idle_reason,
                    volume,
                } => {
                    println!(
                        "[Status] State: {:?}, Position: {:.1}s, Duration: {:?}, IdleReason: {:?}, Volume: {:.0}%",
                        state,
                        position_secs,
                        duration_secs,
                        idle_reason,
                        volume * 100.0
                    );
                }
                CastEvent::Disconnected(reason) => {
                    println!("[Disconnected] {}", reason);
                    break;
                }
            }
        }
    });

    println!("Loading media: {}...", media_url);
    session.load(LoadRequest {
        url: media_url,
        content_type: "audio/mp3".to_string(),
        title: Some("Limusic Track".to_string()),
        artist: Some("Limusic".to_string()),
        image_url: None,
        start_secs: 0.0,
        autoplay: true,
    })?;

    println!("Commands:");
    println!("  p         - Pause");
    println!("  r         - Resume (Play)");
    println!("  s <secs>  - Seek to seconds");
    println!("  v <0-100> - Set volume (0-100)");
    println!("  q         - Quit");

    let stdin = io::stdin();
    let mut line = String::new();

    loop {
        line.clear();
        print!("> ");
        io::stdout().flush()?;
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        match parts[0] {
            "p" => {
                session.pause()?;
                println!("Paused");
            }
            "r" => {
                session.play()?;
                println!("Resumed");
            }
            "s" => {
                if parts.len() > 1 {
                    if let Ok(secs) = parts[1].parse::<f64>() {
                        session.seek(secs)?;
                        println!("Seeking to {:.1}s...", secs);
                    } else {
                        println!("Invalid seconds.");
                    }
                } else {
                    println!("Usage: s <secs>");
                }
            }
            "v" => {
                if parts.len() > 1 {
                    if let Ok(vol) = parts[1].parse::<f32>() {
                        let level = (vol / 100.0).clamp(0.0, 1.0);
                        session.set_volume(level)?;
                        println!("Volume set to {:.0}%", vol);
                    } else {
                        println!("Invalid volume.");
                    }
                } else {
                    println!("Usage: v <0-100>");
                }
            }
            "q" => {
                session.stop()?;
                session.quit()?;
                println!("Quitting...");
                break;
            }
            _ => {
                println!("Unknown command. Use: p, r, s <secs>, v <0-100>, q");
            }
        }
    }

    Ok(())
}
