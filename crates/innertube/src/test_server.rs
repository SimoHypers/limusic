//! A throwaway HTTP server on localhost for tests that need to see what `post` sends and decide
//! what comes back, with no network. `std` threads and sockets only, so it needs no tokio feature.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// One request as the server received it. Header names are lowercased.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub(crate) struct MockServer {
    /// Ends in `/youtubei/v1/`, the shape `InnerTube::set_base_url` takes.
    pub base_url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl MockServer {
    /// `respond` maps a request to a status and a JSON body. One connection per request
    /// (`Connection: close`), so there is no keep-alive state to reason about.
    pub fn start(respond: impl Fn(&Seen) -> (u16, String) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let base_url = format!("http://{}/youtubei/v1/", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        // Never joined: the thread ends with the test process.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                    continue;
                }
                let path = request_line.split_whitespace().nth(1).unwrap_or_default().to_owned();
                let mut headers = Vec::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
                    }
                }
                let len = headers
                    .iter()
                    .find(|(k, _)| k == "content-length")
                    .and_then(|(_, v)| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0; len];
                let _ = reader.read_exact(&mut body);
                let req = Seen { path, headers, body: String::from_utf8_lossy(&body).into_owned() };
                let (status, reply) = respond(&req);
                log.lock().unwrap().push(req);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                    reply.len()
                );
            }
        });
        MockServer { base_url, seen }
    }

    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}
