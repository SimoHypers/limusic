//! The `context` object sent in every request body. context/01, Metrolist `Context.kt`.

use serde::Serialize;

use crate::clients::YouTubeClient;

/// Locale: `gl` = country code, `hl` = BCP-47 language tag. context/01.
#[derive(Debug, Clone, Serialize)]
pub struct Locale {
    pub gl: String,
    pub hl: String,
}

impl Default for Locale {
    fn default() -> Self {
        Locale { gl: "US".into(), hl: "en".into() }
    }
}

// The three load-bearing JSON flags (context/01) are realized structurally here:
// - ignoreUnknownKeys → serde ignores unknown fields on Deserialize by default.
// - explicitNulls = false → `skip_serializing_if = "Option::is_none"` on every Option.
// - encodeDefaults = true → non-Option fields are always emitted (e.g. useSsl, contentCheckOk).

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub client: Client,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub third_party: Option<ThirdParty>,
    pub request: Request,
    pub user: User,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub client_name: String,
    pub client_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_make: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android_sdk_version: Option<String>,
    pub gl: String,
    pub hl: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visitor_data: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThirdParty {
    pub embed_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub internal_experiment_flags: Vec<String>,
    pub use_ssl: bool,
}

impl Default for Request {
    fn default() -> Self {
        Request { internal_experiment_flags: Vec::new(), use_ssl: true }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub locked_safety_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_behalf_of_user: Option<String>,
}

impl YouTubeClient {
    /// Build the `context` object for this client. Port of `YouTubeClient.toContext`.
    /// `on_behalf_of_user` (dataSyncId) is set only when the client supports login.
    pub fn to_context(
        &self,
        locale: &Locale,
        visitor_data: Option<&str>,
        data_sync_id: Option<&str>,
    ) -> Context {
        Context {
            client: Client {
                client_name: self.client_name.clone(),
                client_version: self.client_version.clone(),
                os_name: self.os_name.clone(),
                os_version: self.os_version.clone(),
                device_make: self.device_make.clone(),
                device_model: self.device_model.clone(),
                android_sdk_version: self.android_sdk_version.clone(),
                gl: locale.gl.clone(),
                hl: locale.hl.clone(),
                visitor_data: visitor_data.map(str::to_owned),
            },
            third_party: self.is_embedded.then(|| ThirdParty {
                // embedUrl is filled per-video by the /player builder for embedded clients.
                embed_url: String::new(),
            }),
            request: Request::default(),
            user: User {
                locked_safety_mode: false,
                on_behalf_of_user: if self.login_supported {
                    data_sync_id.map(str::to_owned)
                } else {
                    None
                },
            },
        }
    }
}
