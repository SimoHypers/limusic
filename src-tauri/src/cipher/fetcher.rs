//! `PlayerJsFetcher` (context/05 §1) — fetch and disk-cache YouTube's `player.js`.
//!
//! `iframe_api` → player hash → `base.js`, cached under `cipher_cache/` with a 6-hour TTL. The
//! hash pins the exact player we derive the STS from and decipher with (context/05 §3).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use regex::Regex;

/// Desktop web UA — YouTube serves the web `player.js` to this. context/05.
const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const IFRAME_API: &str = "https://www.youtube.com/iframe_api";
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not find player hash in iframe_api")]
    NoHash,
}

pub struct PlayerJs {
    pub js: String,
    pub hash: String,
}

pub struct PlayerJsFetcher {
    http: reqwest::Client,
    cache_dir: PathBuf,
}

impl PlayerJsFetcher {
    pub fn new(app_data_dir: &Path) -> Self {
        let cache_dir = app_data_dir.join("cipher_cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        let http = reqwest::Client::builder()
            .user_agent(WEB_UA)
            .build()
            .unwrap_or_default();
        PlayerJsFetcher { http, cache_dir }
    }

    /// Return the current `player.js` (cached if fresh, else fetched fresh and cached).
    pub async fn fetch(&self) -> Result<PlayerJs, Error> {
        let hash = self.current_hash().await?;
        let cached = self.cache_dir.join(format!("player_{hash}.js"));
        if let Some(js) = read_if_fresh(&cached) {
            tracing::debug!(hash, "player.js cache hit");
            return Ok(PlayerJs { js, hash });
        }
        let url =
            format!("https://www.youtube.com/s/player/{hash}/player_ias.vflset/en_GB/base.js");
        let js = self.http.get(&url).send().await?.error_for_status()?.text().await?;
        std::fs::write(&cached, &js)?;
        std::fs::write(
            self.cache_dir.join("current_hash.txt"),
            format!("{hash}\n{}", now_secs()),
        )?;
        tracing::info!(hash, bytes = js.len(), "fetched fresh player.js");
        Ok(PlayerJs { js, hash })
    }

    /// Delete cached `player_*.js` + `current_hash.txt` (NOT the shared config files). context/05.
    /// Forces the next `fetch` to re-download — the self-heal path after a stale-signature 403.
    pub fn invalidate(&self) {
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for e in entries.flatten() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("player_") && name.ends_with(".js") || name == "current_hash.txt"
                {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }

    async fn current_hash(&self) -> Result<String, Error> {
        let body = self.http.get(IFRAME_API).send().await?.error_for_status()?.text().await?;
        extract_hash(&body).ok_or(Error::NoHash)
    }
}

/// Extract the player hash from `iframe_api`. The URL there has escaped slashes
/// (`...\/s\/player\/<hash>\/www-widgetapi...`), so the separators are optional-backslash.
fn extract_hash(iframe_api_js: &str) -> Option<String> {
    let re = Regex::new(r"player\\?/([0-9A-Za-z_-]+)\\?/").ok()?;
    re.captures(iframe_api_js)?.get(1).map(|m| m.as_str().to_owned())
}

fn read_if_fresh(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let age = meta.modified().ok()?.elapsed().unwrap_or(CACHE_TTL);
    if age < CACHE_TTL {
        std::fs::read_to_string(path).ok()
    } else {
        None
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_from_escaped_iframe_api() {
        let body = r#"var scriptUrl='https:\/\/www.youtube.com\/s\/player\/4918c89a\/www-widgetapi.vflset\/www-widgetapi.js';"#;
        assert_eq!(extract_hash(body).as_deref(), Some("4918c89a"));
    }

    #[test]
    fn hash_from_plain_url() {
        assert_eq!(
            extract_hash("https://www.youtube.com/s/player/abcd1234/base.js").as_deref(),
            Some("abcd1234")
        );
    }
}
