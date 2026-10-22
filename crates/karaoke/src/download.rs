//! Downloads one pinned file: resumable, verified, cancellable, and never stuck.
//!
//! - Resume: bytes already in `<dest>.part` are kept and only the rest is requested (`Range`). A
//!   server that ignores the range (200 instead of 206) just starts the file over.
//! - Verify: the finished file must have the pinned size and SHA-256 before it is renamed into
//!   place, so `dest` only ever holds a complete, correct file.
//! - Timeouts: connecting (until response headers) and every chunk after that are bounded. A stalled
//!   connection fails with [`Error::Timeout`] instead of hanging the download forever.
//! - Cancel: [`Cancel::cancel`] wakes a waiting download at once, stalled or not. A cancelled or
//!   timed-out download keeps its `.part` for the next attempt.
//! - Hosts: the URL, and wherever redirects end up, must be on the allow-list.

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

/// The file to fetch and what it must turn out to be.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    pub url: &'a str,
    pub size: u64,
    /// Lower or upper hex.
    pub sha256: &'a str,
    /// Hosts the URL and every redirect may point at.
    pub hosts: &'a [&'a str],
    /// Refuse plain `http://` (only tests turn this off, for a local server).
    pub https_only: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// Until the response headers arrive.
    pub connect: Duration,
    /// Between two chunks of the body.
    pub idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { connect: Duration::from_secs(15), idle: Duration::from_secs(30) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    Timeout,
    /// The URL or a redirect points outside [`Source::hosts`], or is not https.
    HostNotAllowed(String),
    /// The server answered with this status.
    Status(u16),
    /// The connection ended early; the `.part` is kept for a resume.
    Incomplete {
        got: u64,
        expected: u64,
    },
    /// More bytes than the pinned size; the `.part` is deleted.
    TooLarge {
        expected: u64,
    },
    /// Complete but not the pinned file; the `.part` is deleted.
    HashMismatch,
    Network(String),
    Io(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::Timeout => write!(f, "the download stalled (no data for too long)"),
            Self::HostNotAllowed(host) => write!(f, "refused to download from {host}"),
            Self::Status(code) => write!(f, "download failed with status {code}"),
            Self::Incomplete { got, expected } => {
                write!(f, "the download stopped early ({got} of {expected} bytes)")
            }
            Self::TooLarge { expected } => write!(f, "the file is larger than {expected} bytes"),
            Self::HashMismatch => write!(f, "the file is corrupted (checksum mismatch)"),
            Self::Network(e) => write!(f, "network error: {e}"),
            Self::Io(e) => write!(f, "disk error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// A cancel switch that also wakes a download blocked on the network.
#[derive(Default)]
pub struct Cancel {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Re-arms the switch for a new download.
    pub fn reset(&self) {
        self.flag.store(false, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Resolves once [`Cancel::cancel`] has been called (immediately if it already was).
    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            // Registered before the check, so a cancel between the two is not missed.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

fn check_url(url: &reqwest::Url, src: &Source<'_>) -> Result<(), Error> {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let scheme_ok = url.scheme() == "https" || (!src.https_only && url.scheme() == "http");
    if scheme_ok && src.hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
        Ok(())
    } else {
        Err(Error::HostNotAllowed(format!("{}://{host}", url.scheme())))
    }
}

fn io(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

/// Downloads `src` to `dest`, through `part` (resumed if it already holds a prefix). `progress`
/// gets the bytes on disk so far (resumed bytes included). Returns once `dest` holds the verified
/// file.
pub async fn fetch(
    client: &reqwest::Client,
    src: &Source<'_>,
    part: &Path,
    dest: &Path,
    timeouts: Timeouts,
    cancel: &Cancel,
    mut progress: impl FnMut(u64),
) -> Result<(), Error> {
    let url = reqwest::Url::parse(src.url).map_err(|e| Error::Network(e.to_string()))?;
    check_url(&url, src)?;

    let mut have = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
    if have > src.size {
        tokio::fs::remove_file(part).await.map_err(io)?;
        have = 0;
    }

    if have < src.size {
        let mut request = client.get(url).header(reqwest::header::USER_AGENT, "Limusic");
        if have > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            r = tokio::time::timeout(timeouts.connect, request.send()) => match r {
                Err(_) => return Err(Error::Timeout),
                Ok(Err(e)) => return Err(Error::Network(e.without_url().to_string())),
                Ok(Ok(resp)) => resp,
            },
        };
        check_url(response.url(), src)?;

        let status = response.status().as_u16();
        let append = match status {
            206 if have > 0 => true,
            200 => {
                have = 0;
                false
            }
            // The kept prefix no longer fits (e.g. the file changed upstream): start over next time.
            416 => {
                let _ = tokio::fs::remove_file(part).await;
                return Err(Error::Status(status));
            }
            _ => return Err(Error::Status(status)),
        };
        progress(have);

        let mut file = if append {
            tokio::fs::OpenOptions::new().append(true).open(part).await.map_err(io)?
        } else {
            tokio::fs::File::create(part).await.map_err(io)?
        };
        let mut body = response.bytes_stream();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = file.flush().await;
                    return Err(Error::Cancelled);
                }
                n = tokio::time::timeout(timeouts.idle, body.next()) => n,
            };
            match next {
                Err(_) => {
                    let _ = file.flush().await;
                    return Err(Error::Timeout);
                }
                Ok(None) => break,
                // The connection broke mid-body (hyper reports a server that closes before the
                // promised length this way): what arrived is kept for a resume.
                Ok(Some(Err(_))) => {
                    let _ = file.flush().await;
                    return Err(Error::Incomplete { got: have, expected: src.size });
                }
                Ok(Some(Ok(chunk))) => {
                    have += chunk.len() as u64;
                    if have > src.size {
                        drop(file);
                        let _ = tokio::fs::remove_file(part).await;
                        return Err(Error::TooLarge { expected: src.size });
                    }
                    file.write_all(&chunk).await.map_err(io)?;
                    progress(have);
                }
            }
        }
        file.flush().await.map_err(io)?;
        file.sync_all().await.map_err(io)?;
        drop(file);
        if have < src.size {
            return Err(Error::Incomplete { got: have, expected: src.size });
        }
    }

    // Hash the whole file, resumed prefix included, off the async runtime.
    let part_buf = part.to_path_buf();
    let digest = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut file = std::fs::File::open(&part_buf)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)?;
        Ok(format!("{:x}", hasher.finalize()))
    })
    .await
    .map_err(|e| Error::Io(e.to_string()))?
    .map_err(io)?;
    if !digest.eq_ignore_ascii_case(src.sha256) {
        let _ = tokio::fs::remove_file(part).await;
        return Err(Error::HashMismatch);
    }
    tokio::fs::rename(part, dest).await.map_err(io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    /// What the test server does with one request.
    #[derive(Clone, Copy)]
    enum Mode {
        /// Honours `Range` with 206.
        Ranges,
        /// Always the whole file with 200.
        IgnoresRange,
        /// Sends this many bytes, then closes the connection.
        CutAfter(usize),
        /// Sends the headers and this many bytes, then goes silent (keeps the socket open).
        StallAfter(usize),
    }

    struct Server {
        url: String,
        requests: Arc<AtomicUsize>,
        ranges: Arc<std::sync::Mutex<Vec<Option<String>>>>,
    }

    /// A localhost HTTP server serving `body` at `/file`, one mode per request in order (the last
    /// one repeats). std threads and sockets only.
    fn serve(body: Vec<u8>, modes: Vec<Mode>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let ranges = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (count, seen) = (requests.clone(), ranges.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let n = count.fetch_add(1, Ordering::SeqCst);
                let mode = modes[n.min(modes.len() - 1)];
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = Some(v.trim().trim_end_matches('-').to_string());
                    }
                }
                seen.lock().unwrap().push(range.clone());
                let start: usize = match (mode, &range) {
                    (Mode::Ranges, Some(r)) => r.parse().unwrap(),
                    _ => 0,
                };
                let slice = &body[start..];
                let status = if start > 0 { "206 Partial Content" } else { "200 OK" };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    slice.len()
                );
                match mode {
                    Mode::Ranges | Mode::IgnoresRange => {
                        let _ = stream.write_all(slice);
                    }
                    Mode::CutAfter(k) => {
                        let _ = stream.write_all(&slice[..k.min(slice.len())]);
                        let _ = stream.flush();
                        let _ = stream.shutdown(std::net::Shutdown::Both);
                        continue;
                    }
                    Mode::StallAfter(k) => {
                        let _ = stream.write_all(&slice[..k.min(slice.len())]);
                        let _ = stream.flush();
                        std::thread::sleep(Duration::from_secs(5));
                    }
                }
                let _ = stream.flush();
                let mut sink = [0u8; 16];
                let _ = stream.read(&mut sink);
            }
        });
        Server { url, requests, ranges }
    }

    fn body() -> Vec<u8> {
        (0..200_000u32).map(|i| (i % 251) as u8).collect()
    }

    fn sha(data: &[u8]) -> String {
        format!("{:x}", Sha256::digest(data))
    }

    fn paths(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("limusic_download_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (dir.join("file.part"), dir.join("file"))
    }

    const LOCAL: &[&str] = &["127.0.0.1"];

    fn source<'a>(url: &'a str, data: &[u8], hash: &'a str) -> Source<'a> {
        Source { url, size: data.len() as u64, sha256: hash, hosts: LOCAL, https_only: false }
    }

    fn quick() -> Timeouts {
        Timeouts { connect: Duration::from_secs(5), idle: Duration::from_millis(300) }
    }

    #[tokio::test]
    async fn a_whole_download_is_verified_and_moved_into_place() {
        let data = body();
        let server = serve(data.clone(), vec![Mode::Ranges]);
        let (part, dest) = paths("whole");
        let hash = sha(&data);
        let mut last = 0;
        fetch(
            &reqwest::Client::new(),
            &source(&server.url, &data, &hash),
            &part,
            &dest,
            quick(),
            &Cancel::new(),
            |n| last = n,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!part.exists());
        assert_eq!(last, data.len() as u64);
    }

    #[tokio::test]
    async fn a_cut_connection_resumes_where_it_stopped() {
        let data = body();
        let server = serve(data.clone(), vec![Mode::CutAfter(70_000), Mode::Ranges]);
        let (part, dest) = paths("resume");
        let hash = sha(&data);
        let src = source(&server.url, &data, &hash);
        let client = reqwest::Client::new();
        let cancel = Cancel::new();

        let first = fetch(&client, &src, &part, &dest, quick(), &cancel, |_| {}).await;
        assert!(matches!(first, Err(Error::Incomplete { got: 70_000, .. })), "{first:?}");
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 70_000, "the prefix is kept");

        fetch(&client, &src, &part, &dest, quick(), &cancel, |_| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(server.ranges.lock().unwrap().as_slice(), &[None, Some("70000".into())]);
    }

    #[tokio::test]
    async fn a_server_that_ignores_ranges_starts_over() {
        let data = body();
        let server = serve(data.clone(), vec![Mode::CutAfter(50_000), Mode::IgnoresRange]);
        let (part, dest) = paths("ignores_range");
        let hash = sha(&data);
        let src = source(&server.url, &data, &hash);
        let client = reqwest::Client::new();
        let _ = fetch(&client, &src, &part, &dest, quick(), &Cancel::new(), |_| {}).await;
        fetch(&client, &src, &part, &dest, quick(), &Cancel::new(), |_| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data, "no duplicated prefix");
    }

    #[tokio::test]
    async fn a_stalled_download_times_out_and_keeps_its_part() {
        let data = body();
        let server = serve(data.clone(), vec![Mode::StallAfter(10_000)]);
        let (part, dest) = paths("stall");
        let hash = sha(&data);
        let started = std::time::Instant::now();
        let got = fetch(
            &reqwest::Client::new(),
            &source(&server.url, &data, &hash),
            &part,
            &dest,
            quick(),
            &Cancel::new(),
            |_| {},
        )
        .await;
        assert_eq!(got, Err(Error::Timeout));
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
        assert!(!dest.exists());
        assert!(part.exists(), "kept for a resume");
    }

    #[tokio::test]
    async fn cancel_wakes_a_stalled_download_at_once() {
        let data = body();
        let server = serve(data.clone(), vec![Mode::StallAfter(10_000)]);
        let (part, dest) = paths("cancel");
        let hash = sha(&data);
        let cancel = Arc::new(Cancel::new());
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            trigger.cancel();
        });
        let slow = Timeouts { connect: Duration::from_secs(5), idle: Duration::from_secs(60) };
        let started = std::time::Instant::now();
        let got = fetch(
            &reqwest::Client::new(),
            &source(&server.url, &data, &hash),
            &part,
            &dest,
            slow,
            &cancel,
            |_| {},
        )
        .await;
        assert_eq!(got, Err(Error::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn a_wrong_file_is_refused_and_removed() {
        let data = body();
        let server = serve(data.clone(), vec![Mode::Ranges]);
        let (part, dest) = paths("hash");
        let wrong = sha(b"something else");
        let got = fetch(
            &reqwest::Client::new(),
            &source(&server.url, &data, &wrong),
            &part,
            &dest,
            quick(),
            &Cancel::new(),
            |_| {},
        )
        .await;
        assert_eq!(got, Err(Error::HashMismatch));
        assert!(!dest.exists() && !part.exists());
    }

    #[tokio::test]
    async fn a_host_off_the_list_is_never_contacted() {
        let data = body();
        let server = serve(data.clone(), vec![Mode::Ranges]);
        let (part, dest) = paths("host");
        let hash = sha(&data);
        let mut src = source(&server.url, &data, &hash);
        src.hosts = &["github.com"];
        let got =
            fetch(&reqwest::Client::new(), &src, &part, &dest, quick(), &Cancel::new(), |_| {})
                .await;
        assert!(matches!(got, Err(Error::HostNotAllowed(_))), "{got:?}");
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);

        src.hosts = LOCAL;
        src.https_only = true;
        let got =
            fetch(&reqwest::Client::new(), &src, &part, &dest, quick(), &Cancel::new(), |_| {})
                .await;
        assert!(matches!(got, Err(Error::HostNotAllowed(_))), "plain http refused: {got:?}");
    }
}
