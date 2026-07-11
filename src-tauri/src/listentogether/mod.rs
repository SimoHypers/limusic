//! Listen Together client. Owns the WebSocket connection to a self-hosted `limusic-sync` server
//! (context/19), the room state, and reconnection. It knows nothing about mpv or the orchestrator:
//! playback for guests is driven through an mpsc channel of [`SyncCommand`]s that a bridge task
//! (in `lib.rs`) applies to `AppState`. The host's playback is broadcast by `AppState` calling
//! [`LtSession::broadcast_playback`].
//!
//! No back-reference to `AppState` (avoids an Arc cycle): everything the connection needs is the
//! `AppHandle` (to emit UI events) and the sync channel.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use listen_protocol::{
    ClientMessage, Playback, PlaybackKind, RoomState, ServerMessage, Suggestion, Track, User,
};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;

/// Playback intents the connection hands to the bridge task to apply to the local player.
#[derive(Debug, Clone)]
pub enum SyncCommand {
    /// Full state (after join / reconnect / request-sync): resolve current track, seek to live
    /// position, set play/pause, mirror the queue.
    ApplyState(RoomState),
    ChangeTrack { track: Track, position_ms: i64, playing: bool, queue: Vec<Track> },
    Play { position_ms: i64, server_time_ms: i64 },
    Pause { position_ms: i64 },
    Seek { position_ms: i64 },
    /// We just became host of a freshly-created room — seed the room with our current now-playing.
    HostSeed,
    /// We left / were kicked / became host — stop applying remote playback.
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    None,
    Host,
    Guest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Disconnected,
    Connecting,
    Connected,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Disconnected => "disconnected",
            Status::Connecting => "connecting",
            Status::Connected => "connected",
        }
    }
}

#[derive(Default)]
struct Inner {
    status_connected: bool,
    connecting: bool,
    /// We've asked to create/join and are waiting for the room to materialize (host approval).
    /// Drives the guest's "waiting" UI and blocks duplicate requests.
    requesting: bool,
    role: Role,
    server_url: String,
    room_code: Option<String>,
    my_id: Option<String>,
    session_token: Option<String>,
    users: Vec<User>,
    current_track: Option<Track>,
    queue: Vec<Track>,
    pending_joins: Vec<(String, String)>, // (user_id, username)
    suggestions: Vec<Suggestion>,
    outbound: Option<mpsc::UnboundedSender<ClientMessage>>,
}

impl Default for Role {
    fn default() -> Self {
        Role::None
    }
}

impl Inner {
    fn status(&self) -> Status {
        if self.status_connected {
            Status::Connected
        } else if self.connecting {
            Status::Connecting
        } else {
            Status::Disconnected
        }
    }

    /// Clear all room membership (leave / kicked / fatal). Keeps `server_url`.
    fn reset_room(&mut self) {
        self.role = Role::None;
        self.room_code = None;
        self.my_id = None;
        self.session_token = None;
        self.users.clear();
        self.current_track = None;
        self.queue.clear();
        self.pending_joins.clear();
        self.suggestions.clear();
        self.outbound = None;
        self.status_connected = false;
        self.connecting = false;
        self.requesting = false;
    }

    fn absorb(&mut self, state: &RoomState) {
        self.room_code = Some(state.room_code.clone());
        self.users = state.users.clone();
        self.current_track = state.current_track.clone();
        self.queue = state.queue.clone();
    }
}

pub struct LtSession {
    app: AppHandle,
    inner: Arc<Mutex<Inner>>,
    sync_tx: mpsc::UnboundedSender<SyncCommand>,
    /// Bumped to cancel the running connection loop (leave / new connection).
    gen: AtomicU64,
}

impl LtSession {
    /// Create the session. Returns the receiver the bridge task drains to drive guest playback.
    pub fn new(app: AppHandle, server_url: String) -> (Arc<Self>, mpsc::UnboundedReceiver<SyncCommand>) {
        // rustls needs a process-wide crypto provider before the first `wss://` handshake.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (sync_tx, sync_rx) = mpsc::unbounded_channel();
        let inner = Inner { server_url, ..Inner::default() };
        let s = Arc::new(LtSession {
            app,
            inner: Arc::new(Mutex::new(inner)),
            sync_tx,
            gen: AtomicU64::new(0),
        });
        (s, sync_rx)
    }

    // --- public API (called by commands + AppState) -----------------------------------------

    pub async fn set_server_url(&self, url: String) {
        self.inner.lock().await.server_url = url;
        self.emit_state().await;
    }

    /// True when we're a guest in a room — the caller should block local playback control.
    pub async fn is_guest(&self) -> bool {
        self.inner.lock().await.role == Role::Guest
    }

    pub async fn is_host(&self) -> bool {
        self.inner.lock().await.role == Role::Host
    }

    /// Host: broadcast a playback action to the room. No-op unless we're the host and connected.
    pub async fn broadcast_playback(&self, p: Playback) {
        let inner = self.inner.lock().await;
        if inner.role != Role::Host {
            return;
        }
        if let Some(tx) = &inner.outbound {
            let _ = tx.send(ClientMessage::Playback(p));
        }
    }

    /// Send any client message on the current connection (best-effort).
    async fn send(&self, msg: ClientMessage) {
        if let Some(tx) = &self.inner.lock().await.outbound {
            let _ = tx.send(msg);
        }
    }

    pub async fn create_room(self: &Arc<Self>, username: String) {
        self.start(ClientMessage::CreateRoom { username }).await;
    }

    pub async fn join_room(self: &Arc<Self>, code: String, username: String) {
        self.start(ClientMessage::JoinRoom { room_code: code, username }).await;
    }

    pub async fn approve_join(&self, user_id: String) {
        self.send(ClientMessage::ApproveJoin { user_id }).await;
    }
    pub async fn reject_join(&self, user_id: String) {
        // Drop it from our local pending list immediately (optimistic).
        {
            let mut inner = self.inner.lock().await;
            inner.pending_joins.retain(|(id, _)| id != &user_id);
        }
        self.send(ClientMessage::RejectJoin { user_id }).await;
        self.emit_state().await;
    }
    pub async fn kick(&self, user_id: String) {
        self.send(ClientMessage::KickUser { user_id }).await;
    }
    pub async fn transfer_host(&self, user_id: String) {
        self.send(ClientMessage::TransferHost { user_id }).await;
    }
    pub async fn suggest(&self, track: Track) {
        self.send(ClientMessage::Suggest { track }).await;
    }
    /// Host: approve a suggestion. Returns its track so the caller can add it to the real playback
    /// queue (the host owns the queue; the server just notifies the suggester).
    pub async fn approve_suggestion(&self, id: String) -> Option<Track> {
        let track = {
            let mut inner = self.inner.lock().await;
            let idx = inner.suggestions.iter().position(|s| s.id == id);
            idx.map(|i| inner.suggestions.remove(i).track)
        };
        self.send(ClientMessage::ApproveSuggestion { id }).await;
        self.emit_state().await;
        track
    }
    pub async fn reject_suggestion(&self, id: String) {
        {
            let mut inner = self.inner.lock().await;
            inner.suggestions.retain(|s| s.id != id);
        }
        self.send(ClientMessage::RejectSuggestion { id }).await;
        self.emit_state().await;
    }
    pub async fn request_sync(&self) {
        self.send(ClientMessage::RequestSync).await;
    }

    /// Leave the room and tear down the connection.
    pub async fn leave(&self) {
        self.send(ClientMessage::LeaveRoom).await;
        self.gen.fetch_add(1, Ordering::SeqCst); // cancel the connection loop
        self.inner.lock().await.reset_room();
        let _ = self.sync_tx.send(SyncCommand::Release);
        self.emit_state().await;
    }

    /// A snapshot of the client-side LT state for the UI.
    pub async fn snapshot(&self) -> serde_json::Value {
        let inner = self.inner.lock().await;
        Self::snapshot_of(&inner)
    }

    // --- connection loop --------------------------------------------------------------------

    async fn start(self: &Arc<Self>, initial: ClientMessage) {
        // Cancel any existing connection, reset, then spawn a fresh loop.
        self.gen.fetch_add(1, Ordering::SeqCst);
        {
            let mut inner = self.inner.lock().await;
            inner.reset_room();
            inner.connecting = true;
            inner.requesting = true; // waiting for the room to materialize
        }
        self.emit_state().await;
        let gen = self.gen.load(Ordering::SeqCst);
        let me = self.clone();
        tokio::spawn(async move {
            me.run(gen, initial).await;
        });
    }

    async fn run(self: Arc<Self>, gen: u64, initial: ClientMessage) {
        let mut next_msg = initial;
        let mut attempt: u32 = 0;
        loop {
            if self.gen.load(Ordering::SeqCst) != gen {
                return;
            }
            let url = self.inner.lock().await.server_url.clone();
            if url.is_empty() {
                self.close_locally("Set a server URL first (Listen Together settings).").await;
                return;
            }
            {
                let mut inner = self.inner.lock().await;
                inner.connecting = true;
                inner.status_connected = false;
            }
            self.emit_state().await;

            match tokio_tungstenite::connect_async(&url).await {
                Ok((ws, _)) => {
                    attempt = 0;
                    let (mut sink, mut read) = ws.split();
                    let (otx, mut orx) = mpsc::unbounded_channel::<ClientMessage>();
                    {
                        let mut inner = self.inner.lock().await;
                        inner.outbound = Some(otx.clone());
                        inner.connecting = false;
                        inner.status_connected = true;
                    }
                    let _ = otx.send(next_msg.clone());
                    self.emit_state().await;

                    // Writer: drain outbound → socket.
                    let writer = tokio::spawn(async move {
                        while let Some(m) = orx.recv().await {
                            if sink.send(Message::Text(m.to_json())).await.is_err() {
                                break;
                            }
                        }
                    });
                    // App-level keepalive.
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

                    loop {
                        // Poll the cancel generation even when idle: `leave()` bumps it but the
                        // server doesn't close the socket on LeaveRoom, so we'd otherwise park here.
                        let next = tokio::select! {
                            biased;
                            next = read.next() => next,
                            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                                if self.gen.load(Ordering::SeqCst) != gen {
                                    break;
                                }
                                continue;
                            }
                        };
                        if self.gen.load(Ordering::SeqCst) != gen {
                            break;
                        }
                        let Some(next) = next else { break }; // stream ended
                        match next {
                            Ok(Message::Text(t)) => {
                                match serde_json::from_str::<ServerMessage>(&t) {
                                    Ok(sm) => {
                                        if self.handle(sm).await {
                                            break; // fatal (kicked / rejected) — stop reading
                                        }
                                    }
                                    Err(e) => tracing::debug!(error = %e, "bad server message"),
                                }
                            }
                            Ok(Message::Close(_)) | Err(_) => break,
                            _ => {}
                        }
                    }
                    writer.abort();
                    ping.abort();
                    self.inner.lock().await.outbound = None;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "listen-together connect failed");
                }
            }

            if self.gen.load(Ordering::SeqCst) != gen {
                return;
            }
            // Reconnect only if we were in a room (have a token). Fresh create/join that never
            // landed just fails.
            let token = self.inner.lock().await.session_token.clone();
            let Some(t) = token else {
                self.close_locally("Couldn't reach the Listen Together server.").await;
                return;
            };
            attempt += 1;
            if attempt > 15 {
                self.close_locally("Lost connection to the room.").await;
                return;
            }
            {
                let mut inner = self.inner.lock().await;
                inner.status_connected = false;
                inner.connecting = true;
            }
            self.emit_state().await;
            next_msg = ClientMessage::Reconnect { session_token: t };
            tokio::time::sleep(backoff_delay(attempt)).await;
        }
    }

    /// Handle one server message. Returns `true` if the connection should close (fatal).
    async fn handle(&self, sm: ServerMessage) -> bool {
        match sm {
            ServerMessage::Pong => return false,

            ServerMessage::RoomCreated { room_code: _, user_id, session_token, state } => {
                {
                    let mut inner = self.inner.lock().await;
                    inner.role = Role::Host;
                    inner.requesting = false;
                    inner.my_id = Some(user_id);
                    inner.session_token = Some(session_token);
                    inner.absorb(&state);
                }
                // Seed the room with whatever we're currently playing.
                let _ = self.sync_tx.send(SyncCommand::HostSeed);
            }

            ServerMessage::JoinRequest { user_id, username } => {
                let is_new = {
                    let mut inner = self.inner.lock().await;
                    let new = !inner.pending_joins.iter().any(|(id, _)| id == &user_id);
                    if new {
                        inner.pending_joins.push((user_id, username.clone()));
                    }
                    new
                };
                if is_new {
                    self.emit_notice(&format!("{username} wants to join")).await;
                }
            }

            ServerMessage::JoinApproved { user_id, session_token, state } => {
                {
                    let mut inner = self.inner.lock().await;
                    inner.role = Role::Guest;
                    inner.requesting = false;
                    inner.my_id = Some(user_id);
                    inner.session_token = Some(session_token);
                    inner.absorb(&state);
                }
                let _ = self.sync_tx.send(SyncCommand::ApplyState(state));
            }

            ServerMessage::JoinRejected { reason } => {
                self.emit_notice(&reason).await;
                self.close_locally("").await;
                return true;
            }

            ServerMessage::Reconnected { user_id, is_host, state } => {
                {
                    let mut inner = self.inner.lock().await;
                    inner.role = if is_host { Role::Host } else { Role::Guest };
                    inner.requesting = false;
                    inner.my_id = Some(user_id);
                    inner.absorb(&state);
                }
                if !is_host {
                    let _ = self.sync_tx.send(SyncCommand::ApplyState(state));
                }
            }

            ServerMessage::UserJoined { user } => {
                let mut inner = self.inner.lock().await;
                inner.pending_joins.retain(|(id, _)| id != &user.user_id);
                if !inner.users.iter().any(|u| u.user_id == user.user_id) {
                    inner.users.push(user);
                }
            }
            ServerMessage::UserLeft { user_id } => {
                let mut inner = self.inner.lock().await;
                inner.users.retain(|u| u.user_id != user_id);
            }
            ServerMessage::UserDisconnected { user_id } => {
                let mut inner = self.inner.lock().await;
                if let Some(u) = inner.users.iter_mut().find(|u| u.user_id == user_id) {
                    u.is_connected = false;
                }
            }
            ServerMessage::UserReconnected { user_id } => {
                let mut inner = self.inner.lock().await;
                if let Some(u) = inner.users.iter_mut().find(|u| u.user_id == user_id) {
                    u.is_connected = true;
                }
            }

            ServerMessage::HostChanged { host_id } => {
                let became_host = {
                    let mut inner = self.inner.lock().await;
                    for u in &mut inner.users {
                        u.is_host = u.user_id == host_id;
                    }
                    let me = inner.my_id.clone();
                    let became = me.as_deref() == Some(host_id.as_str());
                    inner.role = if became { Role::Host } else { Role::Guest };
                    became
                };
                if became_host {
                    // We now control playback locally — stop applying remote sync.
                    let _ = self.sync_tx.send(SyncCommand::Release);
                    self.emit_notice("You're the host now.").await;
                }
            }

            ServerMessage::SyncPlayback(p) => {
                // Only guests apply; the host originated it (server excludes us anyway).
                if self.inner.lock().await.role == Role::Guest {
                    self.apply_playback(p).await;
                }
            }

            ServerMessage::SyncState { state } => {
                {
                    let mut inner = self.inner.lock().await;
                    inner.absorb(&state);
                }
                if self.inner.lock().await.role == Role::Guest {
                    let _ = self.sync_tx.send(SyncCommand::ApplyState(state));
                }
            }

            ServerMessage::SuggestionReceived { suggestion } => {
                let mut inner = self.inner.lock().await;
                if !inner.suggestions.iter().any(|s| s.id == suggestion.id) {
                    inner.suggestions.push(suggestion);
                }
            }
            ServerMessage::SuggestionApproved { .. } => {
                self.emit_notice("Your suggestion was added to the queue.").await;
            }
            ServerMessage::SuggestionRejected { reason, .. } => {
                self.emit_notice(&reason).await;
            }

            ServerMessage::Kicked { reason } => {
                self.emit_notice(&reason).await;
                self.close_locally("").await;
                return true;
            }
            ServerMessage::Error { code, message } => {
                self.emit_notice(&message).await;
                // A failed join/reconnect can't recover — close.
                if code == "room_not_found" || code == "room_full" || code == "session_expired" {
                    self.close_locally("").await;
                    return true;
                }
            }
        }
        self.emit_state().await;
        false
    }

    /// Translate a broadcast playback action into a bridge command (guest side).
    async fn apply_playback(&self, p: Playback) {
        // Keep our mirror of current track/queue current so the UI reflects it.
        {
            let mut inner = self.inner.lock().await;
            match p.kind {
                PlaybackKind::ChangeTrack => {
                    inner.current_track = p.track.clone();
                    if let Some(q) = &p.queue {
                        inner.queue = q.clone();
                    }
                }
                PlaybackKind::SyncQueue => {
                    if let Some(q) = &p.queue {
                        inner.queue = q.clone();
                    }
                }
                _ => {}
            }
        }
        let cmd = match p.kind {
            PlaybackKind::Play => {
                Some(SyncCommand::Play { position_ms: p.position_ms, server_time_ms: p.server_time_ms })
            }
            PlaybackKind::Pause => Some(SyncCommand::Pause { position_ms: p.position_ms }),
            PlaybackKind::Seek => Some(SyncCommand::Seek { position_ms: p.position_ms }),
            PlaybackKind::ChangeTrack => p.track.map(|track| SyncCommand::ChangeTrack {
                track,
                position_ms: p.position_ms,
                playing: p.playing,
                queue: p.queue.unwrap_or_default(),
            }),
            PlaybackKind::SyncQueue | PlaybackKind::SetVolume => None,
        };
        if let Some(cmd) = cmd {
            let _ = self.sync_tx.send(cmd);
        }
    }

    /// Tear down room membership locally (no server round-trip). `notice` optional.
    async fn close_locally(&self, notice: &str) {
        if !notice.is_empty() {
            self.emit_notice(notice).await;
        }
        self.gen.fetch_add(1, Ordering::SeqCst);
        self.inner.lock().await.reset_room();
        let _ = self.sync_tx.send(SyncCommand::Release);
        self.emit_state().await;
    }

    // --- UI events --------------------------------------------------------------------------

    fn snapshot_of(inner: &Inner) -> serde_json::Value {
        let role = match inner.role {
            Role::None => "none",
            Role::Host => "host",
            Role::Guest => "guest",
        };
        serde_json::json!({
            "status": inner.status().as_str(),
            "role": role,
            "requesting": inner.requesting,
            "roomCode": inner.room_code,
            "myId": inner.my_id,
            "serverUrl": inner.server_url,
            "users": inner.users,
            "currentTrack": inner.current_track,
            "queue": inner.queue,
            "pendingJoins": inner.pending_joins.iter()
                .map(|(id, name)| serde_json::json!({ "userId": id, "username": name }))
                .collect::<Vec<_>>(),
            "suggestions": inner.suggestions,
        })
    }

    async fn emit_state(&self) {
        let snap = { Self::snapshot_of(&*self.inner.lock().await) };
        let _ = self.app.emit("lt-state", snap);
    }

    async fn emit_notice(&self, msg: &str) {
        let _ = self.app.emit("lt-notice", msg);
    }
}

/// Exponential backoff, capped at 32s (context/19 Part B §1: the client's real ceiling).
fn backoff_delay(attempt: u32) -> Duration {
    let shift = attempt.clamp(1, 6) - 1; // 0..=5 → 1,2,4,8,16,32
    Duration::from_secs(1u64 << shift)
}
