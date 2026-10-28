//! The app's one outbound HTTP client.
//!
//! There used to be six of these inside `src-tauri` (orchestrator, PoToken, cipher fetcher, cipher
//! config, lyrics, and Last.fm twice), each built from its own `Client::builder()`. `reqwest` pools
//! connections and holds its TLS config per client, so that was six rustls configs, six connection
//! pools and six sets of idle sockets to the same handful of Google hosts.
//!
//! Everything the separate clients were configured for (a User-Agent, a timeout) is per-request
//! state, so it moved to the call sites. The User-Agent is deliberately NOT a default here: what
//! YouTube serves for `player.js` depends on it, so it should be visible at the fetch rather than
//! inherited from somewhere else and quietly lost in a later edit.
//!
//! `crates/innertube` keeps its own client on purpose. It is the other side of the transport
//! boundary (context/11) and it configures a proxy per session, which is client-level state.

use std::sync::OnceLock;

/// Desktop Chrome. YouTube serves the web `player.js` and the BotGuard endpoints against this, so
/// the cipher fetcher and the PoToken minter both send it.
pub const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

static PROXY: OnceLock<String> = OnceLock::new();

/// Point this client at the user's proxy. Called once during setup, before anything fetches:
/// [`client`] reads it when it builds, so a later call changes nothing (the setting already
/// says "restart to apply").
///
/// The `proxy` setting used to reach `crates/innertube` alone, which left player.js, the BotGuard
/// endpoints, lyrics and the music-video proxy going direct (#241).
pub fn set_proxy(proxy: Option<&str>) -> Result<(), &'static str> {
    if let Some(p) = proxy.map(str::trim).filter(|p| !p.is_empty()) {
        validate_proxy(p)?;
        let _ = PROXY.set(p.to_owned());
    }
    Ok(())
}

/// Validate before saving settings or starting any network-capable component.
pub fn validate_proxy(proxy: &str) -> Result<(), &'static str> {
    if proxy.trim().is_empty() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let _ = proxy;
        Err("Configured proxies are unsupported by the macOS WebView. Clear the proxy setting before starting Limusic.")
    }
    #[cfg(not(target_os = "macos"))]
    innertube::transport::validate_proxy(proxy)
}

/// Whether [`client`] was pointed at a user proxy. mpv gets the same setting as its `http-proxy`
/// (#241), and ffmpeg would send a loopback URL through it, so the audio proxy has to stand down
/// when this is set. See `state::mpv_stream_url`.
pub fn has_proxy() -> bool {
    PROXY.get().is_some()
}

/// The validated proxy shared by every WebView. Startup rejects unsupported settings before
/// building a window; this must never fall back to direct routing for a configured proxy.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn webview_proxy() -> Option<tauri::Url> {
    PROXY.get().map(|proxy| webview_url(proxy).expect("validated WebView proxy"))
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn webview_url(proxy: &str) -> Option<tauri::Url> {
    innertube::transport::validate_proxy(proxy).ok()?;
    tauri::Url::parse(proxy).ok()
}

/// Gives a webview window the user's proxy, on Windows. Every webview has to get the same one:
/// wry turns it into a WebView2 browser argument, and WebView2 refuses to create an environment
/// whose options differ from the one already running on the same data folder (ERROR_INVALID_STATE),
/// so a window built without it would fail to open. Linux sets it once on the shared WebKit context
/// instead (`tune_webview`), and macOS would need tauri's `macos-proxy` feature.
pub fn proxied<'a, R: tauri::Runtime, M: tauri::Manager<R>>(
    b: tauri::WebviewWindowBuilder<'a, R, M>,
) -> tauri::WebviewWindowBuilder<'a, R, M> {
    #[cfg(windows)]
    if let Some(url) = webview_proxy() {
        return b.proxy_url(url);
    }
    b
}

pub fn client() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        // No default User-Agent and no default timeout: both are set per request, by the callers
        // that actually need them.
        let mut b = reqwest::Client::builder();
        if let Some(p) = PROXY.get() {
            b = b.proxy(reqwest::Proxy::all(p).expect("validated HTTP proxy"));
        }
        b.build().expect("outbound HTTP client")
    })
}

/// Whether YouTube Music answers at all. Any status counts, only getting no reply means the
/// network is down.
pub async fn network_reachable() -> bool {
    client()
        .head("https://music.youtube.com/")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .is_ok()
}

/// Waits about a minute for [`network_reachable`], probing again on a backoff. Returns false if
/// the network never came up.
pub async fn wait_for_network() -> bool {
    for delay in [0, 2, 4, 8, 15, 30] {
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        if network_reachable().await {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn clearing_a_saved_proxy_is_allowed() {
        assert!(super::validate_proxy("").is_ok());
        assert!(super::validate_proxy("   ").is_ok());
    }
    use super::webview_url;

    #[test]
    fn webview_proxy_refuses_unsupported_authentication_and_schemes() {
        for proxy in [
            "http://user:secret@proxy.lan:8080",
            "socks5://proxy.lan:1080",
            "http://proxy.lan:8080/path",
            "http://proxy.lan:8080?q=1",
            "http://proxy.lan:8080#fragment",
            "http://proxy.lan:0",
            "http://@proxy.lan:8080",
        ] {
            assert!(webview_url(proxy).is_none(), "accepted unsupported proxy");
        }
    }

    #[test]
    fn webview_url_takes_only_what_wry_can_build() {
        assert!(webview_url("http://127.0.0.1:7890").is_some());
        assert!(webview_url("http://[::1]:8080").is_some());
        // Every transport must support the configured scheme.
        assert!(webview_url("https://proxy.lan:8443").is_none());
        assert!(webview_url("socks5h://proxy.lan:1080").is_none());
        // No port reaches wry as an empty one, and the url crate drops a default :80.
        assert!(webview_url("http://proxy.lan").is_none());
        assert!(webview_url("http://proxy.lan:80").is_none());
        assert!(webview_url("127.0.0.1:7890").is_none());
    }
}
