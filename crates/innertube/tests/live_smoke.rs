//! Live-YouTube extraction smoke test (context/17). NOT in the default run:
//!   cargo test -p innertube --features integration-tests -- --nocapture
#![cfg(feature = "integration-tests")]

use innertube::{find_format, AudioQuality, Clients, InnerTube, Session, STREAM_FALLBACK_ORDER};

const VIDEO_ID: &str = "xl9cFAOKg_Y"; // the id from the user's failing run

/// GET the first KB with the given UA — what mpv effectively does on load.
async fn probe(url: &str, ua: Option<&str>) -> reqwest::StatusCode {
    let client = reqwest::Client::new();
    let mut req = client.get(url).header("Range", "bytes=0-1023");
    if let Some(ua) = ua {
        req = req.header("User-Agent", ua);
    }
    req.send().await.expect("probe request").status()
}

#[tokio::test]
async fn direct_clients_resolve_and_stream() {
    let it = InnerTube::new(Session::default(), None).unwrap();
    let vd = it.fetch_visitor_data().await.ok();
    let it = InnerTube::new(
        Session { visitor_data: vd, ..Session::default() },
        None,
    )
    .unwrap();
    let clients = Clients::bundled();

    let mut any_ok = false;
    for key in STREAM_FALLBACK_ORDER {
        let client = clients.get(key).unwrap();
        let resp = match it.player(client, VIDEO_ID, None).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{key}: /player failed: {e}");
                continue;
            }
        };
        if !resp.playability_status.is_ok() {
            eprintln!("{key}: status {}", resp.playability_status.status);
            continue;
        }
        let sd = resp.streaming_data.as_ref().expect("streamingData");
        assert!(sd.expires_in_seconds.is_some(), "{key}: expiry must parse");
        let Some(format) = find_format(sd, AudioQuality::High) else {
            eprintln!("{key}: no audio format");
            continue;
        };
        let Some(url) = format.direct_url() else {
            eprintln!("{key}: itag {} cipher-only", format.itag);
            continue;
        };
        let status = probe(url, Some(&client.user_agent)).await;
        eprintln!("{key}: itag {} -> HTTP {status}", format.itag);
        if status.is_success() {
            any_ok = true;
        }
    }
    assert!(any_ok, "no direct client produced a playable (HTTP 2xx) stream URL");
}

#[tokio::test]
async fn rustypipe_url_is_fetchable() {
    let c = innertube::rustypipe_fallback::resolve(VIDEO_ID, true)
        .await
        .expect("rustypipe resolve");
    let bare = probe(&c.url, None).await;
    eprintln!("rustypipe itag {}: no-UA -> HTTP {bare}", c.itag);
    // mpv sends its own libmpv UA by default; also probe with a browser-ish UA for comparison.
    let browser = probe(&c.url, Some("Mozilla/5.0 (X11; Linux x86_64)")).await;
    eprintln!("rustypipe itag {}: browser-UA -> HTTP {browser}", c.itag);
    assert!(
        bare.is_success() || browser.is_success(),
        "rustypipe URL not fetchable (Raw(-13) root cause)"
    );
}
