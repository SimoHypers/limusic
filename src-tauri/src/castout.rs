//! Casting (#399): the devices on the network, and the one Limusic is playing on.
//!
//! While a cast runs, the device is the player. `AppState` sends the transport here (its `out_*`
//! methods), and what the device reports comes back as the same [`PlayerEvent`]s mpv sends, on
//! the same pump. So the queue, history, scrobbling, MPRIS and the windows treat a cast exactly
//! like local playback, and mpv sits stopped until the cast ends.
//!
//! The device holds two items: the track playing and the next one, which it buffers ahead so
//! songs follow each other without a gap. The audio reaches it through the LAN audio proxy
//! (`audioproxy::register_lan`), never as a YouTube URL: those are signed for the address that
//! resolved them.
//!
//! Each item is known by its URL. Every registration makes a new one, so the URL the device
//! reports says which item it is on, even with the same song twice in the queue.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cast::{CastDevice, Closed, Command, Event, Media, Repeat, State};
use innertube::SongItem;
use player::PlayerEvent;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::orchestrator::PlaybackData;

/// How long discovery keeps running after the picker closes, so reopening it is instant.
const DISCOVERY_LINGER: Duration = Duration::from_secs(180);
/// A load the device hasn't fetched anything for in this long can't reach this computer.
const FETCH_DEADLINE: Duration = Duration::from_secs(8);
/// How long a new cast may take to report its first status before it is abandoned.
const SYNC_DEADLINE: Duration = Duration::from_secs(10);
/// How often the position is carried forward between the device's reports.
const TICK: Duration = Duration::from_millis(250);
/// The fastest the speaker's level is changed during a drag.
const VOLUME_EVERY: Duration = Duration::from_millis(120);

/// What the session tells the pump, tagged with the session it belongs to.
#[derive(Debug)]
pub enum Signal {
    /// The device as a player: what mpv would have said.
    Player(PlayerEvent),
    /// The first status of a new cast: the device has the music.
    Synced,
    /// The speaker's volume, 0 to 100.
    Volume(i64),
    /// The device refused the item, whether it ever fetched a byte of it, and how many items in
    /// a row it has refused.
    Refused { fetched: bool, count: u32 },
    /// The device never fetched the item it was handed.
    Unreachable,
    /// A new cast never reported a status.
    NoStatus,
    /// The connection ended.
    Closed(Closed),
}

/// Why a cast couldn't start, as the code the UI translates (`cast.<code>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    Gone,
    Unreachable,
    Refused,
    LeaveRoom,
}

impl StartError {
    pub fn code(self) -> &'static str {
        match self {
            StartError::Gone => "gone",
            StartError::Unreachable => "unreachable",
            StartError::Refused => "refused",
            StartError::LeaveRoom => "leave_room",
        }
    }
}

/// A device as the windows see it: no address, they have no use for one.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub kind: cast::Kind,
}

impl From<&CastDevice> for DeviceView {
    fn from(d: &CastDevice) -> Self {
        DeviceView { id: d.id.clone(), name: d.name.clone(), model: d.model.clone(), kind: d.kind }
    }
}

/// The carried-forward position: where the device last said it was, and when.
#[derive(Clone, Copy)]
struct Clock {
    base: f64,
    at: Instant,
    running: bool,
}

impl Clock {
    fn now(&self) -> f64 {
        if self.running {
            self.base + self.at.elapsed().as_secs_f64()
        } else {
            self.base
        }
    }

    fn set(&mut self, base: f64, running: bool) {
        *self = Clock { base, at: Instant::now(), running };
    }
}

struct Active {
    device: CastDevice,
    tx: cast::Sender,
    epoch: u64,
    /// The first status arrived.
    synced: bool,
    /// The URL of the item the device is on, and of the one queued after it.
    cur: Option<String>,
    next: Option<String>,
    /// Playing or buffering, as the windows were last told.
    playing: bool,
    idle: bool,
    /// The device is on an item Limusic didn't hand it: the next track is loaded fresh.
    desynced: bool,
    clock: Clock,
    /// The media session the device was on when the last load went out. Its statuses describe
    /// what the load replaced, and are dropped until a newer session shows up.
    load_from: Option<i64>,
    /// The newest media session the device reported.
    session: i64,
    duration: Option<f64>,
    volume: Option<i64>,
    /// A level waiting to be sent.
    volume_due: Option<i64>,
    repeat_one: bool,
    /// Loads refused in a row. Reset once something plays.
    refusals: u32,
}

pub struct CastManager {
    app: AppHandle,
    devices: Mutex<Vec<CastDevice>>,
    browser: Mutex<Option<cast::Browser>>,
    picker_open: AtomicBool,
    /// Bumped every time the picker closes: a stop scheduled by an earlier close is void.
    picker_gen: AtomicU64,
    active: Mutex<Option<Active>>,
    /// The device being connected to, for the picker's spinner.
    connecting: Mutex<Option<String>>,
    epoch: AtomicU64,
    signals: mpsc::UnboundedSender<(u64, Signal)>,
    signals_rx: Mutex<Option<mpsc::UnboundedReceiver<(u64, Signal)>>>,
}

impl CastManager {
    pub fn new(app: AppHandle) -> Self {
        let (signals, rx) = mpsc::unbounded_channel();
        CastManager {
            app,
            devices: Mutex::new(Vec::new()),
            browser: Mutex::new(None),
            picker_open: AtomicBool::new(false),
            picker_gen: AtomicU64::new(0),
            active: Mutex::new(None),
            connecting: Mutex::new(None),
            epoch: AtomicU64::new(0),
            signals,
            signals_rx: Mutex::new(Some(rx)),
        }
    }

    /// The pump's end of the signals. Taken once, at startup.
    pub fn take_signals(&self) -> Option<mpsc::UnboundedReceiver<(u64, Signal)>> {
        self.signals_rx.lock().ok()?.take()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn with<R>(&self, f: impl FnOnce(&mut Active) -> R) -> Option<R> {
        self.active.lock().ok()?.as_mut().map(f)
    }

    pub fn is_active(&self) -> bool {
        self.active.lock().is_ok_and(|a| a.is_some())
    }

    pub fn device(&self) -> Option<CastDevice> {
        self.with(|a| a.device.clone())
    }

    // ---- discovery ----

    /// The picker opened or closed. Discovery starts with the first open, and stops a while
    /// after the last close unless something is casting.
    pub fn set_picker_open(self: &std::sync::Arc<Self>, open: bool) {
        self.picker_open.store(open, Ordering::SeqCst);
        if open {
            self.start_discovery();
            self.emit_devices();
            return;
        }
        let gen = self.picker_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(DISCOVERY_LINGER).await;
            let stale = me.picker_gen.load(Ordering::SeqCst) != gen;
            if !stale && !me.picker_open.load(Ordering::SeqCst) && !me.is_active() {
                me.stop_discovery();
            }
        });
    }

    fn start_discovery(self: &std::sync::Arc<Self>) {
        let Ok(mut browser) = self.browser.lock() else { return };
        if browser.is_some() {
            return;
        }
        let (mut events, handle) = match cast::browse() {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "cast: discovery could not start");
                return;
            }
        };
        *browser = Some(handle);
        tracing::info!("cast: discovery started");
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = events.recv().await {
                if let Ok(mut devices) = me.devices.lock() {
                    match event {
                        cast::DiscoveryEvent::Found(d) => {
                            devices.retain(|x| x.id != d.id);
                            devices.push(d);
                            devices.sort_by_key(|d| d.name.to_lowercase());
                        }
                        cast::DiscoveryEvent::Lost(id) => devices.retain(|x| x.id != id),
                    }
                }
                me.emit_devices();
            }
        });
    }

    fn stop_discovery(&self) {
        if let Ok(mut browser) = self.browser.lock() {
            if browser.take().is_some() {
                tracing::info!("cast: discovery stopped");
            }
        }
    }

    pub fn devices(&self) -> Vec<DeviceView> {
        self.devices.lock().map(|d| d.iter().map(DeviceView::from).collect()).unwrap_or_default()
    }

    fn emit_devices(&self) {
        let _ = self.app.emit("cast-devices", self.devices());
    }

    /// What is casting, and what is being connected to: the `cast-state` payload.
    pub fn state(&self) -> serde_json::Value {
        let device = self.device().as_ref().map(DeviceView::from);
        let connecting = self.connecting.lock().ok().and_then(|c| c.clone());
        serde_json::json!({ "device": device, "connecting": connecting })
    }

    fn emit_state(&self) {
        let _ = self.app.emit("cast-state", self.state());
    }

    // ---- the connection ----

    /// Connect to a device. On success it is the player from here on: the caller hands it the
    /// music. A device already casting is stopped once the new one answers, so two speakers
    /// never play the same song half a beat apart.
    pub async fn connect(self: &std::sync::Arc<Self>, id: &str) -> Result<(), StartError> {
        let device = self
            .devices
            .lock()
            .ok()
            .and_then(|d| d.iter().find(|d| d.id == id).cloned())
            .ok_or(StartError::Gone)?;
        self.set_connecting(Some(id));
        tracing::info!(device = %device.name, model = ?device.model, "cast: connecting");
        let connected = cast::connect(device.ip, device.port).await;
        self.set_connecting(None);
        let (tx, events) = match connected {
            Ok(c) => c,
            Err(Closed::Refused(reason)) => {
                tracing::warn!(reason, "cast: the device refused the receiver");
                return Err(StartError::Refused);
            }
            Err(e) => {
                tracing::warn!(error = ?e, "cast: could not connect");
                return Err(StartError::Unreachable);
            }
        };
        self.stop().await;
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut active) = self.active.lock() {
            *active = Some(Active {
                device: device.clone(),
                tx,
                epoch,
                synced: false,
                cur: None,
                next: None,
                playing: false,
                idle: true,
                desynced: false,
                clock: Clock { base: 0.0, at: Instant::now(), running: false },
                load_from: None,
                session: 0,
                duration: None,
                volume: None,
                volume_due: None,
                repeat_one: false,
                refusals: 0,
            });
        }
        crate::audioproxy::start_lan(device.ip, device.kind == cast::Kind::Group);
        self.spawn_session(epoch, events);
        self.emit_state();
        Ok(())
    }

    fn set_connecting(&self, id: Option<&str>) {
        if let Ok(mut c) = self.connecting.lock() {
            *c = id.map(str::to_owned);
        }
        self.emit_state();
    }

    /// Stop the device and let it go. The caller decides where the music goes next.
    pub async fn stop(&self) {
        let Some(active) = self.active.lock().ok().and_then(|mut a| a.take()) else { return };
        self.epoch.fetch_add(1, Ordering::SeqCst);
        active.tx.close(Duration::from_secs(2)).await;
        crate::audioproxy::stop_lan();
        tracing::info!(device = %active.device.name, "cast: stopped");
        self.emit_state();
    }

    /// The connection ended by itself: forget it without talking to the device.
    pub fn forget(&self, epoch: u64) -> bool {
        let Ok(mut active) = self.active.lock() else { return false };
        if !matches!(active.as_ref(), Some(a) if a.epoch == epoch) {
            return false;
        }
        *active = None;
        drop(active);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        crate::audioproxy::stop_lan();
        self.emit_state();
        true
    }

    /// Whether the first status of this cast arrived.
    pub fn synced(&self) -> bool {
        self.with(|a| a.synced).unwrap_or(false)
    }

    // ---- the player ----

    /// The device's item for a track, published on the LAN for it. `None` when it can't be cast.
    pub fn media_for(&self, item: &SongItem, data: &PlaybackData) -> Option<Media> {
        if data.live || crate::local::is_local_song(&item.video_id) {
            return None;
        }
        let device_ip = self.device()?.ip;
        let url = crate::audioproxy::register_lan(&data.stream_url, &data.headers, device_ip)?;
        let duration = crate::state::parse_duration_ms(item.duration.as_deref());
        Some(Media {
            url,
            content_type: content_type(&data.stream_url, data.itag),
            title: item.title.clone(),
            artist: item.artists.clone(),
            album: item.album.clone(),
            image: item.thumbnail.as_deref().map(cover_url),
            duration: (duration > 0).then(|| duration as f64 / 1000.0),
        })
    }

    /// Play this item from `start`, replacing whatever the device had.
    pub fn load(self: &std::sync::Arc<Self>, media: Media, start: f64, autoplay: bool) {
        let url = media.url.clone();
        let Some((epoch, repeat)) = self.with(|a| {
            a.cur = Some(url.clone());
            a.next = None;
            a.idle = false;
            a.desynced = false;
            a.duration = media.duration;
            a.clock.set(start, false);
            a.load_from = Some(a.session);
            let repeat = if a.repeat_one { Repeat::One } else { Repeat::Off };
            a.tx.send(Command::Load { items: vec![media], start, autoplay, repeat });
            (a.epoch, repeat)
        }) else {
            return;
        };
        tracing::info!(start, autoplay, ?repeat, "cast: load");
        // A device that can't reach this computer says nothing for a long time, and then often
        // only "failed". The proxy knows sooner whether it ever asked.
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FETCH_DEADLINE).await;
            let same = me.with(|a| a.epoch == epoch && a.cur.as_deref() == Some(&url));
            if same == Some(true) && !crate::audioproxy::lan_fetched(&url) {
                let _ = me.signals.send((epoch, Signal::Unreachable));
            }
        });
    }

    /// Queue the next track behind the one playing.
    pub fn append(&self, media: Media) {
        self.with(|a| {
            a.next = Some(media.url.clone());
            a.tx.send(Command::Append(media));
        });
    }

    /// Drop the queued next track (the queue changed under it).
    pub fn drop_next(&self) {
        self.with(|a| {
            if a.next.take().is_some() {
                a.tx.send(Command::DropNext);
            }
        });
    }

    /// Whether the next track is queued on the device already.
    pub fn has_next(&self) -> bool {
        self.with(|a| a.next.is_some()).unwrap_or(false)
    }

    /// Go to the queued next track, which the device has buffered already.
    pub fn next(&self) {
        self.with(|a| a.tx.send(Command::Next));
    }

    /// Play or pause. The windows hear it at once; the device's status follows.
    pub fn set_playing(&self, playing: bool) {
        let changed = self.with(|a| {
            if a.idle {
                return false;
            }
            a.tx.send(if playing { Command::Play } else { Command::Pause });
            let now = a.clock.now();
            a.clock.set(now, playing);
            let changed = a.playing != playing;
            a.playing = playing;
            changed
        });
        if changed == Some(true) {
            self.player(PlayerEvent::Playing(playing));
            self.player(PlayerEvent::Paused(!playing));
        }
    }

    pub fn toggle(&self) {
        if let Some(playing) = self.with(|a| a.playing) {
            self.set_playing(!playing);
        }
    }

    pub fn seek(&self, to: f64) {
        self.with(|a| {
            a.tx.send(Command::Seek(to));
            let running = a.clock.running;
            a.clock.set(to, running);
        });
        self.player(PlayerEvent::Position(to));
    }

    pub fn set_repeat_one(&self, on: bool) {
        self.with(|a| {
            a.repeat_one = on;
            a.tx.send(Command::Repeat(if on { Repeat::One } else { Repeat::Off }));
        });
    }

    /// Nothing to play on the device, as `Player::is_idle` says for mpv.
    pub fn is_idle(&self) -> bool {
        self.with(|a| a.idle || a.cur.is_none() || a.desynced).unwrap_or(true)
    }

    pub fn position(&self) -> Option<f64> {
        self.with(|a| a.clock.now())
    }

    pub fn volume(&self) -> Option<i64> {
        self.with(|a| a.volume).flatten()
    }

    /// The speaker's level, 0 to 100. A slider drag sends one of these per frame; the device
    /// gets the latest at most every [`VOLUME_EVERY`].
    pub fn set_volume(self: &std::sync::Arc<Self>, level: i64) {
        let first = self.with(|a| {
            a.volume = Some(level);
            a.volume_due.replace(level).is_none()
        });
        if first != Some(true) {
            return;
        }
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(VOLUME_EVERY).await;
            me.with(|a| {
                if let Some(level) = a.volume_due.take() {
                    a.tx.send(Command::Volume(level as f64 / 100.0));
                }
            });
        });
    }

    fn player(&self, event: PlayerEvent) {
        let _ = self.signals.send((self.epoch(), Signal::Player(event)));
    }

    // ---- what the device says ----

    fn spawn_session(
        self: &std::sync::Arc<Self>,
        epoch: u64,
        mut events: mpsc::UnboundedReceiver<Event>,
    ) {
        let signals = self.signals.clone();
        let send = move |s: Signal| {
            let _ = signals.send((epoch, s));
        };
        // The clock, carried forward between reports.
        let (me, tick_send) = (self.clone(), send.clone());
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(TICK);
            while me.epoch() == epoch {
                tick.tick().await;
                if let Some(Some(pos)) = me.with(|a| a.clock.running.then(|| a.clock.now())) {
                    tick_send(Signal::Player(PlayerEvent::Position(pos)));
                }
            }
        });
        let (me, sync_send) = (self.clone(), send.clone());
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SYNC_DEADLINE).await;
            let loaded = me.with(|a| a.cur.is_some()) == Some(true);
            if me.epoch() == epoch && loaded && !me.synced() {
                sync_send(Signal::NoStatus);
            }
        });
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = events.recv().await {
                if me.epoch() != epoch {
                    return;
                }
                match event {
                    Event::Media(status) => {
                        for s in me.on_status(status) {
                            send(s);
                        }
                    }
                    Event::Volume { level, muted } => {
                        let level = if muted { 0 } else { (level * 100.0).round() as i64 };
                        if me.with(|a| a.volume.replace(level) != Some(level)) == Some(true) {
                            send(Signal::Volume(level));
                        }
                    }
                    Event::LoadFailed(reason) => {
                        tracing::warn!(reason, "cast: the device refused the track");
                        let url = me.with(|a| a.cur.clone()).flatten();
                        let fetched = url.as_deref().is_some_and(crate::audioproxy::lan_fetched);
                        let count = me
                            .with(|a| {
                                a.idle = true;
                                a.refusals += 1;
                                a.refusals
                            })
                            .unwrap_or(1);
                        send(Signal::Refused { fetched, count });
                    }
                    Event::Closed(reason) => {
                        tracing::info!(?reason, "cast: the connection ended");
                        send(Signal::Closed(reason));
                        return;
                    }
                }
            }
        });
    }

    /// One media status, as the player events it amounts to.
    fn on_status(&self, status: Option<cast::MediaStatus>) -> Vec<Signal> {
        let mut out = Vec::new();
        let Ok(mut guard) = self.active.lock() else { return out };
        let Some(a) = guard.as_mut() else { return out };
        let Some(s) = status else {
            // No media session: the device stopped, or never started. Nothing to report until
            // it has something.
            return out;
        };
        if let Some(from) = a.load_from {
            if s.session <= from {
                return out; // about what the last load replaced
            }
            a.load_from = None;
        }
        a.session = s.session;
        // The device has the music once it plays it (or holds it paused): this computer stops
        // then, not on the first "buffering", so the handover has no silence in it.
        if !a.synced && a.cur.is_some() && matches!(s.state, State::Playing | State::Paused) {
            a.synced = true;
            out.push(Signal::Synced);
        }
        let on = s.content_id.as_deref();
        if on.is_some() && on == a.next.as_deref() {
            // The device moved on to the queued track by itself: what mpv calls a gapless
            // advance. The one it left is cleared off its queue.
            a.cur = a.next.take();
            a.duration = s.duration;
            a.clock.set(s.position, s.state == State::Playing);
            a.tx.send(Command::TrimPlayed);
            out.push(Signal::Player(PlayerEvent::TrackEnded));
        } else if on.is_some() && on != a.cur.as_deref() {
            // An item Limusic didn't hand over (a stale one, or someone else's). The app loads
            // its own next track.
            if !a.desynced && !a.idle {
                tracing::warn!("cast: the device is on an item it wasn't given");
                a.desynced = true;
                out.push(Signal::Player(PlayerEvent::TrackEnded));
            }
            return out;
        }
        let playing = matches!(s.state, State::Playing | State::Buffering);
        match s.state {
            State::Idle => {
                a.clock.set(a.clock.now(), false);
                // Quietly: the queue decides what idle means (the next track, or the end).
                a.playing = false;
                let was_idle = std::mem::replace(&mut a.idle, true);
                match s.idle_reason.as_deref() {
                    _ if was_idle => {}
                    Some("FINISHED") if a.repeat_one => {
                        // A receiver that doesn't loop by itself: start it again from the top.
                        a.idle = false;
                        a.clock.set(0.0, false);
                        a.tx.send(Command::Seek(0.0));
                        a.tx.send(Command::Play);
                        out.push(Signal::Player(PlayerEvent::Looped));
                    }
                    Some("FINISHED") => out.push(Signal::Player(PlayerEvent::TrackEnded)),
                    Some("ERROR") => out.push(Signal::Player(PlayerEvent::TrackFailed(
                        "the cast device could not play this track".into(),
                    ))),
                    // Cancelled or interrupted: our own stop or load.
                    _ => {}
                }
            }
            _ => {
                a.idle = false;
                if s.state == State::Playing {
                    a.refusals = 0;
                }
                // Repeat-one looping on the device: the position jumps back to the top.
                if a.repeat_one && a.clock.base > 10.0 && s.position < 3.0 && a.playing {
                    out.push(Signal::Player(PlayerEvent::Looped));
                }
                a.clock.set(s.position, s.state == State::Playing);
                out.push(Signal::Player(PlayerEvent::Position(s.position)));
            }
        }
        if let Some(d) = s.duration.filter(|d| *d > 0.0) {
            if a.duration != Some(d) {
                a.duration = Some(d);
                out.push(Signal::Player(PlayerEvent::Duration(d)));
            }
        }
        if s.state != State::Idle && a.playing != playing {
            a.playing = playing;
            out.push(Signal::Player(PlayerEvent::Playing(playing)));
            out.push(Signal::Player(PlayerEvent::Paused(!playing)));
        }
        out
    }
}

/// The MIME type the device is told, from the stream URL's `mime` parameter or its itag.
fn content_type(url: &str, itag: i64) -> String {
    let from_url = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "mime").map(|(_, v)| v.into_owned()));
    from_url.unwrap_or_else(|| {
        match itag {
            249..=251 => "audio/webm",
            _ => "audio/mp4",
        }
        .to_owned()
    })
}

/// The cover at a size worth putting on a TV. YouTube's thumbnail URLs carry their size after
/// the last `=`: `=w120-h120-l90-rj` or `=s120`.
fn cover_url(url: &str) -> String {
    let Some((head, spec)) = url.rsplit_once('=') else { return url.to_owned() };
    let mut parts: Vec<&str> = spec.split('-').collect();
    match parts.first() {
        Some(p) if p.starts_with('s') && p[1..].parse::<u32>().is_ok() => parts[0] = "s544",
        Some(p) if p.starts_with('w') && parts.get(1).is_some_and(|h| h.starts_with('h')) => {
            parts[0] = "w544";
            parts[1] = "h544";
        }
        _ => return url.to_owned(),
    }
    format!("{head}={}", parts.join("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_is_told_the_streams_own_media_type() {
        let opus = "https://r1.googlevideo.com/videoplayback?itag=251&mime=audio%2Fwebm&x=1";
        assert_eq!(content_type(opus, 251), "audio/webm");
        assert_eq!(content_type("https://example.com/a", 140), "audio/mp4");
        assert_eq!(content_type("https://example.com/a", 251), "audio/webm");
    }

    #[test]
    fn the_cover_is_asked_for_at_tv_size() {
        assert_eq!(
            cover_url("https://lh3.googleusercontent.com/abc=w120-h120-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj"
        );
        assert_eq!(
            cover_url("https://lh3.googleusercontent.com/abc=s60"),
            "https://lh3.googleusercontent.com/abc=s544"
        );
        assert_eq!(
            cover_url("https://i.ytimg.com/vi/x/hq720.jpg"),
            "https://i.ytimg.com/vi/x/hq720.jpg"
        );
    }

    #[test]
    fn the_clock_runs_only_while_playing() {
        let mut c = Clock { base: 0.0, at: Instant::now(), running: false };
        c.set(30.0, false);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(c.now(), 30.0);
        c.set(30.0, true);
        std::thread::sleep(Duration::from_millis(20));
        assert!(c.now() > 30.0 && c.now() < 31.0);
    }
}
