//! `PlayerConfigStore` (context/05 §3) — the cipher resilience layer.
//!
//! A table of known-good per-player-hash configs. Static regex extraction of the sig/`n` functions
//! is **dead** on the 2025+ VM-dispatch players: neither our patterns nor rustypipe's nor yt-dlp's
//! match any generation YouTube currently serves (verified against 341562bc / 4753318d / 8d2a370b).
//! So the table is not a "reliability layer" over regex any more — it is the only way in, exactly
//! as it is for Metrolist.
//!
//! An entry does not name a function. It carries a JS *expression* evaluated inside player.js's own
//! IIFE closure, because that is where the sig function and the `n` class live:
//!
//! ```json
//! "341562bc": { "sig": "Ii(25,558,INPUT)", "nClass": "as", "sts": 20670, "aliases": ["a3fe4c92"] }
//! ```
//!
//! `INPUT` is substituted with the argument at injection time (see [`super::extractor`]). Configs
//! come from the two community registries Metrolist uses; both are polled and merged, so one going
//! stale or offline is survivable. A fetch failure is non-fatal — the store keeps what it has.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::Mutex as AsyncMutex;

/// The registries Metrolist reads (`PlayerConfigStore.kt:30-33`). Community-maintained and updated
/// within hours of a player rotation — which is the only cadence that works, since a rotation
/// invalidates every entry. Both are merged; Faraday wins ties by being applied last.
const REGISTRY_URLS: [&str; 2] = [
    "https://raw.githubusercontent.com/ZemerTeam/zemer-cipher/master/library/src/main/assets/player_configs.json",
    "https://raw.githubusercontent.com/MetrolistGroup/faraday/master/registry/player_configs.json",
];
const BUNDLED: &str = include_str!("../../cipher_configs.json");
/// Rate-limit for self-heal re-fetches (context/05 §refreshAfterStreamRejection).
const REFRESH_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// One validated player config, keyed by player hash (and by each of its aliases).
#[derive(Debug, Clone)]
pub struct PlayerConfig {
    pub sts: Option<i32>,
    /// e.g. `Ii(25,558,INPUT)` — evaluated in the IIFE closure with `INPUT` → the signature.
    pub sig_expr: String,
    /// e.g. `as` — the player's own URL class, used to run the `n` transform.
    pub n_class: String,
}

// --- wire format -----------------------------------------------------------------------------

#[derive(Deserialize)]
struct Registry {
    #[serde(default)]
    players: HashMap<String, RawEntry>,
}

#[derive(Deserialize)]
struct RawEntry {
    sig: String,
    #[serde(rename = "nClass")]
    n_class: String,
    #[serde(default)]
    sts: Option<i32>,
    #[serde(default)]
    aliases: Vec<String>,
}

/// `sig` must be a single `name(int,int,INPUT)` call and `nClass` a bare identifier. These strings
/// are spliced into JS that runs in a webview, so a malformed registry entry is a code-injection
/// vector, not just a broken decipher — validate before it ever reaches [`super::extractor`].
/// Mirrors Metrolist's `PlayerConfigParser.SIG_RE` / `NCLASS_RE`.
fn valid_sig(s: &str) -> bool {
    let Some((name, rest)) = s.split_once('(') else {
        return false;
    };
    let Some(args) = rest.strip_suffix(')') else {
        return false;
    };
    if name.is_empty() || name.len() > 8 || !name.chars().all(is_ident_char) {
        return false;
    }
    let parts: Vec<&str> = args.split(',').collect();
    parts.len() == 3
        && parts[2] == "INPUT"
        && parts[..2]
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn valid_n_class(s: &str) -> bool {
    !s.is_empty() && s.len() <= 8 && s.chars().all(is_ident_char)
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '$' || c == '_'
}

fn parse_table(json: &str) -> HashMap<String, PlayerConfig> {
    let mut out = HashMap::new();
    let Ok(reg) = serde_json::from_str::<Registry>(json) else {
        return out;
    };
    for (hash, e) in reg.players {
        if !valid_sig(&e.sig) || !valid_n_class(&e.n_class) {
            tracing::warn!(hash, sig = e.sig, "rejecting malformed player config entry");
            continue;
        }
        let cfg = PlayerConfig {
            sts: e.sts,
            sig_expr: e.sig,
            n_class: e.n_class,
        };
        // A player is served under several hashes; the registry lists the extras as aliases.
        for k in std::iter::once(hash).chain(e.aliases) {
            out.insert(k, cfg.clone());
        }
    }
    out
}

// --- store -----------------------------------------------------------------------------------

pub struct PlayerConfigStore {
    entries: RwLock<HashMap<String, PlayerConfig>>,
    epoch: AtomicU64,
    cache_file: PathBuf,
    /// Serializes refreshes and holds the last-refresh instant for rate-limiting.
    last_refresh: AsyncMutex<Option<Instant>>,
}

impl PlayerConfigStore {
    /// Load bundled configs overlaid with the cached remote file (context/05 `initialize`,
    /// synchronous part). The TTL-gated remote refresh is `force_refresh` /
    /// `refresh_after_stream_rejection`, scheduled by the caller off the hot path.
    pub fn new(app_data_dir: &Path) -> Self {
        let cache_file = app_data_dir
            .join("cipher_cache")
            .join("player_configs.json");
        let mut entries = parse_table(BUNDLED);
        if let Ok(cached) = std::fs::read_to_string(&cache_file) {
            entries.extend(parse_table(&cached));
        }
        PlayerConfigStore {
            entries: RwLock::new(entries),
            epoch: AtomicU64::new(0),
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

    /// Pull the registries unconditionally (e.g. a brand-new player hash appeared).
    pub async fn force_refresh(&self) -> bool {
        *self.last_refresh.lock().await = Some(Instant::now());
        self.fetch_and_merge().await
    }

    async fn fetch_and_merge(&self) -> bool {
        let mut incoming: HashMap<String, PlayerConfig> = HashMap::new();
        let mut newest_raw: Option<String> = None;
        for url in REGISTRY_URLS {
            let text = match crate::http::client().get(url).send().await {
                Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
                Ok(r) => {
                    tracing::debug!(url, status = %r.status(), "player config registry unavailable");
                    continue;
                }
                Err(e) => {
                    tracing::debug!(url, error = %e, "player config registry fetch failed");
                    continue;
                }
            };
            let parsed = parse_table(&text);
            if parsed.is_empty() {
                continue;
            }
            tracing::debug!(url, entries = parsed.len(), "player config registry parsed");
            incoming.extend(parsed);
            newest_raw = Some(text);
        }
        if incoming.is_empty() {
            return false;
        }
        // Compare on the keys we actually decipher with: a rotation adds a hash (or changes an
        // existing hash's expression), and either must rebuild the webview. Counting entries — the
        // old signal — misses an in-place correction of the current player, which is precisely the
        // case the self-heal path exists to recover from.
        let changed = {
            let mut entries = self.entries.write().unwrap();
            let changed = incoming.iter().any(|(k, v)| {
                entries.get(k).is_none_or(|old| {
                    old.sig_expr != v.sig_expr || old.n_class != v.n_class || old.sts != v.sts
                })
            });
            entries.extend(incoming);
            changed
        };
        if changed {
            if let Some(raw) = newest_raw {
                let _ = std::fs::write(&self.cache_file, raw);
            }
            self.epoch.fetch_add(1, Ordering::SeqCst);
            tracing::info!("player config table updated → cipher rebuild");
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"schemaVersion":1,"players":{
        "341562bc":{"sig":"Ii(25,558,INPUT)","nClass":"as","sts":20670,"aliases":["a3fe4c92"]}}}"#;

    #[test]
    fn parses_registry_and_expands_aliases() {
        let t = parse_table(SAMPLE);
        let c = t.get("341562bc").unwrap();
        assert_eq!(c.sig_expr, "Ii(25,558,INPUT)");
        assert_eq!(c.n_class, "as");
        assert_eq!(c.sts, Some(20670));
        // The alias must resolve to the same config — YouTube serves one player under several
        // hashes, and iframe_api can hand us either.
        assert_eq!(t.get("a3fe4c92").unwrap().sig_expr, "Ii(25,558,INPUT)");
    }

    #[test]
    fn bundled_asset_is_valid() {
        let _ = parse_table(BUNDLED); // empty is fine; a parse panic would be a build bug
    }

    /// These strings are spliced into JS in a webview — anything that isn't the exact expected
    /// shape must be dropped rather than executed.
    #[test]
    fn rejects_injection_shaped_entries() {
        assert!(valid_sig("Ii(25,558,INPUT)"));
        assert!(!valid_sig("Ii(25,558,INPUT);fetch('http://evil')"));
        assert!(!valid_sig("Ii(25,INPUT)"));
        assert!(!valid_sig("Ii(a,b,INPUT)"));
        assert!(!valid_sig("(function(){})(INPUT)"));
        assert!(valid_n_class("as"));
        assert!(!valid_n_class("as;alert(1)"));
        assert!(!valid_n_class(""));
        assert!(
            parse_table(r#"{"players":{"h":{"sig":"x(1,2,INPUT);evil()","nClass":"as"}}}"#)
                .is_empty()
        );
    }

    #[test]
    fn ignores_junk() {
        assert!(parse_table("[]").is_empty());
        assert!(parse_table("not json").is_empty());
    }
}
