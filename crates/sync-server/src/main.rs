//! Limusic "Listen Together" sync server. A self-hosted WebSocket hub that relays small playback
//! control messages between a host and guests — audio never touches it; every client streams
//! direct from YouTube (context/19). Host-authoritative, server-extrapolated position.
//!
//! Plain `ws://` on `0.0.0.0:PORT` (default 8080). TLS is terminated by whatever you front it with
//! (Tailscale Funnel / Cloudflare Tunnel), so there's none here.
//!
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use listen_protocol::{
    ClientMessage, PlaybackKind, RoomState, ServerMessage, Suggestion, Track, User,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use thiserror::Error;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock, Semaphore};
use tokio_tungstenite::tungstenite::Message;

/// How long a dropped participant's slot (and session token) survives for reconnection.
const RECONNECT_GRACE: Duration = Duration::from_secs(120);
const MAX_USERS_PER_ROOM: usize = 50;
/// Room code alphabet — no `I`/`O` to avoid confusion (context/19 §2.2).
const CODE_ALPHABET: &[u8] = b"1234567890QWERTYUPASDFGHJKLZXCVBNM";

type Tx = mpsc::UnboundedSender<ServerMessage>;

static RNG: LazyLock<std::sync::Mutex<StdRng>> =
    LazyLock::new(|| std::sync::Mutex::new(StdRng::from_entropy()));

#[derive(Clone)]
struct Peer {
    username: String,
    #[allow(dead_code)]
    tx: Tx,
    connected: bool,
    session_token: String,
    disconnected_at: Option<Instant>,
}

#[derive(Debug, Error)]
enum ServerError {
    #[error("The host kicked or denied you from the server.")]
    Removed,

    #[error("Session is invalid or expired.")]
    InvalidSession,

    #[error("Cannot send message to server: ${0}")]
    UnableToSendMessage(String),

    #[error("Server error: ${0}")]
    GenericError(&'static str),
}

/// A joiner awaiting host approval (not yet a room member).
#[derive(Clone)]
struct Pending {
    username: String,
    #[allow(dead_code)]
    tx: Tx,
}

#[derive(Clone)]
struct Room {
    host_id: String,
    peers: HashMap<String, Peer>,
    pending: HashMap<String, Pending>,
    suggestions: HashMap<String, Suggestion>,
    current_track: Option<Track>,
    is_playing: bool,
    position_ms: i64,
    last_update_ms: i64,
    volume: f64,
    queue: Vec<Track>,
}

impl Room {
    fn wire_users(&self) -> Vec<User> {
        self.peers
            .iter()
            .map(|(id, p)| User {
                user_id: id.clone(),
                username: p.username.clone(),
                is_host: *id == self.host_id,
                is_connected: p.connected,
            })
            .collect()
    }

    fn wire_state(&self, code: &str) -> RoomState {
        RoomState {
            room_code: code.to_string(),
            host_id: self.host_id.clone(),
            users: self.wire_users(),
            current_track: self.current_track.clone(),
            is_playing: self.is_playing,
            position_ms: self.position_ms,
            last_update_ms: self.last_update_ms,
            volume: self.volume,
            queue: self.queue.clone(),
        }
    }

    /// Send to every connected peer except `except` (dropped receivers are ignored — the cleanup
    /// path removes them).
    fn broadcast(&self, msg: ServerMessage, except: Option<&str>) {
        for (id, p) in &self.peers {
            if p.connected && Some(id.as_str()) != except {
                let _ = p.tx.send(msg.clone());
            }
        }
    }

    fn send_to(&self, uid: &str, msg: ServerMessage) {
        if let Some(p) = self.peers.get(uid) {
            let _ = p.tx.send(msg);
        }
    }

    fn is_host(&self, uid: &str) -> bool {
        self.host_id == uid
    }

    /// Pick any connected non-host peer to promote (for host handoff). None if nobody's around.
    fn any_connected_other(&self, besides: &str) -> Option<String> {
        self.peers
            .iter()
            .find(|(id, p)| p.connected && id.as_str() != besides)
            .map(|(id, _)| id.clone())
    }

    /// Is there a host who can actually answer a join request right now?
    fn host_is_reachable(&self) -> bool {
        self.peers.get(&self.host_id).map(|p| p.connected).unwrap_or(false)
    }
}

#[derive(Default)]
struct Server {
    rooms: RwLock<HashMap<String, Room>>,
    session_index: RwLock<HashMap<String, (String, String)>>,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn gen_code() -> String {
    let mut rng = RNG.lock().unwrap();
    (0..8).map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char).collect()
}

fn gen_user_id() -> String {
    let mut rng = RNG.lock().unwrap();

    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);

    format!("user_{}_{}", nanos, rng.gen_range(0..10000))
}

fn gen_token() -> String {
    let mut rng = RNG.lock().unwrap();

    let bytes: [u8; 16] = rng.gen();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn err(code: &str, message: &str) -> ServerMessage {
    ServerMessage::Error { code: code.into(), message: message.into() }
}

impl Server {
    /// Handle one decoded message. `uid`/`room_code` track this connection's identity and are
    /// updated in place as it creates/joins/reconnects.
    async fn dispatch(
        &self,
        cm: ClientMessage,
        tx: &Tx,
        uid: &mut Option<String>,
        room_code: &mut Option<String>,
    ) -> Result<(), ServerError> {
        match cm {
            ClientMessage::Ping => {
                match tx.send(ServerMessage::Pong) {
                    Ok(value) => value,
                    Err(_) => {
                        return Err(ServerError::UnableToSendMessage(String::from(
                            "Error recived during Pong message.",
                        )))
                    }
                }

                Ok(())
            }

            ClientMessage::CreateRoom { username } => {
                let mut rooms = self.rooms.write().await;
                let code = loop {
                    let c = gen_code();
                    if !rooms.contains_key(&c) {
                        break c;
                    }
                };
                let user_id = gen_user_id();
                let token = gen_token();

                let mut peers = HashMap::new();
                peers.insert(
                    user_id.clone(),
                    Peer {
                        username: sanitize(&username),
                        tx: tx.clone(),
                        connected: true,
                        session_token: token.clone(),
                        disconnected_at: None,
                    },
                );
                let room = Room {
                    host_id: user_id.clone(),
                    peers,
                    pending: HashMap::new(),
                    suggestions: HashMap::new(),
                    current_track: None,
                    is_playing: false,
                    position_ms: 0,
                    last_update_ms: 0,
                    volume: 1.0,
                    queue: Vec::new(),
                };
                let state = room.wire_state(&code);
                self.session_index
                    .write()
                    .await
                    .insert(token.clone(), (code.clone(), user_id.clone()));

                rooms.insert(code.clone(), room);
                *uid = Some(user_id.clone());
                *room_code = Some(code.clone());
                let _ = tx.send(ServerMessage::RoomCreated {
                    room_code: code,
                    user_id,
                    session_token: token,
                    state,
                });

                Ok(())
            }

            ClientMessage::JoinRoom { room_code: code, username } => {
                let code = code.trim().to_uppercase();
                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&code) else {
                    return tx.send(err("room_not_found", "No room with that code.")).map_err(
                        |_| ServerError::GenericError("Unable to find room with that code."),
                    );
                };

                // No reachable host means nobody can ever approve this request — say so instead of
                // parking the joiner on "waiting for the host" forever.
                if !room.host_is_reachable() {
                    return tx
                        .send(err("host_unavailable", "The host isn't in that room right now."))
                        .map_err(|_| {
                            ServerError::GenericError("The host isn't in the chat room currently.")
                        });
                }
                if room.peers.len() + room.pending.len() >= MAX_USERS_PER_ROOM {
                    return tx
                        .send(err("room_full", "That room is full."))
                        .map_err(|_| ServerError::GenericError("That room is full."));
                }

                let user_id = gen_user_id();
                room.pending.insert(
                    user_id.clone(),
                    Pending { username: sanitize(&username), tx: tx.clone() },
                );
                let host_id = room.host_id.clone();
                let uname = room.pending[&user_id].username.clone();
                room.send_to(
                    &host_id,
                    ServerMessage::JoinRequest { user_id: user_id.clone(), username: uname },
                );
                *uid = Some(user_id);
                *room_code = Some(code);

                Ok(())
            }

            ClientMessage::ApproveJoin { user_id: joiner } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else {
                    return Ok(());
                };
                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&code) else { return Ok(()) };
                if !room.is_host(&me) {
                    return tx
                        .send(err("not_host", "Only the host can approve joins."))
                        .map_err(|_| ServerError::GenericError("Only the host can allow joins"));
                }
                let Some(p) = room.pending.remove(&joiner) else { return Ok(()) };
                let token = gen_token();

                self.session_index
                    .write()
                    .await
                    .insert(token.clone(), (code.clone(), joiner.clone()));

                room.peers.insert(
                    joiner.clone(),
                    Peer {
                        username: p.username.clone(),
                        tx: p.tx.clone(),
                        connected: true,
                        session_token: token.clone(),
                        disconnected_at: None,
                    },
                );
                // Normalize to the live position so the joiner seeks to where the host *is now*,
                // not where the host last reported (else they'd restart mid-song). context/19 §6.2.
                let now = now_ms();
                let mut state = room.wire_state(&code);
                state.position_ms = state.live_position_ms(now);
                state.last_update_ms = now;
                let user = User {
                    user_id: joiner.clone(),
                    username: p.username,
                    is_host: false,
                    is_connected: true,
                };
                // The joiner gets the full state (their client resolves current_track + seeks to the
                // live position); everyone else just sees a new participant.
                p.tx.send(ServerMessage::JoinApproved {
                    user_id: joiner.clone(),
                    session_token: token,
                    state,
                })
                .map_err(|_| ServerError::GenericError("Unable to approve join"))?;

                room.broadcast(ServerMessage::UserJoined { user }, Some(&joiner));
                Ok(())
            }

            ClientMessage::RejectJoin { user_id: joiner } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else {
                    return Ok(());
                };

                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&code) else { return Ok(()) };
                if !room.is_host(&me) {
                    return Ok(());
                }
                if let Some(p) = room.pending.remove(&joiner) {
                    return p
                        .tx
                        .send(ServerMessage::JoinRejected {
                            reason: "The host declined your request.".into(),
                        })
                        .map_err(|_| ServerError::JoinRejected);
                }
                Ok(())
            }

            ClientMessage::Playback(mut p) => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else {
                    return Ok(());
                };
                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&code) else { return Ok(()) };
                if !room.is_host(&me) {
                    return tx.send(err("not_host", "Only the host controls playback.")).map_err(
                        |_| ServerError::GenericError("Only the host controls playback."),
                    );
                }
                let now = now_ms();
                match p.kind {
                    PlaybackKind::Play => {
                        room.is_playing = true;
                        room.position_ms = p.position_ms;
                        room.last_update_ms = now;
                        p.server_time_ms = now; // clients offset their start by this
                    }
                    PlaybackKind::Pause => {
                        room.is_playing = false;
                        room.position_ms = p.position_ms;
                        room.last_update_ms = now;
                    }
                    PlaybackKind::Seek => {
                        room.position_ms = p.position_ms;
                        room.last_update_ms = now;
                    }
                    PlaybackKind::ChangeTrack => {
                        room.current_track = p.track.clone();
                        room.position_ms = p.position_ms;
                        room.is_playing = p.playing;
                        room.last_update_ms = now;
                        if let Some(q) = &p.queue {
                            room.queue = q.clone();
                        }
                    }
                    PlaybackKind::SyncQueue => {
                        if let Some(q) = &p.queue {
                            room.queue = q.clone();
                        }
                    }
                    PlaybackKind::SetVolume => {
                        room.volume = p.volume;
                    }
                }
                // Host already applied it locally; broadcast to guests only (no echo).
                room.broadcast(ServerMessage::SyncPlayback(p), Some(&me));
                Ok(())
            }

            ClientMessage::RequestSync => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else {
                    return Ok(());
                };
                let rooms = self.rooms.read().await;
                let Some(room) = rooms.get(&code) else { return Ok(()) };
                if !room.peers.contains_key(&me) {
                    return Ok(()); // a joiner still awaiting approval doesn't get the room's state
                }
                // Normalize to the live position so the requester seeks to the right spot.
                let now = now_ms();
                let mut state = room.wire_state(&code);
                state.position_ms = state.live_position_ms(now);
                state.last_update_ms = now;
                let _ = tx.send(ServerMessage::SyncState { state });

                Ok(())
            }

            ClientMessage::Suggest { track } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else {
                    return Ok(());
                };
                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&code) else { return Ok(()) };
                if room.is_host(&me) {
                    return Ok(()); // host adds tracks directly
                }
                let Some(peer) = room.peers.get(&me) else { return Ok(()) };
                let suggestion = Suggestion {
                    id: gen_token(),
                    from_user_id: me.clone(),
                    from_username: peer.username.clone(),
                    track,
                };
                room.suggestions.insert(suggestion.id.clone(), suggestion.clone());
                let host_id = room.host_id.clone();
                room.send_to(&host_id, ServerMessage::SuggestionReceived { suggestion });

                Ok(())
            }

            ClientMessage::ApproveSuggestion { id } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else {
                    return Ok(());
                };
                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&code) else { return Ok(()) };
                if !room.is_host(&me) {
                    return Ok(());
                }
                // The host client does the actual queueing (it owns the playback queue) and will
                // broadcast a SyncQueue; here we just drop the pending suggestion and notify the
                // suggester.
                if let Some(s) = room.suggestions.remove(&id) {
                    room.send_to(&s.from_user_id, ServerMessage::SuggestionApproved { id });
                }

                Ok(())
            }

            ClientMessage::RejectSuggestion { id } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else {
                    return Ok(());
                };
                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&code) else { return Ok(()) };
                if !room.is_host(&me) {
                    return Ok(());
                }
                if let Some(s) = room.suggestions.remove(&id) {
                    room.send_to(
                        &s.from_user_id,
                        ServerMessage::SuggestionRejected {
                            id,
                            reason: "The host declined your suggestion.".into(),
                        },
                    );
                }

                Ok(())
            }

            ClientMessage::KickUser { user_id: target } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else {
                    return Ok(());
                };
                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&code) else { return Ok(()) };
                if !room.is_host(&me) || target == me {
                    return Ok(());
                }
                if let Some(p) = room.peers.remove(&target) {
                    room.broadcast(ServerMessage::UserLeft { user_id: target }, None);
                    return p
                        .tx
                        .send(ServerMessage::Kicked {
                            reason: "The host removed you from the room.".into(),
                        })
                        .map_err(|_| ServerError::Removed);
                }

                Ok(())
            }

            ClientMessage::TransferHost { user_id: target } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else {
                    return Ok(());
                };
                let mut rooms = self.rooms.write().await;
                let Some(room) = rooms.get_mut(&code) else { return Ok(()) };
                if !room.is_host(&me) || !room.peers.contains_key(&target) {
                    return Ok(());
                }
                room.host_id = target.clone();
                room.broadcast(ServerMessage::HostChanged { host_id: target }, None);

                Ok(())
            }

            ClientMessage::Reconnect { session_token } => {
                let (code, user_id) = {
                    let mut index = self.session_index.write().await;
                    match index.get(&session_token) {
                        Some(v) => v.clone(),
                        None => {
                            return tx
                                .send(err("session_expired", "Your session has expired."))
                                .map_err(|_| ServerError::InvalidSession);
                        }
                    }
                };

                let mut rooms = self.rooms.write().await;
                let room = rooms.get_mut(&code).unwrap();
                {
                    let peer = room.peers.get_mut(&user_id).unwrap();
                    peer.tx = tx.clone();

                    peer.connected = true;
                    peer.disconnected_at = None;
                }

                let is_host = room.is_host(&user_id);
                let now = now_ms();
                let mut state = room.wire_state(&code);
                state.position_ms = state.live_position_ms(now);
                state.last_update_ms = now;
                tx.send(ServerMessage::Reconnected { user_id: user_id.clone(), is_host, state })
                    .map_err(|info| {
                        ServerError::UnableToSendMessage(format!(
                            "unable to send a message: {:?}",
                            info.0
                        ))
                    })?;

                room.broadcast(
                    ServerMessage::UserReconnected { user_id: user_id.clone() },
                    Some(&user_id),
                );
                *uid = Some(user_id);
                *room_code = Some(code);

                Ok(())
            }

            ClientMessage::LeaveRoom => {
                if let (Some(me), Some(code)) = (uid.take(), room_code.take()) {
                    self.remove_member_if_still_expired(&code, &me).await;
                }
                Ok(())
            }
        }
    }

    /// Conditionally remove a member only if they're still truly expired.
    /// Prevents race where a peer reconnects between cleanup's discovery phase and removal phase.
    async fn remove_member_if_still_expired(&self, code: &str, user_id: &str) {
        let mut rooms = self.rooms.write().await;
        let Some(room) = rooms.get_mut(code) else { return };

        let should_remove = if let Some(peer) = room.peers.get(user_id) {
            !peer.connected
                && peer
                    .disconnected_at
                    .is_some_and(|time| Instant::now().duration_since(time) > RECONNECT_GRACE)
        } else {
            false
        };

        if !should_remove {
            return;
        }

        if let Some(peer) = room.peers.get(user_id) {
            self.session_index.write().await.remove(&peer.session_token);
        }

        room.pending.remove(user_id);
        if room.peers.remove(user_id).is_some() {
            room.broadcast(ServerMessage::UserLeft { user_id: user_id.to_string() }, None);
        }
        
        if room.host_id == user_id {
            if let Some(next) = room.any_connected_other(user_id) {
                room.host_id = next.clone();
                room.broadcast(ServerMessage::HostChanged { host_id: next }, None);
            }
        }
        
        if room.peers.is_empty() {
            for (_, p) in room.pending.drain() {
                let _ = p.tx.send(ServerMessage::JoinRejected {
                    reason: "The host closed the session.".into(),
                });
            }
            rooms.remove(code);
            tracing::info!(room = %code, "room closed (last member left)");
        }
    }

    /// A socket dropped without leaving. Keep the slot for reconnection, but hand off host now so
    /// nobody is stuck (fixes Metrolist's up-to-15-min dead zone, context/19 §4.7).
    async fn handle_disconnect(&self, uid: Option<String>, room_code: Option<String>) {
        let (Some(me), Some(code)) = (uid, room_code) else { return };
        let mut rooms = self.rooms.write().await;
        let Some(room) = rooms.get_mut(&code) else { return };
        // A still-pending joiner just disappears.
        if room.pending.remove(&me).is_some() {
            return;
        }
        let Some(peer) = room.peers.get_mut(&me) else { return };
        peer.connected = false;
        peer.disconnected_at = Some(Instant::now());
        room.broadcast(ServerMessage::UserDisconnected { user_id: me.clone() }, Some(&me));
        if room.host_id == me {
            if let Some(next) = room.any_connected_other(&me) {
                room.host_id = next.clone();
                room.broadcast(ServerMessage::HostChanged { host_id: next }, None);
            }
        }
    }

    /// Sweep expired reconnection slots and empty rooms.
    async fn cleanup(server: Arc<Server>) {
        let expired: Vec<(String, String)> = {
            let rooms = &server.rooms.read().await;
            let instant = Instant::now();

            rooms
                .iter()
                .flat_map(|(code, room)| {
                    room.peers
                        .iter()
                        .filter(|(_, p)| {
                            !p.connected
                                && p.disconnected_at
                                    .is_some_and(|time| instant - time > RECONNECT_GRACE)
                        })
                        .map(move |(id, _)| (code.clone(), id.clone()))
                })
                .collect()
        };

        if expired.is_empty() {
            return;
        }

        let limit = Arc::new(Semaphore::new(2));
        let mut futures = Vec::with_capacity(expired.len());

        for (code, id) in expired {
            let server = Arc::clone(&server);
            let sem = Arc::clone(&limit);
            futures.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                server.remove_member_if_still_expired(&code, &id).await;
            }));
        }

        for f in futures {
            let _ = f.await;
        }
    }
}

/// Trim + cap a user-supplied string (username), stripping control chars.
fn sanitize(s: &str) -> String {
    let cleaned: String = s.trim().chars().filter(|c| !c.is_control()).take(50).collect();
    if cleaned.is_empty() {
        "Guest".to_string()
    } else {
        cleaned
    }
}

async fn handle_conn(stream: TcpStream, server: Arc<Server>) {
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::debug!(error = %e, "ws handshake failed");
            return;
        }
    };
    let (mut sink, mut read) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();

    // Writer task: drain outbound messages to the socket.
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(Message::Text(msg.to_json())).await.is_err() {
                break;
            }
        }
    });

    let mut uid: Option<String> = None;
    let mut room_code: Option<String> = None;

    while let Some(next) = read.next().await {
        match next {
            Ok(Message::Text(t)) => match serde_json::from_str::<ClientMessage>(&t) {
                Ok(cm) => {
                    if let Err(e) = server.dispatch(cm, &tx, &mut uid, &mut room_code).await {
                        tracing::warn!(error = %e, "dispatch failed for client");
                    }
                }
                Err(e) => tracing::debug!(error = %e, "bad client message"),
            },
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {} // ping/pong/binary ignored (tungstenite auto-pongs)
        }
    }

    server.handle_disconnect(uid, room_code).await;
    writer.abort();
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sync_server=info".into()),
        )
        .init();

    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await.expect("bind");
    tracing::info!(%addr, "limusic-sync listening (plain ws — front with Tailscale/Cloudflare for wss)");

    let server = Arc::new(Server::default());

    // Background cleanup sweep.
    {
        let server = server.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Server::cleanup(server.clone()).await;
            }
        });
    }

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let server = server.clone();
                tokio::spawn(handle_conn(stream, server));
            }
            Err(e) => tracing::warn!(error = %e, "accept failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use listen_protocol::Playback;

    fn dummy_tx() -> (Tx, mpsc::UnboundedReceiver<ServerMessage>) {
        mpsc::unbounded_channel()
    }

    #[tokio::test]
    async fn host_handoff_on_disconnect() {
        let server = Server::default();
        let (htx, _hrx) = dummy_tx();
        let (gtx, _grx) = dummy_tx();

        // Host creates a room.
        let (mut huid, mut hcode) = (None, None);
        server
            .dispatch(
                ClientMessage::CreateRoom { username: "host".into() },
                &htx,
                &mut huid,
                &mut hcode,
            )
            .await
            .unwrap();
        let code = hcode.clone().unwrap();
        let host_id = huid.clone().unwrap();

        // Guest joins + host approves.
        let (mut guid, mut gcode) = (None, None);
        server
            .dispatch(
                ClientMessage::JoinRoom { room_code: code.clone(), username: "guest".into() },
                &gtx,
                &mut guid,
                &mut gcode,
            )
            .await
            .unwrap();
        let guest_id = guid.clone().unwrap();
        server
            .dispatch(
                ClientMessage::ApproveJoin { user_id: guest_id.clone() },
                &htx,
                &mut huid,
                &mut hcode,
            )
            .await
            .unwrap();

        // Host drops → host role must move to the connected guest (no dead zone).
        server.handle_disconnect(Some(host_id.clone()), Some(code.clone())).await;
        let rooms = server.rooms.read().await;
        assert_eq!(rooms[&code].host_id, guest_id, "host should hand off to the connected guest");
    }

    /// Full happy path over a real TCP/WebSocket socket: create → join → approve → host broadcast
    /// reaches the guest. Exercises the accept loop, framing, and JSON serde end to end.
    #[tokio::test]
    async fn end_to_end_over_socket() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::net::TcpStream;
        use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

        type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;
        async fn recv(ws: &mut Ws) -> ServerMessage {
            loop {
                if let Message::Text(t) = ws.next().await.unwrap().unwrap() {
                    return serde_json::from_str(&t).unwrap();
                }
            }
        }
        async fn send(ws: &mut Ws, m: ClientMessage) {
            ws.send(Message::Text(m.to_json())).await.unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = Arc::new(Server::default());
        {
            let server = server.clone();
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(handle_conn(stream, server.clone()));
                }
            });
        }
        let url = format!("ws://{addr}");

        let (mut host, _) = connect_async(&url).await.unwrap();
        send(&mut host, ClientMessage::CreateRoom { username: "host".into() }).await;
        let code = match recv(&mut host).await {
            ServerMessage::RoomCreated { room_code, .. } => room_code,
            m => panic!("expected RoomCreated, got {m:?}"),
        };

        let (mut guest, _) = connect_async(&url).await.unwrap();
        send(&mut guest, ClientMessage::JoinRoom { room_code: code, username: "guest".into() })
            .await;
        let guest_id = match recv(&mut host).await {
            ServerMessage::JoinRequest { user_id, .. } => user_id,
            m => panic!("expected JoinRequest, got {m:?}"),
        };
        send(&mut host, ClientMessage::ApproveJoin { user_id: guest_id }).await;
        assert!(matches!(recv(&mut guest).await, ServerMessage::JoinApproved { .. }));

        // Host broadcasts a track change → the guest must receive it as SyncPlayback.
        let mut p = Playback::new(PlaybackKind::ChangeTrack);
        p.track = Some(Track {
            id: "vid".into(),
            title: "t".into(),
            artist: "a".into(),
            thumbnail: None,
            duration_ms: 0,
            queued_by: None,
        });
        p.playing = true;
        send(&mut host, ClientMessage::Playback(p)).await;
        match recv(&mut guest).await {
            ServerMessage::SyncPlayback(pb) => assert_eq!(pb.kind, PlaybackKind::ChangeTrack),
            m => panic!("expected SyncPlayback, got {m:?}"),
        }
    }

    #[tokio::test]
    async fn guest_playback_is_rejected() {
        let server = Server::default();
        let (htx, _hrx) = dummy_tx();
        let (gtx, mut grx) = dummy_tx();
        let (mut huid, mut hcode) = (None, None);
        server
            .dispatch(
                ClientMessage::CreateRoom { username: "host".into() },
                &htx,
                &mut huid,
                &mut hcode,
            )
            .await
            .unwrap();
        let code = hcode.clone().unwrap();
        let (mut guid, mut gcode) = (None, None);
        server
            .dispatch(
                ClientMessage::JoinRoom { room_code: code.clone(), username: "g".into() },
                &gtx,
                &mut guid,
                &mut gcode,
            )
            .await
            .unwrap();
        server
            .dispatch(
                ClientMessage::ApproveJoin { user_id: guid.clone().unwrap() },
                &htx,
                &mut huid,
                &mut hcode,
            )
            .await
            .unwrap();
        // drain the JoinApproved
        while grx.try_recv().is_ok() {}
        // Guest tries to control playback → must get a not_host error, not a broadcast.
        server
            .dispatch(
                ClientMessage::Playback(Playback::at(PlaybackKind::Pause, 0)),
                &gtx,
                &mut guid,
                &mut gcode,
            )
            .await
            .unwrap();
        let msg = grx.try_recv().expect("guest gets a reply");
        assert!(matches!(msg, ServerMessage::Error { code, .. } if code == "not_host"));
    }

    /// The host makes a room, copies the code and leaves: the room must be gone, and anyone
    /// joining with that code must be told so rather than parked on "waiting for the host".
    #[tokio::test]
    async fn room_closes_when_the_last_member_leaves() {
        let server = Server::default();
        let (htx, _hrx) = dummy_tx();
        let (mut huid, mut hcode) = (None, None);
        server
            .dispatch(
                ClientMessage::CreateRoom { username: "host".into() },
                &htx,
                &mut huid,
                &mut hcode,
            )
            .await
            .unwrap();
        let code = hcode.clone().unwrap();
        server.dispatch(ClientMessage::LeaveRoom, &htx, &mut huid, &mut hcode).await.unwrap();
        assert!(!server.rooms.read().await.contains_key(&code), "room outlived its last member");

        let (jtx, mut jrx) = dummy_tx();
        let (mut juid, mut jcode) = (None, None);
        server
            .dispatch(
                ClientMessage::JoinRoom { room_code: code, username: "joiner".into() },
                &jtx,
                &mut juid,
                &mut jcode,
            )
            .await
            .unwrap();
        let msg = jrx.try_recv().expect("joiner must get an answer, not silence");
        assert!(matches!(msg, ServerMessage::Error { code, .. } if code == "room_not_found"));
    }

    /// Same symptom, other route: the host's socket dropped and their slot is still in its
    /// reconnect grace window, so the room exists but nobody can approve a join.
    #[tokio::test]
    async fn join_is_refused_while_the_host_is_offline() {
        let server = Server::default();
        let (htx, _hrx) = dummy_tx();
        let (mut huid, mut hcode) = (None, None);
        server
            .dispatch(
                ClientMessage::CreateRoom { username: "host".into() },
                &htx,
                &mut huid,
                &mut hcode,
            )
            .await
            .unwrap();
        let code = hcode.clone().unwrap();
        server.handle_disconnect(huid.clone(), hcode.clone()).await;

        let (jtx, mut jrx) = dummy_tx();
        let (mut juid, mut jcode) = (None, None);
        server
            .dispatch(
                ClientMessage::JoinRoom { room_code: code, username: "joiner".into() },
                &jtx,
                &mut juid,
                &mut jcode,
            )
            .await
            .unwrap();
        let msg = jrx.try_recv().expect("joiner must get an answer, not silence");
        assert!(matches!(msg, ServerMessage::Error { code, .. } if code == "host_unavailable"));
    }

    /// A joiner still knocking when the host closes the room gets turned away, not stranded.
    #[tokio::test]
    async fn pending_joiner_is_told_when_the_host_closes_the_room() {
        let server = Server::default();
        let (htx, _hrx) = dummy_tx();
        let (mut huid, mut hcode) = (None, None);
        server
            .dispatch(
                ClientMessage::CreateRoom { username: "host".into() },
                &htx,
                &mut huid,
                &mut hcode,
            )
            .await
            .unwrap();
        let code = hcode.clone().unwrap();

        let (jtx, mut jrx) = dummy_tx();
        let (mut juid, mut jcode) = (None, None);
        server
            .dispatch(
                ClientMessage::JoinRoom { room_code: code.clone(), username: "joiner".into() },
                &jtx,
                &mut juid,
                &mut jcode,
            )
            .await
            .unwrap();

        server.dispatch(ClientMessage::LeaveRoom, &htx, &mut huid, &mut hcode).await.unwrap();
        let msg = jrx.try_recv().expect("pending joiner must be told the room is gone");
        assert!(matches!(msg, ServerMessage::JoinRejected { .. }));
        assert!(!server.rooms.read().await.contains_key(&code));
    }
}
