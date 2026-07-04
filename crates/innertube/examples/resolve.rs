//! Debug tool: print the stream URL the fallback chain resolves for a videoId.
//!   cargo run -p innertube --example resolve -- <videoId> [--rustypipe]
//! Prints the URL on stdout (pipe into `cargo run -p player --example play`).

use innertube::{find_format, AudioQuality, Clients, InnerTube, Session, STREAM_FALLBACK_ORDER};

#[tokio::main]
async fn main() {
    let video_id = std::env::args().nth(1).expect("usage: resolve <videoId> [--rustypipe]");
    let force_rustypipe = std::env::args().any(|a| a == "--rustypipe");

    if !force_rustypipe {
        let it = InnerTube::new(Session::default(), None).unwrap();
        let vd = it.fetch_visitor_data().await.ok();
        let it = InnerTube::new(Session { visitor_data: vd, ..Session::default() }, None).unwrap();
        let clients = Clients::bundled();
        for key in STREAM_FALLBACK_ORDER {
            let client = clients.get(key).unwrap();
            let Ok(resp) = it.player(client, &video_id, None, None, None).await else { continue };
            if !resp.playability_status.is_ok() {
                continue;
            }
            let Some(sd) = resp.streaming_data.as_ref() else { continue };
            let Some(url) = find_format(sd, AudioQuality::High).and_then(|f| f.direct_url()) else {
                continue;
            };
            eprintln!("resolved via {key}");
            println!("{url}");
            return;
        }
        eprintln!("direct clients failed → rustypipe");
    }
    let c = innertube::rustypipe_fallback::resolve(&video_id, true).await.expect("rustypipe");
    eprintln!("resolved via rustypipe (itag {})", c.itag);
    println!("{}", c.url);
}
