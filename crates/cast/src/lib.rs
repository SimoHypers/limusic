//! Google Cast for Limusic: finding devices and talking to them.
//!
//! Speaks CastV2 directly to Google's Default Media Receiver: no Google SDK, no account, no key.
//! Knows nothing about Tauri, YouTube or mpv; the app hands it URLs and metadata.

pub mod channel;
pub mod discovery;
mod proto;

pub use channel::{connect, Closed, Command, Event, Media, MediaStatus, Repeat, Sender, State};
pub use discovery::{browse, Browser, CastDevice, DiscoveryEvent, Kind};
