//! The channel against a fake receiver over real TLS: what it sends, in what order, and how it
//! notices a device that goes away. The fake answers the way the Default Media Receiver does for
//! the messages used here; it is not a replacement for a test on hardware.

use super::*;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// How the fake behaves, and what it heard.
#[derive(Default)]
struct Fake {
    /// The receiver app is already running when the sender connects.
    running: bool,
    /// Stop pinging and answering after the receiver app starts.
    go_silent: bool,
    /// Another app runs instead of the receiver (a phone cast YouTube).
    other_app: bool,
    /// The media messages it got, as `TYPE` or `TYPE(mediaSessionId)`.
    heard: Vec<String>,
    session: i64,
    ids: Vec<i64>,
    next_id: i64,
}

type Shared = Arc<Mutex<Fake>>;

async fn fake_receiver(fake: Shared) -> u16 {
    let cert = CertificateDer::from(
        include_bytes!("../../tests/fixtures/fake-receiver-cert.der").to_vec(),
    );
    let key = PrivateKeyDer::try_from(
        include_bytes!("../../tests/fixtures/fake-receiver-key.der").to_vec(),
    )
    .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = acceptor.accept(tcp).await.unwrap();
        let (mut rd, wr) = tokio::io::split(tls);
        let wr = Arc::new(tokio::sync::Mutex::new(wr));
        let send =
            |wr: Arc<tokio::sync::Mutex<WriteHalf<_>>>, ns: &'static str, from: &str, v: Value| {
                let msg = proto::frame(ns, from, SENDER, &v.to_string());
                async move {
                    let _ = wr.lock().await.write_all(&msg).await;
                }
            };
        let pinger = {
            let (wr, fake) = (wr.clone(), fake.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    if fake.lock().unwrap().go_silent && fake.lock().unwrap().session >= 0 {
                        let started = fake.lock().unwrap().running;
                        if started {
                            continue;
                        }
                    }
                    send(wr.clone(), NS_HEARTBEAT, PLATFORM, json!({ "type": "PING" })).await;
                }
            })
        };
        loop {
            let mut len = [0u8; 4];
            if rd.read_exact(&mut len).await.is_err() {
                break;
            }
            let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
            if rd.read_exact(&mut body).await.is_err() {
                break;
            }
            let frame = proto::decode(&body).unwrap();
            let msg: Value = serde_json::from_str(&frame.payload).unwrap();
            let kind = msg["type"].as_str().unwrap_or_default().to_owned();
            if fake.lock().unwrap().go_silent && fake.lock().unwrap().running {
                continue;
            }
            let apps = |f: &Fake| {
                if f.other_app {
                    json!([{ "appId": "233637DE", "sessionId": "y1", "transportId": "y1" }])
                } else if f.running {
                    json!([{ "appId": DEFAULT_MEDIA_RECEIVER, "sessionId": "s1", "transportId": "t1" }])
                } else {
                    json!([])
                }
            };
            let receiver_status = |f: &Fake| {
                json!({ "type": "RECEIVER_STATUS", "status": {
                    "applications": apps(f), "volume": { "level": 0.3, "muted": false } } })
            };
            let media_status = |f: &Fake, state: &str| {
                json!({ "type": "MEDIA_STATUS", "status": [{
                    "mediaSessionId": f.session, "playerState": state, "currentTime": 1.0,
                    "currentItemId": f.ids.first(), "media": { "contentId": "http://lan/a" } }] })
            };
            match (frame.namespace.as_str(), kind.as_str()) {
                (NS_RECEIVER, "GET_STATUS") => {
                    let v = receiver_status(&fake.lock().unwrap());
                    send(wr.clone(), NS_RECEIVER, PLATFORM, v).await;
                }
                (NS_RECEIVER, "LAUNCH") => {
                    let v = {
                        let mut f = fake.lock().unwrap();
                        f.heard.push("LAUNCH".into());
                        f.running = true;
                        receiver_status(&f)
                    };
                    send(wr.clone(), NS_RECEIVER, PLATFORM, v).await;
                }
                (NS_MEDIA, _) => {
                    let reply = {
                        let mut f = fake.lock().unwrap();
                        let named = msg["mediaSessionId"].as_i64();
                        f.heard.push(match named {
                            Some(s) => format!("{kind}({s})"),
                            None => kind.clone(),
                        });
                        match kind.as_str() {
                            "QUEUE_LOAD" => {
                                f.session += 1;
                                f.ids.clear();
                                for _ in msg["items"].as_array().unwrap() {
                                    f.next_id += 1;
                                    let id = f.next_id;
                                    f.ids.push(id);
                                }
                                Some(media_status(&f, "PLAYING"))
                            }
                            "QUEUE_INSERT" => {
                                f.next_id += 1;
                                let id = f.next_id;
                                f.ids.push(id);
                                None
                            }
                            "QUEUE_REMOVE" => {
                                let gone: Vec<i64> = msg["itemIds"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .filter_map(Value::as_i64)
                                    .collect();
                                f.ids.retain(|id| !gone.contains(id));
                                None
                            }
                            "QUEUE_GET_ITEM_IDS" => {
                                Some(json!({ "type": "QUEUE_ITEM_IDS", "itemIds": f.ids }))
                            }
                            "PAUSE" => Some(media_status(&f, "PAUSED")),
                            _ => None,
                        }
                    };
                    if let Some(v) = reply {
                        send(wr.clone(), NS_MEDIA, "t1", v).await;
                    }
                }
                _ => {}
            }
        }
        pinger.abort();
    });
    port
}

fn track(url: &str) -> Media {
    Media {
        url: url.into(),
        content_type: "audio/webm".into(),
        title: "A song".into(),
        artist: "Someone".into(),
        album: None,
        image: None,
        duration: Some(200.0),
    }
}

async fn open(fake: &Shared) -> (Sender, mpsc::UnboundedReceiver<Event>) {
    let port = fake_receiver(fake.clone()).await;
    connect("127.0.0.1".parse().unwrap(), port).await.expect("connects")
}

/// Wait for the fake to have heard `n` media messages.
async fn heard(fake: &Shared, n: usize) -> Vec<String> {
    for _ in 0..250 {
        let heard = fake.lock().unwrap().heard.clone();
        if heard.len() >= n {
            return heard;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    fake.lock().unwrap().heard.clone()
}

#[tokio::test]
async fn a_cast_launches_the_receiver_and_reports_the_speakers_volume() {
    let fake = Shared::default();
    let (_tx, mut events) = open(&fake).await;
    assert_eq!(heard(&fake, 1).await, vec!["LAUNCH"]);
    assert_eq!(events.recv().await, Some(Event::Volume { level: 0.3, muted: false }));
}

#[tokio::test]
async fn a_running_receiver_is_joined_not_launched_again() {
    let fake = Shared::new(Mutex::new(Fake { running: true, ..Fake::default() }));
    let (_tx, _events) = open(&fake).await;
    // The join asks for the media status; nothing launches.
    assert_eq!(heard(&fake, 1).await, vec!["GET_STATUS"]);
}

#[tokio::test]
async fn commands_after_a_load_wait_for_the_session_it_makes() {
    let fake = Shared::default();
    let (tx, mut events) = open(&fake).await;
    tx.send(Command::Load {
        items: vec![track("http://lan/a")],
        start: 12.0,
        autoplay: true,
        repeat: Repeat::Off,
    });
    tx.send(Command::Pause);
    let heard = heard(&fake, 4).await;
    // The pause names the session the load made (1), never the empty one before it (0).
    let at = |m: &str| heard.iter().position(|h| h == m);
    assert!(at("QUEUE_LOAD") < at("PAUSE(1)") && at("PAUSE(1)").is_some(), "{heard:?}");
    assert!(!heard.iter().any(|h| h.ends_with("(0)")), "{heard:?}");
    loop {
        if let Some(Event::Media(Some(s))) = events.recv().await {
            assert_eq!(s.content_id.as_deref(), Some("http://lan/a"));
            break;
        }
    }
}

#[tokio::test]
async fn dropping_the_next_item_removes_the_one_that_was_appended() {
    let fake = Shared::default();
    let (tx, _events) = open(&fake).await;
    tx.send(Command::Load {
        items: vec![track("http://lan/a")],
        start: 0.0,
        autoplay: true,
        repeat: Repeat::Off,
    });
    tx.send(Command::Append(track("http://lan/b")));
    tx.send(Command::DropNext);
    let heard = heard(&fake, 6).await;
    assert!(heard.contains(&"QUEUE_REMOVE(1)".to_owned()), "{heard:?}");
    assert_eq!(fake.lock().unwrap().ids, vec![1], "only the playing item is left");
}

#[tokio::test]
async fn a_device_that_goes_quiet_is_reported_lost() {
    let fake = Shared::new(Mutex::new(Fake { go_silent: true, ..Fake::default() }));
    let (_tx, mut events) = open(&fake).await;
    let closed = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(Event::Closed(c)) = events.recv().await {
                return c;
            }
        }
    })
    .await
    .expect("noticed within seconds");
    assert_eq!(closed, Closed::Lost);
}

async fn closed_after(fake: &Shared, change: impl FnOnce(&mut Fake)) -> Closed {
    let (_tx, mut events) = open(fake).await;
    heard(fake, 1).await;
    change(&mut fake.lock().unwrap());
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(Event::Closed(c)) = events.recv().await {
                return c;
            }
        }
    })
    .await
    .expect("noticed on the next status poll")
}

#[tokio::test]
async fn another_app_on_the_device_ends_the_cast_as_taken_over() {
    let fake = Shared::default();
    assert_eq!(closed_after(&fake, |f| f.other_app = true).await, Closed::TakenOver);
}

#[tokio::test]
async fn a_receiver_stopped_on_the_device_ends_the_cast_as_stopped() {
    let fake = Shared::default();
    assert_eq!(closed_after(&fake, |f| f.running = false).await, Closed::Stopped);
}
