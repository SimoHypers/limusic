//! Loopback HTTP proxy for the player view's music video: the `<video>` element fetches its bytes
//! from here, never from googlevideo (context/11: no YouTube shapes past the command boundary).
//!
//! **Why a socket and not a `limusicvideo://` custom scheme.** The scheme was the plan, and it does
//! work for `fetch`, XHR and an iframe: measured on WebKitGTK, a registered scheme answers a
//! textbook `206` with `Content-Range: bytes 0-1445/35729196` and `Content-Type: video/webm`. A
//! `<video>` still refuses it with `MEDIA_ERR_SRC_NOT_SUPPORTED` and never asks again, because the
//! element hands the URI to GStreamer, whose source element only claims `http`, `https` and `blob`.
//! The same bytes over `https` played at 1280x720. So the boundary is kept with a loopback socket
//! instead: `http://127.0.0.1:<ephemeral>/<token>/<videoId>`, bound to localhost, with a random
//! per-launch token in the path so nothing else on the machine can guess a URL.
//!
//! `video_stream` has already put the real googlevideo URL in [`AppState`] under that videoId; this
//! is a thin range-proxy on top of it.

use std::convert::Infallible;
use std::io;
use std::net::{Ipv4Addr, TcpListener};
use std::sync::{Arc, OnceLock};

use futures_util::TryStreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Empty, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};

use crate::state::AppState;

/// Streamed, never buffered: a 35 MB video passes through in chunks, so the proxy costs a few KB of
/// RAM no matter how long the track is.
type ProxyBody = BoxBody<Bytes, io::Error>;

/// `(port, token)` of the running server. Set once at startup; unset if the bind failed, which just
/// means video mode never finds a URL and the view keeps the artwork.
static ENDPOINT: OnceLock<(u16, String)> = OnceLock::new();

/// Bind the loopback listener and start serving. Binds synchronously so [`url_for`] is usable the
/// moment this returns, then hands the socket to tokio.
pub fn start(state: Arc<AppState>) {
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "video proxy: bind failed (music videos disabled)");
            return;
        }
    };
    let port = match listener.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            tracing::warn!(error = %e, "video proxy: local_addr failed");
            return;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        tracing::warn!(error = %e, "video proxy: set_nonblocking failed");
        return;
    }
    let token = format!("{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>());
    if ENDPOINT.set((port, token)).is_err() {
        return; // already started
    }
    tracing::info!(port, "video proxy listening on loopback");

    tauri::async_runtime::spawn(async move {
        let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return };
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    // EMFILE/ENFILE/ENOBUFS return immediately, so `continue` alone was a tight
                    // loop pinning a core for the rest of the process.
                    tracing::warn!(error = %e, "video proxy: accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            };
            let state = state.clone();
            tauri::async_runtime::spawn(async move {
                let svc = service_fn(move |req| serve(req, state.clone()));
                // No connection cap: the three timeouts here remove the unbounded cases, and a
                // semaphore is real book-keeping for a proxy that serves one <video> element.
                // A dropped connection is what a seek looks like from here, so errors are expected.
                let _ = http1::Builder::new()
                    // hyper 1.x panics the moment it arms a timeout with no timer installed, so
                    // without this every single connection to the proxy died on arrival and took a
                    // tokio worker with it. That is why music videos never played.
                    .timer(TokioTimer::new())
                    .header_read_timeout(std::time::Duration::from_secs(15))
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
}

/// The URL the UI puts in `<video src>`, or `None` if the server never came up.
pub fn url_for(video_id: &str) -> Option<String> {
    let (port, token) = ENDPOINT.get()?;
    Some(format!("http://127.0.0.1:{port}/{token}/{video_id}"))
}

/// `/<token>/<videoId>` to the videoId, once the token matches this launch's.
fn video_id_from<'a>(path: &'a str, token: &str) -> Option<&'a str> {
    let (t, id) = path.trim_start_matches('/').split_once('/')?;
    (t == token && !id.is_empty()).then_some(id)
}

fn empty(status: StatusCode) -> Response<ProxyBody> {
    let mut r = Response::new(Empty::<Bytes>::new().map_err(|e| match e {}).boxed());
    *r.status_mut() = status;
    r
}

async fn serve(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Result<Response<ProxyBody>, Infallible> {
    Ok(handle(req, state).await.unwrap_or_else(empty))
}

async fn handle(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Result<Response<ProxyBody>, StatusCode> {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }
    let (_, token) = ENDPOINT.get().ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let video_id = video_id_from(req.uri().path(), token).ok_or(StatusCode::NOT_FOUND)?;
    // Nothing resolved this id: a stale element, or a `video_stream` that returned None.
    let upstream = state.video_url(video_id).ok_or(StatusCode::NOT_FOUND)?;

    // The element's Range goes upstream untouched and googlevideo does the arithmetic, so there is
    // no range maths here to get wrong. Everything else the webview sent is dropped.
    // No upstream timeout on purpose. `reqwest` 0.12 exposes `read_timeout` only on the *client*
    // builder, and `crate::http::client()` is shared with lyrics and the orchestrator (http.rs says
    // why it is one client), so bounding the byte gap here would change behaviour for every caller.
    // `.timeout()` is not the substitute: it is total duration and would cut a long stream
    // mid-track. Revisit if reqwest gains a request-level `read_timeout`.
    let mut out = crate::http::client().get(&upstream);
    if let Some(range) = req.headers().get(header::RANGE) {
        out = out.header(header::RANGE.as_str(), range);
    }
    let upstream_resp = match out.send().await {
        Ok(r) if r.status().is_success() => r,
        // Expired URL (googlevideo links last ~6h) or a network failure. The element errors and the
        // view falls back to artwork; see plan 031's maintenance notes.
        Ok(r) => {
            tracing::debug!(video_id, status = %r.status(), "video proxy: upstream refused");
            return Err(StatusCode::BAD_GATEWAY);
        }
        Err(e) => {
            tracing::debug!(video_id, error = %e, "video proxy: upstream failed");
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    // Pass the shape of the answer through as-is: the status decides 200 vs 206, and Content-Range
    // carries the total size the element needs for the duration.
    let mut builder = Response::builder()
        .status(upstream_resp.status().as_u16())
        .header(header::ACCEPT_RANGES, "bytes");
    for name in [header::CONTENT_TYPE, header::CONTENT_LENGTH, header::CONTENT_RANGE] {
        if let Some(v) = upstream_resp.headers().get(&name) {
            builder = builder.header(name, v);
        }
    }

    let body = if req.method() == Method::HEAD {
        Empty::<Bytes>::new().map_err(|e| match e {}).boxed()
    } else {
        StreamBody::new(upstream_resp.bytes_stream().map_ok(Frame::data).map_err(io::Error::other))
            .boxed()
    };
    builder.body(body).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The token is the only thing keeping another local process off the proxy, so a wrong or
    /// missing one has to miss.
    #[test]
    fn only_the_right_token_routes() {
        assert_eq!(video_id_from("/abc123/dQw4w9WgXcQ", "abc123"), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id_from("/wrong/dQw4w9WgXcQ", "abc123"), None);
        assert_eq!(video_id_from("/dQw4w9WgXcQ", "abc123"), None);
        assert_eq!(video_id_from("/abc123/", "abc123"), None);
    }
}
