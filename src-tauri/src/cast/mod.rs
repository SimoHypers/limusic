//! Cast: play what Limusic plays on a device on the network.
//!
//! Two kinds of device are supported, and they are found differently:
//!
//! - **Chromecast / Google Home / Nest speakers** announce themselves over mDNS
//!   (`_googlecast._tcp.local`) and are controlled over CastV2 ([`chromecast`]).
//! - **DLNA renderers** answer an SSDP search ([`dlna`]).
//!
//! Both are fed the same thing: a URL the device can fetch. Where that URL points is the
//! device's own business, and the two kinds differ:
//!
//! - A Cast device takes YouTube's own address. The receiver refuses a URL on a private
//!   address before it fetches it at all, so a LAN URL is not an option.
//! - A DLNA renderer takes the loopback audio proxy, published on the LAN for this purpose
//!   (see [`crate::audioproxy`]): a renderer only speaks `http://`, the stream URL needs the
//!   headers this app signed it with, and googlevideo throttles an open-ended request to
//!   about twice realtime. The proxy answers all three.
//!
//! The session owns the device list and the connection to the device that is playing. What
//! plays *is* the queue's business, so the session asks the player to move on when the device
//! says the track ended ([`CastCommand`]).

pub mod chromecast;
pub mod dlna;
pub mod proto;

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use innertube::SongItem;
use serde::Serialize;
use tauri::Emitter;
use tokio::sync::mpsc;

/// The mDNS service a Cast device announces itself under.
const CAST_SERVICE: &str = "_googlecast._tcp.local.";

/// How often the LAN is asked for DLNA renderers. A renderer that is switched on between two
/// searches would otherwise only show up minutes later.
const DLNA_SEARCH_INTERVAL: Duration = Duration::from_secs(45);

/// How often the device is asked what it is doing.
const POLL: Duration = Duration::from_secs(3);

/// How long a renderer may stay silent before it is taken off the list. A search that missed
/// it is not the same as a renderer that is gone.
const RENDERER_GRACE: i64 = 180;

/// A device that can play what Limusic plays.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Stable across renames: the Cast device id, or the renderer's UDN.
    pub id: String,
    /// The name the owner gave it.
    pub name: String,
    /// "chromecast" or "dlna".
    pub kind: String,
    /// The model, when the device reports one.
    pub model: Option<String>,
    /// Where it is.
    pub host: String,
    /// The port it is reached on.
    pub port: u16,
    /// The renderer's control URL. A Cast device has none: it is not SOAP.
    #[serde(skip)]
    control: Option<String>,
    /// The renderer's volume service, when it has one.
    #[serde(skip)]
    rendering: Option<String>,
    /// The mDNS instance name, which is how a device that goes away is recognised. A Cast
    /// device only.
    #[serde(skip)]
    fullname: Option<String>,
    /// When the device was last seen, in unix seconds. A renderer that missed one search is
    /// not gone; it is quiet.
    #[serde(skip)]
    seen_at: i64,
}

/// What to play on a device.
#[derive(Debug, Clone, Default)]
pub struct Media {
    pub url: String,
    /// The MIME type of that URL, as the device is told.
    pub content_type: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// The cover art URL. The device fetches it itself, so it has to be a public one.
    pub thumbnail: Option<String>,
    /// The track's length, when we know it.
    pub duration: Option<f64>,
    /// Where in the track to start.
    pub start_at: Option<f64>,
}

/// One item in the queue handed to a Cast receiver.
#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub media: Media,
    pub video_id: String,
}

/// What the player has to do for the cast session.
#[derive(Debug, Clone)]
pub enum CastCommand {
    /// The device's playhead moved.
    Position {
        position: f64,
        duration: Option<f64>,
        playing: bool,
        /// What the device is doing, so the clock knows whether it is running.
        state: chromecast::PlayerState,
        /// What the device is playing, when it says.
        track: Option<chromecast::DeviceTrack>,
        /// The device's own volume, when it says.
        volume: Option<f64>,
        /// The YouTube id of the item the receiver is now on, when it says.
        video_id: Option<String>,
        /// The URL of the item the receiver is now on. The receiver reports this even when
        /// it drops `customData`.
        content_id: Option<String>,
        /// The receiver queue's repeat mode, when it reports one.
        repeat_mode: Option<String>,
        /// The receiver ran out of queue items.
        finished: bool,
    },
    /// The device went away, or stopped by itself.
    Stopped(String),
}

/// The session: the devices, and the one that is playing.
pub struct CastSession {
    app: tauri::AppHandle,
    inner: Mutex<Inner>,
    /// What the session asks the player for.
    commands: mpsc::UnboundedSender<CastCommand>,
    /// Bumped when the cast starts or stops. The watcher stops when it moves.
    epoch: AtomicU64,
}

struct Inner {
    /// Everything that was found, by id.
    devices: Vec<Device>,
    /// The device that is playing, if any.
    active: Option<Active>,
    /// Whether that device has answered yet. Until it does, the app does not know what it is
    /// playing, and the windows are told so rather than guessing.
    synced: bool,
}

/// The connection to the device that is playing.
struct Active {
    device: Device,
    target: Target,
}

/// How the session talks to the device that is playing.
#[derive(Clone)]
enum Target {
    /// A Cast device: a live channel.
    Cast { sender: chromecast::Sender },
    /// A DLNA renderer: SOAP calls.
    Dlna {
        control: String,
        /// The renderer's volume service, when it has one.
        rendering: Option<String>,
    },
}

/// What the watcher follows. The Cast device's status arrives on the channel; a renderer has
/// to be asked.
enum WatchTarget {
    Cast(chromecast::Chromecast),
    Dlna { control: String },
}

impl CastSession {
    /// The session, and the channel the player drains.
    pub fn new(app: tauri::AppHandle) -> (Arc<Self>, mpsc::UnboundedReceiver<CastCommand>) {
        let (commands, rx) = mpsc::unbounded_channel();
        let session = Arc::new(CastSession {
            app,
            inner: Mutex::new(Inner { devices: Vec::new(), active: None, synced: false }),
            commands,
            epoch: AtomicU64::new(0),
        });
        (session, rx)
    }

    /// Every device that was found.
    pub fn devices(&self) -> Vec<Device> {
        self.inner.lock().map(|i| i.devices.clone()).unwrap_or_default()
    }

    /// Whether something is playing on a device.
    pub fn is_active(&self) -> bool {
        self.inner.lock().map(|i| i.active.is_some()).unwrap_or(false)
    }

    /// The current connection epoch. Callers use it to stop work belonging to an older device.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// The device that is playing, if any.
    pub fn active_device(&self) -> Option<Device> {
        self.inner.lock().ok().and_then(|i| i.active.as_ref().map(|a| a.device.clone()))
    }

    /// Start casting to a device. A Chromecast is contacted before the old device is stopped;
    /// a renderer reports an unreachable control URL when its first track is loaded.
    pub async fn start(self: &Arc<Self>, device_id: &str) -> Result<(), String> {
        let Some(device) = self.device(device_id) else {
            return Err("that device is no longer on the network".to_owned());
        };
        let (target, watcher) = match device.kind.as_str() {
            "chromecast" => {
                let cc = chromecast::Device {
                    id: device.id.clone(),
                    name: device.name.clone(),
                    model: device.model.clone(),
                    host: device.host.clone(),
                    port: device.port,
                };
                tracing::info!(target: "app_lib::cast", device = %device.name, host = %device.host, port = device.port, "cast: connecting");
                let (handle, sender) = chromecast::Chromecast::connect(&cc).await?;
                (Target::Cast { sender }, WatchTarget::Cast(handle))
            }
            _ => {
                let control = device
                    .control
                    .clone()
                    .ok_or_else(|| "that device cannot be played on".to_owned())?;
                let rendering = device.rendering.clone();
                (
                    Target::Dlna { control: control.clone(), rendering },
                    WatchTarget::Dlna { control },
                )
            }
        };

        // The replacement is ready to take over. Stop the old one now so two speakers do not
        // play the same song half a beat apart.
        self.stop().await;

        {
            let mut inner = self.inner.lock().map_err(|_| "the cast session is locked")?;
            inner.active = Some(Active { device, target });
            inner.synced = false;
        }
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.emit_state();
        spawn_watcher(self.clone(), watcher, epoch);
        Ok(())
    }

    /// Stop what the device is playing, without letting go of it.
    ///
    /// This is what the app does before it hands the device the next track: a load that
    /// arrives while the device is still tearing the old one down is refused, and the
    /// device is the one that has to finish with it first.
    pub async fn stop_media(&self) -> Result<(), String> {
        let target = {
            let inner = self.inner.lock().map_err(|_| "the cast session is locked")?;
            inner.active.as_ref().map(|a| a.target.clone())
        };
        let Some(target) = target else {
            return Err("nothing is casting".to_owned());
        };
        match target {
            Target::Cast { sender } => sender.stop().await,
            Target::Dlna { control, .. } => dlna::stop(&control).await,
        }
    }

    /// Stop casting. The device is told to stop too, so it doesn't keep playing on its own.
    pub async fn stop(self: &Arc<Self>) {
        let active = self.inner.lock().ok().and_then(|mut i| {
            i.synced = false;
            i.active.take()
        });
        if let Some(active) = active {
            match &active.target {
                Target::Cast { sender } => {
                    // The device stops playing before the channel goes: a speaker left running
                    // on a song nobody is listening to is the wrong kind of cast.
                    let _ = sender.stop().await;
                    let _ = sender.disconnect();
                }
                Target::Dlna { control, .. } => {
                    let _ = dlna::stop(control).await;
                }
            }
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.emit_state();
    }

    /// Hand the device a window of tracks. A Cast receiver keeps a queue of its own with it,
    /// which is what makes next, previous, shuffle and repeat work from the phone and from a
    /// voice assistant; a renderer only ever gets the first track.
    pub async fn load_queue(
        &self,
        entries: Vec<QueueEntry>,
        start_index: usize,
        repeat_mode: &str,
        current_time: f64,
    ) -> Result<(), String> {
        let target = {
            let inner = self.inner.lock().map_err(|_| "the cast session is locked")?;
            inner.active.as_ref().map(|a| a.target.clone())
        };
        let Some(target) = target else {
            return Err("nothing is casting".to_owned());
        };
        match target {
            Target::Cast { sender } => {
                sender.load_queue(entries, start_index, repeat_mode, current_time)
            }
            Target::Dlna { control, .. } => {
                let Some(entry) = entries.get(start_index) else {
                    return Err("nothing to play on the renderer".to_owned());
                };
                let media = &entry.media;
                let meta = dlna::metadata(
                    media.title.as_deref().unwrap_or_default(),
                    media.artist.as_deref(),
                    media.album.as_deref(),
                    media.thumbnail.as_deref(),
                    &media.content_type,
                    &media.url,
                    media.duration,
                );
                dlna::play(&control, &media.url, &meta, media.start_at).await
            }
        }
    }

    /// Change the receiver queue's repeat mode without reloading it.
    pub async fn set_repeat(&self, mode: &str) -> Result<(), String> {
        let target = {
            let inner = self.inner.lock().map_err(|_| "the cast session is locked")?;
            inner.active.as_ref().map(|a| a.target.clone())
        };
        match target {
            Some(Target::Cast { sender }) => sender.set_repeat(mode),
            _ => Ok(()),
        }
    }

    /// Shuffle the receiver's own queue order.
    pub async fn shuffle(&self) -> Result<(), String> {
        let target = {
            let inner = self.inner.lock().map_err(|_| "the cast session is locked")?;
            inner.active.as_ref().map(|a| a.target.clone())
        };
        match target {
            Some(Target::Cast { sender }) => sender.shuffle(),
            _ => Ok(()),
        }
    }

    /// Whether the device that is playing holds a queue of its own (a Cast receiver does, a
    /// DLNA renderer does not).
    pub fn uses_queue(&self) -> bool {
        self.inner
            .lock()
            .map(|i| i.active.as_ref().is_some_and(|a| matches!(a.target, Target::Cast { .. })))
            .unwrap_or(false)
    }

    /// Pause the device.
    pub async fn pause(&self) -> Result<(), String> {
        self.toggle(true).await
    }

    /// Resume the device.
    pub async fn resume(&self) -> Result<(), String> {
        self.toggle(false).await
    }

    /// Pause the device (`playing` is what it is doing now) or resume it.
    pub async fn toggle(&self, playing: bool) -> Result<(), String> {
        let target = {
            let inner = self.inner.lock().map_err(|_| "the cast session is locked")?;
            inner.active.as_ref().map(|a| a.target.clone())
        };
        let Some(target) = target else {
            return Err("nothing is casting".to_owned());
        };
        match target {
            Target::Cast { sender } => {
                if playing {
                    sender.pause()
                } else {
                    sender.play()
                }
            }
            Target::Dlna { control, .. } => {
                if playing {
                    dlna::pause(&control).await
                } else {
                    dlna::resume(&control).await
                }
            }
        }
    }

    /// The device's volume, 0.0 to 1.0. It is the speaker's volume, not this desktop's.
    pub async fn set_volume(&self, level: f64) -> Result<(), String> {
        let target = {
            let inner = self.inner.lock().map_err(|_| "the cast session is locked")?;
            inner.active.as_ref().map(|a| a.target.clone())
        };
        let Some(target) = target else {
            return Err("nothing is casting".to_owned());
        };
        match target {
            Target::Cast { sender } => sender.set_volume(level),
            Target::Dlna { rendering, .. } => {
                // The volume lives in the renderer's other service.
                let rendering = rendering.ok_or("that device has no volume")?;
                dlna::set_volume(&rendering, level).await
            }
        }
    }

    /// Move the device to `seconds` in the track.
    pub async fn seek(&self, seconds: f64) -> Result<(), String> {
        let target = {
            let inner = self.inner.lock().map_err(|_| "the cast session is locked")?;
            inner.active.as_ref().map(|a| a.target.clone())
        };
        let Some(target) = target else {
            return Err("nothing is casting".to_owned());
        };
        match target {
            Target::Cast { sender } => sender.seek(seconds),
            Target::Dlna { control, .. } => dlna::seek(&control, seconds).await,
        }
    }

    /// Look for DLNA renderers again, now. The Cast devices arrive on their own over mDNS, so
    /// only the SSDP search has to be asked for.
    pub async fn refresh(&self) -> Vec<Device> {
        if let Ok(found) = dlna::discover().await {
            for device in found {
                self.upsert(Device {
                    id: device.id,
                    name: device.name,
                    kind: "dlna".to_owned(),
                    model: device.model,
                    host: device.host,
                    port: device.port,
                    control: Some(device.control),
                    rendering: device.rendering,
                    fullname: None,
                    seen_at: crate::db::now_secs(),
                });
            }
        }
        self.devices()
    }

    fn device(&self, id: &str) -> Option<Device> {
        self.inner.lock().ok()?.devices.iter().find(|d| d.id == id).cloned()
    }

    /// Remember a device that was found, and tell the UI about it.
    fn upsert(&self, device: Device) {
        {
            let Ok(mut inner) = self.inner.lock() else { return };
            match inner.devices.iter_mut().find(|d| d.id == device.id) {
                Some(existing) => {
                    let fullname = device.fullname.clone().or(existing.fullname.clone());
                    let control = device.control.clone().or(existing.control.clone());
                    *existing = Device { seen_at: device.seen_at, ..device };
                    existing.fullname = fullname;
                    existing.control = control;
                }
                None => inner.devices.push(device),
            }
            inner.devices.sort_by_key(|d| d.name.to_lowercase());
        }
        self.emit_devices();
    }

    /// Forget a device that is gone.
    fn remove(&self, id: &str) {
        {
            let Ok(mut inner) = self.inner.lock() else { return };
            inner.devices.retain(|d| d.id != id);
        }
        self.emit_devices();
    }

    /// The device list, for the UI.
    fn emit_devices(&self) {
        let devices = self.devices();
        let _ = self.app.emit("cast-devices", devices);
    }

    /// Whether something is casting, where, and whether the device has answered yet.
    fn emit_state(&self) {
        let device = self.active_device();
        let synced = self.inner.lock().map(|i| i.synced).unwrap_or(false);
        let _ = self.app.emit(
            "cast-state",
            serde_json::json!({ "active": device.is_some(), "device": device, "synced": synced }),
        );
    }

    /// The device has answered: the app knows what it is playing.
    fn mark_synced(&self) {
        let changed = {
            let Ok(mut inner) = self.inner.lock() else { return };
            let was = inner.synced;
            inner.synced = true;
            !was
        };
        if changed {
            self.emit_state();
        }
    }
}

/// Find the devices and keep the list current.
pub fn spawn_discovery(session: Arc<CastSession>) {
    spawn_mdns(session.clone());
    spawn_ssdp(session);
}

/// The Cast devices, over mDNS.
fn spawn_mdns(session: Arc<CastSession>) {
    let daemon = match mdns_sd::ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "cast: mDNS could not start (no Chromecast will be found)");
            return;
        }
    };
    let receiver = match daemon.browse(CAST_SERVICE) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "cast: mDNS browse failed");
            return;
        }
    };
    // The daemon has to outlive the browse, or it stops answering.
    Box::leak(Box::new(daemon));

    tauri::async_runtime::spawn(async move {
        while let Ok(event) = receiver.recv_async().await {
            match event {
                mdns_sd::ServiceEvent::ServiceResolved(info) => {
                    let Some(id) = info.get_property_val_str("id") else { continue };
                    let name = info
                        .get_property_val_str("fn")
                        .map(str::to_owned)
                        .unwrap_or_else(|| "Cast device".to_owned());
                    let model = info.get_property_val_str("md").map(str::to_owned);
                    let port = info.get_port();
                    // The address arrives with the resolution; a device that has not answered
                    // its address record yet is left out until the next event.
                    let Some(host) = info.get_addresses_v4().iter().next().copied() else {
                        continue;
                    };
                    session.upsert(Device {
                        id: id.to_owned(),
                        name,
                        kind: "chromecast".to_owned(),
                        model,
                        host: host.to_string(),
                        port,
                        control: None,
                        rendering: None,
                        fullname: Some(info.get_fullname().trim_end_matches('.').to_owned()),
                        seen_at: crate::db::now_secs(),
                    });
                }
                mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                    // The removal names the instance, not the device id the list is keyed by.
                    let name = fullname.trim_end_matches('.').to_owned();
                    let id = session
                        .devices()
                        .into_iter()
                        .find(|d| d.kind == "chromecast" && d.fullname.as_deref() == Some(&name))
                        .map(|d| d.id);
                    if let Some(id) = id {
                        session.remove(&id);
                    }
                }
                _ => {}
            }
        }
    });
}

/// The DLNA renderers, over SSDP.
fn spawn_ssdp(session: Arc<CastSession>) {
    tauri::async_runtime::spawn(async move {
        loop {
            match dlna::discover().await {
                Ok(found) => {
                    for device in found {
                        session.upsert(Device {
                            id: device.id,
                            name: device.name,
                            kind: "dlna".to_owned(),
                            model: device.model,
                            host: device.host,
                            port: device.port,
                            control: Some(device.control),
                            rendering: device.rendering,
                            fullname: None,
                            seen_at: crate::db::now_secs(),
                        });
                    }
                    // A renderer that stopped answering is gone. One that merely missed a
                    // search is not: it stays for a few minutes.
                    let now = crate::db::now_secs();
                    let gone: Vec<String> = session
                        .devices()
                        .into_iter()
                        .filter(|d| d.kind == "dlna" && now - d.seen_at > RENDERER_GRACE)
                        .map(|d| d.id)
                        .collect();
                    for id in gone {
                        session.remove(&id);
                    }
                }
                Err(e) => tracing::debug!(error = %e, "cast: the DLNA search failed"),
            }
            tokio::time::sleep(DLNA_SEARCH_INTERVAL).await;
        }
    });
}

/// Watch the device that is playing, and tell the player when the track ends.
///
/// The device is the only one that knows when the song is over: mpv is paused while a cast is
/// running, so nothing here would notice a track that ran out.
fn spawn_watcher(session: Arc<CastSession>, mut target: WatchTarget, epoch: u64) {
    tauri::async_runtime::spawn(async move {
        let mut watch = Watch::default();
        loop {
            // The cast ended (or moved to another device): nothing left to watch.
            if session.epoch.load(Ordering::SeqCst) != epoch {
                return;
            }

            match &mut target {
                WatchTarget::Cast(cast) => {
                    // A Cast device pushes its status when it changes, often several times a
                    // second. Asking as well covers a device that only answers when spoken to,
                    // but the asking is on its own clock: one question per pushed status would
                    // talk to the device far more than it talks back.
                    let mut ask = tokio::time::interval(POLL);
                    ask.tick().await;
                    loop {
                        tokio::select! {
                            event = cast.next_event() => {
                                // The cast ended (or moved to another device) while this was
                                // waiting: the status belongs to a device that is no longer
                                // ours.
                                if session.epoch.load(Ordering::SeqCst) != epoch {
                                    return;
                                }
                                let Some(event) = event else {
                                    let _ = session.commands.send(CastCommand::Stopped(
                                        "the cast connection closed".to_owned(),
                                    ));
                                    return;
                                };
                                match event {
                                    chromecast::Event::Status(s) => {
                                        tracing::debug!(
                                            target: "app_lib::cast",
                                            state = ?s.state,
                                            position = s.position,
                                            duration = ?s.duration,
                                            "cast: the device's status"
                                        );
                                        // The track's end is the app's business: it knows how
                                        // long the song is. The device only says where it
                                        // is — and only about the track that is playing now.
                                        let status = Status {
                                            state: s.state,
                                            position: s.position,
                                            duration: s.duration,
                                            media_session: s.media_session,
                                        };
                                        if !watch.is_current(&status) {
                                            continue;
                                        }
                                        session.mark_synced();
                                        let playing = s.state == chromecast::PlayerState::Playing;
                                        let _ = session.commands.send(CastCommand::Position {
                                            position: s.position,
                                            duration: s.duration,
                                            playing,
                                            state: s.state,
                                            track: s.track.clone(),
                                            volume: s.volume,
                                            video_id: s.video_id.clone(),
                                            content_id: s.content_id.clone(),
                                            repeat_mode: s.repeat_mode.clone(),
                                            finished: s.finished,
                                        });
                                    }
                                    chromecast::Event::Closed(e) => {
                                        let _ = session.commands.send(CastCommand::Stopped(e));
                                        return;
                                    }
                                }
                            }
                            _ = ask.tick() => {
                                let _ = cast.sender().request_status();
                                // The speaker's volume comes from the receiver's status.
                                let _ = cast.sender().request_receiver_status();
                            }
                        }
                    }
                }
                WatchTarget::Dlna { control } => {
                    let control = control.clone();
                    let status = match dlna::position(&control).await {
                        Ok((position, duration)) => {
                            let state = dlna::transport_state(&control)
                                .await
                                .unwrap_or(dlna::TransportState::Other);
                            Some(Status {
                                state: match state {
                                    dlna::TransportState::Playing => {
                                        chromecast::PlayerState::Playing
                                    }
                                    dlna::TransportState::Paused => chromecast::PlayerState::Paused,
                                    dlna::TransportState::Stopped => chromecast::PlayerState::Idle,
                                    dlna::TransportState::Other => chromecast::PlayerState::Other,
                                },
                                position,
                                duration: if duration > 0.0 { Some(duration) } else { None },
                                media_session: 0,
                            })
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "cast: the renderer stopped answering");
                            let _ = session.commands.send(CastCommand::Stopped(e));
                            return;
                        }
                    };
                    if let Some(status) = status {
                        if !watch.is_current(&status) {
                            continue;
                        }
                        let playing = status.state == chromecast::PlayerState::Playing;
                        let _ = session.commands.send(CastCommand::Position {
                            position: status.position,
                            duration: status.duration,
                            playing,
                            state: status.state,
                            track: None,
                            volume: None,
                            video_id: None,
                            content_id: None,
                            repeat_mode: None,
                            finished: false,
                        });
                    }
                    tokio::time::sleep(POLL).await;
                }
            }
        }
    });
}

/// The device's answer, in the shape both kinds of device report.
struct Status {
    state: chromecast::PlayerState,
    position: f64,
    duration: Option<f64>,
    /// The media session the device is playing. A renderer has none: it plays one thing at a
    /// time and says so by going idle.
    media_session: i64,
}

/// What the watcher remembers about the track that is playing.
#[derive(Default)]
struct Watch {
    /// The newest media session the device has reported.
    ///
    /// The receiver makes one per track, and a status about an older one can still be on its
    /// way when the next track starts. Its position belongs to a track that is over, and
    /// taking it would drag the clock back.
    session: i64,
}

impl Watch {
    /// Whether a status is about the track that is playing now.
    fn is_current(&mut self, status: &Status) -> bool {
        // A renderer has no media sessions: it plays one thing at a time.
        if status.media_session == 0 {
            return true;
        }
        if status.media_session < self.session {
            return false; // about a track that is over
        }
        self.session = status.media_session;
        true
    }
}

/// The address of this machine that a device can reach.
///
/// The one on the same network as the device, so a laptop with both Wi-Fi and a VPN does not
/// hand out an address the speaker cannot route to.
pub fn local_address_for(device: IpAddr) -> Option<Ipv4Addr> {
    let interfaces = if_addrs::get_if_addrs().ok()?;
    let mut fallback = None;
    for interface in interfaces {
        if interface.is_loopback() {
            continue;
        }
        let IpAddr::V4(addr) = interface.ip() else { continue };
        if !is_private(addr) {
            continue;
        }
        if fallback.is_none() {
            fallback = Some(addr);
        }
        if let IpAddr::V4(peer) = device {
            if same_network(addr, peer) {
                return Some(addr);
            }
        }
    }
    fallback
}

/// Whether an address is on a private network.
fn is_private(addr: Ipv4Addr) -> bool {
    addr.is_private() || addr.is_link_local()
}

/// Whether two addresses are on the same network, near enough: the first three octets.
fn same_network(a: Ipv4Addr, b: Ipv4Addr) -> bool {
    a.octets()[0..3] == b.octets()[0..3]
}

/// The metadata a device gets for a track: its title, its artist and its cover.
pub fn media_for(
    item: &SongItem,
    url: String,
    content_type: String,
    start_at: Option<f64>,
) -> Media {
    Media {
        url,
        content_type,
        title: Some(item.title.clone()),
        artist: Some(item.artists.clone()),
        album: item.album.clone(),
        thumbnail: item.thumbnail.clone(),
        duration: {
            let ms = crate::state::parse_duration_ms(item.duration.as_deref());
            (ms > 0).then(|| ms as f64 / 1000.0)
        },
        start_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cast::chromecast::PlayerState;

    fn status(state: PlayerState, position: f64, duration: Option<f64>) -> Status {
        Status { state, position, duration, media_session: 1 }
    }

    fn status_with_session(
        state: PlayerState,
        position: f64,
        duration: Option<f64>,
        media_session: i64,
    ) -> Status {
        Status { media_session, ..status(state, position, duration) }
    }

    #[test]
    fn a_status_about_an_older_track_is_dropped() {
        // The receiver makes a media session per track, and a status about the old one can
        // still be on its way when the next track starts. Its position belongs to a track
        // that is over.
        let mut watch = Watch::default();
        assert!(watch.is_current(&status_with_session(PlayerState::Playing, 10.0, Some(200.0), 7)));
        assert!(watch.is_current(&status_with_session(PlayerState::Playing, 2.0, Some(200.0), 8)));
        assert!(!watch.is_current(&status_with_session(PlayerState::Idle, 0.0, None, 7)));
    }

    #[test]
    fn a_renderer_has_no_media_sessions() {
        let mut watch = Watch::default();
        assert!(watch.is_current(&status(PlayerState::Playing, 10.0, Some(200.0))));
    }

    #[test]
    fn private_addresses_are_recognised() {
        assert!(is_private("192.168.1.5".parse().unwrap()));
        assert!(is_private("10.0.0.4".parse().unwrap()));
        assert!(!is_private("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn two_addresses_are_on_the_same_network_when_the_first_three_octets_match() {
        assert!(same_network("192.168.1.5".parse().unwrap(), "192.168.1.90".parse().unwrap()));
        assert!(!same_network("192.168.1.5".parse().unwrap(), "192.168.2.90".parse().unwrap()));
    }
}
