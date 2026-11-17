use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub use rust_cast::channels::media::{IdleReason, PlayerState};

use rust_cast::{
    channels::media::{
        Image, LoadOptions, Media, Metadata, MusicTrackMediaMetadata,
        StatusEntry, StreamType,
    },
    channels::receiver::CastDeviceApp,
    CastDevice, ChannelMessage,
};

/// Request parameters for loading media onto the Chromecast.
#[derive(Clone, Debug)]
pub struct LoadRequest {
    pub url: String,
    pub content_type: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub image_url: Option<String>,
    pub start_secs: f64,
    pub autoplay: bool,
}

/// Events emitted by the Chromecast session.
#[derive(Clone, Debug, PartialEq)]
pub enum CastEvent {
    Status {
        state: PlayerState,
        position_secs: f64,
        duration_secs: Option<f64>,
        idle_reason: Option<IdleReason>,
        volume: f64,
    },
    Disconnected(String),
}

#[derive(Debug)]
enum Command {
    Load(LoadRequest),
    Play,
    Pause,
    Seek(f64),
    Stop,
    SetVolume(f32),
    SetMuted(bool),
    Quit,
}

/// Manages an active Chromecast session and its background control thread.
#[derive(Clone)]
pub struct CastSession {
    cmd_tx: std::sync::mpsc::Sender<Command>,
    event_rx: Arc<Mutex<Option<UnboundedReceiver<CastEvent>>>>,
    join_handle: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
}

impl CastSession {
    /// Connects to a Chromecast device at the given IP and port.
    pub fn connect(ip: IpAddr, port: u16) -> anyhow::Result<Self> {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let handle = std::thread::spawn(move || {
            if let Err(e) = run_worker(ip, port, cmd_rx, event_tx.clone()) {
                let _ = event_tx.send(CastEvent::Disconnected(e.to_string()));
            }
        });

        Ok(Self {
            cmd_tx,
            event_rx: Arc::new(Mutex::new(Some(event_rx))),
            join_handle: Arc::new(Mutex::new(Some(handle))),
        })
    }

    /// Shuts down the session synchronously, waiting for the worker thread to exit.
    pub fn shutdown(&self, timeout: std::time::Duration) {
        let _ = self.cmd_tx.send(Command::Stop);
        let _ = self.cmd_tx.send(Command::Quit);

        let handle = self.join_handle.lock().unwrap().take();
        if let Some(h) = handle {
            let start = Instant::now();
            loop {
                if h.is_finished() {
                    break;
                }
                if start.elapsed() >= timeout {
                    tracing::warn!("cast: shutdown timed out");
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    /// Returns the event receiver for streaming player status and disconnection events.
    pub fn events(&self) -> UnboundedReceiver<CastEvent> {
        self.event_rx
            .lock()
            .unwrap()
            .take()
            .expect("events receiver already taken")
    }

    pub fn load(&self, req: LoadRequest) -> anyhow::Result<()> {
        self.cmd_tx.send(Command::Load(req))?;
        Ok(())
    }

    pub fn play(&self) -> anyhow::Result<()> {
        self.cmd_tx.send(Command::Play)?;
        Ok(())
    }

    pub fn pause(&self) -> anyhow::Result<()> {
        self.cmd_tx.send(Command::Pause)?;
        Ok(())
    }

    pub fn seek(&self, secs: f64) -> anyhow::Result<()> {
        self.cmd_tx.send(Command::Seek(secs))?;
        Ok(())
    }

    pub fn stop(&self) -> anyhow::Result<()> {
        self.cmd_tx.send(Command::Stop)?;
        Ok(())
    }

    pub fn set_volume(&self, level: f32) -> anyhow::Result<()> {
        self.cmd_tx.send(Command::SetVolume(level))?;
        Ok(())
    }

    pub fn set_muted(&self, muted: bool) -> anyhow::Result<()> {
        self.cmd_tx.send(Command::SetMuted(muted))?;
        Ok(())
    }

    pub fn quit(&self) -> anyhow::Result<()> {
        self.cmd_tx.send(Command::Quit)?;
        Ok(())
    }
}

fn run_worker(
    ip: IpAddr,
    port: u16,
    cmd_rx: std::sync::mpsc::Receiver<Command>,
    event_tx: UnboundedSender<CastEvent>,
) -> anyhow::Result<()> {
    let host = ip.to_string();
    // Connect without host verification because Chromecasts use self-signed certificates.
    let device = CastDevice::connect_without_host_verification(&host, port)?;
    device.connection.connect("receiver-0")?;

    // Attach to running Default Media Receiver if active, otherwise launch it.
    let receiver_status = device.receiver.get_status()?;
    let app = if let Some(app) = receiver_status
        .applications
        .into_iter()
        .find(|a| a.app_id == "CC1AD845")
    {
        app
    } else {
        device.receiver.launch_app(&CastDeviceApp::DefaultMediaReceiver)?
    };

    device.connection.connect(&app.transport_id)?;

    let session_id = app.session_id;
    let transport_id = app.transport_id;
    let mut media_session_id: Option<i32> = None;
    let mut current_volume = receiver_status.volume.level.unwrap_or(1.0) as f64;

    let refresh_media_session = || -> Option<i32> {
        match device.media.get_status(&transport_id, None) {
            Ok(status) => {
                if let Some(entry) = status.entries.iter().max_by_key(|e| e.media_session_id).or_else(|| status.entries.first()) {
                    Some(entry.media_session_id)
                } else {
                    tracing::warn!("cast: no media status entries found");
                    None
                }
            }
            Err(_) => {
                tracing::warn!("cast: failed to get media status");
                None
            }
        }
    };

    let mut last_status_poll = Instant::now();
    let mut last_heartbeat = Instant::now();

    loop {
        // Take a command with a 300 ms timeout to allow periodic polling and heartbeats.
        match cmd_rx.recv_timeout(Duration::from_millis(300)) {
            Ok(cmd) => match cmd {
                Command::Load(req) => {
                    let images = req
                        .image_url
                        .map(|u| vec![Image::new(u)])
                        .unwrap_or_default();
                    let metadata = Metadata::MusicTrack(MusicTrackMediaMetadata {
                        album_name: None,
                        title: req.title,
                        album_artist: None,
                        artist: req.artist,
                        composer: None,
                        track_number: None,
                        disc_number: None,
                        images,
                        release_date: None,
                    });
                    let media = Media {
                        content_id: req.url,
                        stream_type: StreamType::Buffered,
                        content_type: req.content_type,
                        metadata: Some(metadata),
                        duration: None,
                    };
                    let options = LoadOptions {
                        current_time: req.start_secs,
                        autoplay: req.autoplay,
                    };
                    let status = device
                        .media
                        .load_with_opts(&transport_id, &session_id, &media, options)?;
                    if let Some(entry) = status.entries.first() {
                        media_session_id = Some(entry.media_session_id);
                        emit_status(&event_tx, entry, current_volume);
                    }
                }
                Command::Play => {
                    if let Some(msid) = refresh_media_session() {
                        media_session_id = Some(msid);
                        let entry = device.media.play(&transport_id, msid)?;
                        emit_status(&event_tx, &entry, current_volume);
                    }
                }
                Command::Pause => {
                    if let Some(msid) = refresh_media_session() {
                        media_session_id = Some(msid);
                        let entry = device.media.pause(&transport_id, msid)?;
                        emit_status(&event_tx, &entry, current_volume);
                    }
                }
                Command::Seek(secs) => {
                    if let Some(msid) = refresh_media_session() {
                        media_session_id = Some(msid);
                        let entry = device
                            .media
                            .seek(&transport_id, msid, Some(secs as f32), None)?;
                        emit_status(&event_tx, &entry, current_volume);
                    }
                }
                Command::Stop => {
                    if let Some(msid) = refresh_media_session() {
                        let _ = device.media.stop(&transport_id, msid);
                    }
                    let _ = device.receiver.stop_app(&session_id);
                    break;
                }
                Command::SetVolume(level) => {
                    let vol = device.receiver.set_volume(level)?;
                    if let Some(l) = vol.level {
                        current_volume = l as f64;
                    }
                }
                Command::SetMuted(muted) => {
                    let vol = device.receiver.set_volume(muted)?;
                    if let Some(l) = vol.level {
                        current_volume = l as f64;
                    }
                }
                Command::Quit => {
                    break;
                }
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }

        // Send heartbeat ping every 5 seconds to keep connection alive.
        if last_heartbeat.elapsed() >= Duration::from_secs(5) {
            device.heartbeat.ping()?;
            last_heartbeat = Instant::now();
        }

        // Poll media status every ~1 second.
        if last_status_poll.elapsed() >= Duration::from_secs(1) {
            match device.media.get_status(&transport_id, media_session_id) {
                Ok(status) => {
                    if let Some(entry) = status.entries.first() {
                        media_session_id = Some(entry.media_session_id);
                        emit_status(&event_tx, entry, current_volume);
                    }
                }
                Err(_) => {
                    // Try receiving incoming message (e.g. heartbeat ping) if get_status fails or times out.
                    if let Ok(msg) = device.receive() {
                        if let ChannelMessage::Heartbeat(rust_cast::channels::heartbeat::HeartbeatResponse::Ping) = msg {
                            let _ = device.heartbeat.pong();
                        }
                    }
                }
            }
            last_status_poll = Instant::now();
        }
    }

    Ok(())
}

fn emit_status(
    event_tx: &UnboundedSender<CastEvent>,
    entry: &StatusEntry,
    volume: f64,
) {
    let position_secs = entry.current_time.unwrap_or(0.0) as f64;
    let duration_secs = entry.media.as_ref().and_then(|m| m.duration.map(|d| d as f64));
    let _ = event_tx.send(CastEvent::Status {
        state: entry.player_state,
        position_secs,
        duration_secs,
        idle_reason: entry.idle_reason,
        volume,
    });
}
