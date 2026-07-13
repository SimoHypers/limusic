//! Limusic "Listen Together" sync server. A self-hosted WebSocket hub that relays small playback
//! control messages between a host and guests — audio never touches it; every client streams
//! direct from YouTube (context/19). Host-authoritative, server-extrapolated position.
//!
//! Plain `ws://` on `0.0.0.0:PORT` (default 8080). TLS is terminated by whatever you front it with
//! (Tailscale Funnel / Cloudflare Tunnel), so there's none here.
//!
//! ponytail: one global `Mutex<HashMap<room, Room>>` and unbounded per-client channels — right for
//! a personal server (tens of users). If it ever needs to scale, shard by room / add backpressure.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use listen_protocol::{
    ClientMessage, PlaybackKind, RoomState, ServerMessage, Suggestion, Track, User,
};
use rand::Rng;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;

/// How long a dropped participant's slot (and session token) survives for reconnection.
const RECONNECT_GRACE: Duration = Duration::from_secs(120);
/// How long a fully-empty room lingers before deletion.
const EMPTY_ROOM_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_USERS_PER_ROOM: usize = 50;
/// Room code alphabet — no `I`/`O` to avoid confusion (context/19 §2.2).
const CODE_ALPHABET: &[u8] = b"1234567890QWERTYUPASDFGHJKLZXCVBNM";

type Tx = mpsc::UnboundedSender<ServerMessage>;

struct Peer {
    username: String,
    tx: Tx,
    connected: bool,
    session_token: String,
    disconnected_at: Option<Instant>,
}

/// A joiner awaiting host approval (not yet a room member).
struct Pending {
    username: String,
    tx: Tx,
}

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
    /// When the room last had zero peers — drives cleanup.
    empty_since: Option<Instant>,
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
    fn broadcast(&self, msg: &ServerMessage, except: Option<&str>) {
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
}

#[derive(Default)]
struct Server {
    rooms: Mutex<HashMap<String, Room>>,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn gen_code() -> String {
    let mut rng = rand::thread_rng();
    (0..8).map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char).collect()
}

fn gen_user_id() -> String {
    let nanos =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("user_{}_{}", nanos, rand::thread_rng().gen_range(0..10000))
}

fn gen_token() -> String {
    let bytes: [u8; 16] = rand::thread_rng().gen();
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
    ) {
        match cm {
            ClientMessage::Ping => {
                let _ = tx.send(ServerMessage::Pong);
            }

            ClientMessage::CreateRoom { username } => {
                let mut rooms = self.rooms.lock().await;
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
                    empty_since: None,
                };
                let state = room.wire_state(&code);
                rooms.insert(code.clone(), room);
                *uid = Some(user_id.clone());
                *room_code = Some(code.clone());
                let _ = tx.send(ServerMessage::RoomCreated {
                    room_code: code,
                    user_id,
                    session_token: token,
                    state,
                });
            }

            ClientMessage::JoinRoom { room_code: code, username } => {
                let code = code.trim().to_uppercase();
                let mut rooms = self.rooms.lock().await;
                let Some(room) = rooms.get_mut(&code) else {
                    let _ = tx.send(err("room_not_found", "No room with that code."));
                    return;
                };
                if room.peers.len() + room.pending.len() >= MAX_USERS_PER_ROOM {
                    let _ = tx.send(err("room_full", "That room is full."));
                    return;
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
            }

            ClientMessage::ApproveJoin { user_id: joiner } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else { return };
                let mut rooms = self.rooms.lock().await;
                let Some(room) = rooms.get_mut(&code) else { return };
                if !room.is_host(&me) {
                    let _ = tx.send(err("not_host", "Only the host can approve joins."));
                    return;
                }
                let Some(p) = room.pending.remove(&joiner) else { return };
                let token = gen_token();
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
                let _ = p.tx.send(ServerMessage::JoinApproved {
                    user_id: joiner.clone(),
                    session_token: token,
                    state,
                });
                room.broadcast(&ServerMessage::UserJoined { user }, Some(&joiner));
            }

            ClientMessage::RejectJoin { user_id: joiner } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else { return };
                let mut rooms = self.rooms.lock().await;
                let Some(room) = rooms.get_mut(&code) else { return };
                if !room.is_host(&me) {
                    return;
                }
                if let Some(p) = room.pending.remove(&joiner) {
                    let _ = p.tx.send(ServerMessage::JoinRejected {
                        reason: "The host declined your request.".into(),
                    });
                }
            }

            ClientMessage::Playback(mut p) => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else { return };
                let mut rooms = self.rooms.lock().await;
                let Some(room) = rooms.get_mut(&code) else { return };
                if !room.is_host(&me) {
                    let _ = tx.send(err("not_host", "Only the host controls playback."));
                    return;
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
                room.broadcast(&ServerMessage::SyncPlayback(p), Some(&me));
            }

            ClientMessage::RequestSync => {
                let (Some(_me), Some(code)) = (uid.clone(), room_code.clone()) else { return };
                let rooms = self.rooms.lock().await;
                let Some(room) = rooms.get(&code) else { return };
                // Normalize to the live position so the requester seeks to the right spot.
                let now = now_ms();
                let mut state = room.wire_state(&code);
                state.position_ms = state.live_position_ms(now);
                state.last_update_ms = now;
                let _ = tx.send(ServerMessage::SyncState { state });
            }

            ClientMessage::Suggest { track } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else { return };
                let mut rooms = self.rooms.lock().await;
                let Some(room) = rooms.get_mut(&code) else { return };
                if room.is_host(&me) {
                    return; // host adds tracks directly
                }
                let Some(peer) = room.peers.get(&me) else { return };
                let suggestion = Suggestion {
                    id: gen_token(),
                    from_user_id: me.clone(),
                    from_username: peer.username.clone(),
                    track,
                };
                room.suggestions.insert(suggestion.id.clone(), suggestion.clone());
                let host_id = room.host_id.clone();
                room.send_to(&host_id, ServerMessage::SuggestionReceived { suggestion });
            }

            ClientMessage::ApproveSuggestion { id } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else { return };
                let mut rooms = self.rooms.lock().await;
                let Some(room) = rooms.get_mut(&code) else { return };
                if !room.is_host(&me) {
                    return;
                }
                // The host client does the actual queueing (it owns the playback queue) and will
                // broadcast a SyncQueue; here we just drop the pending suggestion and notify the
                // suggester.
                if let Some(s) = room.suggestions.remove(&id) {
                    room.send_to(&s.from_user_id, ServerMessage::SuggestionApproved { id });
                }
            }

            ClientMessage::RejectSuggestion { id } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else { return };
                let mut rooms = self.rooms.lock().await;
                let Some(room) = rooms.get_mut(&code) else { return };
                if !room.is_host(&me) {
                    return;
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
            }

            ClientMessage::KickUser { user_id: target } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else { return };
                let mut rooms = self.rooms.lock().await;
                let Some(room) = rooms.get_mut(&code) else { return };
                if !room.is_host(&me) || target == me {
                    return;
                }
                if let Some(p) = room.peers.remove(&target) {
                    let _ = p.tx.send(ServerMessage::Kicked {
                        reason: "The host removed you from the room.".into(),
                    });
                    room.broadcast(&ServerMessage::UserLeft { user_id: target }, None);
                }
            }

            ClientMessage::TransferHost { user_id: target } => {
                let (Some(me), Some(code)) = (uid.clone(), room_code.clone()) else { return };
                let mut rooms = self.rooms.lock().await;
                let Some(room) = rooms.get_mut(&code) else { return };
                if !room.is_host(&me) || !room.peers.contains_key(&target) {
                    return;
                }
                room.host_id = target.clone();
                room.broadcast(&ServerMessage::HostChanged { host_id: target }, None);
            }

            ClientMessage::Reconnect { session_token } => {
                let mut rooms = self.rooms.lock().await;
                // Find the room + user owning this token.
                let found = rooms.iter_mut().find_map(|(code, room)| {
                    room.peers
                        .iter()
                        .find(|(_, p)| p.session_token == session_token)
                        .map(|(id, _)| (code.clone(), id.clone()))
                });
                let Some((code, user_id)) = found else {
                    let _ = tx.send(err("session_expired", "Your session has expired."));
                    return;
                };
                let room = rooms.get_mut(&code).unwrap();
                {
                    let peer = room.peers.get_mut(&user_id).unwrap();
                    peer.tx = tx.clone();
                    peer.connected = true;
                    peer.disconnected_at = None;
                }
                room.empty_since = None;
                let is_host = room.is_host(&user_id);
                let now = now_ms();
                let mut state = room.wire_state(&code);
                state.position_ms = state.live_position_ms(now);
                state.last_update_ms = now;
                let _ = tx.send(ServerMessage::Reconnected {
                    user_id: user_id.clone(),
                    is_host,
                    state,
                });
                room.broadcast(
                    &ServerMessage::UserReconnected { user_id: user_id.clone() },
                    Some(&user_id),
                );
                *uid = Some(user_id);
                *room_code = Some(code);
            }

            ClientMessage::LeaveRoom => {
                if let (Some(me), Some(code)) = (uid.take(), room_code.take()) {
                    let mut rooms = self.rooms.lock().await;
                    self.remove_member(&mut rooms, &code, &me, /*graceful=*/ true);
                }
            }
        }
    }

    /// Remove a member entirely (explicit leave, or reconnect grace expired). Handles host handoff
    /// and empty-room bookkeeping. `graceful` distinguishes an explicit leave (broadcast UserLeft)
    /// from a grace-expiry sweep (already announced as disconnected).
    fn remove_member(
        &self,
        rooms: &mut HashMap<String, Room>,
        code: &str,
        me: &str,
        graceful: bool,
    ) {
        let Some(room) = rooms.get_mut(code) else { return };
        room.pending.remove(me);
        if room.peers.remove(me).is_some() && graceful {
            room.broadcast(&ServerMessage::UserLeft { user_id: me.to_string() }, None);
        }
        // Host left → hand off to any connected peer.
        if room.host_id == me {
            if let Some(next) = room.any_connected_other(me) {
                room.host_id = next.clone();
                room.broadcast(&ServerMessage::HostChanged { host_id: next }, None);
            }
        }
        if room.peers.is_empty() {
            room.empty_since = Some(Instant::now());
        }
    }

    /// A socket dropped without leaving. Keep the slot for reconnection, but hand off host now so
    /// nobody is stuck (fixes Metrolist's up-to-15-min dead zone, context/19 §4.7).
    async fn handle_disconnect(&self, uid: Option<String>, room_code: Option<String>) {
        let (Some(me), Some(code)) = (uid, room_code) else { return };
        let mut rooms = self.rooms.lock().await;
        let Some(room) = rooms.get_mut(&code) else { return };
        // A still-pending joiner just disappears.
        if room.pending.remove(&me).is_some() {
            return;
        }
        let Some(peer) = room.peers.get_mut(&me) else { return };
        peer.connected = false;
        peer.disconnected_at = Some(Instant::now());
        room.broadcast(&ServerMessage::UserDisconnected { user_id: me.clone() }, Some(&me));
        if room.host_id == me {
            if let Some(next) = room.any_connected_other(&me) {
                room.host_id = next.clone();
                room.broadcast(&ServerMessage::HostChanged { host_id: next }, None);
            }
        }
        if room.peers.values().all(|p| !p.connected) {
            room.empty_since = Some(Instant::now());
        }
    }

    /// Sweep expired reconnection slots and empty rooms.
    async fn cleanup(&self) {
        let mut rooms = self.rooms.lock().await;
        let now = Instant::now();
        let codes: Vec<String> = rooms.keys().cloned().collect();
        for code in codes {
            // Collect peers past the grace window, then remove them via the shared path.
            let expired: Vec<String> = {
                let room = &rooms[&code];
                room.peers
                    .iter()
                    .filter(|(_, p)| {
                        !p.connected
                            && p.disconnected_at.map(|t| now - t > RECONNECT_GRACE).unwrap_or(false)
                    })
                    .map(|(id, _)| id.clone())
                    .collect()
            };
            for id in expired {
                self.remove_member(&mut rooms, &code, &id, /*graceful=*/ false);
            }
            let drop_room = rooms
                .get(&code)
                .map(|r| {
                    r.peers.is_empty()
                        && r.empty_since.map(|t| now - t > EMPTY_ROOM_TIMEOUT).unwrap_or(true)
                })
                .unwrap_or(false);
            if drop_room {
                rooms.remove(&code);
                tracing::info!(room = %code, "cleaned up empty room");
            }
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
                Ok(cm) => server.dispatch(cm, &tx, &mut uid, &mut room_code).await,
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
                server.cleanup().await;
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
            .await;
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
            .await;
        let guest_id = guid.clone().unwrap();
        server
            .dispatch(
                ClientMessage::ApproveJoin { user_id: guest_id.clone() },
                &htx,
                &mut huid,
                &mut hcode,
            )
            .await;

        // Host drops → host role must move to the connected guest (no dead zone).
        server.handle_disconnect(Some(host_id.clone()), Some(code.clone())).await;
        let rooms = server.rooms.lock().await;
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
            .await;
        let code = hcode.clone().unwrap();
        let (mut guid, mut gcode) = (None, None);
        server
            .dispatch(
                ClientMessage::JoinRoom { room_code: code.clone(), username: "g".into() },
                &gtx,
                &mut guid,
                &mut gcode,
            )
            .await;
        server
            .dispatch(
                ClientMessage::ApproveJoin { user_id: guid.clone().unwrap() },
                &htx,
                &mut huid,
                &mut hcode,
            )
            .await;
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
            .await;
        let msg = grx.try_recv().expect("guest gets a reply");
        assert!(matches!(msg, ServerMessage::Error { code, .. } if code == "not_host"));
    }
}
