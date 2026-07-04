//! `PlayerConfigStore` / `PlayerConfigParser` (context/05 §3) — the cipher resilience layer.
//!
//! A table of known-good per-player-hash configs (which sig/n function + STS). Regex extraction
//! breaks whenever YouTube rotates `player.js`; a config-backed entry (`isHardcoded`) is reliable.
//! Ships as a bundled asset overlaid by a remotely-updatable JSON file so a rotation can be fixed
//! without an app release. `config_epoch` bumps when the table changes, forcing the cipher webview
//! to rebuild → self-heal without restart.
//!
//! Open decision D-Q#4 (context/17): the remote host is a raw file in this repo. A fetch failure
//! is non-fatal — the store keeps whatever it already has and playback degrades to regex/fallback.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex as AsyncMutex;

/// Raw-file overlay (D-Q#4). Points at this repo; a 404/parse-error is a graceful no-op.
const REMOTE_CONFIG_URL: &str =
    "https://raw.githubusercontent.com/SimoHypers/Limusic/master/player_configs.json";
const BUNDLED: &str = include_str!("../../cipher_configs.json");
/// Rate-limit for self-heal re-fetches (context/05 §refreshAfterStreamRejection).
const REFRESH_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// One validated player config. `sig_fn`/`n_fn` are function *names* inside that player's
/// `player.js`; `None` means "not known — fall back to regex/brute-force for this one".
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerConfig {
    pub hash: String,
    #[serde(default)]
    pub sts: Option<i32>,
    #[serde(default)]
    pub sig_fn: Option<String>,
    #[serde(default)]
    pub n_fn: Option<String>,
}

pub struct PlayerConfigStore {
    entries: RwLock<HashMap<String, PlayerConfig>>,
    epoch: AtomicU64,
    http: reqwest::Client,
    cache_file: PathBuf,
    /// Serializes refreshes and holds the last-refresh instant for rate-limiting.
    last_refresh: AsyncMutex<Option<Instant>>,
}

impl PlayerConfigStore {
    /// Load bundled configs overlaid with the cached remote file (context/05 `initialize`,
    /// synchronous part). The TTL-gated remote refresh is `force_refresh` /
    /// `refresh_after_stream_rejection`, scheduled by the caller off the hot path.
    pub fn new(app_data_dir: &Path) -> Self {
        let cache_file = app_data_dir.join("cipher_cache").join("player_configs.json");
        let mut entries = parse_table(BUNDLED);
        if let Ok(cached) = std::fs::read_to_string(&cache_file) {
            merge(&mut entries, parse_table(&cached));
        }
        PlayerConfigStore {
            entries: RwLock::new(entries),
            epoch: AtomicU64::new(0),
            http: reqwest::Client::new(),
            cache_file,
            last_refresh: AsyncMutex::new(None),
        }
    }

    pub fn get(&self, hash: &str) -> Option<PlayerConfig> {
        self.entries.read().unwrap().get(hash).cloned()
    }

    /// Increments whenever the table changes — the cipher webview watches this to rebuild.
    pub fn config_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Self-heal after a deciphered URL got rejected (403): rate-limited remote re-fetch. Returns
    /// true if the table changed (caller should rebuild the cipher webview). context/05.
    pub async fn refresh_after_stream_rejection(&self) -> bool {
        let mut last = self.last_refresh.lock().await;
        if let Some(t) = *last {
            if t.elapsed() < REFRESH_COOLDOWN {
                return false;
            }
        }
        *last = Some(Instant::now());
        drop(last);
        self.fetch_and_merge().await
    }

    /// Pull the remote overlay unconditionally (e.g. a brand-new player hash appeared).
    pub async fn force_refresh(&self) -> bool {
        *self.last_refresh.lock().await = Some(Instant::now());
        self.fetch_and_merge().await
    }

    async fn fetch_and_merge(&self) -> bool {
        let text = match self.http.get(REMOTE_CONFIG_URL).send().await {
            Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
            Ok(r) => {
                tracing::debug!(status = %r.status(), "remote player config not available");
                return false;
            }
            Err(e) => {
                tracing::debug!(error = %e, "remote player config fetch failed");
                return false;
            }
        };
        let incoming = parse_table(&text);
        if incoming.is_empty() {
            return false;
        }
        let changed = {
            let mut entries = self.entries.write().unwrap();
            let before = entries.len();
            merge(&mut entries, incoming);
            entries.len() != before // coarse change signal; enough to trigger a rebuild
        };
        if changed {
            let _ = std::fs::write(&self.cache_file, &text);
            self.epoch.fetch_add(1, Ordering::SeqCst);
            tracing::info!("player config table updated → cipher rebuild");
        }
        changed
    }
}

fn parse_table(json: &str) -> HashMap<String, PlayerConfig> {
    serde_json::from_str::<Vec<PlayerConfig>>(json)
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.hash.clone(), c))
        .collect()
}

fn merge(into: &mut HashMap<String, PlayerConfig>, from: HashMap<String, PlayerConfig>) {
    for (k, v) in from {
        into.insert(k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_looks_up() {
        let table = parse_table(
            r#"[{"hash":"abc","sts":20632,"sigFn":"Lf"},{"hash":"def","nFn":"En"}]"#,
        );
        assert_eq!(table.get("abc").unwrap().sts, Some(20632));
        assert_eq!(table.get("abc").unwrap().sig_fn.as_deref(), Some("Lf"));
        assert_eq!(table.get("def").unwrap().n_fn.as_deref(), Some("En"));
        assert!(table.get("def").unwrap().sig_fn.is_none());
    }

    #[test]
    fn bundled_asset_is_valid_json() {
        // Guards against a corrupt bundled table (a build bug). Empty is fine.
        let _ = parse_table(BUNDLED);
    }

    #[test]
    fn merge_overlays_incoming() {
        let mut a = parse_table(r#"[{"hash":"x","sts":1}]"#);
        merge(&mut a, parse_table(r#"[{"hash":"x","sts":2},{"hash":"y","sts":3}]"#));
        assert_eq!(a.get("x").unwrap().sts, Some(2));
        assert_eq!(a.get("y").unwrap().sts, Some(3));
    }
}
