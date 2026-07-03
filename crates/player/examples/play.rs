//! Audible gapless check: `cargo run -p player --example play -- <fileA> <fileB>`
//! Defaults to the Phase-0 spike tones. Plays A then B gaplessly and prints events.

use std::collections::HashMap;

use player::{Player, PlayerEvent};

#[tokio::main]
async fn main() {
    let a = std::env::args().nth(1).unwrap_or_else(|| "spikes/tone_a.opus".into());
    let b = std::env::args().nth(2).unwrap_or_else(|| "spikes/tone_b.opus".into());

    let cache = std::env::temp_dir().join("limusic-player-example");
    std::fs::create_dir_all(&cache).ok();

    let mut p = Player::new(cache.to_str().unwrap()).expect("player");
    let mut events = p.take_events().unwrap();

    p.load(&a, &HashMap::new(), None).expect("load A");
    p.enqueue(&b).expect("enqueue B");
    p.play().expect("play");

    let mut ended = 0;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_secs(2), events.recv()).await {
            Ok(Some(PlayerEvent::TrackEnded)) => {
                ended += 1;
                println!("track ended ({ended}/2)");
                if ended >= 2 {
                    println!("OK: both tracks played gaplessly");
                    return;
                }
            }
            Ok(Some(ev)) => println!("event: {ev:?}"),
            Ok(None) => break,
            Err(_) => {}
        }
    }
    eprintln!("did not observe 2 track endings");
    std::process::exit(1);
}
