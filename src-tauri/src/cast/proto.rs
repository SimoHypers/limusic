//! The CastV2 channel envelope.
//!
//! A CastV2 frame is a 4-byte big-endian length followed by a protobuf `CastMessage`
//! (Chromium's `components/cast_channel/cast_channel.proto`):
//!
//! ```proto
//! message CastMessage {
//!   enum ProtocolVersion { CASTV2_1_0 = 0; /* … */ }
//!   required ProtocolVersion protocol_version = 1;
//!   required string source_id = 2;
//!   required string destination_id = 3;
//!   required string namespace = 4;
//!   enum PayloadType { STRING = 0; BINARY = 1; }
//!   required PayloadType payload_type = 5;
//!   optional string payload_utf8 = 6;
//!   optional bytes payload_binary = 7;
//! }
//! ```
//!
//! Only the envelope is protobuf: every namespace this app uses takes a JSON payload, which the
//! receiver parses as JSON once it has unwrapped the envelope. So the protobuf here is seven
//! fields, of which four are read back — the rest are skipped by wire type.

/// The `STRING` payload type. Every message this app sends is JSON.
const PAYLOAD_TYPE_STRING: u64 = 0;

/// Wire type 0: a varint.
const WIRE_VARINT: u64 = 0;
/// Wire type 2: length-delimited (strings, bytes, submessages).
const WIRE_LEN: u64 = 2;

/// A decoded frame: the namespace it belongs to and its JSON payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub namespace: String,
    pub payload: String,
}

/// Frame one message: the length prefix, then the envelope.
pub fn frame(namespace: &str, source: &str, destination: &str, payload: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(payload.len() + namespace.len() + source.len() + 32);
    put_varint_field(&mut body, 1, 0); // protocol_version: CASTV2_1_0
    put_string_field(&mut body, 2, source);
    put_string_field(&mut body, 3, destination);
    put_string_field(&mut body, 4, namespace);
    put_varint_field(&mut body, 5, PAYLOAD_TYPE_STRING);
    put_string_field(&mut body, 6, payload);

    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// Read one frame body (without the length prefix) back.
pub fn decode(body: &[u8]) -> Option<Frame> {
    let mut namespace = String::new();
    let mut payload = String::new();
    let mut at = 0usize;
    while at < body.len() {
        let (tag, next) = read_varint(body, at)?;
        at = next;
        let field = tag >> 3;
        let wire = tag & 0x7;
        match wire {
            WIRE_VARINT => {
                let (_, next) = read_varint(body, at)?;
                at = next;
            }
            WIRE_LEN => {
                let (len, next) = read_varint(body, at)?;
                let len = len as usize;
                let end = next.checked_add(len)?;
                let bytes = body.get(next..end)?;
                match field {
                    4 => namespace = String::from_utf8_lossy(bytes).into_owned(),
                    6 => payload = String::from_utf8_lossy(bytes).into_owned(),
                    _ => {}
                }
                at = end;
            }
            // Fixed64 and fixed32: skip their bytes. The envelope has neither, but a future
            // protocol version might, and a frame we cannot walk is worse than one we can.
            1 => at = at.checked_add(8)?,
            5 => at = at.checked_add(4)?,
            _ => return None,
        }
    }
    Some(Frame { namespace, payload })
}

/// One tag: the field number and the wire type.
fn put_tag(out: &mut Vec<u8>, field: u64, wire: u64) {
    put_varint(out, (field << 3) | wire);
}

fn put_varint_field(out: &mut Vec<u8>, field: u64, value: u64) {
    put_tag(out, field, WIRE_VARINT);
    put_varint(out, value);
}

fn put_string_field(out: &mut Vec<u8>, field: u64, value: &str) {
    put_tag(out, field, WIRE_LEN);
    put_varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Read a varint, returning it and where it ended.
fn read_varint(buf: &[u8], at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    let mut at = at;
    loop {
        let byte = *buf.get(at)?;
        at += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((value, at));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips() {
        let body = frame(
            "urn:x-cast:com.google.cast.media",
            "sender-0",
            "receiver-0",
            r#"{"type":"LOAD"}"#,
        );
        assert_eq!(&body[..4], &(body.len() as u32 - 4).to_be_bytes());
        let decoded = decode(&body[4..]).expect("decodes");
        assert_eq!(decoded.namespace, "urn:x-cast:com.google.cast.media");
        assert_eq!(decoded.payload, r#"{"type":"LOAD"}"#);
    }

    #[test]
    fn varints_survive_the_boundaries() {
        for value in [0u64, 1, 127, 128, 300, 16_383, 16_384, u32::MAX as u64] {
            let mut buf = Vec::new();
            put_varint(&mut buf, value);
            let (read, at) = read_varint(&buf, 0).expect("reads back");
            assert_eq!(read, value);
            assert_eq!(at, buf.len());
        }
    }

    #[test]
    fn an_unknown_field_is_skipped_not_fatal() {
        // Field 7 (payload_binary) is bytes this app never sends, but a receiver may answer with
        // one, and a frame we cannot walk is a frame we drop.
        let mut body = Vec::new();
        put_varint_field(&mut body, 1, 0);
        put_string_field(&mut body, 4, "ns");
        put_tag(&mut body, 7, WIRE_LEN);
        put_varint(&mut body, 3);
        body.extend_from_slice(&[1, 2, 3]);
        put_string_field(&mut body, 6, "{}");
        let decoded = decode(&body).expect("decodes");
        assert_eq!(decoded.namespace, "ns");
        assert_eq!(decoded.payload, "{}");
    }

    #[test]
    fn a_truncated_frame_is_refused() {
        let body = frame("ns", "s", "d", "{}");
        assert!(decode(&body[4..body.len() - 2]).is_none());
    }
}
