//! PoToken / BotGuard generator (context/04, context/13).
//!
//! Two tokens per session/video (context/04): a session token (minted from visitorData, cached in
//! Rust with its TTL — [`PoTokenGenerator::get_session_po_token`]) -> the `/player` request body; a
//! per-video streaming token (minted from videoId, lazy —
//! [`PoTokenGenerator::get_streaming_po_token`]) -> `&pot=` on the stream URL. Everything is wrapped
//! so a failure or timeout returns `None` and the orchestrator falls through to the non-PoToken
//! clients (graceful degradation — context/06 §5).

#[cfg(not(target_os = "android"))]
pub(crate) mod botguard;
#[cfg(not(target_os = "android"))]
mod desktop;
#[cfg(not(target_os = "android"))]
pub use desktop::*;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use android::*;
