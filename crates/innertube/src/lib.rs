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

pub use clients::{Clients, YouTubeClient, MAIN_CLIENT, METADATA_CLIENT, STREAM_FALLBACK_ORDER};
pub use models::browse::{
    AlbumPage, ArtistCarousel, ArtistPage, BrowseItem, HomePage, PlaylistContinuation, PlaylistPage,
    SearchResults, Section,
};
pub use models::context::Locale;
pub use models::metadata::{AccountInfo, NextResult, SearchResult, SongItem};
pub use models::player::{
    find_format, AudioQuality, Format, PlaybackTracking, PlayerResponse, StreamingData,
};
pub use rustypipe_fallback::{FallbackError, StreamCandidate};
pub use transport::{cookie_sapisid, generate_cpn, Error, InnerTube, Session};
