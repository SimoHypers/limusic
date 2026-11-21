//! One connection to a Cast device, talking to Google's Default Media Receiver.
//!
//! A Cast device listens on TLS (port 8009 for a single device, another port for a speaker
//! group) and speaks CastV2: a protobuf envelope ([`crate::proto`]) carrying JSON. Four
//! namespaces are used:
//!
//! - `tp.connection`: CONNECT opens a virtual connection to the platform (`receiver-0`) or to the
//!   receiver app's transport; CLOSE ends one.
//! - `tp.heartbeat`: PING/PONG. Both sides ping; a side that hears nothing drops the other.
//! - `receiver`: LAUNCH the receiver app, GET_STATUS, the speaker's volume, STOP the app.
//! - `media`: the queue and the transport.
//!
//! The socket is owned by one task. Commands go in over a channel, [`Event`]s come back over
//! another. The device pings every 5 s, so 15 s without a single frame means it is gone, and the
//! connection ends with [`Closed::Lost`] instead of waiting minutes for TCP to give up.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::rustls;

use crate::proto;

const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";

/// The device itself, before any app runs on it.
const PLATFORM: &str = "receiver-0";
/// This end of every virtual connection.
const SENDER: &str = "sender-0";

/// Google's built-in receiver. Needs no registration, shows title, artist and cover.
pub const DEFAULT_MEDIA_RECEIVER: &str = "CC1AD845";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// From the TLS handshake to the receiver app answering on its transport.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const HEARTBEAT: Duration = Duration::from_secs(5);
/// No frame for this long and the device is gone. It pings every 5 s.
const IDLE: Duration =
    if cfg!(test) { Duration::from_millis(2000) } else { Duration::from_secs(15) };
/// How long commands wait for the media session a load creates before they go out anyway.
const GATE: Duration = Duration::from_secs(8);
/// How long a queue edit waits for the item ids before it goes ahead with what it has.
const IDS_WAIT: Duration = Duration::from_secs(3);
/// A backstop for statuses the device didn't push.
const STATUS_POLL: Duration = Duration::from_secs(5);
/// How long the next song is buffered before the current one ends.
const PRELOAD_SECS: f64 = 20.0;

/// One track for the device.
#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    /// Where the device fetches it from.
    pub url: String,
    pub content_type: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Cover art. The device fetches it itself, so it must be reachable from the device.
    pub image: Option<String>,
    pub duration: Option<f64>,
}

/// The receiver's repeat mode. Repeat-all is the app's business: it wraps its own queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Off,
    One,
}

impl Repeat {
    fn wire(self) -> &'static str {
        match self {
            Repeat::Off => "REPEAT_OFF",
            Repeat::One => "REPEAT_SINGLE",
        }
    }
}

/// What the app asks of the device.
#[derive(Debug)]
pub enum Command {
    /// Replace the device's queue. `start` is the position in the first item.
    Load {
        items: Vec<Media>,
        start: f64,
        autoplay: bool,
        repeat: Repeat,
    },
    /// Add one item after the last one.
    Append(Media),
    /// Remove every item after the one playing.
    DropNext,
    /// Remove every item before the one playing.
    TrimPlayed,
    Repeat(Repeat),
    /// Jump to the next item.
    Next,
    Play,
    Pause,
    Seek(f64),
    /// The speaker's level, 0.0 to 1.0.
    Volume(f64),
    /// Stop the media and the receiver app, then close. The ack fires once it is written.
    Close(oneshot::Sender<()>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Playing,
    Paused,
    Buffering,
}

/// The media status the device reports.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaStatus {
    pub session: i64,
    pub state: State,
    /// Why the device went idle: `FINISHED`, `ERROR`, `CANCELLED`, `INTERRUPTED`.
    pub idle_reason: Option<String>,
    pub position: f64,
    pub duration: Option<f64>,
    /// The URL of the item playing. Each URL handed over is unique, so this names the item.
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The media status changed. `None`: the device has no media session.
    Media(Option<MediaStatus>),
    /// The speaker's volume, 0.0 to 1.0.
    Volume { level: f64, muted: bool },
    /// The device refused a load.
    LoadFailed(String),
    /// The connection ended. Always the last event.
    Closed(Closed),
}

/// Why a connection ended or never started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closed {
    /// It couldn't be reached at all.
    Unreachable(String),
    /// It refused to start the receiver.
    Refused(String),
    /// It stopped answering.
    Lost,
    /// Another sender started something else on it.
    TakenOver,
    /// The receiver app was stopped on the device (the Google Home app, a voice command).
    Stopped,
    /// The app closed it, or the socket ended.
    Ended(String),
}

/// The app's end of a connection. Dropping every clone closes it without stopping the device.
#[derive(Clone, Debug)]
pub struct Sender(mpsc::UnboundedSender<Command>);

impl Sender {
    pub fn send(&self, cmd: Command) {
        let _ = self.0.send(cmd);
    }

    /// Stop the device and close, waiting at most `limit` for the stop to be written.
    pub async fn close(&self, limit: Duration) {
        let (tx, rx) = oneshot::channel();
        if self.0.send(Command::Close(tx)).is_ok() {
            let _ = tokio::time::timeout(limit, rx).await;
        }
    }
}

/// Connect to a device and start (or join) the Default Media Receiver on it.
pub async fn connect(
    ip: IpAddr,
    port: u16,
) -> Result<(Sender, mpsc::UnboundedReceiver<Event>), Closed> {
    let unreachable = |e: &dyn std::fmt::Display| Closed::Unreachable(e.to_string());
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((ip, port)))
        .await
        .map_err(|e| unreachable(&e))?
        .map_err(|e| unreachable(&e))?;
    let _ = tcp.set_nodelay(true);
    let tls = tokio_rustls::TlsConnector::from(Arc::new(tls_config()));
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, tls.connect(ip.into(), tcp))
        .await
        .map_err(|e| unreachable(&e))?
        .map_err(|e| unreachable(&e))?;
    start(stream).await
}

/// Run the protocol over an open stream. Split from [`connect`] so tests can hand it any stream.
async fn start<S>(stream: S) -> Result<(Sender, mpsc::UnboundedReceiver<Event>), Closed>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    tokio::spawn(run(stream, cmd_rx, ev_tx, ready_tx));
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, ready_rx).await {
        Ok(Ok(Ok(()))) => Ok((Sender(cmd_tx), ev_rx)),
        Ok(Ok(Err(closed))) => Err(closed),
        _ => Err(Closed::Unreachable("the receiver did not answer".into())),
    }
}

/// What the socket task knows about the receiver.
struct Link<W> {
    writer: WriteHalf<W>,
    request: i64,
    /// The receiver app's session (named in payloads) and transport (the address of its
    /// connection). Both `None` until the app answers.
    session: Option<String>,
    transport: Option<String>,
    launched: bool,
    /// The media session playing; transport commands name it. 0: none.
    media_session: i64,
    /// A load is in flight: until the device reports a session other than this one, commands
    /// that name a session would name the old one, so they wait.
    gate: Option<(Instant, i64)>,
    /// The commands waiting, in order.
    waiting: Vec<Command>,
    /// The queue's item ids in order, and the one playing. Queue edits are by id.
    ids: Vec<i64>,
    current_item: Option<i64>,
    /// The ids were asked for and haven't come back.
    ids_asked: Option<Instant>,
    last_volume: Option<(f64, bool)>,
}

async fn run<S>(
    stream: S,
    mut cmds: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<Event>,
    ready: oneshot::Sender<Result<(), Closed>>,
) where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut reader, writer) = tokio::io::split(stream);
    // The reader is its own task: a read cancelled by `select!` would lose half a frame.
    let (frame_tx, mut frames) = mpsc::unbounded_channel::<Result<proto::Frame, String>>();
    let reader_task = tokio::spawn(async move {
        loop {
            let mut len = [0u8; 4];
            if let Err(e) = reader.read_exact(&mut len).await {
                let _ = frame_tx.send(Err(e.to_string()));
                return;
            }
            let len = u32::from_be_bytes(len) as usize;
            if len > proto::MAX_FRAME {
                let _ = frame_tx.send(Err("oversized frame".into()));
                return;
            }
            let mut body = vec![0u8; len];
            if let Err(e) = reader.read_exact(&mut body).await {
                let _ = frame_tx.send(Err(e.to_string()));
                return;
            }
            if let Some(frame) = proto::decode(&body) {
                if frame_tx.send(Ok(frame)).is_err() {
                    return;
                }
            }
        }
    });

    let mut link = Link {
        writer,
        request: 0,
        session: None,
        transport: None,
        launched: false,
        media_session: 0,
        gate: None,
        waiting: Vec::new(),
        ids: Vec::new(),
        current_item: None,
        ids_asked: None,
        last_volume: None,
    };
    let mut ready = Some(ready);
    let mut last_heard = Instant::now();
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let mut poll = tokio::time::interval(STATUS_POLL);

    let opened = async {
        link.write(NS_CONNECTION, PLATFORM, connect_payload()).await?;
        link.write_receiver(json!({ "type": "GET_STATUS" })).await
    };
    let closed = match opened.await {
        Err(e) => e,
        Ok(()) => loop {
            let step = tokio::select! {
                cmd = cmds.recv() => match cmd {
                    // Every sender dropped: close without stopping the device.
                    None => Err(Closed::Ended("closed by the app".into())),
                    Some(Command::Close(ack)) => {
                        link.close().await;
                        let _ = ack.send(());
                        Err(Closed::Ended("closed by the app".into()))
                    }
                    Some(cmd) => link.command(cmd).await,
                },
                frame = frames.recv() => match frame {
                    Some(Ok(frame)) => {
                        last_heard = Instant::now();
                        link.frame(frame, &events, &mut ready).await
                    }
                    Some(Err(e)) => Err(Closed::Ended(e)),
                    None => Err(Closed::Ended("the connection closed".into())),
                },
                _ = heartbeat.tick() => {
                    link.write(NS_HEARTBEAT, PLATFORM, json!({ "type": "PING" })).await
                }
                _ = tick.tick() => {
                    if last_heard.elapsed() > IDLE {
                        Err(Closed::Lost)
                    } else {
                        link.timers().await
                    }
                }
                _ = poll.tick() => link.poll().await,
            };
            if let Err(e) = step {
                break e;
            }
        },
    };
    reader_task.abort();
    match ready.take() {
        Some(ready) => {
            let _ = ready.send(Err(closed));
        }
        None => {
            let _ = events.send(Event::Closed(closed));
        }
    }
}

type Step = Result<(), Closed>;

impl<W: AsyncWrite> Link<W> {
    async fn write(&mut self, ns: &str, to: &str, mut payload: Value) -> Step {
        self.request += 1;
        payload["requestId"] = json!(self.request);
        let msg = proto::frame(ns, SENDER, to, &payload.to_string());
        self.writer.write_all(&msg).await.map_err(|e| Closed::Ended(e.to_string()))
    }

    async fn write_receiver(&mut self, payload: Value) -> Step {
        self.write(NS_RECEIVER, PLATFORM, payload).await
    }

    /// A media message to the receiver app. Dropped when there is no app yet.
    async fn write_media(&mut self, payload: Value) -> Step {
        let Some(to) = self.transport.clone() else { return Ok(()) };
        self.write(NS_MEDIA, &to, payload).await
    }

    /// A transport command naming the media session playing.
    async fn write_session(&mut self, mut payload: Value) -> Step {
        payload["mediaSessionId"] = json!(self.media_session);
        self.write_media(payload).await
    }

    async fn command(&mut self, cmd: Command) -> Step {
        match cmd {
            Command::Volume(level) => {
                self.write_receiver(json!({
                    "type": "SET_VOLUME", "volume": { "level": level.clamp(0.0, 1.0) },
                }))
                .await
            }
            Command::Load { .. } if self.gate.is_some() || self.transport.is_none() => {
                // The queue is the truth: whatever waited belongs to the queue this replaces.
                self.waiting.clear();
                self.waiting.push(cmd);
                Ok(())
            }
            Command::Load { items, start, autoplay, repeat } => {
                let items: Vec<Value> = items
                    .iter()
                    .enumerate()
                    .map(|(i, m)| queue_item(m, i > 0 || autoplay))
                    .collect();
                let session = self.session.clone().unwrap_or_default();
                self.write_media(json!({
                    "type": "QUEUE_LOAD", "sessionId": session, "items": items, "startIndex": 0,
                    "currentTime": start, "repeatMode": repeat.wire(),
                }))
                .await?;
                self.gate = Some((Instant::now() + GATE, self.media_session));
                self.ids.clear();
                self.current_item = None;
                if !autoplay {
                    // In case the receiver ignores the item's `autoplay`: a pause on a paused
                    // session changes nothing.
                    self.waiting.push(Command::Pause);
                }
                Ok(())
            }
            cmd if self.gate.is_some() => {
                self.wait(cmd);
                Ok(())
            }
            // Queue edits keep their order behind ones that wait for ids; the transport doesn't
            // have to, or a pause would sit behind a reorder for seconds.
            cmd if !self.waiting.is_empty() && !is_transport(&cmd) => {
                self.wait(cmd);
                Ok(())
            }
            // Nothing is loaded: there is nothing for a transport command to act on.
            _ if self.media_session == 0 => Ok(()),
            cmd @ (Command::DropNext | Command::TrimPlayed) if self.ids_asked.is_some() => {
                self.wait(cmd);
                Ok(())
            }
            Command::DropNext => {
                let at = self.playing_index();
                let after: Vec<i64> = at.map(|i| self.ids[i + 1..].to_vec()).unwrap_or_default();
                self.remove(after).await
            }
            Command::TrimPlayed => {
                let at = self.playing_index();
                let before: Vec<i64> = at.map(|i| self.ids[..i].to_vec()).unwrap_or_default();
                self.remove(before).await
            }
            Command::Append(media) => {
                self.write_session(
                    json!({ "type": "QUEUE_INSERT", "items": [queue_item(&media, true)] }),
                )
                .await?;
                self.ask_ids().await
            }
            Command::Repeat(repeat) => {
                self.write_session(json!({ "type": "QUEUE_UPDATE", "repeatMode": repeat.wire() }))
                    .await
            }
            Command::Next => self.write_session(json!({ "type": "QUEUE_UPDATE", "jump": 1 })).await,
            Command::Play => self.write_session(json!({ "type": "PLAY" })).await,
            Command::Pause => self.write_session(json!({ "type": "PAUSE" })).await,
            Command::Seek(at) => {
                self.write_session(json!({ "type": "SEEK", "currentTime": at.max(0.0) })).await
            }
            Command::Close(_) => Ok(()), // handled by the loop
        }
    }

    /// Hold a command, keeping only the latest of a kind where an older one means nothing.
    fn wait(&mut self, cmd: Command) {
        let same = |c: &Command| {
            matches!(
                (&cmd, c),
                (Command::Play | Command::Pause, Command::Play | Command::Pause)
                    | (Command::Seek(_), Command::Seek(_))
                    | (Command::Repeat(_), Command::Repeat(_))
            )
        };
        self.waiting.retain(|c| !same(c));
        self.waiting.push(cmd);
    }

    fn playing_index(&self) -> Option<usize> {
        let current = self.current_item?;
        self.ids.iter().position(|id| *id == current)
    }

    async fn remove(&mut self, ids: Vec<i64>) -> Step {
        if ids.is_empty() {
            return Ok(());
        }
        self.write_session(json!({ "type": "QUEUE_REMOVE", "itemIds": ids })).await?;
        self.ask_ids().await
    }

    async fn ask_ids(&mut self) -> Step {
        self.ids_asked = Some(Instant::now());
        self.write_session(json!({ "type": "QUEUE_GET_ITEM_IDS" })).await
    }

    /// Send what waited, in order, until something has to wait again.
    async fn flush(&mut self) -> Step {
        let waiting = std::mem::take(&mut self.waiting);
        let mut rest = waiting.into_iter();
        while let Some(cmd) = rest.next() {
            let needs_ids = matches!(cmd, Command::DropNext | Command::TrimPlayed);
            if self.gate.is_some() || (needs_ids && self.ids_asked.is_some()) {
                // Keep the order: this and what follows wait behind what just went out.
                self.waiting.push(cmd);
                self.waiting.extend(rest);
                break;
            }
            self.command(cmd).await?;
        }
        Ok(())
    }

    async fn timers(&mut self) -> Step {
        let now = Instant::now();
        if self.gate.is_some_and(|(until, _)| now >= until) {
            tracing::debug!("cast: the device never reported the load's session");
            self.gate = None;
        }
        if self.ids_asked.is_some_and(|at| now >= at + IDS_WAIT) {
            self.ids_asked = None;
        }
        if self.gate.is_none() && self.ids_asked.is_none() && !self.waiting.is_empty() {
            self.flush().await?;
        }
        Ok(())
    }

    async fn poll(&mut self) -> Step {
        if self.transport.is_some() {
            self.write_media(json!({ "type": "GET_STATUS" })).await?;
        }
        self.write_receiver(json!({ "type": "GET_STATUS" })).await
    }

    async fn close(&mut self) {
        if self.media_session != 0 {
            let _ = self.write_session(json!({ "type": "STOP" })).await;
        }
        if let Some(session) = self.session.clone() {
            let _ = self.write_receiver(json!({ "type": "STOP", "sessionId": session })).await;
        }
        if let Some(to) = self.transport.clone() {
            let _ = self.write(NS_CONNECTION, &to, json!({ "type": "CLOSE" })).await;
        }
        let _ = self.writer.flush().await;
    }

    async fn frame(
        &mut self,
        frame: proto::Frame,
        events: &mpsc::UnboundedSender<Event>,
        ready: &mut Option<oneshot::Sender<Result<(), Closed>>>,
    ) -> Step {
        let Ok(msg) = serde_json::from_str::<Value>(&frame.payload) else { return Ok(()) };
        let kind = msg.get("type").and_then(Value::as_str).unwrap_or_default();
        match (frame.namespace.as_str(), kind) {
            (NS_HEARTBEAT, "PING") => {
                self.write(NS_HEARTBEAT, &frame.source, json!({ "type": "PONG" })).await
            }
            (NS_CONNECTION, "CLOSE") if Some(&frame.source) == self.transport.as_ref() => {
                Err(Closed::TakenOver)
            }
            (NS_RECEIVER, "RECEIVER_STATUS") => self.receiver_status(&msg, events, ready).await,
            (NS_RECEIVER, "LAUNCH_ERROR") => {
                let reason = msg.get("reason").and_then(Value::as_str).unwrap_or("LAUNCH_ERROR");
                Err(Closed::Refused(reason.to_owned()))
            }
            (NS_MEDIA, "MEDIA_STATUS") => {
                let entry = msg.get("status").and_then(Value::as_array).and_then(|s| s.first());
                let status = entry.and_then(media_status);
                if let Some(entry) = entry {
                    self.current_item = entry.get("currentItemId").and_then(Value::as_i64);
                    // Only a fallback for a receiver that never answers QUEUE_GET_ITEM_IDS: the
                    // list in a status isn't promised to be the whole queue.
                    if self.ids.is_empty() {
                        if let Some(items) = entry.get("items").and_then(Value::as_array) {
                            self.ids = items.iter().filter_map(|i| i["itemId"].as_i64()).collect();
                        }
                    }
                }
                let session = status.as_ref().map_or(0, |s| s.session);
                self.media_session = session;
                if self.gate.is_some_and(|(_, old)| session != 0 && session != old) {
                    self.gate = None;
                    self.ask_ids().await?;
                }
                let _ = events.send(Event::Media(status));
                Ok(())
            }
            (NS_MEDIA, "QUEUE_ITEM_IDS") => {
                if let Some(ids) = msg.get("itemIds").and_then(Value::as_array) {
                    self.ids = ids.iter().filter_map(Value::as_i64).collect();
                }
                self.ids_asked = None;
                if self.gate.is_none() {
                    self.flush().await?;
                }
                Ok(())
            }
            (NS_MEDIA, "QUEUE_CHANGE") => self.ask_ids().await,
            (NS_MEDIA, "LOAD_FAILED" | "LOAD_CANCELLED" | "INVALID_REQUEST") => {
                let reason = msg.get("reason").and_then(Value::as_str).unwrap_or(kind);
                tracing::warn!(kind, reason, "cast: the device refused a request");
                // A command against a session that just ended is answered this way and means
                // nothing, and a cancelled load was replaced or stopped on purpose. A refusal
                // while a load is in flight is that load.
                let stale = reason == "INVALID_MEDIA_SESSION_ID" || kind == "LOAD_CANCELLED";
                let was_loading = self.gate.take().is_some();
                if !stale && (was_loading || kind == "LOAD_FAILED") {
                    self.waiting.retain(|c| matches!(c, Command::Load { .. }));
                    let _ = events.send(Event::LoadFailed(reason.to_owned()));
                }
                self.flush().await
            }
            _ => Ok(()),
        }
    }

    async fn receiver_status(
        &mut self,
        msg: &Value,
        events: &mpsc::UnboundedSender<Event>,
        ready: &mut Option<oneshot::Sender<Result<(), Closed>>>,
    ) -> Step {
        let status = &msg["status"];
        if let Some(level) = status["volume"]["level"].as_f64() {
            let muted = status["volume"]["muted"].as_bool().unwrap_or(false);
            if self.last_volume != Some((level, muted)) {
                self.last_volume = Some((level, muted));
                let _ = events.send(Event::Volume { level, muted });
            }
        }
        // No application list: nothing is running on the device.
        let none = Vec::new();
        let apps = status.get("applications").and_then(Value::as_array).unwrap_or(&none);
        let ours = apps
            .iter()
            .find(|a| a["appId"].as_str() == Some(DEFAULT_MEDIA_RECEIVER))
            .and_then(|a| Some((a["sessionId"].as_str()?, a["transportId"].as_str()?)));
        match (ours, self.session.as_deref()) {
            // Still running, the same session.
            (Some((session, _)), Some(known)) if session == known => Ok(()),
            // Stopped, with nothing in its place.
            (None, Some(_)) if apps.is_empty() => Err(Closed::Stopped),
            // Replaced by another app, or by another sender's session.
            (_, Some(_)) => Err(Closed::TakenOver),
            // Running before this connection (left from an earlier one, or another sender's):
            // join it rather than launching it again.
            (Some((session, transport)), None) => {
                self.session = Some(session.to_owned());
                self.transport = Some(transport.to_owned());
                self.write(NS_CONNECTION, transport, connect_payload()).await?;
                self.write_media(json!({ "type": "GET_STATUS" })).await?;
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Ok(()));
                }
                self.flush().await
            }
            (None, None) => self.launch().await,
        }
    }

    async fn launch(&mut self) -> Step {
        if self.launched {
            return Ok(());
        }
        self.launched = true;
        self.write_receiver(json!({ "type": "LAUNCH", "appId": DEFAULT_MEDIA_RECEIVER })).await
    }
}

/// Commands that act on the playing session and not on the queue.
fn is_transport(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Play | Command::Pause | Command::Seek(_) | Command::Repeat(_) | Command::Volume(_)
    )
}

/// The CONNECT payload. `userAgent` names the sender; it can't rename the receiver app, which
/// Google calls "Default Media Receiver".
fn connect_payload() -> Value {
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
}

fn queue_item(media: &Media, autoplay: bool) -> Value {
    let mut metadata = json!({ "metadataType": 3, "title": media.title, "artist": media.artist });
    if let Some(album) = &media.album {
        metadata["albumName"] = json!(album);
    }
    if let Some(image) = &media.image {
        metadata["images"] = json!([{ "url": image }]);
    }
    let mut info = json!({
        "contentId": media.url,
        "contentUrl": media.url,
        "contentType": media.content_type,
        "streamType": "BUFFERED",
        "metadata": metadata,
    });
    // A null duration reads as zero on the receiver, which then thinks the track is over.
    if let Some(d) = media.duration.filter(|d| *d > 0.0) {
        info["duration"] = json!(d);
    }
    json!({ "media": info, "autoplay": autoplay, "preloadTime": PRELOAD_SECS })
}

/// One entry of a MEDIA_STATUS.
fn media_status(entry: &Value) -> Option<MediaStatus> {
    let state = match entry.get("playerState")?.as_str()? {
        "PLAYING" => State::Playing,
        "PAUSED" => State::Paused,
        "BUFFERING" => State::Buffering,
        _ => State::Idle,
    };
    Some(MediaStatus {
        session: entry.get("mediaSessionId").and_then(Value::as_i64).unwrap_or(0),
        state,
        idle_reason: entry.get("idleReason").and_then(Value::as_str).map(str::to_owned),
        position: entry.get("currentTime").and_then(Value::as_f64).unwrap_or(0.0),
        duration: entry["media"]["duration"].as_f64(),
        content_id: entry["media"]["contentId"].as_str().map(str::to_owned),
    })
}

/// The device's certificate is self-signed, so there is nothing to check it against. The
/// device is found on the local network and identified by its mDNS id.
fn tls_config() -> rustls::ClientConfig {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    #[derive(Debug)]
    struct AnyCertificate(Arc<rustls::crypto::CryptoProvider>);

    impl ServerCertVerifier for AnyCertificate {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring speaks TLS 1.2 and 1.3")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
        .with_no_client_auth()
}

#[cfg(test)]
mod tests;
