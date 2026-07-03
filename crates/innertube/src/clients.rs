//! YouTube client identities (impersonation). context/02.
//!
//! Constants are copied verbatim from Metrolist's `YouTubeClient.kt` into the bundled
//! `clients.json` (config, not hardcoded — see context/10 D-table). An optional override
//! file in the app data dir can replace it without a recompile when versions rotate.

use std::collections::HashMap;

use serde::Deserialize;

/// A bag of identity strings + feature flags. context/02.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YouTubeClient {
    /// Goes in `context.client.clientName` and is the string name.
    pub client_name: String,
    pub client_version: String,
    /// The NUMERIC id → `X-YouTube-Client-Name` header (as a string).
    pub client_id: String,
    pub user_agent: String,

    #[serde(default)]
    pub os_name: Option<String>,
    #[serde(default)]
    pub os_version: Option<String>,
    #[serde(default)]
    pub device_make: Option<String>,
    #[serde(default)]
    pub device_model: Option<String>,
    #[serde(default)]
    pub android_sdk_version: Option<String>,
    #[serde(default)]
    pub build_id: Option<String>,
    #[serde(default)]
    pub cronet_version: Option<String>,
    #[serde(default)]
    pub package_name: Option<String>,
    #[serde(default)]
    pub friendly_name: Option<String>,

    #[serde(default)]
    pub login_supported: bool,
    #[serde(default)]
    pub login_required: bool,
    #[serde(default)]
    pub use_signature_timestamp: bool,
    #[serde(default)]
    pub is_embedded: bool,
    /// Web client: needs PoToken + n-transform (deferred to Phase 2).
    #[serde(default)]
    pub use_web_po_tokens: bool,
}

const BUNDLED: &str = include_str!("../clients.json");

/// The client registry, loaded once at startup. Phase 1 ships four:
/// `WEB_REMIX` (metadata endpoints only — search/next), and the three direct-URL stream
/// clients `VISIONOS`, `ANDROID_VR_1_43_32`, `IOS`.
#[derive(Debug, Clone)]
pub struct Clients(HashMap<String, YouTubeClient>);

impl Clients {
    /// Parse the bundled `clients.json`. Panics only on a corrupt bundled asset (a build bug).
    pub fn bundled() -> Self {
        Clients(serde_json::from_str(BUNDLED).expect("bundled clients.json is valid"))
    }

    /// Parse a caller-supplied override (app data dir). Falls back to bundled on error.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        Ok(Clients(serde_json::from_str(json)?))
    }

    /// Look up a client by its registry key (e.g. `"VISIONOS"`, `"ANDROID_VR_1_43_32"`).
    pub fn get(&self, key: &str) -> Option<&YouTubeClient> {
        self.0.get(key)
    }
}

/// Registry keys for the Phase 1 stream fallback order (context/06 §minimal-but-correct).
/// Direct-URL clients only — no cipher, no PoToken this phase.
pub const STREAM_FALLBACK_ORDER: [&str; 3] = ["VISIONOS", "ANDROID_VR_1_43_32", "IOS"];

/// The metadata client for search/next (renderer shape only comes back as WEB_REMIX).
/// NOT used for `/player` this phase.
pub const METADATA_CLIENT: &str = "WEB_REMIX";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_clients_parse() {
        let clients = Clients::bundled();
        for key in STREAM_FALLBACK_ORDER {
            assert!(clients.get(key).is_some(), "missing stream client {key}");
        }
        assert!(clients.get(METADATA_CLIENT).is_some());
    }

    #[test]
    fn client_numeric_ids_are_strings() {
        let c = Clients::bundled();
        assert_eq!(c.get("WEB_REMIX").unwrap().client_id, "67");
        assert_eq!(c.get("VISIONOS").unwrap().client_id, "101");
        assert_eq!(c.get("ANDROID_VR_1_43_32").unwrap().client_id, "28");
        assert_eq!(c.get("IOS").unwrap().client_id, "5");
    }
}
