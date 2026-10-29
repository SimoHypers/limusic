//! Shared wire types for Limusic "Listen Together". JSON over WebSocket, used by both the sync
//! server (`crates/sync-server`) and the Tauri client (`src-tauri/src/listentogether`).
//!
//! The design is copied from Metrolist's `metroserver` (see `context/19-listen-together.md`):
//! host-authoritative, push-based sync, server-extrapolated playback position. The differences:
//! JSON instead of protobuf, and the server hands the host role off 20s after a disconnect instead
//! of freezing the room for 15 minutes (context/19 §4.7 "host-gone dead zone").

use serde::{Deserialize, Serialize};

/// A track in the shared room. `id` is a raw YouTube videoId — the client resolves and plays it
/// locally through the normal orchestrator; the server only relays it (context/19 Part B §6).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// Track length in ms (0 if unknown). Only used for clamping/UI.
    #[serde(default)]
    pub duration_ms: i64,
    /// Username of the guest who added this track to the shared queue (Spotify-Jam-style adds).
    /// `None` for tracks from the host's own playlist. Stamped by the host client on enqueue
    /// (mirrors Metrolist's `TrackInfo.SuggestedBy`, context/19 §3.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_by: Option<String>,
}

/// A room participant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub user_id: String,
    pub username: String,
    pub is_host: bool,
    pub is_connected: bool,
}

/// The authoritative shared state the server owns and syncs to everyone.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoomState {
    pub room_code: String,
    pub host_id: String,
    pub users: Vec<User>,
    pub current_track: Option<Track>,
    pub is_playing: bool,
    pub position_ms: i64,
    /// Server wall-clock ms when `position_ms` was last set. Lets clients extrapolate the live
    /// position (context/19 §6.2).
    pub last_update_ms: i64,
    pub volume: f64,
    pub queue: Vec<Track>,
}

impl RoomState {
    /// Extrapolated live position at `now_ms`: if playing, last reported position plus elapsed
    /// wall-clock since it was set (context/19 §6.2 `livePlaybackPosition`).
    pub fn live_position_ms(&self, now_ms: i64) -> i64 {
        let mut p = self.position_ms;
        if self.is_playing && self.last_update_ms > 0 {
            let elapsed = now_ms - self.last_update_ms;
            if elapsed > 0 {
                p += elapsed;
            }
        }
        p.max(0)
    }
}

/// A pending track suggestion from a guest, awaiting host approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub from_user_id: String,
    pub from_username: String,
    pub track: Track,
}

/// What kind of playback action this is. Flat struct (below) mirrors Metrolist's
/// `PlaybackActionPayload` (context/19 §6.3) — one shape carries both the host command and the
/// broadcast, with only the relevant fields populated per kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackKind {
    Play,
    Pause,
    Seek,
    ChangeTrack,
    SyncQueue,
    SetVolume,
}

/// The core sync payload. Host → server → all clients. Only the fields relevant to `kind` are set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playback {
    pub kind: PlaybackKind,
    #[serde(default)]
    pub position_ms: i64,
    /// Stamped by the server on `Play` rebroadcast so clients can offset for latency (context/19
    /// §6.5). 0 on the client→server request.
    #[serde(default)]
    pub server_time_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<Vec<Track>>,
    #[serde(default)]
    pub playing: bool,
    #[serde(default)]
    pub volume: f64,
}

impl Playback {
    pub fn new(kind: PlaybackKind) -> Self {
        Playback {
            kind,
            position_ms: 0,
            server_time_ms: 0,
            track: None,
            queue: None,
            playing: false,
            volume: 0.0,
        }
    }
    pub fn at(kind: PlaybackKind, position_ms: i64) -> Self {
        Playback { position_ms, ..Playback::new(kind) }
    }
}

/// A control for another linked device (device link). Clients send it inside `remote`; the server
/// forwards the JSON without reading it, so a new action needs no server change, and a client
/// ignores an action it doesn't know.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteAction {
    Play,
    Pause,
    Next,
    Previous,
    Seek {
        position_ms: i64,
    },
    /// 0.0..=1.0
    SetVolume {
        volume: f64,
    },
    /// One song, with its radio behind it.
    PlayTrack {
        track: Track,
    },
    /// A list: a playlist, album or radio. `start` is the tapped row, `None` = just play it.
    PlayTracks {
        tracks: Vec<Track>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<usize>,
        /// Lets autoplay carry on with the list's radio when it runs out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_name: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        shuffle: bool,
    },
    /// The queue row at `index` in the player's whole queue ([`LinkStatus::offset`] + position).
    /// `id` is the row's track: a queue that moved meanwhile is searched for it instead.
    PlayIndex {
        index: usize,
        id: String,
    },
    Remove {
        index: usize,
        id: String,
    },
    /// Drag to reorder: queue indices, as `move_in_queue` takes them.
    Move {
        from: usize,
        to: usize,
    },
    PlayNext {
        tracks: Vec<Track>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    AddToQueue {
        tracks: Vec<Track>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    ClearQueued,
    SetShuffle {
        on: bool,
    },
    /// "off" | "all" | "one"
    SetRepeat {
        mode: String,
    },
    /// The player's own Autoplay setting.
    SetAutoplay {
        on: bool,
    },
    /// To a device that isn't playing: play what the player plays, from where it is now, with its
    /// queue (Continue here, asked from another device).
    TakeOver,
}

/// The player's queue, for the other devices' player views (device link). Sent as `link_status`
/// whenever it changes; the server keeps the last one without reading it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LinkStatus {
    /// The queue around the playing track, in play order.
    pub items: Vec<LinkItem>,
    /// Where `items[0]` is in the whole queue: remote actions name rows by queue index.
    #[serde(default)]
    pub offset: usize,
    /// The playing row, in `items`.
    #[serde(default)]
    pub current: usize,
    #[serde(default)]
    pub shuffle: bool,
    /// "off" | "all" | "one"
    #[serde(default)]
    pub repeat: String,
    /// The playlist or album the queue plays from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The player's Autoplay setting; absent from a player that predates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoplay: Option<bool>,
}

/// A queue row with the marks the queue panel draws.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkItem {
    #[serde(flatten)]
    pub track: Track,
    /// Play next.
    #[serde(default, skip_serializing_if = "is_false")]
    pub queued: bool,
    /// Add to queue.
    #[serde(default, skip_serializing_if = "is_false")]
    pub queued_end: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_from: Option<String>,
    /// Put there by autoplay.
    #[serde(default, skip_serializing_if = "is_false")]
    pub autoplay: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Client → server. Internally tagged on `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Create a room and become host.
    CreateRoom {
        username: String,
    },
    /// Request to join an existing room (host must approve).
    JoinRoom {
        room_code: String,
        username: String,
    },
    /// Re-attach after a dropped socket, using the token from create/join.
    Reconnect {
        session_token: String,
    },
    /// Leave the current room.
    LeaveRoom,
    /// Host: approve/reject a pending joiner.
    ApproveJoin {
        user_id: String,
    },
    RejectJoin {
        user_id: String,
    },
    /// Host: remove a participant.
    KickUser {
        user_id: String,
    },
    /// Host: hand the host role to another participant.
    TransferHost {
        user_id: String,
    },
    /// Host: a playback action to broadcast to everyone.
    Playback(Playback),
    /// Guest: suggest a track to the host.
    Suggest {
        track: Track,
    },
    /// Host: act on a suggestion.
    ApproveSuggestion {
        id: String,
    },
    RejectSuggestion {
        id: String,
    },
    /// Pull the current authoritative state (drift correction / after reconnect).
    RequestSync,
    /// Keepalive.
    Ping,
    /// Device link: join (or create) the link room keyed by `secret`. Holding the secret is
    /// membership, so there is no approval step.
    JoinLink {
        secret: String,
        device_id: String,
        name: String,
    },
    /// Device link member: ask for a 6-letter pairing code for this room.
    PairStart,
    /// A device not in any room trades a pairing code for the link secret.
    PairClaim {
        code: String,
    },
    /// Device link: a control for another device, the player unless `to` names one. `action` is
    /// a [`RemoteAction`]; the server forwards it unread.
    Remote {
        action: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
    /// Device link, from the player: its queue as a [`LinkStatus`], kept and passed on unread.
    LinkStatus {
        status: serde_json::Value,
    },
}

/// Server → client. Internally tagged on `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    RoomCreated {
        room_code: String,
        user_id: String,
        session_token: String,
        state: RoomState,
    },
    /// Host receives this when someone requests to join.
    JoinRequest {
        user_id: String,
        username: String,
    },
    JoinApproved {
        user_id: String,
        session_token: String,
        state: RoomState,
    },
    JoinRejected {
        reason: String,
    },
    Reconnected {
        user_id: String,
        is_host: bool,
        state: RoomState,
    },
    UserJoined {
        user: User,
    },
    UserLeft {
        user_id: String,
    },
    UserDisconnected {
        user_id: String,
    },
    UserReconnected {
        user_id: String,
    },
    HostChanged {
        host_id: String,
    },
    /// A playback action to apply (guests) / echo (host).
    SyncPlayback(Playback),
    /// Full state, in reply to `RequestSync`.
    SyncState {
        state: RoomState,
    },
    SuggestionReceived {
        suggestion: Suggestion,
    },
    SuggestionApproved {
        id: String,
    },
    SuggestionRejected {
        id: String,
        reason: String,
    },
    Kicked {
        reason: String,
    },
    Error {
        code: String,
        message: String,
    },
    Pong,
    /// Device link: reply to `JoinLink`. `room_code` is empty so the secret is never echoed.
    /// `status` is the player's last `link_status`.
    LinkJoined {
        state: RoomState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<serde_json::Value>,
    },
    PairCode {
        code: String,
        expires_in_s: u32,
    },
    Paired {
        secret: String,
    },
    /// Device link: a control forwarded from another device. `from` is the sender's device id.
    Remote {
        from: String,
        action: serde_json::Value,
    },
    /// Device link: the player's queue changed.
    LinkStatus {
        status: serde_json::Value,
    },
}

impl ClientMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serialize ClientMessage")
    }
}

impl ServerMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("serialize ServerMessage")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_roundtrips_flat() {
        let p = Playback {
            kind: PlaybackKind::ChangeTrack,
            position_ms: 4200,
            server_time_ms: 0,
            track: Some(Track {
                id: "dQw4w9WgXcQ".into(),
                title: "x".into(),
                artist: "y".into(),
                thumbnail: None,
                duration_ms: 180_000,
                queued_by: None,
            }),
            queue: Some(vec![]),
            playing: true,
            volume: 0.0,
        };
        let msg = ClientMessage::Playback(p);
        let json = msg.to_json();
        // Internally-tagged newtype variant flattens the struct fields alongside the tag.
        assert!(json.contains("\"type\":\"playback\""));
        assert!(json.contains("\"kind\":\"change_track\""));
        let back: ClientMessage = serde_json::from_str(&json).unwrap();
        match back {
            ClientMessage::Playback(p) => {
                assert_eq!(p.kind, PlaybackKind::ChangeTrack);
                assert_eq!(p.position_ms, 4200);
                assert_eq!(p.track.unwrap().id, "dQw4w9WgXcQ");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn unit_variant_roundtrips() {
        let json = ClientMessage::LeaveRoom.to_json();
        assert_eq!(json, "{\"type\":\"leave_room\"}");
        assert!(matches!(
            serde_json::from_str::<ClientMessage>(&json).unwrap(),
            ClientMessage::LeaveRoom
        ));
    }

    #[test]
    fn live_position_extrapolates_only_while_playing() {
        let mut s = RoomState { position_ms: 1000, last_update_ms: 10_000, ..Default::default() };
        // Paused: no extrapolation.
        assert_eq!(s.live_position_ms(15_000), 1000);
        // Playing: + elapsed wall clock.
        s.is_playing = true;
        assert_eq!(s.live_position_ms(15_000), 6000);
        // Never negative.
        assert_eq!(s.live_position_ms(5_000), 1000);
    }

    /// The device-link wire lines, byte for byte. Android's `MessageCodecTest` holds the same
    /// strings, so a drift fails on both sides.
    const SECRET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
    const DEVICE: &str = "0b6e1f9a-3c2d-4e5f-8a7b-9c0d1e2f3a4b";

    fn client_line(json: &str, msg: ClientMessage) {
        assert_eq!(msg.to_json(), json);
        assert_eq!(serde_json::from_str::<ClientMessage>(json).unwrap().to_json(), json);
    }

    fn server_line(json: &str, msg: ServerMessage) {
        assert_eq!(msg.to_json(), json);
        assert_eq!(serde_json::from_str::<ServerMessage>(json).unwrap().to_json(), json);
    }

    #[test]
    fn device_link_lines() {
        assert_eq!(SECRET.len(), 43);
        client_line(
            &format!(
                r#"{{"type":"join_link","secret":"{SECRET}","device_id":"{DEVICE}","name":"fedora"}}"#
            ),
            ClientMessage::JoinLink {
                secret: SECRET.into(),
                device_id: DEVICE.into(),
                name: "fedora".into(),
            },
        );
        server_line(
            &format!(
                r#"{{"type":"link_joined","state":{{"room_code":"","host_id":"","users":[{{"user_id":"{DEVICE}","username":"fedora","is_host":false,"is_connected":true}}],"current_track":null,"is_playing":false,"position_ms":0,"last_update_ms":0,"volume":1.0,"queue":[]}}}}"#
            ),
            ServerMessage::LinkJoined {
                state: RoomState {
                    users: vec![User {
                        user_id: DEVICE.into(),
                        username: "fedora".into(),
                        is_host: false,
                        is_connected: true,
                    }],
                    volume: 1.0,
                    ..Default::default()
                },
                status: None,
            },
        );
        client_line(r#"{"type":"pair_start"}"#, ClientMessage::PairStart);
        server_line(
            r#"{"type":"pair_code","code":"K7QX2M","expires_in_s":120}"#,
            ServerMessage::PairCode { code: "K7QX2M".into(), expires_in_s: 120 },
        );
        client_line(
            r#"{"type":"pair_claim","code":"K7QX2M"}"#,
            ClientMessage::PairClaim { code: "K7QX2M".into() },
        );
        server_line(
            &format!(r#"{{"type":"paired","secret":"{SECRET}"}}"#),
            ServerMessage::Paired { secret: SECRET.into() },
        );
        client_line(
            r#"{"type":"remote","action":{"kind":"seek","position_ms":61000}}"#,
            ClientMessage::Remote {
                action: serde_json::json!({"kind": "seek", "position_ms": 61000}),
                to: None,
            },
        );
        client_line(
            &format!(r#"{{"type":"remote","action":{{"kind":"take_over"}},"to":"{DEVICE}"}}"#),
            ClientMessage::Remote {
                action: serde_json::json!({"kind": "take_over"}),
                to: Some(DEVICE.into()),
            },
        );
        server_line(
            &format!(r#"{{"type":"remote","from":"{DEVICE}","action":{{"kind":"next"}}}}"#),
            ServerMessage::Remote {
                from: DEVICE.into(),
                action: serde_json::json!({"kind": "next"}),
            },
        );
        client_line(
            r#"{"type":"link_status","status":{"current":0,"items":[],"offset":0}}"#,
            ClientMessage::LinkStatus {
                status: serde_json::json!({"current": 0, "items": [], "offset": 0}),
            },
        );
        server_line(
            r#"{"type":"link_status","status":{"current":0,"items":[],"offset":0}}"#,
            ServerMessage::LinkStatus {
                status: serde_json::json!({"current": 0, "items": [], "offset": 0}),
            },
        );
    }

    fn track() -> Track {
        Track {
            id: "dQw4w9WgXcQ".into(),
            title: "x".into(),
            artist: "y".into(),
            thumbnail: None,
            duration_ms: 0,
            queued_by: None,
        }
    }

    #[test]
    fn remote_actions_roundtrip() {
        for a in [
            RemoteAction::Play,
            RemoteAction::Pause,
            RemoteAction::Next,
            RemoteAction::Previous,
            RemoteAction::Seek { position_ms: 5 },
            RemoteAction::SetVolume { volume: 0.5 },
            RemoteAction::PlayTrack { track: track() },
            RemoteAction::PlayTracks {
                tracks: vec![track(), track()],
                start: Some(1),
                source_id: Some("PL1".into()),
                source_name: Some("Mix".into()),
                shuffle: true,
            },
            RemoteAction::PlayIndex { index: 3, id: "a".into() },
            RemoteAction::Remove { index: 4, id: "b".into() },
            RemoteAction::Move { from: 5, to: 2 },
            RemoteAction::PlayNext { tracks: vec![track()], from: Some("Album".into()) },
            RemoteAction::AddToQueue { tracks: vec![track()], from: None },
            RemoteAction::ClearQueued,
            RemoteAction::SetShuffle { on: true },
            RemoteAction::SetRepeat { mode: "one".into() },
            RemoteAction::SetAutoplay { on: false },
            RemoteAction::TakeOver,
        ] {
            // Through the wire message the way a client sends it and the server passes it on.
            let json =
                ClientMessage::Remote { action: serde_json::to_value(&a).unwrap(), to: None }
                    .to_json();
            match serde_json::from_str::<ClientMessage>(&json).unwrap() {
                ClientMessage::Remote { action, .. } => {
                    assert_eq!(serde_json::from_value::<RemoteAction>(action).unwrap(), a, "{json}")
                }
                m => panic!("wrong variant {m:?}"),
            }
        }
        // Android writes these by hand: the shapes, field for field.
        assert_eq!(
            serde_json::to_string(&RemoteAction::SetVolume { volume: 0.5 }).unwrap(),
            r#"{"kind":"set_volume","volume":0.5}"#
        );
        assert_eq!(
            serde_json::to_string(&RemoteAction::PlayIndex { index: 3, id: "a".into() }).unwrap(),
            r#"{"kind":"play_index","index":3,"id":"a"}"#
        );
        assert_eq!(
            serde_json::to_string(&RemoteAction::SetShuffle { on: false }).unwrap(),
            r#"{"kind":"set_shuffle","on":false}"#
        );
        // A newer client's action: an error to skip, not a crash.
        assert!(serde_json::from_value::<RemoteAction>(serde_json::json!({"kind": "teleport"}))
            .is_err());
    }

    #[test]
    fn link_status_roundtrips() {
        let s = LinkStatus {
            items: vec![
                LinkItem {
                    track: track(),
                    queued: false,
                    queued_end: false,
                    queued_from: None,
                    autoplay: false,
                },
                LinkItem {
                    track: track(),
                    queued: true,
                    queued_end: false,
                    queued_from: Some("Album".into()),
                    autoplay: true,
                },
            ],
            offset: 2,
            current: 1,
            shuffle: true,
            repeat: "all".into(),
            source: Some("Mix".into()),
            autoplay: Some(true),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<LinkStatus>(&json).unwrap(), s);
        // Unset marks stay off the wire.
        assert_eq!(
            serde_json::to_string(&s.items[0]).unwrap(),
            r#"{"id":"dQw4w9WgXcQ","title":"x","artist":"y","thumbnail":null,"duration_ms":0}"#
        );
    }
}
