//! A playlist's continuation token says where the next page starts, so the page at any position
//! can be asked for directly instead of walked to one token at a time (#536). Checked live on
//! WEB_REMIX, 2026-10-24: page 1 of a 492-track playlist handed out
//!
//! ```text
//! 80226972 { 2: "VL<id>", 3: "<params>", 35: "<id>" }
//!   params = percent-encoded base64url of { 15: "PT:<pt>", 18: { 1: 570 }, 30: 0 }
//!   pt     = unpadded base64url of { 1: 103, 4: "07AAEEE4EA16ACC1" }
//! ```
//!
//! `pt.1` is the position, counting rows YouTube doesn't hand back (that page had 100), and `pt.4`
//! the `setVideoId` the page ended on. A token rebuilt with position 442 and no anchor answered
//! the last 46 rows and no further token. Invidious builds every playlist page the same way.
//!
//! Only that exact shape is rewritten. A token carrying anything else (a sort, a field YouTube
//! adds later) gets `None` back, and the caller walks as before.

use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;

const OUTER: u64 = 80226972;

enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

fn varint(buf: &[u8], i: &mut usize) -> Option<u64> {
    let mut n = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*i)?;
        *i += 1;
        n |= u64::from(b & 0x7f) << shift;
        if b < 0x80 {
            return Some(n);
        }
    }
    None
}

/// Varint and length-delimited fields only, which is all these tokens hold.
fn fields(buf: &[u8]) -> Option<Vec<(u64, Value<'_>)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let key = varint(buf, &mut i)?;
        let value = match key & 7 {
            0 => Value::Varint(varint(buf, &mut i)?),
            2 => {
                let len = usize::try_from(varint(buf, &mut i)?).ok()?;
                let bytes = buf.get(i..i.checked_add(len)?)?;
                i += len;
                Value::Bytes(bytes)
            }
            _ => return None,
        };
        out.push((key >> 3, value));
    }
    Some(out)
}

fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push(n as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn encode(fields: &[(u64, Value<'_>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (num, value) in fields {
        match value {
            Value::Varint(n) => {
                put_varint(&mut out, num << 3);
                put_varint(&mut out, *n);
            }
            Value::Bytes(b) => {
                put_varint(&mut out, num << 3 | 2);
                put_varint(&mut out, b.len() as u64);
                out.extend_from_slice(b);
            }
        }
    }
    out
}

/// Percent-encoded base64url, padded or not, as YouTube writes both layers.
fn unwrap(s: &str) -> Option<Vec<u8>> {
    let s = urlencoding::decode(s).ok()?;
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

fn wrap(bytes: &[u8]) -> String {
    urlencoding::encode(&URL_SAFE.encode(bytes)).into_owned()
}

fn only(fields: &[(u64, Value<'_>)], allowed: &[u64]) -> bool {
    fields.iter().all(|(n, _)| allowed.contains(n))
}

fn bytes<'a>(fields: &[(u64, Value<'a>)], num: u64) -> Option<&'a [u8]> {
    fields.iter().find_map(|(n, v)| match v {
        Value::Bytes(b) if *n == num => Some(*b),
        _ => None,
    })
}

/// The three layers of a playlist token, or `None` for any other shape.
struct Parts {
    outer: Vec<u8>,
    params: Vec<u8>,
    pt: Vec<u8>,
}

fn parts(token: &str) -> Option<Parts> {
    let root = unwrap(token)?;
    let top = fields(&root)?;
    let [(OUTER, Value::Bytes(outer))] = top.as_slice() else { return None };
    let outer = outer.to_vec();
    let inner = fields(&outer)?;
    if !only(&inner, &[2, 3, 35]) {
        return None;
    }
    let params = unwrap(std::str::from_utf8(bytes(&inner, 3)?).ok()?)?;
    let p = fields(&params)?;
    if !only(&p, &[15, 18, 30]) {
        return None;
    }
    let pt = std::str::from_utf8(bytes(&p, 15)?).ok()?.strip_prefix("PT:")?;
    let pt = URL_SAFE_NO_PAD.decode(pt).ok()?;
    let f = fields(&pt)?;
    if !only(&f, &[1, 4]) || !f.iter().any(|(n, v)| *n == 1 && matches!(v, Value::Varint(_))) {
        return None;
    }
    Some(Parts { outer, params, pt })
}

/// Where the page behind `token` starts.
pub fn position(token: &str) -> Option<u64> {
    let p = parts(token)?;
    fields(&p.pt)?.into_iter().find_map(|(n, v)| match v {
        Value::Varint(pos) if n == 1 => Some(pos),
        _ => None,
    })
}

/// `token` pointed at `position` instead, every other field kept as YouTube wrote it.
pub fn at(token: &str, position: u64) -> Option<String> {
    let p = parts(token)?;
    // The anchor names the row the old position followed, so it goes: YouTube takes the position
    // on its own.
    let pt = encode(&[(1, Value::Varint(position))]);
    let pt = format!("PT:{}", URL_SAFE_NO_PAD.encode(pt));
    let params: Vec<_> = fields(&p.params)?
        .into_iter()
        .map(|(n, v)| if n == 15 { (n, Value::Bytes(pt.as_bytes())) } else { (n, v) })
        .collect();
    let params = wrap(&encode(&params));
    let inner: Vec<_> = fields(&p.outer)?
        .into_iter()
        .map(|(n, v)| if n == 3 { (n, Value::Bytes(params.as_bytes())) } else { (n, v) })
        .collect();
    Some(wrap(&encode(&[(OUTER, Value::Bytes(&encode(&inner)))])))
}

/// Page 1's token for `PL5dDx681T4bR7ZF1IuWzOv1omlRbE7PiJ`, a public playlist, 2026-10-24.
#[cfg(test)]
pub(crate) const SAMPLE: &str = "4qmFsgKHARIkVkxQTDVkRHg2ODFUNGJSN1pGMUl1V3pPdjFvbWxSYkU3UGlKGjplaDVRVkRwRFIyTnBSVVJCTTFGVlJrWlNWVlV3VWxWRmVFNXJSa1JSZWtXU0FRTUl1Z1R3QVFBJTNEmgIiUEw1ZER4NjgxVDRiUjdaRjFJdVd6T3Yxb21sUmJFN1BpSg%3D%3D";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_position() {
        assert_eq!(position(SAMPLE), Some(103));
    }

    #[test]
    fn moves_the_position_and_keeps_the_rest() {
        let moved = at(SAMPLE, 442).expect("known shape");
        assert_eq!(position(&moved), Some(442));
        // The token that answered the last 46 rows live.
        assert_eq!(moved, "4qmFsgJlEiRWTFBMNWREeDY4MVQ0YlI3WkYxSXVXek92MW9tbFJiRTdQaUoaGGVnZFFWRHBEVEc5RWtnRURDTG9FOEFFQZoCIlBMNWREeDY4MVQ0YlI3WkYxSXVXek92MW9tbFJiRTdQaUo%3D");
    }

    #[test]
    fn leaves_other_shapes_alone() {
        // A params field it has never seen, as a sorted list's token might carry.
        let p = encode(&[(15, Value::Bytes(b"PT:CGc")), (7, Value::Varint(1))]);
        let p = wrap(&p);
        let inner = encode(&[(2, Value::Bytes(b"VLx")), (3, Value::Bytes(p.as_bytes()))]);
        let token = wrap(&encode(&[(OUTER, Value::Bytes(&inner))]));
        assert_eq!(position(&token), None);
        assert_eq!(at(&token, 200), None);
        assert_eq!(position("fixture-token-3"), None);
        assert_eq!(position(""), None);
    }
}
