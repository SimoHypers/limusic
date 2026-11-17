//! The Chromecast control channel: Google Cast, and the Google Home / Nest speakers that speak it.
//!
//! A Cast device listens on TLS :8009 and speaks CastV2: a protobuf envelope (see [`super::proto`])
//! whose payload is JSON. Three namespaces are involved:
//!
//! - `urn:x-cast:com.google.cast.tp.connection` — the CONNECT that opens a channel to an
//!   endpoint (the platform itself, or the receiver app's session).
//! - `urn:x-cast:com.google.cast.receiver` — LAUNCH, which starts the default media receiver
//!   (`CC1AD845`, an app id Google registers) and answers with the session id to talk to.
//! - `urn:x-cast:com.google.cast.media` — LOAD / PLAY / PAUSE / SEEK and the status that comes
//!   back.
//!
//! The connection is a self-signed TLS one, so the certificate cannot be verified against
//! anything: the device is identified by its mDNS id, and the channel is a local one. Refusing
//! an unverifiable certificate would mean no Chromecast at all.
//!
//! The socket is owned by one task. Commands go in over a channel, frames come back over
//! another, and the task answers the heartbeat itself: a receiver that gets no answer within
//! ten seconds drops the sender and stops the music.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::TlsConnector;

use super::proto;
use super::{Media, QueueEntry};

/// The channel to the platform, before an app is running.
const RECEIVER: &str = "receiver-0";
/// Our end of the channel. The receiver routes by it; a fixed id is what other senders use.
const SENDER: &str = "sender-0";

const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";

/// Google's built-in receiver. Its registered display name is "Default Media Receiver".
const DEFAULT_MEDIA_RECEIVER: &str = "CC1AD845";

/// The receiver app to launch. A registered Styled Media Receiver named Limusic replaces the
/// default one; the environment override lets a developer test its Application ID locally.
fn receiver_app_id() -> String {
    std::env::var("LIMUSIC_CAST_APP_ID")
        .ok()
        .map(|id| id.trim().to_ascii_uppercase())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| DEFAULT_MEDIA_RECEIVER.to_owned())
}

/// How long the handshake may take before the device is written off.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);

/// How often the sender pings the receiver. The receiver answers with a PONG; silence for a
/// while means the channel is gone.
const HEARTBEAT: Duration = Duration::from_secs(5);

/// How long the device may take to report the session a load made before the commands
/// behind it go out anyway.
const LOAD_GATE: Duration = Duration::from_secs(8);

/// How long a stop waits for the socket task to write it.
const STOP_ACK_TIMEOUT: Duration = Duration::from_secs(2);

/// A Chromecast found on the network.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// The device id the receiver knows it by (the mDNS `id` property).
    pub id: String,
    /// The name the owner gave it (the mDNS `fn` property).
    pub name: String,
    /// The model (the mDNS `md` property): "Google Nest Mini", "Chromecast", …
    pub model: Option<String>,
    /// Where it is.
    pub host: String,
    /// The control port. 8009 unless the device says otherwise.
    pub port: u16,
}

/// What the device is doing with the media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerState {
    Idle,
    Playing,
    Paused,
    Buffering,
    /// A state this app doesn't know (the receiver may add one).
    Other,
}

impl PlayerState {
    fn parse(value: &str) -> Self {
        match value {
            "IDLE" => PlayerState::Idle,
            "PLAYING" => PlayerState::Playing,
            "PAUSED" => PlayerState::Paused,
            "BUFFERING" => PlayerState::Buffering,
            _ => PlayerState::Other,
        }
    }
}

/// The device's answer about the media.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: PlayerState,
    /// Where the device is in the track, in seconds.
    pub position: f64,
    /// The track's length, when the device reports one.
    pub duration: Option<f64>,
    /// The media session the device is playing. The transport commands name it: the receiver
    /// refuses one without it (`INVALID_MEDIA_SESSION_ID`).
    pub media_session: i64,
    /// The receiver's own id for the queue item it is on. The window does not set them, so this
    /// is the receiver's numbering; it is only trusted once the track it points at is the one
    /// the device says it is playing.
    pub item_id: Option<i64>,
    /// What the device is playing, when it says: the title, the artist and the cover.
    pub track: Option<DeviceTrack>,
    /// The device's own volume, 0.0 to 1.0. The speaker's, not the media session's: the
    /// receiver reports that one, the media status the other.
    pub volume: Option<f64>,
    /// Whether the device is muted.
    pub muted: Option<bool>,
    /// The video id the receiver's current queue item carries, when it reports one.
    pub video_id: Option<String>,
    /// The URL of the current item. The receiver reports this even when it drops
    /// `customData`, so it is the reliable way to tell which queue item is playing.
    pub content_id: Option<String>,
    /// The receiver's queue order, when the phone changed it.
    pub repeat_mode: Option<String>,
    /// Whether the receiver ran out of items (`idleReason: FINISHED`).
    pub finished: bool,
}

/// What a device is playing, as it reports it.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTrack {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub thumbnail: Option<String>,
}

/// Something the device said.
#[derive(Debug, Clone)]
pub enum Event {
    /// Its media status changed.
    Status(Status),
    /// The channel died.
    Closed(String),
}

/// A command for the device.
enum Cmd {
    LoadQueue {
        entries: Vec<QueueEntry>,
        start_index: usize,
        repeat_mode: String,
        current_time: f64,
    },
    /// Change the queue's repeat mode, leaving the items alone.
    QueueRepeat(String),
    Play,
    Pause,
    Stop {
        ack: oneshot::Sender<Result<(), String>>,
    },
    Seek(f64),
    /// Ask for the status without waiting for a change.
    Status,
    /// Set the device's volume, 0.0 to 1.0.
    SetVolume(f64),
    /// Ask the receiver for its status: the speaker's volume comes from there.
    ReceiverStatus,
    /// Close the channel.
    Disconnect,
}

/// The sending half of a connection. Cloneable: the session keeps one, the poll loop another.
#[derive(Clone)]
pub struct Sender {
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl Sender {
    /// Hand the device a window of tracks, so its own next/previous, shuffle and repeat
    /// controls have something to work on.
    pub fn load_queue(
        &self,
        entries: Vec<QueueEntry>,
        start_index: usize,
        repeat_mode: &str,
        current_time: f64,
    ) -> Result<(), String> {
        self.send(Cmd::LoadQueue {
            entries,
            start_index,
            repeat_mode: repeat_mode.to_owned(),
            current_time,
        })
    }

    /// Change the receiver queue's repeat mode without reloading it.
    pub fn set_repeat(&self, mode: &str) -> Result<(), String> {
        self.send(Cmd::QueueRepeat(mode.to_owned()))
    }

    pub fn play(&self) -> Result<(), String> {
        self.send(Cmd::Play)
    }

    pub fn pause(&self) -> Result<(), String> {
        self.send(Cmd::Pause)
    }

    /// Stop playback and wait until the socket task has written the command.
    pub async fn stop(&self) -> Result<(), String> {
        let (ack, answer) = oneshot::channel();
        self.cmd.send(Cmd::Stop { ack }).map_err(|_| "the cast connection is closed".to_owned())?;
        match tokio::time::timeout(STOP_ACK_TIMEOUT, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("the cast connection closed before the stop was sent".to_owned()),
            Err(_) => Err("the cast socket did not confirm the stop".to_owned()),
        }
    }

    /// Move to `seconds` in the track.
    pub fn seek(&self, seconds: f64) -> Result<(), String> {
        self.send(Cmd::Seek(seconds))
    }

    pub fn request_status(&self) -> Result<(), String> {
        self.send(Cmd::Status)
    }

    /// The device's own volume, 0.0 to 1.0. It is the speaker's volume, not this desktop's.
    pub fn set_volume(&self, level: f64) -> Result<(), String> {
        self.send(Cmd::SetVolume(level.clamp(0.0, 1.0)))
    }

    /// Ask the receiver for its status, which carries the speaker's volume.
    pub fn request_receiver_status(&self) -> Result<(), String> {
        self.send(Cmd::ReceiverStatus)
    }

    /// Close the channel to the device.
    pub fn disconnect(&self) -> Result<(), String> {
        self.send(Cmd::Disconnect)
    }

    fn send(&self, cmd: Cmd) -> Result<(), String> {
        self.cmd.send(cmd).map_err(|_| "the cast connection is closed".to_owned())
    }
}
/// A live connection to a device.
pub struct Chromecast {
    sender: Sender,
    events: mpsc::UnboundedReceiver<Event>,
}

impl Chromecast {
    /// Connect to a device and start the default media receiver on it.
    ///
    /// Returns once the receiver has answered with the session to talk to, so the first LOAD
    /// after this goes straight out.
    pub async fn connect(device: &Device) -> Result<(Chromecast, Sender), String> {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Cmd>();
        let (event_tx, event_rx) = mpsc::unbounded_channel::<Event>();
        let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

        let device = device.clone();
        tauri::async_runtime::spawn(async move {
            run(device, cmd_rx, event_tx, ready_tx).await;
        });

        match tokio::time::timeout(HANDSHAKE_TIMEOUT, ready_rx).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(e))) => return Err(e),
            Ok(Err(_)) => return Err("the cast connection closed during the handshake".to_owned()),
            Err(_) => return Err("the device did not answer the cast handshake".to_owned()),
        }

        let sender = Sender { cmd: cmd_tx };
        Ok((Chromecast { sender: sender.clone(), events: event_rx }, sender))
    }

    /// The sending half, for callers that only send.
    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }

    /// Wait for the next thing the device says.
    pub async fn next_event(&mut self) -> Option<Event> {
        self.events.recv().await
    }
}

/// What came off the socket.
enum Incoming {
    Frame(proto::Frame),
    Closed(String),
}

/// The socket task: one TLS stream, both directions.
async fn run(
    device: Device,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
    events: mpsc::UnboundedSender<Event>,
    ready: oneshot::Sender<Result<(), String>>,
) {
    let stream = match connect_tls(&device).await {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            let _ = events.send(Event::Closed(e));
            return;
        }
    };
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<Incoming>();

    // The reader is its own task: a cancelled read would eat half a frame, so the frame loop
    // must never interrupt one.
    let reader_task = tauri::async_runtime::spawn(async move {
        loop {
            let mut len = [0u8; 4];
            if let Err(e) = reader.read_exact(&mut len).await {
                let _ = frame_tx.send(Incoming::Closed(e.to_string()));
                return;
            }
            let len = u32::from_be_bytes(len) as usize;
            if len > 1 << 20 {
                let _ =
                    frame_tx.send(Incoming::Closed("the device sent an oversized frame".into()));
                return;
            }
            let mut body = vec![0u8; len];
            if let Err(e) = reader.read_exact(&mut body).await {
                let _ = frame_tx.send(Incoming::Closed(e.to_string()));
                return;
            }
            if let Some(frame) = proto::decode(&body) {
                if frame_tx.send(Incoming::Frame(frame)).is_err() {
                    return;
                }
            }
        }
    });

    // Open the channel to the platform and start the receiver on it. The answer carries the
    // session id that the media messages go to.
    let app_id = receiver_app_id();
    let launch = json!({
        "type": "LAUNCH",
        "requestId": 1,
        "appId": app_id,
    });
    let opening = [
        proto::frame(NS_CONNECTION, SENDER, RECEIVER, &connect_payload()),
        proto::frame(NS_RECEIVER, SENDER, RECEIVER, &launch.to_string()),
    ];
    for msg in opening {
        if let Err(e) = writer.write_all(&msg).await {
            let _ = ready.send(Err(e.to_string()));
            let _ = events.send(Event::Closed(e.to_string()));
            reader_task.abort();
            return;
        }
    }

    let mut session: Option<String> = None;
    // The media session the device is playing. The transport commands name it.
    let mut media_session: i64 = 0;
    // The commands that wait for the device to be ready. A load takes a moment to come back
    // as a session, and a command sent before that names a session the device has already
    // left — which it answers with INVALID_MEDIA_SESSION_ID. A burst of them also leaves
    // the device loading one thing after another without ever playing any of it.
    let mut waiting: Vec<Cmd> = Vec::new();
    let mut gate: Option<std::time::Instant> = None;
    // The media session the device was on when the gate shut. The device reports the new one
    // when it has made it, and that is what opens the gate.
    let mut gate_session: i64 = 0;
    // The speaker's own volume, which the receiver reports and the media status does not.
    let mut receiver_volume: Option<(f64, Option<bool>)> = None;
    let mut ready = Some(ready);
    let mut request_id = 1i64;
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.tick().await; // the first tick is immediate
    let mut gate_tick = tokio::time::interval(Duration::from_millis(250));
    gate_tick.tick().await; // the first tick is immediate

    let result = loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break Ok(()) };
                if matches!(cmd, Cmd::Disconnect) {
                    break Ok(());
                }
                let Some(session) = session.clone() else {
                    waiting.push(cmd);
                    continue;
                };
                if gate.is_some() && !matches!(&cmd, Cmd::Stop { .. }) {
                    // The device is still busy with the last load. The commands wait for it.
                    //
                    // A load that waits supersedes what waits before it: those commands are
                    // about the track the load replaces, and the queue is the truth — the
                    // device only has to end up on the track the queue is on.
                    if matches!(cmd, Cmd::LoadQueue { .. }) {
                        waiting.clear();
                    }
                    waiting.push(cmd);
                    continue;
                }
                // A stop answers the gate rather than waiting for it: the commands behind it
                // name a track the stop is about to end, and one of them reaching the device
                // after the stop would start the speaker again with the app no longer casting.
                if matches!(&cmd, Cmd::Stop { .. }) {
                    waiting.clear();
                    gate = None;
                }
                let is_load = matches!(cmd, Cmd::LoadQueue { .. });
                let sent = match cmd {
                    Cmd::SetVolume(level) => {
                        write_volume(&mut writer, level, &mut request_id).await
                    }
                    Cmd::ReceiverStatus => {
                        write_receiver_status(&mut writer, &session, &mut request_id).await
                    }
                    cmd => {
                        write_cmd(&mut writer, cmd, &session, media_session, &mut request_id).await
                    }
                };
                if let Err(e) = sent {
                    break Err(e);
                }
                // A load leaves the device busy until it reports the session it made.
                if is_load {
                    arm_gate(&mut gate, &mut gate_session, media_session);
                }
            }
            _ = gate_tick.tick() => {
                // The device did not report a new session in time. It is either still loading
                // or it never will: the commands go out anyway, so a stuck load cannot wedge
                // the controls.
                if let Some(until) = gate {
                    if std::time::Instant::now() >= until {
                        gate = None;
                        let flushed = flush(
                            &mut writer,
                            &mut waiting,
                            session.as_deref().unwrap_or(RECEIVER),
                            media_session,
                            &mut request_id,
                        )
                        .await;
                        match flushed {
                            Ok(true) => {
                                arm_gate(&mut gate, &mut gate_session, media_session)
                            }
                            Ok(false) => {}
                            Err(e) => break Err(e),
                        }
                    }
                }
            }
            _ = heartbeat.tick() => {
                let destination = session.as_deref().unwrap_or(RECEIVER);
                let msg = proto::frame(NS_HEARTBEAT, SENDER, destination, r#"{"type":"PING"}"#);
                if let Err(e) = writer.write_all(&msg).await {
                    break Err(e.to_string());
                }
            }
            incoming = frame_rx.recv() => {
                let Some(incoming) = incoming else { break Err("the cast connection closed".to_owned()) };
                let frame = match incoming {
                    Incoming::Frame(f) => f,
                    Incoming::Closed(e) => break Err(e),                };
                match frame.namespace.as_str() {
                    NS_HEARTBEAT => {
                        // The receiver pings too, and drops the sender that does not answer.
                        if frame.payload.contains("PING") {
                            let destination = session.as_deref().unwrap_or(RECEIVER);
                            let msg = proto::frame(NS_HEARTBEAT, SENDER, destination, r#"{"type":"PONG"}"#);
                            if let Err(e) = writer.write_all(&msg).await {
                                break Err(e.to_string());
                            }
                        }
                    }
                    NS_RECEIVER => {
                        tracing::debug!(target: "app_lib::cast", payload = %frame.payload, "cast: the receiver says");
                        if let Some(volume) = volume_from_receiver(&frame.payload) {
                            receiver_volume = Some(volume);
                        }
                        if let Some((id, transport)) = session_from_status(&frame.payload, &app_id) {
                            // The protocol keeps the two apart: payloads name the session, and the
                            // app's virtual connection is the transport. Both are logged so a run
                            // can show whether the receiver hands out different values.
                            tracing::info!(target: "app_lib::cast", session = %id, transport = ?transport, "cast: the receiver answered");
                            session = Some(id.clone());
                            // The channel to the app's session is opened separately: the app
                            // only accepts messages from a sender that connected to it.
                            let msg = proto::frame(NS_CONNECTION, SENDER, &id, &connect_payload());
                            if let Err(e) = writer.write_all(&msg).await {
                                break Err(e.to_string());
                            }
                            if let Some(tx) = ready.take() {
                                let _ = tx.send(Ok(()));
                            }
                            // Only a status that arrives with no load in flight may let the
                            // queue out: a status carrying the receiver's session says
                            // nothing about a media session the pending load has not made
                            // yet, and sending them now names the session being replaced.
                            if gate.is_none() {
                                let flushed = flush(
                                    &mut writer,
                                    &mut waiting,
                                    &id,
                                    media_session,
                                    &mut request_id,
                                )
                                .await;
                                match flushed {
                                    Ok(true) => {
                                        arm_gate(&mut gate, &mut gate_session, media_session)
                                    }
                                    Ok(false) => {}
                                    Err(e) => break Err(e),
                                }
                            }
                        } else if frame.payload.contains("LAUNCH_ERROR") {
                            let reason = error_reason(&frame.payload)
                                .unwrap_or_else(|| "the device refused to start the receiver".to_owned());
                            if reason.contains("ALREADY_RUNNING") {
                                // The default receiver survived the previous sender. Ask the
                                // platform for its session instead of trying to launch it again.
                                request_id += 1;
                                let payload = json!({
                                    "type": "GET_STATUS", "requestId": request_id,
                                });
                                let msg =
                                    proto::frame(NS_RECEIVER, SENDER, RECEIVER, &payload.to_string());
                                if let Err(e) = writer.write_all(&msg).await {
                                    break Err(e.to_string());
                                }
                                continue;
                            }
                            let _ = events.send(Event::Closed(reason.clone()));
                            if let Some(tx) = ready.take() {
                                let _ = tx.send(Err(reason));
                            }
                            break Err("the device refused to start the receiver".to_owned());
                        }
                    }
                    NS_MEDIA => {
                        tracing::debug!(target: "app_lib::cast", payload = %frame.payload, "cast: the device says");
                        if let Some(mut status) = status_from_media(&frame.payload) {
                            if status.media_session > 0 {
                                // The device has made the session the load asked for: it is
                                // ready for what is queued behind it. The new session is the
                                // one the queued commands name, so it is taken first.
                                let switched = status.media_session != gate_session;
                                media_session = status.media_session;
                                if switched {
                                    gate = None;
                                    let flushed = flush(
                                        &mut writer,
                                        &mut waiting,
                                        session.as_deref().unwrap_or(RECEIVER),
                                        media_session,
                                        &mut request_id,
                                    )
                                    .await;
                                    match flushed {
                                        Ok(true) => {
                                            arm_gate(&mut gate, &mut gate_session, media_session)
                                        }
                                        Ok(false) => {}
                                        Err(e) => break Err(e),
                                    }
                                }
                            }
                            if let Some((level, muted)) = receiver_volume {
                                status.volume = Some(level);
                                status.muted = muted;
                            }
                            let _ = events.send(Event::Status(status));
                        } else if frame.payload.contains("LOAD_FAILED") {
                            let reason = error_reason(&frame.payload)
                                .unwrap_or_else(|| "unknown".to_owned());
                            tracing::warn!(target: "app_lib::cast", reason, "cast: the device refused the track");
                            let _ = events.send(Event::Closed(format!("the device refused the track: {reason}")));
                        } else if frame.payload.contains("INVALID_REQUEST") {
                            // A transport command against a media session the device does not
                            // have is answered this way whenever nothing is loaded. A refusal
                            // while a load is in flight is that load, and the app has to hear
                            // it: left alone, the cast keeps showing a track the device never
                            // took, and every play/pause after that is refused too.
                            let reason = error_reason(&frame.payload).unwrap_or_default();
                            tracing::warn!(target: "app_lib::cast", reason, "cast: the device refused a request");
                            if gate.is_some() && !reason.contains("INVALID_MEDIA_SESSION_ID") {
                                break Err(format!("the device refused the track: {reason}"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    };

    reader_task.abort();
    match result {
        Ok(()) => {
            let _ = events.send(Event::Closed("the cast connection closed".to_owned()));
        }
        Err(e) => {
            if let Some(tx) = ready.take() {
                let _ = tx.send(Err(e.clone()));
            }
            let _ = events.send(Event::Closed(e));
        }
    }
}

/// The CONNECT payload.
///
/// `userAgent` identifies this client to the receiver. It does not rename the receiver
/// application: Google gives the built-in receiver the fixed name "Default Media Receiver".
fn connect_payload() -> String {
    json!({
        "type": "CONNECT",
        "origin": {},
        "userAgent": "Limusic",
        "senderInfo": {
            "sdkType": 2,
            "version": env!("CARGO_PKG_VERSION"),
            "browserVersion": "1.0.0",
            "platform": 4,
            "systemVersion": "Limusic",
            "connectionType": 1,
        },
    })
    .to_string()
}

/// One media description for `LOAD` or a queue item. The YouTube id rides in `customData`,
/// which the receiver echoes back in its status, so an advance can be matched to the queue.
fn media_payload(media: &Media, video_id: &str) -> Value {
    let mut metadata = json!({ "metadataType": 3 }); // a music track
    if let Some(t) = &media.title {
        metadata["title"] = json!(t);
    }
    if let Some(a) = &media.artist {
        metadata["artist"] = json!(a);
    }
    if let Some(a) = &media.album {
        metadata["albumName"] = json!(a);
    }
    if let Some(thumb) = &media.thumbnail {
        // The picture the phone shows next to the song. The queue's thumbnail is the
        // small one the player bar uses; the device gets a larger one, which is the
        // same picture at a size worth looking at.
        metadata["images"] = json!([{ "url": larger_cover(thumb) }]);
        metadata["thumb"] = json!(thumb);
    }
    let mut info = json!({
        "contentId": media.url,
        "contentType": media.content_type,
        "streamType": "BUFFERED",
        "metadata": metadata,
    });
    // A null duration is not the same as no duration: the receiver reads the field as
    // zero and would treat the track as over.
    if let Some(d) = media.duration {
        info["duration"] = json!(d);
    }
    if !video_id.is_empty() {
        info["customData"] = json!({ "videoId": video_id });
    }
    info
}

/// Send one command to the receiver app.
async fn write_cmd(
    writer: &mut tokio::io::WriteHalf<tokio_rustls::client::TlsStream<TcpStream>>,
    cmd: Cmd,
    session: &str,
    media_session: i64,
    request_id: &mut i64,
) -> Result<(), String> {
    *request_id += 1;
    let id = *request_id;
    let payload = match cmd {
        Cmd::LoadQueue { entries, start_index, repeat_mode, current_time } => {
            // No `itemId` per item: the receiver assigns them, and a load that carries its own
            // is refused as `INVALID_PARAMS`. What it assigns comes back as `currentItemId`.
            let items: Vec<Value> = entries
                .iter()
                .map(|entry| {
                    json!({
                        "autoplay": true,
                        "media": media_payload(&entry.media, entry.video_id.as_str()),
                    })
                })
                .collect();
            json!({
                "type": "QUEUE_LOAD",
                "requestId": id,
                "sessionId": session,
                "items": items,
                "startIndex": start_index,
                "currentTime": current_time,
                "repeatMode": repeat_mode,
                "autoplay": true,
                "customData": {},
            })
        }
        Cmd::QueueRepeat(mode) => json!({
            "type": "QUEUE_UPDATE",
            "requestId": id,
            "sessionId": session,
            "mediaSessionId": media_session,
            "repeatMode": mode,
            "customData": {},
        }),
        // The transport commands name the media session: without it the receiver answers
        // INVALID_MEDIA_SESSION_ID and does nothing.
        Cmd::Play => json!({
            "type": "PLAY", "requestId": id, "sessionId": session,
            "mediaSessionId": media_session, "customData": {},
        }),
        Cmd::Pause => json!({
            "type": "PAUSE", "requestId": id, "sessionId": session,
            "mediaSessionId": media_session, "customData": {},
        }),
        Cmd::Stop { ack } => {
            let payload = json!({
                "type": "STOP", "requestId": id, "sessionId": session,
                "mediaSessionId": media_session, "customData": {},
            });
            let msg = proto::frame(NS_MEDIA, SENDER, session, &payload.to_string());
            tracing::debug!(target: "app_lib::cast", ?session, payload = %payload, "cast: sending");
            let result = writer.write_all(&msg).await.map_err(|e| e.to_string());
            let _ = ack.send(result.clone());
            return result;
        }
        Cmd::Seek(seconds) => json!({
            "type": "SEEK", "requestId": id, "sessionId": session,
            "mediaSessionId": media_session, "currentTime": seconds, "customData": {},
        }),
        Cmd::Status => json!({ "type": "GET_STATUS", "requestId": id }),
        // All three are handled by the caller: the volume and the receiver's status go on
        // another namespace, and the socket is closed rather than messaged.
        Cmd::SetVolume(_) => return Ok(()),
        Cmd::ReceiverStatus => return Ok(()),
        Cmd::Disconnect => return Ok(()),
    };
    let msg = proto::frame(NS_MEDIA, SENDER, session, &payload.to_string());
    tracing::debug!(target: "app_lib::cast", ?session, payload = %payload, "cast: sending");
    writer.write_all(&msg).await.map_err(|e| e.to_string())
}

/// Ask the receiver for the status of the app that is running, which is where the speaker's
/// own volume is reported.
async fn write_receiver_status(
    writer: &mut tokio::io::WriteHalf<tokio_rustls::client::TlsStream<TcpStream>>,
    session: &str,
    request_id: &mut i64,
) -> Result<(), String> {
    *request_id += 1;
    let payload = json!({ "type": "GET_STATUS", "requestId": *request_id });
    let msg = proto::frame(NS_RECEIVER, SENDER, session, &payload.to_string());
    writer.write_all(&msg).await.map_err(|e| e.to_string())
}

/// Send the commands that were waiting for the device to be ready.
///
/// A load is where it stops: it makes a media session of its own, and everything queued
/// behind it names that one. The rest stays in `waiting`, and the `true` says a gate is owed
/// so they only go out once the load reports its session.
async fn flush(
    writer: &mut tokio::io::WriteHalf<tokio_rustls::client::TlsStream<TcpStream>>,
    waiting: &mut Vec<Cmd>,
    session: &str,
    media_session: i64,
    request_id: &mut i64,
) -> Result<bool, String> {
    while !waiting.is_empty() {
        let cmd = waiting.remove(0);
        let is_load = matches!(&cmd, Cmd::LoadQueue { .. });
        let sent = match cmd {
            Cmd::SetVolume(level) => write_volume(writer, level, request_id).await,
            Cmd::ReceiverStatus => write_receiver_status(writer, session, request_id).await,
            cmd => write_cmd(writer, cmd, session, media_session, request_id).await,
        };
        sent?;
        if is_load {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Hold the commands until the device reports the media session a load made.
fn arm_gate(gate: &mut Option<std::time::Instant>, gate_session: &mut i64, media_session: i64) {
    *gate_session = media_session;
    *gate = Some(std::time::Instant::now() + LOAD_GATE);
}

/// Set the device's volume. It belongs to the receiver itself, not to the media session, so
/// it goes to the platform's own endpoint (`receiver-0`) on the receiver's namespace.
async fn write_volume(
    writer: &mut tokio::io::WriteHalf<tokio_rustls::client::TlsStream<TcpStream>>,
    level: f64,
    request_id: &mut i64,
) -> Result<(), String> {
    *request_id += 1;
    let payload = json!({
        "type": "SET_VOLUME",
        "requestId": *request_id,
        "volume": { "level": level },
    });
    let msg = proto::frame(NS_RECEIVER, SENDER, RECEIVER, &payload.to_string());
    tracing::debug!(target: "app_lib::cast", payload = %payload, "cast: sending");
    writer.write_all(&msg).await.map_err(|e| e.to_string())
}

/// The device's volume out of a receiver status.
fn volume_from_receiver(payload: &str) -> Option<(f64, Option<bool>)> {
    let value: Value = serde_json::from_str(payload).ok()?;
    let volume = value.get("status")?.get("volume")?;
    let level = volume.get("level").and_then(Value::as_f64)?;
    let muted = volume.get("muted").and_then(Value::as_bool);
    Some((level, muted))
}

/// A larger picture of the same cover, when the address carries a size.
///
/// YouTube's thumbnail addresses end in something like `=w120-h120-l90-rj`. Asking for a
/// bigger one is a matter of changing the numbers.
fn larger_cover(url: &str) -> String {
    // The size is the last thing in the address: `…=w120-h120-l90-rj`, with whatever the
    // address carries after it.
    let Some((head, tail)) = url.rsplit_once('=') else { return url.to_owned() };
    let (spec, query) = match tail.split_once('&') {
        Some((spec, query)) => (spec, Some(query)),
        None => (tail, None),
    };
    let mut parts: Vec<&str> = spec.split('-').collect();
    if parts.len() < 2 || !parts[0].starts_with('w') || !parts[1].starts_with('h') {
        return url.to_owned();
    }
    parts[0] = "w600";
    parts[1] = "h600";
    let spec = parts.join("-");
    match query {
        Some(query) => format!("{head}={spec}&{query}"),
        None => format!("{head}={spec}"),
    }
}

/// The `reason` out of a receiver error or refusal.
fn error_reason(payload: &str) -> Option<String> {
    serde_json::from_str::<Value>(payload).ok()?.get("reason")?.as_str().map(str::to_owned)
}

/// The receiver app's session id, and the transport id for its connection.
///
/// Cast names the session in payloads and addresses the app's virtual connection by transport.
/// This app uses the session for both, which the receivers tested accept; the transport is
/// carried out so a run can show whether the receiver gives it a different value.
fn session_from_status(payload: &str, app_id: &str) -> Option<(String, Option<String>)> {
    let value: Value = serde_json::from_str(payload).ok()?;
    let apps = value.get("status")?.get("applications")?.as_array()?;
    let app = apps.iter().find(|app| app.get("appId").and_then(Value::as_str) == Some(app_id))?;
    let session = app.get("sessionId")?.as_str()?.to_owned();
    let transport = app.get("transportId").and_then(Value::as_str).map(str::to_owned);
    Some((session, transport))
}

/// The media status out of a media message.
fn status_from_media(payload: &str) -> Option<Status> {
    let value: Value = serde_json::from_str(payload).ok()?;
    let list = value.get("status")?.as_array()?;
    // An empty list is the receiver saying it has no media session: nothing is playing. It is
    // also how a track that ran out announces itself, so it is a status, not a silence.
    let Some(first) = list.first() else {
        return Some(Status {
            state: PlayerState::Idle,
            position: 0.0,
            duration: None,
            media_session: 0,
            item_id: None,
            track: None,
            volume: None,
            muted: None,
            video_id: None,
            content_id: None,
            repeat_mode: None,
            finished: false,
        });
    };
    let state = PlayerState::parse(first.get("playerState")?.as_str()?);
    let position = first.get("currentTime").and_then(Value::as_f64).unwrap_or(0.0);
    let duration = first.get("media").and_then(|m| m.get("duration")).and_then(Value::as_f64);
    let media_session = first.get("mediaSessionId").and_then(Value::as_i64).unwrap_or(0);
    let item_id = first.get("currentItemId").and_then(Value::as_i64);
    let track = first.get("media").and_then(|media| {
        let metadata = media.get("metadata")?;
        let title = metadata.get("title").and_then(Value::as_str).map(str::to_owned);
        let artist = metadata.get("artist").and_then(Value::as_str).map(str::to_owned);
        let thumbnail = metadata
            .get("images")
            .and_then(Value::as_array)
            .and_then(|images| images.first())
            .and_then(|image| image.get("url"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        (title.is_some() || artist.is_some()).then_some(DeviceTrack { title, artist, thumbnail })
    });
    let volume = first.get("volume").and_then(|v| v.get("level")).and_then(Value::as_f64);
    let muted = first.get("volume").and_then(|v| v.get("muted")).and_then(Value::as_bool);
    let video_id = first
        .get("media")
        .and_then(|media| media.get("customData"))
        .and_then(|data| data.get("videoId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let content_id = first
        .get("media")
        .and_then(|media| media.get("contentId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let repeat_mode = first.get("repeatMode").and_then(Value::as_str).map(str::to_owned);
    let finished = first.get("idleReason").and_then(Value::as_str) == Some("FINISHED");
    Some(Status {
        state,
        position,
        duration,
        media_session,
        item_id,
        track,
        volume,
        muted,
        video_id,
        content_id,
        repeat_mode,
        finished,
    })
}

/// Open the TLS channel to a device.
async fn connect_tls(
    device: &Device,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    let addr = format!("{}:{}", device.host, device.port);
    let tcp = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("could not reach the device on {addr}: {e}"))?;
    let connector = TlsConnector::from(Arc::new(tls_config()));
    let name = rustls::pki_types::ServerName::try_from(device.host.clone())
        .map_err(|e| format!("the device has no usable address: {e}"))?;
    connector.connect(name, tcp).await.map_err(|e| format!("the cast handshake failed: {e}"))
}

/// The TLS config. The device's certificate is self-signed, so it cannot be checked against a
/// root store; the device is identified by its id and the channel never leaves the network.
fn tls_config() -> rustls::ClientConfig {
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    #[derive(Debug)]
    struct AnyCertificate;

    impl rustls::client::danger::ServerCertVerifier for AnyCertificate {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("the ring provider speaks TLS 1.2 and 1.3")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate))
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_id_is_read_out_of_the_receiver_status() {
        let payload = r#"{"type":"RECEIVER_STATUS","status":{"applications":[{"appId":"CC1AD845","sessionId":"12345-abc","transportId":"web-42","namespaces":[]}]}}"#;
        assert_eq!(
            session_from_status(payload, DEFAULT_MEDIA_RECEIVER).map(|(session, _)| session),
            Some("12345-abc".to_owned())
        );
        assert_eq!(
            session_from_status(payload, DEFAULT_MEDIA_RECEIVER)
                .and_then(|(_, transport)| transport),
            Some("web-42".to_owned())
        );
        assert_eq!(
            session_from_status(r#"{"type":"RECEIVER_STATUS"}"#, DEFAULT_MEDIA_RECEIVER),
            None
        );
        let payload = r#"{"type":"RECEIVER_STATUS","status":{"applications":[{"appId":"OTHER","sessionId":"wrong"},{"appId":"CC1AD845","sessionId":"right"}]}}"#;
        assert_eq!(
            session_from_status(payload, DEFAULT_MEDIA_RECEIVER).map(|(session, _)| session),
            Some("right".to_owned())
        );
    }

    #[test]
    fn a_media_status_is_read_out_of_the_media_message() {
        let payload = r#"{"type":"MEDIA_STATUS","status":[{"playerState":"PLAYING","currentTime":12.5,"media":{"duration":213.4}}]}"#;
        let status = status_from_media(payload).expect("parses");
        assert_eq!(status.state, PlayerState::Playing);
        assert_eq!(status.position, 12.5);
        assert_eq!(status.duration, Some(213.4));
        assert_eq!(status.item_id, None);
    }

    #[test]
    fn the_queue_item_the_receiver_is_on_comes_out_of_the_status() {
        let payload = r#"{"type":"MEDIA_STATUS","status":[{"playerState":"PLAYING","currentTime":3.0,"currentItemId":2,"media":{}}]}"#;
        assert_eq!(status_from_media(payload).expect("parses").item_id, Some(2));
    }

    #[test]
    fn an_idle_receiver_reads_as_idle() {
        let payload = r#"{"type":"MEDIA_STATUS","status":[{"playerState":"IDLE"}]}"#;
        let status = status_from_media(payload).expect("parses");
        assert_eq!(status.state, PlayerState::Idle);
        assert_eq!(status.duration, None);
    }

    #[test]
    fn a_bigger_cover_is_the_same_picture() {
        assert_eq!(
            larger_cover("https://yt3.googleusercontent.com/abc=w120-h120-l90-rj"),
            "https://yt3.googleusercontent.com/abc=w600-h600-l90-rj"
        );
        // An address that says nothing about its size is left alone.
        assert_eq!(larger_cover("https://example.com/cover.png"), "https://example.com/cover.png");
    }

    #[test]
    fn an_unknown_player_state_is_not_a_guess() {
        assert_eq!(PlayerState::parse("SOMETHING_NEW"), PlayerState::Other);
    }

    #[test]
    fn a_queue_status_carries_the_item_and_repeat_mode() {
        let payload = r#"{"type":"MEDIA_STATUS","status":[{
            "playerState":"PLAYING","currentTime":3.5,"mediaSessionId":2,
            "media":{"duration":180.0,"contentId":"https://example.com/a","customData":{"videoId":"abc123"}},
            "repeatMode":"REPEAT_ALL","supportedMediaCommands":12303
        }]}"#;
        let status = status_from_media(payload).expect("parses");
        assert_eq!(status.video_id.as_deref(), Some("abc123"));
        assert_eq!(status.content_id.as_deref(), Some("https://example.com/a"));
        assert_eq!(status.repeat_mode.as_deref(), Some("REPEAT_ALL"));
        assert!(!status.finished);
    }

    #[test]
    fn a_status_without_custom_data_still_names_the_item_url() {
        let payload = r#"{"type":"MEDIA_STATUS","status":[{
            "playerState":"PLAYING","media":{"contentId":"https://example.com/b"}
        }]}"#;
        let status = status_from_media(payload).expect("parses");
        assert_eq!(status.video_id, None);
        assert_eq!(status.content_id.as_deref(), Some("https://example.com/b"));
    }

    #[test]
    fn a_finished_queue_reads_as_finished() {
        let payload = r#"{"type":"MEDIA_STATUS","status":[{
            "playerState":"IDLE","currentTime":180.0,"mediaSessionId":2,
            "idleReason":"FINISHED"
        }]}"#;
        let status = status_from_media(payload).expect("parses");
        assert!(status.finished);
        assert_eq!(status.state, PlayerState::Idle);
    }

    #[test]
    fn a_queue_item_carries_its_video_id_back() {
        let media = Media {
            url: "https://example.com/a".to_owned(),
            content_type: "audio/mp4".to_owned(),
            title: Some("Song".to_owned()),
            duration: Some(180.0),
            ..Default::default()
        };
        let payload = media_payload(&media, "abc123");
        assert_eq!(payload["customData"]["videoId"], "abc123");
        assert_eq!(payload["duration"], 180.0);
        // A plain LOAD carries no id.
        assert!(media_payload(&media, "").get("customData").is_none());
    }
}
