//! Loopback HTTP proxy for the *audio* stream: mpv fetches its bytes from here, never straight from
//! googlevideo.
//!
//! **Why this exists.** googlevideo throttles a *streaming* download (the open-ended
//! `Range: bytes=X-`, or a plain GET, that ffmpeg sends) to roughly 2x realtime. A *bounded* range
//! (`bytes=X-Y`) is served at full speed instead: measured against the same URL, the middle of a
//! 60 MB mix came back at 32 KB/s open-ended and 9.4 MB/s for a 4 MiB bounded range, and a whole
//! 122 MB mix pulled in chunks took 51 s (~2.4 MB/s) instead of hours. So the throttle is on the *request shape*, not the client: VISIONOS throttles exactly
//! like WEB_REMIX, and the URLs carry no `n`-parameter to blame.
//!
//! ffmpeg's HTTP source cannot be told to chunk its reads. This module can: mpv opens
//! `http://127.0.0.1:<ephemeral>/<token>/a/<id>`, and the proxy answers its open-ended request by
//! fetching bounded chunks upstream and streaming them back. Seeks become new loopback requests
//! that are served at full speed (a few tens of ms for the first ~40 KB a WebM needs to restart)
//! instead of waiting ~1.3 s for the same bytes at 2x realtime, and mpv's forward cache fills in
//! seconds rather than at playback rate.
//!
//! Same shape as [`crate::videoproxy`]: loopback-only, a random per-launch token in the path so no
//! other local process can drive it, and upstream headers (User-Agent, and the cookie an upload
//! needs) attached here rather than by mpv.
//!
//! Set `LIMUSIC_NO_AUDIO_PROXY=1` to fall back to handing mpv the googlevideo URL directly.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::io;
use std::net::{Ipv4Addr, TcpListener};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Empty, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::sync::mpsc;

/// Streamed, never buffered: only one upstream chunk lives in RAM at a time, so a two-hour mix
/// costs the same few MB as a three-minute song.
type ProxyBody = BoxBody<Bytes, io::Error>;

/// How much of the file to ask upstream for in one range request. Large enough to amortise the
/// per-request round trip and land in the "served at full speed" regime, small enough that a seek
/// abandons little work when mpv drops the connection.
const CHUNK: u64 = 4 * 1024 * 1024;

/// How long one upstream read may make no progress before the chunk is declared dead.
///
/// It has to be here: `http::client()` sets no timeout on purpose (they are per call site) and
/// reqwest's read timeout is client-wide, so nothing else bounds this. Without it a connection that
/// goes quiet without closing hangs the pump forever, and mpv is left holding a loopback socket
/// that neither errors nor delivers a byte, with no track-failed event and so no recovery: the
/// silent stall issue #188 is about, moved inside our own process.
///
/// A stall timeout, not a total one. A whole-request deadline would have to be generous enough for
/// a 4 MiB chunk on a slow connection, which is long enough to be useless as a hang guard; this
/// fires on no progress, so a slow-but-moving download is never cut off.
const STALL: Duration = Duration::from_secs(20);

/// `send()` (or one body read) with the stall guard on it.
pub(crate) async fn no_stall<F: std::future::Future<Output = T>, T>(f: F) -> Result<T, io::Error> {
    tokio::time::timeout(STALL, f).await.map_err(|_| io::Error::other("upstream stalled"))
}

/// Keep at most this many stream URLs registered. A registration is the URL plus its headers (a
/// cookie on an upload), so this is a few KB; the FIFO only has to be deeper than the current and
/// gapless-next tracks, which are always the two most recent.
const MAX_REGISTRATIONS: usize = 64;

/// The running proxy.
struct Proxy {
    /// The loopback listener's port.
    port: u16,
    /// The bearer credential in loopback URLs. It is random per launch.
    token: String,
    /// The LAN listener's `(port, token)`, started when the first cast needs it. Its token is
    /// separate from the loopback one, and it is only ever handed to a device the user picked.
    lan: Mutex<Option<(u16, String)>>,
}

/// What a listener may serve. The LAN listener is deliberately narrower than the loopback one.
#[derive(Clone, Copy)]
enum ListenerScope {
    Loopback,
    Lan,
}

impl ListenerScope {
    fn allows(self, kind: &str) -> bool {
        match self {
            Self::Loopback => matches!(kind, "a" | "k"),
            Self::Lan => kind == "a",
        }
    }
}

/// The running proxy's address. Set once at startup; unset if the bind failed, which just means
/// [`register`] returns `None` and callers keep the direct URL.
static ENDPOINT: OnceLock<Proxy> = OnceLock::new();

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

/// What a resolved URL answers: its total length and content type. Learned from the first chunk
/// response's `Content-Range` and reused for every later connection, so a seek does not pay a
/// separate round trip just to learn the size.
#[derive(Clone)]
struct Meta {
    total: u64,
    content_type: Option<String>,
}

struct Registration {
    url: String,
    headers: HashMap<String, String>,
    meta: Mutex<Option<Meta>>,
}

struct Registry {
    map: HashMap<String, Arc<Registration>>,
    order: VecDeque<String>,
}

/// Bind the loopback listener and start serving. Binds synchronously so [`register`] is usable the
/// moment this returns, then hands the socket to tokio.
pub fn start() {
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "audio proxy: bind failed (streams go direct)");
            return;
        }
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            tracing::warn!(error = %e, "audio proxy: local_addr failed");
            return;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        tracing::warn!(error = %e, "audio proxy: set_nonblocking failed");
        return;
    }
    let token = format!("{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>());
    if ENDPOINT.set(Proxy { port, token, lan: Mutex::new(None) }).is_err() {
        return; // already started
    }
    let _ = REGISTRY.set(Mutex::new(Registry { map: HashMap::new(), order: VecDeque::new() }));
    tracing::info!(port, "audio proxy listening on loopback");

    tauri::async_runtime::spawn(accept_loop(listener, ListenerScope::Loopback));
}

/// Accept connections and hand each one to the server.
async fn accept_loop(listener: TcpListener, scope: ListenerScope) {
    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return };
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                // EMFILE/ENFILE/ENOBUFS return immediately, so `continue` alone would spin.
                tracing::warn!(error = %e, "audio proxy: accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        tracing::debug!(target: "app_lib::audioproxy", %peer, "audio proxy: a client connected");
        tauri::async_runtime::spawn(async move {
            let svc = service_fn(move |req| serve(req, scope));
            let _ = http1::Builder::new()
                // hyper 1.x panics when it arms a timeout without a timer installed.
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(15))
                .serve_connection(TokioIo::new(stream), svc)
                .await;
        });
    }
}

/// Register one YouTube stream URL (with the headers it needs) and return the loopback URL mpv
/// should be handed. `None` means the direct URL must be used: the proxy never came up, the
/// kill-switch is set, or the URL is not one of YouTube's stream hosts (an RSS-feed podcast's
/// enclosure is not throttled, and may not honour ranges, #294).
pub fn register(url: &str, headers: &HashMap<String, String>) -> Option<String> {
    if !crate::orchestrator::is_youtube_stream(url)
        || std::env::var_os("LIMUSIC_NO_AUDIO_PROXY").is_some()
    {
        return None;
    }
    let proxy = ENDPOINT.get()?;
    let id = register_id(url, headers)?;
    Some(format!("http://127.0.0.1:{}/{}/a/{}", proxy.port, proxy.token, id))
}

/// Register one YouTube stream URL for a device on the LAN, and return the URL that device can
/// fetch.
///
/// This is the renderer path. A DLNA renderer speaks `http://` and not `https://`, and the
/// stream URL needs the headers this app signed it with, so the bytes have to come through this
/// proxy rather than straight from googlevideo. The LAN listener is bound on first use and
/// carries a token of its own: only the device the user picked is ever given a URL with it.
///
/// `extension` is the file extension the URL ends with. A renderer that guesses the format
/// from the address rather than from the metadata gets the right one this way.
pub fn register_lan(
    url: &str,
    headers: &HashMap<String, String>,
    address: std::net::Ipv4Addr,
    extension: &str,
) -> Option<String> {
    if !crate::orchestrator::is_youtube_stream(url)
        || std::env::var_os("LIMUSIC_NO_AUDIO_PROXY").is_some()
    {
        return None;
    }
    let (port, token) = lan_endpoint()?;
    let id = register_id(url, headers)?;
    Some(format!("http://{address}:{port}/{token}/a/{id}.{extension}"))
}

/// The LAN listener's `(port, token)`, started on first use.
fn lan_endpoint() -> Option<(u16, String)> {
    let proxy = ENDPOINT.get()?;
    let mut slot = proxy.lan.lock().ok()?;
    if let Some((port, token)) = slot.as_ref() {
        return Some((*port, token.clone()));
    }
    let listener = match TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "audio proxy: the LAN bind failed");
            return None;
        }
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            tracing::warn!(error = %e, "audio proxy: local_addr failed");
            return None;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        tracing::warn!(error = %e, "audio proxy: set_nonblocking failed");
        return None;
    }
    let token = format!("{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>());
    tracing::info!(port, "audio proxy listening on the LAN for cast targets");
    tauri::async_runtime::spawn(accept_loop(listener, ListenerScope::Lan));
    *slot = Some((port, token.clone()));
    Some((port, token))
}

/// Register a stream URL and return its id.
fn register_id(url: &str, headers: &HashMap<String, String>) -> Option<String> {
    let id = format!("{:016x}", rand::random::<u64>());
    let reg = Arc::new(Registration {
        url: url.to_owned(),
        headers: headers.clone(),
        meta: Mutex::new(None),
    });
    let mut r = REGISTRY.get()?.lock().ok()?;
    while r.order.len() >= MAX_REGISTRATIONS {
        if let Some(old) = r.order.pop_front() {
            r.map.remove(&old);
        }
    }
    r.map.insert(id.clone(), reg);
    r.order.push_back(id.clone());
    Some(id)
}

/// `/<token>/<kind>/<id>` to `(kind, id)`, once the token matches one of this launch's.
fn route<'a>(path: &'a str, tokens: &[String]) -> Option<(&'a str, &'a str)> {
    let (t, rest) = path.strip_prefix('/')?.split_once('/')?;
    let (kind, id) = rest.split_once('/')?;
    (tokens.iter().any(|known| known == t) && !id.is_empty()).then_some((kind, id))
}

/// The token this listener accepts. Each listener has its own credential and route scope.
fn known_tokens(scope: ListenerScope) -> Vec<String> {
    let Some(proxy) = ENDPOINT.get() else { return Vec::new() };
    match scope {
        ListenerScope::Loopback => vec![proxy.token.clone()],
        ListenerScope::Lan => {
            let Ok(slot) = proxy.lan.lock() else { return Vec::new() };
            slot.as_ref().map(|(_, token)| vec![token.clone()]).unwrap_or_default()
        }
    }
}

/// `/<token>/a/<id>` to the registration id. The id may carry the file extension a cast device
/// needs (see [`register_lan`]); it is not part of the id.
fn id_from<'a>(path: &'a str, tokens: &[String]) -> Option<&'a str> {
    let id = route(path, tokens).filter(|(kind, _)| *kind == "a").map(|(_, id)| id)?;
    Some(strip_extension(id))
}

/// The file extension off the end of an id, if it has one.
fn strip_extension(id: &str) -> &str {
    for extension in [".m4a", ".mp4", ".mp3", ".webm", ".aac", ".ogg", ".wav"] {
        if let Some(stripped) = id.strip_suffix(extension) {
            return stripped;
        }
    }
    id
}

/// Karaoke instrumentals still being written (`/<token>/k/<id>`): the song that is playing, served
/// while the rest of it is prepared (`karaoke::KaraokeStream`).
static GROWING: OnceLock<Mutex<HashMap<String, Arc<karaoke::stream::GrowingWav>>>> =
    OnceLock::new();

/// The loopback URL mpv plays `wav` from, or `None` if the proxy never came up.
pub fn register_growing(wav: Arc<karaoke::stream::GrowingWav>) -> Option<String> {
    let proxy = ENDPOINT.get()?;
    let id = format!("{:016x}", rand::random::<u64>());
    GROWING.get_or_init(Default::default).lock().ok()?.insert(id.clone(), wav);
    Some(format!("http://127.0.0.1:{}/{}/k/{}", proxy.port, proxy.token, id))
}

/// Forget a stream registered by [`register_growing`]. A connection already reading keeps it.
pub fn unregister_growing(url: &str) {
    if let (Some(id), Some(m)) = (url.rsplit('/').next(), GROWING.get()) {
        if let Ok(mut m) = m.lock() {
            m.remove(id);
        }
    }
}

/// Serves a growing WAV. Bytes not written yet are waited for (the separation is outrunning
/// playback, or playback would not be on it), until the job is abandoned or mpv hangs up.
async fn serve_growing(
    req: &Request<Incoming>,
    id: &str,
) -> Result<Response<ProxyBody>, StatusCode> {
    let wav = GROWING
        .get()
        .and_then(|m| m.lock().ok().and_then(|m| m.get(id).cloned()))
        .ok_or(StatusCode::NOT_FOUND)?;
    let total = wav.len();
    let want = parse_range(req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()));
    let Some((start, end)) = resolve_window(want, total) else {
        let mut r = empty(StatusCode::RANGE_NOT_SATISFIABLE);
        if let Ok(v) = format!("bytes */{total}").parse() {
            r.headers_mut().insert(header::CONTENT_RANGE, v);
        }
        return Ok(r);
    };
    let ranged = !matches!(want, Want::Full);
    tracing::debug!(target: "app_lib::karaoke", start, end, total, method = %req.method(), "karaoke stream request");
    let mut builder = Response::builder()
        .status(if ranged { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK }.as_u16())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, end - start + 1)
        .header(header::CONTENT_TYPE, "audio/wav");
    if ranged {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"));
    }
    let body = if req.method() == Method::HEAD {
        Empty::<Bytes>::new().map_err(|e| match e {}).boxed()
    } else {
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(8);
        tauri::async_runtime::spawn(async move { pump_growing(wav, start, end, tx).await });
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item.map(Frame::data), rx))
        });
        BodyExt::boxed(StreamBody::new(stream))
    };
    builder.body(body).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn pump_growing(
    wav: Arc<karaoke::stream::GrowingWav>,
    start: u64,
    end: u64,
    tx: mpsc::Sender<Result<Bytes, io::Error>>,
) {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut file = match tokio::fs::File::open(wav.path()).await {
        Ok(f) => f,
        Err(e) => {
            let _ = tx.send(Err(e)).await;
            return;
        }
    };
    let mut buf = vec![0u8; 256 * 1024];
    let mut pos = start;
    let mut waited = false;
    while pos <= end {
        let ready = wav.servable_from(pos);
        if ready == 0 {
            if wav.is_abandoned() || tx.is_closed() {
                return;
            }
            if !waited {
                waited = true;
                tracing::debug!(target: "app_lib::karaoke", at = pos, "karaoke stream: waiting for bytes");
            }
            // ponytail: a 50 ms poll, not a wakeup per write. Chunks land seconds apart.
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        }
        let n = ready.min(buf.len() as u64).min(end - pos + 1) as usize;
        let read = async {
            file.seek(io::SeekFrom::Start(pos)).await?;
            file.read_exact(&mut buf[..n]).await
        };
        if let Err(e) = read.await {
            let _ = tx.send(Err(e)).await;
            return;
        }
        if tx.send(Ok(Bytes::copy_from_slice(&buf[..n]))).await.is_err() {
            return;
        }
        pos += n as u64;
    }
}

fn empty(status: StatusCode) -> Response<ProxyBody> {
    let mut r = Response::new(Empty::<Bytes>::new().map_err(|e| match e {}).boxed());
    *r.status_mut() = status;
    r
}

async fn serve(
    req: Request<Incoming>,
    scope: ListenerScope,
) -> Result<Response<ProxyBody>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let answer = handle(req, scope).await;
    match &answer {
        Ok(r) => {
            tracing::debug!(target: "app_lib::audioproxy", %method, %path, status = r.status().as_u16(), "audio proxy: answered")
        }
        Err(e) => {
            tracing::debug!(target: "app_lib::audioproxy", %method, %path, status = e.as_u16(), "audio proxy: refused")
        }
    }
    Ok(answer.unwrap_or_else(empty))
}

async fn handle(
    req: Request<Incoming>,
    scope: ListenerScope,
) -> Result<Response<ProxyBody>, StatusCode> {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    let tokens = known_tokens(scope);
    if tokens.is_empty() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    if let Some(("k", id)) = route(req.uri().path(), &tokens) {
        if scope.allows("k") {
            return serve_growing(&req, id).await;
        }
        return Err(StatusCode::NOT_FOUND);
    }
    let id = id_from(req.uri().path(), &tokens).ok_or(StatusCode::NOT_FOUND)?;
    if !scope.allows("a") {
        return Err(StatusCode::NOT_FOUND);
    }
    let reg = {
        let r = REGISTRY
            .get()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?
            .lock()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        r.map.get(id).cloned()
    }
    .ok_or(StatusCode::NOT_FOUND)?;

    let meta = meta_for(&reg).await?;

    let want = parse_range(req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()));
    let Some((start, end)) = resolve_window(want, meta.total) else {
        // The only malformed-but-well-formed case: a range that starts past the end of the file.
        let mut r = empty(StatusCode::RANGE_NOT_SATISFIABLE);
        if let Ok(v) = format!("bytes */{}", meta.total).parse() {
            r.headers_mut().insert(header::CONTENT_RANGE, v);
        }
        return Ok(r);
    };

    let ranged = !matches!(want, Want::Full);
    let status = if ranged { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
    let len = end - start + 1;

    let mut builder = Response::builder()
        .status(status.as_u16())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, len);
    if let Some(ct) = meta.content_type.as_deref() {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    if ranged {
        builder =
            builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{}", meta.total));
    }

    let body = if req.method() == Method::HEAD {
        Empty::<Bytes>::new().map_err(|e| match e {}).boxed()
    } else {
        // Pull upstream in a task and hand the bytes over a bounded channel: the client body then
        // holds only the receiver (Send + Sync, which `boxed` requires), and when mpv drops the
        // connection the receiver drops, the send fails, and the task stops fetching.
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
        let pump = reg.clone();
        tauri::async_runtime::spawn(async move { pump_chunks(pump, start, end, tx).await });
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item.map(Frame::data), rx))
        });
        BodyExt::boxed(StreamBody::new(stream))
    };
    builder.body(body).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// The cached [`Meta`], probing upstream once per registration if nothing has learned it yet.
async fn meta_for(reg: &Registration) -> Result<Meta, StatusCode> {
    if let Some(m) = reg.meta.lock().ok().and_then(|g| g.clone()) {
        return Ok(m);
    }
    let m = probe(reg).await?;
    if let Ok(mut g) = reg.meta.lock() {
        *g = Some(m.clone());
    }
    Ok(m)
}

/// Ask upstream for a single byte (`bytes=0-0`) just to read the total length and content type off
/// the answer. `meta_for` keeps the result, so only the first connection to a registration pays it.
async fn probe(reg: &Registration) -> Result<Meta, StatusCode> {
    let resp = upstream_get(reg, 0, 0).await?;
    let status = resp.status();
    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        let total = resp
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(total_from_content_range)
            .ok_or(StatusCode::BAD_GATEWAY)?;
        Ok(Meta { total, content_type: content_type(&resp) })
    } else {
        // Anything but a 206 is a dead end here, including a 200 from a server that ignored the
        // range: every byte this proxy serves comes from a bounded range (`pump_chunks` accepts
        // only 206), so taking the whole-file answer would just move the failure from this handled
        // spot to an I/O error mid-body. Only googlevideo URLs are ever registered and those
        // honour ranges, so in practice this is an expired URL (they last ~6h) or a refusal. mpv
        // errors and the app re-resolves.
        tracing::debug!(status = %status, "audio proxy: upstream refused");
        Err(StatusCode::BAD_GATEWAY)
    }
}

/// A bounded upstream range request with the registration's headers. Range and content-encoding do
/// not mix: without the explicit `identity` reqwest offers gzip/br and the server could answer 200
/// with an encoded whole file, which is exactly the throttled shape this proxy exists to avoid.
fn upstream_request(reg: &Registration, start: u64, end: u64) -> reqwest::RequestBuilder {
    let mut req = crate::http::client()
        .get(&reg.url)
        .header(header::RANGE.as_str(), format!("bytes={start}-{end}"))
        .header(header::ACCEPT_ENCODING.as_str(), "identity");
    for (k, v) in &reg.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    req
}

async fn upstream_get(
    reg: &Registration,
    start: u64,
    end: u64,
) -> Result<reqwest::Response, StatusCode> {
    let sent = no_stall(upstream_request(reg, start, end).send()).await.map_err(|e| {
        tracing::debug!(error = %e, "audio proxy: upstream stalled");
        StatusCode::GATEWAY_TIMEOUT
    })?;
    sent.map_err(|e| {
        tracing::debug!(error = %e, "audio proxy: upstream failed");
        StatusCode::BAD_GATEWAY
    })
}

fn content_type(resp: &reqwest::Response) -> Option<String> {
    resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

fn total_from_content_range(value: &str) -> Option<u64> {
    // `bytes 0-0/122414474`, or `bytes 0-0/*` when the length is unknown.
    value.rsplit_once('/')?.1.trim().parse().ok()
}

/// The last byte to ask for in a chunk that starts at `pos`, clamped to the window's last byte.
fn chunk_end(pos: u64, end: u64) -> u64 {
    (pos + CHUNK - 1).min(end)
}

/// Walk `pos` to `end` inclusive in bounded upstream ranges, forwarding each response's bytes into
/// the channel. Ends when the window is done or the receiver is gone (a seek dropped the
/// connection), which is also what cancels the in-flight upstream request.
async fn pump_chunks(
    reg: Arc<Registration>,
    start: u64,
    end: u64,
    tx: mpsc::Sender<Result<Bytes, io::Error>>,
) {
    let mut pos = start;
    while pos <= end {
        let last = chunk_end(pos, end);
        let resp = match no_stall(upstream_request(&reg, pos, last).send()).await {
            Ok(Ok(r)) if r.status() == reqwest::StatusCode::PARTIAL_CONTENT => r,
            Ok(Ok(r)) => {
                let _ = tx
                    .send(Err(io::Error::other(format!(
                        "audio proxy: upstream status {}",
                        r.status()
                    ))))
                    .await;
                return;
            }
            Ok(Err(e)) => {
                let _ = tx.send(Err(io::Error::other(e))).await;
                return;
            }
            Err(e) => {
                let _ = tx.send(Err(e)).await;
                return;
            }
        };
        pos = last + 1;
        let mut body = resp.bytes_stream();
        loop {
            let item = match no_stall(body.next()).await {
                Ok(Some(item)) => item.map_err(io::Error::other),
                Ok(None) => break,
                Err(e) => Err(e),
            };
            let failed = item.is_err();
            if tx.send(item).await.is_err() {
                return; // client gone, stop pulling and drop the connection upstream
            }
            if failed {
                return;
            }
        }
    }
}

/// What the client asked for, once parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Want {
    /// No `Range` header, or one this proxy does not understand: the whole file.
    Full,
    /// `bytes=X-`
    From(u64),
    /// `bytes=X-Y`
    Bounded(u64, u64),
    /// `bytes=-N` (the last N bytes)
    Suffix(u64),
}

fn parse_range(value: Option<&str>) -> Want {
    let Some(v) = value else { return Want::Full };
    let Some(spec) = v.strip_prefix("bytes=") else { return Want::Full };
    // A multi-range request is legal but nothing here (or in ffmpeg) sends one; take the first.
    let spec = spec.split(',').next().unwrap_or(spec).trim();
    let Some((a, b)) = spec.split_once('-') else { return Want::Full };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        return match b.parse() {
            Ok(n) => Want::Suffix(n),
            Err(_) => Want::Full,
        };
    }
    let Ok(start) = a.parse() else { return Want::Full };
    if b.is_empty() {
        Want::From(start)
    } else {
        match b.parse::<u64>() {
            Ok(end) if end >= start => Want::Bounded(start, end),
            _ => Want::Full,
        }
    }
}

/// The inclusive window to serve for `want`, or `None` when the request cannot be satisfied.
fn resolve_window(want: Want, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    match want {
        Want::Full => Some((0, total - 1)),
        Want::From(s) => (s < total).then_some((s, total - 1)),
        Want::Bounded(s, e) => (s < total).then(|| (s, e.min(total - 1))),
        Want::Suffix(n) => (n > 0).then(|| (total.saturating_sub(n), total - 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_right_token_and_kind_route() {
        let tokens = vec!["abc".to_owned()];
        assert_eq!(id_from("/abc/a/deadbeef", &tokens), Some("deadbeef"));
        assert_eq!(id_from("/wrong/a/deadbeef", &tokens), None);
        assert_eq!(id_from("/abc/x/deadbeef", &tokens), None);
        assert_eq!(id_from("/abc/a/", &tokens), None);
        assert_eq!(id_from("/abc/a", &tokens), None);
        assert_eq!(route("/abc/k/beef", &tokens), Some(("k", "beef")));
        assert_eq!(id_from("/abc/k/beef", &tokens), None, "a stream id is not an audio one");
    }

    #[test]
    fn the_lan_listener_cannot_serve_loopback_only_routes() {
        assert!(ListenerScope::Loopback.allows("a"));
        assert!(ListenerScope::Loopback.allows("k"));
        assert!(ListenerScope::Lan.allows("a"));
        assert!(!ListenerScope::Lan.allows("k"));
    }

    #[test]
    fn ranges_parse_the_ways_ffmpeg_sends_them() {
        assert_eq!(parse_range(None), Want::Full);
        assert_eq!(parse_range(Some("bytes=0-")), Want::From(0));
        assert_eq!(parse_range(Some("bytes=1234-")), Want::From(1234));
        assert_eq!(parse_range(Some("bytes=10-19")), Want::Bounded(10, 19));
        assert_eq!(parse_range(Some("bytes=0-0")), Want::Bounded(0, 0));
        assert_eq!(parse_range(Some("bytes=-500")), Want::Suffix(500));
        // Anything unusable is treated as no range, per the spec's ignore-unknown rule.
        assert_eq!(parse_range(Some("items=0-")), Want::Full);
        assert_eq!(parse_range(Some("bytes=oops")), Want::Full);
        assert_eq!(parse_range(Some("bytes=19-10")), Want::Full);
    }

    #[test]
    fn windows_clamp_and_reject() {
        assert_eq!(resolve_window(Want::Full, 1000), Some((0, 999)));
        assert_eq!(resolve_window(Want::From(0), 1000), Some((0, 999)));
        assert_eq!(resolve_window(Want::From(500), 1000), Some((500, 999)));
        assert_eq!(resolve_window(Want::Bounded(0, 5000), 1000), Some((0, 999)));
        assert_eq!(resolve_window(Want::Suffix(100), 1000), Some((900, 999)));
        assert_eq!(resolve_window(Want::From(1000), 1000), None);
        assert_eq!(resolve_window(Want::Suffix(0), 1000), None);
        assert_eq!(resolve_window(Want::Full, 0), None);
    }

    /// The chunks have to tile the window exactly: no gap to stall on, no overlap to duplicate.
    #[test]
    fn chunks_tile_the_window() {
        assert_eq!(chunk_end(0, 10 * 1024 * 1024), CHUNK - 1);
        assert_eq!(chunk_end(CHUNK, 10 * 1024 * 1024), 2 * CHUNK - 1);
        // Last chunk is clamped to the window's inclusive last byte, not one before it.
        assert_eq!(chunk_end(8 * 1024 * 1024, 10 * 1024 * 1024), 10 * 1024 * 1024);
        assert_eq!(chunk_end(0, 99), 99);

        let mut pos = 0u64;
        let end = 9 * 1024 * 1024 + 123;
        let mut seen = 0u64;
        while pos <= end {
            let last = chunk_end(pos, end);
            assert!(last <= end);
            seen += last - pos + 1;
            pos = last + 1;
        }
        assert_eq!(pos, end + 1);
        assert_eq!(seen, end + 1);
    }

    #[test]
    fn total_is_read_from_content_range() {
        assert_eq!(total_from_content_range("bytes 0-0/122414474"), Some(122414474));
        assert_eq!(total_from_content_range("bytes 0-0/*"), None);
        assert_eq!(total_from_content_range("nonsense"), None);
    }
}
