//! Device link: Limusic on a phone as a remote for this desktop, and "Continue here" both ways.
//!
//! A link room lives on the Listen Together server (`crates/sync-server`), keyed by a long secret
//! instead of a room code: holding the secret is membership. The room's host is the device that is
//! playing. Starting playback on a device that isn't the player is a takeover (a `change_track`
//! with `playing`), and the old player pauses when it hears `host_changed`. Remote controls from
//! the phone arrive as `remote` messages and go through the same code paths as the OS media keys.
//!
//! It has its own socket, independent of [`crate::listentogether::LtSession`], and none at all
//! until the user links a device: no `device_link` settings row means no connection. Like the
//! Listen Together session it holds no reference to `AppState`; what it needs done to the player
//! goes out as a [`LinkCommand`] that a bridge task in `lib.rs` applies.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use listen_protocol::{
    ClientMessage, LinkStatus, Playback, PlaybackKind, RemoteAction, RoomState, ServerMessage,
    Track, User,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::db::Db;
use crate::listentogether::{backoff_delay, connect_ws, shareable, DEFAULT_SERVER};

/// What the player has to do for the link. Drained by the bridge task in `lib.rs`.
#[derive(Debug, Clone)]
pub enum LinkCommand {
    /// A control from another device. Only forwarded while we are the player.
    Remote(RemoteAction),
    /// Another device became the player: pause if we are playing.
    HostLost,
    /// We (re)joined the room: if we are playing, say so, or the room keeps what it last heard.
    Republish,
    /// Another device asks this one to play what the player plays (Continue here, from there).
    TakeOver,
    /// The link's state changed: whether this desktop shows another device's playback may have.
    Changed,
}

/// Paused longer than this, the device that played last stops being the one this desktop shows
/// and controls (the phone's rule too).
const STALE_PAUSE_MS: i64 = 10 * 60 * 1000;
/// A phone on mobile data drops for a moment now and then, and so can this socket: for this long
/// the player bar keeps showing the device it controls instead of flipping to ours and back.
const RECONNECT_GRACE: Duration = Duration::from_secs(10);

/// What another linked device plays, for this desktop to show and control (Spotify Connect style).
pub struct RemoteView {
    pub track: Track,
    pub playing: bool,
    /// Live, extrapolated from its last report.
    pub position_ms: i64,
    /// Its queue, once it has sent one.
    pub status: Option<LinkStatus>,
    /// The room's upcoming tracks, until it has.
    pub upcoming: Vec<Track>,
}

/// What [`LinkSession::publish`] did with a message. See [`send_rule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publish {
    Sent,
    /// A resume on a device that isn't the player: the caller sends a `change_track` built from
    /// what it plays instead.
    Takeover,
    Dropped,
}

/// The send rule. Only the player tells the room what is happening; a device that isn't playing
/// may only take over, by starting a track or resuming. This is why a desktop that restores its
/// last queue paused at startup never steals the room from a phone that is playing.
pub fn send_rule(is_player: bool, p: &Playback) -> Publish {
    if is_player {
        return Publish::Sent;
    }
    match p.kind {
        PlaybackKind::ChangeTrack if p.playing => Publish::Sent,
        PlaybackKind::Play => Publish::Takeover,
        _ => Publish::Dropped,
    }
}

/// The settings row. Never logged, never in `UI_SETTINGS`, never in the diagnostics report.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LinkConfig {
    secret: String,
    /// The Listen Together server when the link was made; empty = [`DEFAULT_SERVER`].
    server: String,
    device_id: String,
}

const SETTING: &str = "device_link";

/// How long the server gets to answer `join_link` or `pair_start`. An older server ignores both.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Two missed pongs plus slack, as in Listen Together.
const SILENCE_LIMIT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Inner {
    config: Option<LinkConfig>,
    connecting: bool,
    /// `link_joined` arrived on the current socket.
    joined: bool,
    /// The server never answered: it predates linking. No retry until the user tries again.
    unsupported: bool,
    error: Option<String>,
    /// "Link a phone" was pressed and no code has come back yet.
    want_pair: bool,
    /// This pairing attempt made the link, so cancelling it undoes the link too.
    pair_made_link: bool,
    /// When `pair_start` went out on the joined socket: the server's answer is timed from here.
    pair_sent: Option<std::time::Instant>,
    /// The live pairing code and when it expires (unix ms, this machine's clock).
    pair: Option<(String, i64)>,
    /// The room as the server last described it.
    room: RoomState,
    /// The player's queue, when another device plays.
    status: Option<LinkStatus>,
    /// The last `link_status` we sent as the player, so an unchanged queue isn't sent again.
    sent_status: Option<String>,
    /// The player bar shows another device's playback right now (`AppState` decides).
    showing_remote: bool,
    /// When this socket dropped from a joined room, and when the player went offline (unix ms):
    /// within [`RECONNECT_GRACE`] of either, the room as last heard still counts.
    lost_at: Option<i64>,
    host_lost_at: Option<i64>,
    outbound: Option<mpsc::UnboundedSender<ClientMessage>>,
}

impl Inner {
    fn me(&self) -> &str {
        self.config.as_ref().map(|c| c.device_id.as_str()).unwrap_or("")
    }
    fn is_player(&self) -> bool {
        !self.room.host_id.is_empty() && self.room.host_id == self.me()
    }
    fn send(&self, m: ClientMessage) {
        if let Some(tx) = &self.outbound {
            let _ = tx.send(m);
        }
    }
    fn others(&self) -> usize {
        self.room.users.iter().filter(|u| u.user_id != self.me()).count()
    }
}

pub struct LinkSession {
    app: AppHandle,
    db: Arc<Db>,
    inner: Mutex<Inner>,
    cmd_tx: mpsc::UnboundedSender<LinkCommand>,
    /// Bumped to cancel the running connection loop.
    gen: AtomicU64,
    name: String,
}

impl LinkSession {
    pub fn new(app: AppHandle, db: Arc<Db>) -> (Arc<Self>, mpsc::UnboundedReceiver<LinkCommand>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let config = db.get_setting(SETTING).and_then(|s| serde_json::from_str(&s).ok());
        let name = gethostname::gethostname().to_string_lossy().trim().to_string();
        let s = Arc::new(LinkSession {
            app,
            db,
            inner: Mutex::new(Inner { config, ..Inner::default() }),
            cmd_tx,
            gen: AtomicU64::new(0),
            name: if name.is_empty() { "Limusic desktop".into() } else { name },
        });
        if s.linked() {
            s.connect();
        }
        (s, cmd_rx)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether this desktop is linked at all. Cheap: checked on every track change.
    pub fn linked(&self) -> bool {
        self.lock().config.is_some()
    }

    /// Send a playback change through the [`send_rule`]. Never blocks.
    pub fn publish(&self, mut p: Playback) -> Publish {
        let decision = {
            let mut inner = self.lock();
            if !inner.joined {
                return Publish::Dropped;
            }
            let decision = send_rule(inner.is_player(), &p);
            if decision == Publish::Sent {
                keep_linkable(&mut p);
                inner.send(ClientMessage::Playback(p.clone()));
                // Our own mirror too: the device list's Playing badge reads it, and a takeover is
                // ours from now on rather than once the server's `host_changed` comes back.
                if p.kind == PlaybackKind::ChangeTrack && p.playing {
                    inner.room.host_id = inner.me().to_string();
                }
                apply_playback(&mut inner.room, &p, now_ms());
            }
            decision
        };
        if decision == Publish::Sent && p.kind != PlaybackKind::SetVolume {
            self.emit_state();
        }
        decision
    }

    /// Our queue for the other devices, while we are the player. Local files keep their place in
    /// it with their name only, as the playing track does (`keep_linkable`). Never blocks.
    pub fn publish_status(&self, mut status: LinkStatus) {
        let mut inner = self.lock();
        if !inner.joined || !inner.is_player() {
            return;
        }
        for item in &mut status.items {
            if !shareable(&item.track) {
                item.track.id.clear();
                item.track.thumbnail = None;
            }
        }
        let Ok(value) = serde_json::to_value(&status) else { return };
        let json = value.to_string();
        if inner.sent_status.as_deref() == Some(json.as_str()) {
            return;
        }
        inner.sent_status = Some(json);
        inner.send(ClientMessage::LinkStatus { status: value });
    }

    /// Another device's playback, while it is the player, online, and playing or paused less than
    /// [`STALE_PAUSE_MS`] ago.
    pub fn remote_view(&self) -> Option<RemoteView> {
        let inner = self.lock();
        let now = now_ms();
        let recent =
            |at: Option<i64>| at.is_some_and(|t| now - t < RECONNECT_GRACE.as_millis() as i64);
        if (!inner.joined && !recent(inner.lost_at)) || inner.is_player() {
            return None;
        }
        let host = inner.room.users.iter().find(|u| u.user_id == inner.room.host_id)?;
        if !host.is_connected && !recent(inner.host_lost_at) {
            return None;
        }
        let track = inner.room.current_track.clone()?;
        if !inner.room.is_playing && now - inner.room.last_update_ms > STALE_PAUSE_MS {
            return None;
        }
        Some(RemoteView {
            track,
            playing: inner.room.is_playing,
            position_ms: inner.room.live_position_ms(now),
            status: inner.status.clone(),
            upcoming: inner.room.queue.clone(),
        })
    }

    /// A control for the device that plays, or for `to`.
    pub fn remote(&self, action: RemoteAction, to: Option<String>) {
        if let Ok(action) = serde_json::to_value(&action) {
            self.lock().send(ClientMessage::Remote { action, to });
        }
    }

    /// The room's player, when it is another device.
    pub fn player_id(&self) -> Option<String> {
        let inner = self.lock();
        (!inner.room.host_id.is_empty() && !inner.is_player()).then(|| inner.room.host_id.clone())
    }

    /// `AppState` says whether the player bar shows another device; the device picker reads it.
    pub fn set_showing_remote(&self, on: bool) {
        let changed = {
            let mut inner = self.lock();
            std::mem::replace(&mut inner.showing_remote, on) != on
        };
        if changed {
            let _ = self.app.emit("link-state", self.snapshot());
        }
    }

    /// The room's track, upcoming queue and live position, when another device holds them and
    /// they can be played here.
    pub fn handoff(&self) -> Option<(Track, Vec<Track>, i64)> {
        let inner = self.lock();
        let track = inner.room.current_track.clone().filter(shareable)?;
        if inner.is_player() {
            return None;
        }
        let queue = inner.room.queue.iter().filter(|t| shareable(t)).cloned().collect();
        Some((track, queue, inner.room.live_position_ms(now_ms())))
    }

    /// "Link a phone": make the link if there is none, join the room and ask for a code.
    pub fn start_pairing(self: &Arc<Self>) {
        let start = {
            let mut inner = self.lock();
            if inner.config.is_none() {
                let mut secret = [0u8; 32];
                rand::Rng::fill(&mut rand::thread_rng(), &mut secret);
                let config = LinkConfig {
                    secret: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret),
                    server: self.db.get_setting("lt_server_url").unwrap_or_default(),
                    device_id: new_device_id(),
                };
                self.db.set_setting(SETTING, &serde_json::to_string(&config).unwrap_or_default());
                inner.config = Some(config);
                inner.pair_made_link = true;
            }
            inner.want_pair = true;
            inner.pair = None;
            inner.error = None;
            if inner.joined {
                inner.send(ClientMessage::PairStart);
                inner.pair_sent = Some(std::time::Instant::now());
            }
            // A previous attempt hit an old server and stopped: try again from scratch.
            let start = inner.unsupported || (inner.outbound.is_none() && !inner.connecting);
            inner.unsupported = false;
            start
        };
        if start {
            self.connect();
        }
        self.emit_state();
        // Timed from when `pair_start` reaches a joined socket, not from the press: a connect or a
        // reconnect backoff can take longer than the timeout on a server that supports linking. A
        // server that ignores `join_link` altogether is the run loop's to catch.
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let timed_out = {
                    let inner = me.lock();
                    if !inner.want_pair || inner.pair.is_some() || inner.unsupported {
                        return;
                    }
                    inner.joined && inner.pair_sent.is_some_and(|t| t.elapsed() > REPLY_TIMEOUT)
                };
                if timed_out {
                    me.give_up_unsupported();
                    return;
                }
            }
        });
    }

    /// Cancel on the pairing screen. With nothing linked yet that undoes the link itself.
    pub fn cancel_pairing(&self) {
        let nobody = {
            let mut inner = self.lock();
            inner.want_pair = false;
            inner.pair = None;
            // Only a link this attempt made: an existing one can show no devices while the room
            // is still rejoining, and cancelling then must not delete it.
            std::mem::take(&mut inner.pair_made_link) && inner.others() == 0
        };
        if nobody {
            self.forget();
        } else {
            self.emit_state();
        }
    }

    /// Unlink another device. Unlinking the last one leaves the room and deletes the link.
    pub fn unlink(&self, device_id: &str) {
        let nobody = {
            let mut inner = self.lock();
            inner.send(ClientMessage::KickUser { user_id: device_id.to_string() });
            inner.room.users.retain(|u| u.user_id != device_id);
            inner.others() == 0
        };
        if nobody {
            self.forget();
        } else {
            self.emit_state();
        }
    }

    /// Leave the room and delete the link.
    pub fn forget(&self) {
        {
            let mut inner = self.lock();
            inner.send(ClientMessage::LeaveRoom);
            *inner = Inner::default();
        }
        self.gen.fetch_add(1, Ordering::SeqCst);
        self.db.delete_setting(SETTING);
        self.emit_state();
    }

    /// The UI's view (the `link-state` event).
    pub fn snapshot(&self) -> serde_json::Value {
        let inner = self.lock();
        let me = inner.me();
        let status = if inner.config.is_none() {
            "off"
        } else if inner.unsupported {
            "unsupported"
        } else if inner.joined {
            "connected"
        } else {
            "connecting"
        };
        let devices: Vec<_> = inner
            .room
            .users
            .iter()
            .filter(|u| u.user_id != me)
            .map(|u| {
                serde_json::json!({
                    "id": u.user_id,
                    "name": u.username,
                    "online": u.is_connected,
                    "playing": u.user_id == inner.room.host_id && inner.room.is_playing,
                })
            })
            .collect();
        let player = inner.room.users.iter().find(|u| u.user_id == inner.room.host_id);
        serde_json::json!({
            "status": status,
            "error": inner.error,
            "server": inner.config.as_ref().map(|c| c.server.as_str()).unwrap_or(""),
            "selfName": self.name,
            "pairing": inner.pair.as_ref().map(|(code, until)| serde_json::json!({
                "code": code,
                "expiresAt": until,
            })),
            "devices": devices,
            // Another device that is or was last playing, with what it plays. Drives the strip.
            "player": player.filter(|u| u.user_id != me).map(|u| serde_json::json!({
                "id": u.user_id,
                "name": u.username,
                "online": u.is_connected,
            })),
            "track": inner.room.current_track,
            "isPlaying": inner.room.is_playing,
            "remote": inner.showing_remote,
        })
    }

    /// Look again once a grace period is over: nothing else changes when nothing comes back.
    fn recheck_after_grace(&self) {
        let tx = self.cmd_tx.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(RECONNECT_GRACE + Duration::from_millis(100)).await;
            let _ = tx.send(LinkCommand::Changed);
        });
    }

    fn emit_state(&self) {
        let _ = self.app.emit("link-state", self.snapshot());
        let _ = self.cmd_tx.send(LinkCommand::Changed);
    }

    fn give_up_unsupported(&self) {
        {
            let mut inner = self.lock();
            inner.unsupported = true;
            inner.want_pair = false;
            inner.connecting = false;
            inner.joined = false;
            inner.outbound = None;
        }
        self.gen.fetch_add(1, Ordering::SeqCst);
        self.emit_state();
    }

    // --- connection ----------------------------------------------------------------------------

    fn connect(self: &Arc<Self>) {
        let gen = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.lock().connecting = true;
        let me = self.clone();
        tauri::async_runtime::spawn(async move { me.run(gen).await });
    }

    async fn run(self: Arc<Self>, gen: u64) {
        let mut attempt: u32 = 0;
        loop {
            let live = || self.gen.load(Ordering::SeqCst) == gen;
            let Some(config) = self.lock().config.clone() else { return };
            let url = if config.server.is_empty() { DEFAULT_SERVER } else { &config.server };
            {
                let mut inner = self.lock();
                inner.connecting = true;
                inner.joined = false;
            }
            self.emit_state();

            match connect_ws(url).await {
                Ok(ws) => {
                    if !live() {
                        return;
                    }
                    let (mut sink, mut read) = ws.split();
                    let (otx, mut orx) = mpsc::unbounded_channel::<ClientMessage>();
                    let _ = otx.send(ClientMessage::JoinLink {
                        secret: config.secret.clone(),
                        device_id: config.device_id.clone(),
                        name: self.name.clone(),
                    });
                    self.lock().outbound = Some(otx.clone());
                    let writer = tokio::spawn(async move {
                        while let Some(m) = orx.recv().await {
                            if sink.send(Message::Text(m.to_json())).await.is_err() {
                                break;
                            }
                        }
                    });
                    let ping = {
                        let tx = otx.clone();
                        tokio::spawn(async move {
                            loop {
                                tokio::time::sleep(Duration::from_secs(25)).await;
                                if tx.send(ClientMessage::Ping).is_err() {
                                    break;
                                }
                            }
                        })
                    };
                    let opened = std::time::Instant::now();
                    let mut last_heard = opened;
                    let mut unsupported = false;
                    loop {
                        let next = tokio::select! {
                            biased;
                            next = read.next() => next,
                            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                                if !live() || last_heard.elapsed() > SILENCE_LIMIT {
                                    break;
                                }
                                if !self.lock().joined && opened.elapsed() > REPLY_TIMEOUT {
                                    unsupported = true;
                                    break;
                                }
                                continue;
                            }
                        };
                        if !live() {
                            break;
                        }
                        let Some(next) = next else { break };
                        last_heard = std::time::Instant::now();
                        match next {
                            Ok(Message::Text(t)) => match serde_json::from_str::<ServerMessage>(&t)
                            {
                                Ok(sm) => {
                                    if self.handle(sm) {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    tracing::debug!(error = %e, "device link: bad server message")
                                }
                            },
                            Ok(Message::Close(_)) | Err(_) => break,
                            _ => {}
                        }
                    }
                    ping.abort();
                    if live() {
                        let mut inner = self.lock();
                        if inner.joined {
                            inner.lost_at = Some(now_ms());
                            self.recheck_after_grace();
                        }
                        inner.outbound = None;
                        inner.joined = false;
                    }
                    // Let a queued LeaveRoom go out (see LtSession::run).
                    drop(otx);
                    let mut writer = writer;
                    if tokio::time::timeout(Duration::from_secs(2), &mut writer).await.is_err() {
                        writer.abort();
                    }
                    if unsupported && live() {
                        tracing::warn!("device link: the server doesn't answer join_link");
                        self.give_up_unsupported();
                        return;
                    }
                    if live() {
                        attempt = 0;
                    }
                }
                Err(e) => tracing::warn!(error = %e, "device link: connect failed"),
            }
            if !live() {
                return;
            }
            // The link is the user's devices, so it keeps trying for as long as it exists.
            attempt = attempt.saturating_add(1);
            self.emit_state();
            tokio::time::sleep(backoff_delay(attempt)).await;
        }
    }

    /// One server message. Returns true when the connection should stop for good.
    fn handle(&self, sm: ServerMessage) -> bool {
        let mut stop = false;
        let mut alone = false;
        {
            let mut inner = self.lock();
            match sm {
                ServerMessage::LinkJoined { mut state, status } => {
                    state.queue.retain(shareable);
                    // Another device took over while our socket was down: it stays the player.
                    let other_player = !state.host_id.is_empty() && state.host_id != inner.me();
                    inner.room = state;
                    inner.lost_at = None;
                    inner.host_lost_at = None;
                    inner.status = status.and_then(|s| serde_json::from_value(s).ok());
                    // A restarted server has none of ours: the republish below sends it again.
                    inner.sent_status = None;
                    inner.joined = true;
                    inner.connecting = false;
                    if inner.want_pair {
                        inner.send(ClientMessage::PairStart);
                        inner.pair_sent = Some(std::time::Instant::now());
                    }
                    let _ = self.cmd_tx.send(if other_player {
                        LinkCommand::HostLost
                    } else {
                        LinkCommand::Republish
                    });
                }
                ServerMessage::PairCode { code, expires_in_s } => {
                    inner.want_pair = false;
                    inner.pair = Some((code, now_ms() + i64::from(expires_in_s) * 1000));
                }
                ServerMessage::UserJoined { user } => {
                    // The phone took the code: the pairing screen gives way to the device list.
                    inner.pair = None;
                    inner.want_pair = false;
                    inner.pair_made_link = false;
                    inner.room.users.retain(|u| u.user_id != user.user_id);
                    inner.room.users.push(user);
                }
                ServerMessage::UserLeft { user_id } => {
                    inner.room.users.retain(|u| u.user_id != user_id);
                    if inner.room.host_id == user_id {
                        inner.room.host_id.clear();
                        inner.room.is_playing = false;
                    }
                    // The last other device unlinked itself: same as unlinking it from here.
                    alone = inner.others() == 0 && inner.pair.is_none() && !inner.want_pair;
                }
                ServerMessage::UserDisconnected { user_id } => {
                    if user_id == inner.room.host_id {
                        inner.host_lost_at = Some(now_ms());
                        self.recheck_after_grace();
                    }
                    set_online(&mut inner.room.users, &user_id, false)
                }
                ServerMessage::UserReconnected { user_id } => {
                    if user_id == inner.room.host_id {
                        inner.host_lost_at = None;
                    }
                    set_online(&mut inner.room.users, &user_id, true)
                }
                ServerMessage::HostChanged { host_id } => {
                    for u in &mut inner.room.users {
                        u.is_host = u.user_id == host_id;
                    }
                    inner.room.host_id = host_id;
                    // The new player's queue follows; ours goes out again if we play again.
                    inner.status = None;
                    inner.sent_status = None;
                    if !inner.is_player() {
                        let _ = self.cmd_tx.send(LinkCommand::HostLost);
                    }
                }
                ServerMessage::SyncPlayback(mut p) => {
                    if let Some(q) = &mut p.queue {
                        q.retain(shareable);
                    }
                    apply_playback(&mut inner.room, &p, now_ms());
                }
                ServerMessage::SyncState { mut state } => {
                    state.queue.retain(shareable);
                    inner.room = state;
                }
                ServerMessage::LinkStatus { status } => {
                    inner.status = serde_json::from_value(status).ok();
                }
                ServerMessage::Remote { action, .. } => {
                    match serde_json::from_value::<RemoteAction>(action) {
                        // Only a device that isn't playing can take over; everything else is for
                        // the player.
                        Ok(RemoteAction::TakeOver) if !inner.is_player() => {
                            let _ = self.cmd_tx.send(LinkCommand::TakeOver);
                        }
                        Ok(RemoteAction::TakeOver) => {}
                        Ok(action) if inner.is_player() => {
                            let _ = self.cmd_tx.send(LinkCommand::Remote(action));
                        }
                        Ok(_) => {}
                        // A newer device's action: nothing to do with it here.
                        Err(e) => tracing::debug!(error = %e, "device link: unknown remote action"),
                    }
                }
                ServerMessage::Kicked { .. } => {
                    // Unlinked from another device.
                    *inner = Inner::default();
                    self.db.delete_setting(SETTING);
                    self.gen.fetch_add(1, Ordering::SeqCst);
                    stop = true;
                }
                ServerMessage::Error { code, message } => {
                    tracing::warn!(%code, "device link: server error");
                    if matches!(code.as_str(), "bad_secret" | "room_full" | "server_full") {
                        inner.error = Some(message);
                        inner.joined = false;
                        self.gen.fetch_add(1, Ordering::SeqCst);
                        stop = true;
                    }
                }
                ServerMessage::Pong => return false,
                // Listen Together only, or a reply to a claim this desktop never makes.
                ServerMessage::RoomCreated { .. }
                | ServerMessage::JoinRequest { .. }
                | ServerMessage::JoinApproved { .. }
                | ServerMessage::JoinRejected { .. }
                | ServerMessage::Reconnected { .. }
                | ServerMessage::SuggestionReceived { .. }
                | ServerMessage::SuggestionApproved { .. }
                | ServerMessage::SuggestionRejected { .. }
                | ServerMessage::Paired { .. } => return false,
            }
        }
        if alone {
            self.forget();
            return true;
        }
        self.emit_state();
        stop
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn set_online(users: &mut [User], id: &str, online: bool) {
    if let Some(u) = users.iter_mut().find(|u| u.user_id == id) {
        u.is_connected = online;
    }
}

/// Fold one playback change into the room as the server does (`crates/sync-server`), so the strip
/// and Continue here see what the player last said.
fn apply_playback(room: &mut RoomState, p: &Playback, now_ms: i64) {
    match p.kind {
        PlaybackKind::Play => {
            room.is_playing = true;
            room.position_ms = p.position_ms;
            room.last_update_ms = now_ms;
        }
        PlaybackKind::Pause => {
            room.is_playing = false;
            room.position_ms = p.position_ms;
            room.last_update_ms = now_ms;
        }
        PlaybackKind::Seek => {
            room.position_ms = p.position_ms;
            room.last_update_ms = now_ms;
        }
        PlaybackKind::ChangeTrack => {
            room.current_track = p.track.clone();
            room.position_ms = p.position_ms;
            room.is_playing = p.playing;
            room.last_update_ms = now_ms;
            if let Some(q) = &p.queue {
                room.queue = q.clone();
            }
        }
        PlaybackKind::SyncQueue => {
            if let Some(q) = &p.queue {
                room.queue = q.clone();
            }
        }
        PlaybackKind::SetVolume => room.volume = p.volume,
    }
}

/// Our own devices may see that a local file is playing, but never its path: the track keeps its
/// title and artist and loses its id and artwork (a local path too). The phone shows it as "a
/// local file" with the controls only. Local tracks leave the queue, nobody else can play them.
fn keep_linkable(p: &mut Playback) {
    if let Some(t) = p.track.as_mut().filter(|t| !shareable(t)) {
        t.id.clear();
        t.thumbnail = None;
    }
    if let Some(q) = &mut p.queue {
        q.retain(shareable);
    }
}

/// A random version-4 UUID, made once per link.
fn new_device_id() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str) -> Track {
        Track {
            id: id.into(),
            title: "t".into(),
            artist: "a".into(),
            thumbnail: Some("/home/me/cover.jpg".into()),
            duration_ms: 0,
            queued_by: None,
        }
    }

    #[test]
    fn the_send_rule() {
        let change = |playing| {
            let mut p = Playback::new(PlaybackKind::ChangeTrack);
            p.playing = playing;
            p
        };
        let play = Playback::new(PlaybackKind::Play);
        let pause = Playback::new(PlaybackKind::Pause);
        let seek = Playback::new(PlaybackKind::Seek);
        let queue = Playback::new(PlaybackKind::SyncQueue);
        let volume = Playback::new(PlaybackKind::SetVolume);

        for p in [&change(true), &change(false), &play, &pause, &seek, &queue, &volume] {
            assert_eq!(send_rule(true, p), Publish::Sent, "{:?}", p.kind);
        }
        assert_eq!(send_rule(false, &change(true)), Publish::Sent);
        assert_eq!(send_rule(false, &play), Publish::Takeover);
        for p in [&change(false), &pause, &seek, &queue, &volume] {
            assert_eq!(send_rule(false, p), Publish::Dropped, "{:?}", p.kind);
        }
    }

    #[test]
    fn local_files_cross_without_their_path() {
        let mut p = Playback::new(PlaybackKind::ChangeTrack);
        p.track = Some(track("LOCAL:/home/me/a.mp3"));
        p.queue = Some(vec![track("dQw4w9WgXcQ"), track("LOCAL:/home/me/b.mp3")]);
        keep_linkable(&mut p);
        let t = p.track.unwrap();
        assert_eq!((t.id.as_str(), t.title.as_str(), t.thumbnail), ("", "t", None));
        assert_eq!(p.queue.unwrap().len(), 1);

        let mut p = Playback::new(PlaybackKind::ChangeTrack);
        p.track = Some(track("dQw4w9WgXcQ"));
        keep_linkable(&mut p);
        assert_eq!(p.track.unwrap().thumbnail.as_deref(), Some("/home/me/cover.jpg"));
    }

    #[test]
    fn the_mirror_follows_the_player() {
        let mut room = RoomState::default();
        let mut p = Playback::at(PlaybackKind::ChangeTrack, 1000);
        p.track = Some(track("dQw4w9WgXcQ"));
        p.queue = Some(vec![track("a-B_c123456")]);
        p.playing = true;
        apply_playback(&mut room, &p, 10_000);
        assert!(room.is_playing);
        assert_eq!(room.live_position_ms(12_000), 3000);
        assert_eq!(room.queue.len(), 1);

        apply_playback(&mut room, &Playback::at(PlaybackKind::Pause, 2500), 11_500);
        assert!(!room.is_playing);
        assert_eq!(room.live_position_ms(20_000), 2500);

        let mut v = Playback::new(PlaybackKind::SetVolume);
        v.volume = 0.4;
        apply_playback(&mut room, &v, 0);
        assert_eq!(room.volume, 0.4);
    }

    #[test]
    fn device_ids_are_uuids() {
        let id = new_device_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert_ne!(id, new_device_id());
    }
}
