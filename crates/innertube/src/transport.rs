//! HTTP transport. context/01. Pure — no Tauri/webview/mpv.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use sha1::{Digest, Sha1};

use crate::clients::YouTubeClient;
use crate::models::context::Locale;

pub const BASE_URL: &str = "https://music.youtube.com/youtubei/v1/";
pub const ORIGIN: &str = "https://music.youtube.com";
pub const REFERER: &str = "https://music.youtube.com/";
pub const SW_JS_DATA_URL: &str = "https://music.youtube.com/sw.js_data";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("visitorData not found in sw.js_data")]
    VisitorDataNotFound,
    #[error("{0}")]
    Other(String),
}

/// Session state, set once at startup / login. context/01 §mutable session state.
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub locale: Locale,
    pub visitor_data: Option<String>,
    pub data_sync_id: Option<String>,
    /// Full cookie string (Phase 3). Present ⇒ authenticated requests possible.
    pub cookie: Option<String>,
}

impl Session {
    /// Pull the `SAPISID` value out of the cookie string, if present.
    fn sapisid(&self) -> Option<String> {
        let cookie = self.cookie.as_ref()?;
        cookie.split(';').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            // __Secure-3PAPISID is the modern alias YouTube also accepts.
            (matches!(k.trim(), "SAPISID" | "__Secure-3PAPISID")).then(|| v.trim().to_owned())
        })
    }
}

/// The transport client. One shared `reqwest::Client`; proxy must be set before the
/// first request or reqwest snapshots it as none (context/12, the App.kt gotcha).
#[derive(Clone)]
pub struct InnerTube {
    http: reqwest::Client,
    pub session: Session,
}

impl InnerTube {
    pub fn new(session: Session, proxy: Option<&str>) -> Result<Self, Error> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(10);
        if let Some(p) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(p)?);
        }
        Ok(InnerTube { http: builder.build()?, session })
    }

    /// POST a JSON body to an InnerTube endpoint with this client's headers, retrying
    /// transient network errors (3 attempts, 500ms × 2 backoff). context/01 §retry.
    pub async fn post<B: Serialize>(
        &self,
        path: &str,
        client: &YouTubeClient,
        body: &B,
        set_login: bool,
    ) -> Result<serde_json::Value, Error> {
        let url = format!("{BASE_URL}{path}?prettyPrint=false");
        let headers = self.headers(client, set_login);
        let body = serde_json::to_vec(body)?;

        let mut delay = Duration::from_millis(500);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let res = self
                .http
                .post(&url)
                .headers(headers.clone())
                .body(body.clone())
                .send()
                .await
                .and_then(|r| r.error_for_status());
            match res {
                Ok(resp) => return Ok(resp.json().await?),
                // Retry only on connect/timeout (transient), matching Metrolist's IOException filter.
                Err(e) if attempt < 3 && (e.is_timeout() || e.is_connect() || e.is_request()) => {
                    tracing::warn!(attempt, error = %e, "retrying InnerTube POST {path}");
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Per-request headers. context/01 §ytClient. Note `X-YouTube-Client-Name` carries the
    /// numeric client **id**, not the name string — intentional and required.
    fn headers(&self, client: &YouTubeClient, set_login: bool) -> HeaderMap {
        let mut h = HeaderMap::new();
        let set = |h: &mut HeaderMap, k: &'static str, v: &str| {
            if let Ok(val) = HeaderValue::from_str(v) {
                h.insert(HeaderName::from_static(k), val);
            }
        };
        set(&mut h, "content-type", "application/json");
        set(&mut h, "accept", "application/json");
        set(&mut h, "accept-language", "en-US,en;q=0.9");
        set(&mut h, "x-goog-api-format-version", "1");
        set(&mut h, "x-youtube-client-name", &client.client_id);
        set(&mut h, "x-youtube-client-version", &client.client_version);
        set(&mut h, "x-origin", ORIGIN);
        set(&mut h, "referer", REFERER);
        set(&mut h, "user-agent", &client.user_agent);
        if let Some(vd) = &self.session.visitor_data {
            set(&mut h, "x-goog-visitor-id", vd);
        }

        // SAPISIDHASH cookie auth — only when logged in AND the client supports it (Phase 3).
        if set_login && client.login_supported {
            if let Some(cookie) = &self.session.cookie {
                set(&mut h, "cookie", cookie);
                if let Some(sapisid) = self.session.sapisid() {
                    if let Ok(val) = HeaderValue::from_str(&sapisid_hash(&sapisid, ORIGIN)) {
                        h.insert(HeaderName::from_static("authorization"), val);
                    }
                }
            }
        }
        h
    }

    /// Bootstrap `visitorData` anonymously by scraping `sw.js_data`. context/04 §A.
    pub async fn fetch_visitor_data(&self) -> Result<String, Error> {
        let text = self.http.get(SW_JS_DATA_URL).send().await?.error_for_status()?.text().await?;
        parse_visitor_data(&text)
    }

    #[cfg(any(test, feature = "integration-tests"))]
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

/// `Authorization: SAPISIDHASH <epoch>_<sha1(epoch SAPISID origin)>`. context/01.
pub fn sapisid_hash(sapisid: &str, origin: &str) -> String {
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("SAPISIDHASH {epoch}_{}", sha1_hex(&format!("{epoch} {sapisid} {origin}")))
}

fn sha1_hex(input: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// The `sw.js_data` body starts with a 4–5 char junk prefix (`)]}'`); strip it, parse JSON,
/// and find the element matching `^Cg[ts]` in `[0][2]`. context/04 §A.
fn parse_visitor_data(body: &str) -> Result<String, Error> {
    // Drop everything up to and including the first newline or the `)]}'` guard.
    let json_start = body.find('[').ok_or(Error::VisitorDataNotFound)?;
    let value: serde_json::Value = serde_json::from_str(&body[json_start..])?;
    let arr = value
        .get(0)
        .and_then(|v| v.get(2))
        .and_then(|v| v.as_array())
        .ok_or(Error::VisitorDataNotFound)?;
    arr.iter()
        .filter_map(|v| v.as_str())
        .find(|s| s.starts_with("Cgt") || s.starts_with("Cgs"))
        .map(str::to_owned)
        .ok_or(Error::VisitorDataNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_vector() {
        // SHA1("abc") = a9993e364706816aba3e25717850c26c9cd0d89d
        assert_eq!(sha1_hex("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn sapisid_hash_shape() {
        let h = sapisid_hash("MYSAPISID", ORIGIN);
        assert!(h.starts_with("SAPISIDHASH "));
        let rest = &h["SAPISIDHASH ".len()..];
        let (epoch, hash) = rest.split_once('_').unwrap();
        assert!(epoch.parse::<u64>().is_ok());
        assert_eq!(hash.len(), 40); // sha1 hex
    }

    #[test]
    fn parse_visitor_data_from_blob() {
        // Shape of sw.js_data: outer array; [0][2] holds the visitorData among other strings.
        let blob = r#")]}'
[["wrs","x",["junk","CgtABCDEFG1234567%3D%3D","more"]],null]"#;
        assert_eq!(parse_visitor_data(blob).unwrap(), "CgtABCDEFG1234567%3D%3D");
    }

    #[test]
    fn sapisid_extracted_from_cookie() {
        let s = Session {
            cookie: Some("FOO=bar; SAPISID=secret123; OTHER=x".into()),
            ..Default::default()
        };
        assert_eq!(s.sapisid().as_deref(), Some("secret123"));
    }
}
