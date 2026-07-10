//! `CipherDeobfuscator` (context/05) — the signature/`n`-transform runtime the orchestrator calls.
//!
//! Ties [`fetcher`] (player.js) + [`extractor`]/[`config`] (function names) + a hidden cipher
//! webview ([`crate::webview`]) that runs YouTube's own code. Every public method degrades
//! gracefully: a webview or extraction failure yields `None` / the original URL, and the
//! orchestrator falls through to the non-cipher fallback clients (context/06 §5).

mod config;
mod extractor;
mod fetcher;

pub use config::PlayerConfigStore;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tauri::AppHandle;
use tokio::sync::Mutex;

use crate::webview::Bridge;
use fetcher::PlayerJsFetcher;

const CIPHER_LABEL: &str = "limusic-cipher";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);

/// Minimal harness: predefine `_yt_player` (the IIFE arg) so the injected player.js can run.
const HARNESS: &str = "<!doctype html><html><head><meta charset=utf-8></head><body>\
<script>window._yt_player=window._yt_player||{};</script></body></html>";

/// Discovery/validation for the `n` function (context/05): accept `_nTransformFunc` if it maps a
/// sample input to a valid changed string, else brute-force a 1-arg fn on `window`.
const DISCOVERY_JS: &str = r#"(function(){
  var t="grut12Abc_-";
  function ok(s){return typeof s==='string'&&/^[A-Za-z0-9_-]+$/.test(s)&&s!==t;}
  window.__n_ok=false;
  try{
    if(typeof window._nTransformFunc==='function'&&ok(window._nTransformFunc(t))){window.__n_ok=true;}
    else{for(var k in window){try{var f=window[k];if(typeof f==='function'&&f.length===1){var r=f(t);if(ok(r)){window._nTransformFunc=f;window.__n_ok=true;break;}}}catch(e){}}}
  }catch(e){}
  window.__sig_ok=(typeof window._cipherSigFunc==='function');
  window.__cipher_loaded=true;
})();"#;

#[derive(Default)]
struct Inner {
    bridge: Option<Bridge>,
    sts: Option<i32>,
    built_epoch: u64,
    n_available: bool,
    /// Whether an `_cipherSigFunc` export exists (i.e. a sig function name was found). When false,
    /// deciphering is impossible on this player regardless of freshness — so we skip refetch/retry.
    sig_available: bool,
}

pub struct CipherDeobfuscator {
    app: AppHandle,
    fetcher: PlayerJsFetcher,
    config: Arc<PlayerConfigStore>,
    inner: Mutex<Inner>,
}

impl CipherDeobfuscator {
    pub fn new(app: AppHandle, app_data_dir: &Path, config: Arc<PlayerConfigStore>) -> Self {
        CipherDeobfuscator {
            fetcher: PlayerJsFetcher::new(app_data_dir),
            config,
            inner: Mutex::new(Inner::default()),
            app,
        }
    }

    /// STS of the player.js we decipher with (preferred over any other source). context/05.
    pub async fn signature_timestamp(&self) -> Option<i32> {
        if self.ensure_ready().await.is_err() {
            return None;
        }
        self.inner.lock().await.sts
    }

    /// `signatureCipher` string → a full, signed stream URL. `None` on any failure. context/05.
    pub async fn deobfuscate_stream_url(&self, cipher: &str, video_id: &str) -> Option<String> {
        if self.ensure_ready().await.is_err() {
            return None;
        }
        // No sig function on this player (obfuscation defeated extraction) → deciphering is
        // impossible and a fresh player.js won't change that. Skip the refetch/rebuild churn and
        // let the orchestrator degrade to the direct clients. context/05 (config table is the fix).
        if !self.inner.lock().await.sig_available {
            return None;
        }
        if let Some(u) = self.try_deobfuscate(cipher).await {
            return Some(u);
        }
        // One self-heal retry: a stale player.js can silently produce a wrong signature. context/05.
        tracing::warn!(video_id, "decipher failed — refetching player.js and retrying once");
        self.fetcher.invalidate();
        {
            self.inner.lock().await.bridge = None; // force rebuild
        }
        self.try_deobfuscate(cipher).await
    }

    async fn try_deobfuscate(&self, cipher: &str) -> Option<String> {
        self.ensure_ready().await.ok()?;
        let (s, sp, base) = parse_cipher(cipher)?;
        let bridge = self.inner.lock().await.bridge.clone()?;
        let js = format!(
            "(function(){{try{{return String(window._cipherSigFunc({}));}}catch(e){{return null;}}}})()",
            js_string(&s)
        );
        let sig = match bridge.eval_json(js, CALL_TIMEOUT).await.ok()? {
            Value::String(sig) if !sig.is_empty() => sig,
            _ => return None,
        };
        let sep = if base.contains('?') { '&' } else { '?' };
        Some(format!("{base}{sep}{sp}={}", urlencoding::encode(&sig)))
    }

    /// Replace `&n=` with its throttling-deobfuscated value. Returns the URL UNCHANGED on any
    /// failure so playback still attempts (context/05). Only meaningful for web clients.
    pub async fn transform_n_param_in_url(&self, url: &str) -> String {
        match self.try_transform_n(url).await {
            Some(u) => u,
            None => url.to_owned(),
        }
    }

    async fn try_transform_n(&self, url: &str) -> Option<String> {
        self.ensure_ready().await.ok()?;
        let inner = self.inner.lock().await;
        if !inner.n_available {
            return None;
        }
        let bridge = inner.bridge.clone()?;
        drop(inner);

        let re = regex::Regex::new(r"[?&]n=([^&]+)").ok()?;
        let enc = re.captures(url)?.get(1)?.as_str().to_owned();
        let decoded = urlencoding::decode(&enc).ok()?.into_owned();
        let js = format!(
            "(function(){{try{{return String(window._nTransformFunc({}));}}catch(e){{return null;}}}})()",
            js_string(&decoded)
        );
        match bridge.eval_json(js, CALL_TIMEOUT).await.ok()? {
            Value::String(newn) if !newn.is_empty() && newn != decoded => {
                Some(url.replacen(&format!("n={enc}"), &format!("n={}", urlencoding::encode(&newn)), 1))
            }
            _ => None,
        }
    }

    /// Self-heal after a 403 on a deciphered URL: refresh the config table + invalidate player.js.
    /// Returns true if something changed (caller may clear WEB_REMIX failure memory). context/05, 06.
    pub async fn on_stream_rejected(&self) -> bool {
        let table_changed = self.config.refresh_after_stream_rejection().await;
        self.fetcher.invalidate();
        self.inner.lock().await.bridge = None; // next ensure_ready rebuilds
        table_changed
    }

    /// Warm the cipher webview off the first-play path (context/04 §startup). Non-fatal.
    pub async fn prewarm(&self) {
        if let Err(e) = self.ensure_ready().await {
            tracing::warn!(error = %e, "cipher prewarm failed (will retry on demand)");
        }
    }

    /// Build (or rebuild, on config-epoch change) the cipher webview with player.js loaded and the
    /// sig/n functions exported + discovered.
    async fn ensure_ready(&self) -> Result<(), String> {
        let epoch = self.config.config_epoch();
        {
            let inner = self.inner.lock().await;
            if let Some(b) = &inner.bridge {
                if inner.built_epoch == epoch && b.exists() {
                    return Ok(());
                }
            }
        }
        // Fetch player.js and resolve sig/n names (config table wins, regex is the fallback).
        let player = self.fetcher.fetch().await.map_err(|e| e.to_string())?;
        let cfg = self.config.get(&player.hash);
        if cfg.is_none() {
            // Unknown player hash — pull the remote overlay off the hot path; a validated config
            // for it lands on the next rebuild (context/05 §forceRefresh). This run uses regex.
            let config = self.config.clone();
            tauri::async_runtime::spawn(async move {
                config.force_refresh().await;
            });
        }
        let sts = cfg
            .as_ref()
            .and_then(|c| c.sts)
            .or_else(|| extractor::extract_sts(&player.js));
        let sig_fn = cfg
            .as_ref()
            .and_then(|c| c.sig_fn.clone())
            .or_else(|| extractor::find_sig_fn(&player.js));
        let n_fn = cfg
            .as_ref()
            .and_then(|c| c.n_fn.clone())
            .or_else(|| extractor::find_n_fn(&player.js));
        tracing::info!(hash = player.hash, ?sts, ?sig_fn, ?n_fn, "cipher: building webview");
        let injected = extractor::build_injection(&player.js, sig_fn.as_deref(), n_fn.as_deref());

        // Tear down any stale webview, then create fresh and load the player.
        {
            let mut inner = self.inner.lock().await;
            if let Some(b) = inner.bridge.take() {
                let _ = b.destroy();
            }
        }
        let bridge = Bridge::create(&self.app, CIPHER_LABEL, HARNESS, "")
            .await
            .map_err(|e| e.to_string())?;
        if let Err(e) = Self::load_player(&bridge, &injected).await {
            let _ = bridge.destroy(); // don't orphan the hidden window on a failed load
            return Err(e);
        }
        let n_available = matches!(
            bridge.eval_json("window.__n_ok?true:false".into(), CALL_TIMEOUT).await,
            Ok(Value::Bool(true))
        );
        let sig_available = matches!(
            bridge.eval_json("window.__sig_ok?true:false".into(), CALL_TIMEOUT).await,
            Ok(Value::Bool(true))
        );

        let mut inner = self.inner.lock().await;
        inner.bridge = Some(bridge);
        inner.sts = sts;
        inner.built_epoch = epoch;
        inner.n_available = n_available;
        inner.sig_available = sig_available;
        tracing::info!(sig_available, n_available, "cipher webview ready");
        Ok(())
    }

    /// Inject player.js + discovery into a freshly-built cipher `bridge` and wait for discovery to
    /// finish. Split out so `ensure_ready` can destroy the webview on any of these failures.
    async fn load_player(bridge: &Bridge, injected: &str) -> Result<(), String> {
        bridge.eval(injected).map_err(|e| e.to_string())?;
        bridge.eval(DISCOVERY_JS).map_err(|e| e.to_string())?;
        // Wait for discovery to finish, then the caller reads whether n/sig are usable.
        bridge
            .call_async("window.__cipher_loaded?true:new Promise(r=>{var i=setInterval(()=>{if(window.__cipher_loaded){clearInterval(i);r(true);}},50);})", LOAD_TIMEOUT)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Parse a `signatureCipher` query string → `(s, sp, base_url)` with values percent-decoded.
/// `sp` defaults to `"signature"` (context/05). Returns `None` if `s` or `url` is missing.
fn parse_cipher(cipher: &str) -> Option<(String, String, String)> {
    let mut s = None;
    let mut sp = None;
    let mut url = None;
    for pair in cipher.split('&') {
        let (k, v) = pair.split_once('=')?;
        let v = urlencoding::decode(v).ok()?.into_owned();
        match k {
            "s" => s = Some(v),
            "sp" => sp = Some(v),
            "url" => url = Some(v),
            _ => {}
        }
    }
    Some((s?, sp.unwrap_or_else(|| "signature".into()), url?))
}

/// A JS string literal for the given value (properly escaped via JSON).
fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_signature_cipher() {
        let c = "s=ABC%3D%3D&sp=sig&url=https%3A%2F%2Fx.com%2Fv%3Fitag%3D251";
        let (s, sp, url) = parse_cipher(c).unwrap();
        assert_eq!(s, "ABC==");
        assert_eq!(sp, "sig");
        assert_eq!(url, "https://x.com/v?itag=251");
    }

    #[test]
    fn cipher_defaults_sp_to_signature() {
        let (_, sp, _) = parse_cipher("s=X&url=https%3A%2F%2Fx.com").unwrap();
        assert_eq!(sp, "signature");
    }

    #[test]
    fn cipher_missing_url_is_none() {
        assert!(parse_cipher("s=X&sp=sig").is_none());
    }

    #[test]
    fn js_string_escapes() {
        assert_eq!(js_string(r#"a"b\c"#), r#""a\"b\\c""#);
    }
}
