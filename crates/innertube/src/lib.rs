//! Pure-Rust InnerTube transport + client identities + models + endpoints + rustypipe fallback.
//!
//! The boundary rule (context/11): this crate knows nothing about Tauri, webviews, mpv, or the
//! OS. It is unit-testable against JSON fixtures with no network. Cipher/PoToken/WEB_REMIX
//! streaming are Phase 2 and deliberately absent here.

pub mod clients;
pub mod endpoints;
pub mod models;
pub mod rustypipe_fallback;
pub mod transport;

pub use clients::{Clients, YouTubeClient, METADATA_CLIENT, STREAM_FALLBACK_ORDER};
pub use models::context::Locale;
pub use models::metadata::{NextResult, SearchResult, SongItem};
pub use models::player::{find_format, AudioQuality, Format, PlayerResponse, StreamingData};
pub use rustypipe_fallback::{FallbackError, StreamCandidate};
pub use transport::{Error, InnerTube, Session};
