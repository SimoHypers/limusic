//! Pure byte-math ports of Metrolist's `JavaScriptUtil.kt` (context/04). No network, no webview —
//! fully unit-testable. Harvested from the Phase-0 `botguard-spike`, which minted a real PoToken
//! end-to-end against live YouTube, so these transforms are verified against reality.

use base64::Engine;
use serde_json::Value;

/// Decode YouTube's base64 variant (URL-safe or standard, padding optional).
/// context/04 `base64ToByteString` (`-`→`+`, `_`→`/`, then pad).
pub fn b64_decode_loose(s: &str) -> Result<Vec<u8>, String> {
    let std = s.replace('-', "+").replace('_', "/");
    let padded = match std.len() % 4 {
        0 => std,
        m => format!("{std}{}", "=".repeat(4 - m)),
    };
    base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .map_err(|e| format!("b64 decode: {e}"))
}

/// Encode bytes to YouTube's URL-safe base64 without padding (`+`→`-`, `/`→`_`).
/// context/04 `u8ToBase64` (the final PoToken form appended to the stream URL as `&pot=`).
pub fn b64url_encode_no_pad(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Descramble the `/Create` challenge: base64-decode, add 97 to each byte (mod 256), JSON-parse.
/// context/04 `parseChallengeData` step 1 (verified live in the spike).
pub fn descramble(scrambled: &str) -> Result<Value, String> {
    let bytes = b64_decode_loose(scrambled)?;
    let shifted: Vec<u8> = bytes.iter().map(|b| b.wrapping_add(97)).collect();
    let s = String::from_utf8(shifted).map_err(|e| format!("descramble utf8: {e}"))?;
    serde_json::from_str(&s).map_err(|e| format!("descramble json: {e}"))
}

/// Reshape a descrambled `/Create` challenge array into the `challengeData` object the harness's
/// `runBotGuard(data)` expects. context/04 `parseChallengeData`; field layout verified in the spike:
/// `[msgId, wrappedScript, wrappedUrl, hash, program, globalName, _, expBlob]`.
pub fn parse_challenge_data(scrambled: &str) -> Result<Value, String> {
    let arr = descramble(scrambled)?;
    let interpreter_js = arr
        .get(1)
        .and_then(|v| v.as_array())
        .and_then(|a| a.iter().find_map(|x| x.as_str()))
        .ok_or("no interpreter js in challenge[1]")?;
    let program = arr.get(4).and_then(|v| v.as_str()).ok_or("no program[4]")?;
    let global_name = arr.get(5).and_then(|v| v.as_str()).ok_or("no globalName[5]")?;
    Ok(serde_json::json!({
        "globalName": global_name,
        "program": program,
        "interpreterJavascript": { "privateDoNotAccessOrElseSafeScriptWrappedValue": interpreter_js }
    }))
}

/// Parse the `/GenerateIT` response into `(integrityToken bytes, ttlSeconds)`.
/// context/04 `parseIntegrityTokenData`: `[0]` = integrity token (base64), `[1]` = ttl seconds.
pub fn parse_integrity_token_data(response_body: &str) -> Result<(Vec<u8>, u64), String> {
    let raw: Value =
        serde_json::from_str(response_body).map_err(|e| format!("genit json: {e} :: {response_body}"))?;
    let tok_b64 = raw.get(0).and_then(|v| v.as_str()).ok_or("genit[0] not string")?;
    let ttl = raw.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
    Ok((b64_decode_loose(tok_b64)?, ttl))
}

/// Render bytes as a JS numeric-array literal `[1,2,3]` for `new Uint8Array([...])`.
/// context/04 `newUint8Array` / `stringToU8`.
pub fn js_byte_array(bytes: &[u8]) -> String {
    let parts: Vec<String> = bytes.iter().map(|b| b.to_string()).collect();
    format!("[{}]", parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descramble_round_trips() {
        // Inverse of descramble: subtract 97, base64-encode → the scrambled form YouTube sends.
        let target = r#"["msg",["var x=1;"],null,"hash","PROGRAM","GLOBAL",null,"exp"]"#;
        let pre: Vec<u8> = target.bytes().map(|b| b.wrapping_sub(97)).collect();
        let scrambled = base64::engine::general_purpose::STANDARD.encode(&pre);
        assert_eq!(descramble(&scrambled).unwrap(), serde_json::from_str::<Value>(target).unwrap());
    }

    #[test]
    fn parse_challenge_extracts_fields() {
        let target = r#"["msg",["INTERP_JS"],null,"hash","PROG","GLOB",null,"exp"]"#;
        let pre: Vec<u8> = target.bytes().map(|b| b.wrapping_sub(97)).collect();
        let scrambled = base64::engine::general_purpose::STANDARD.encode(&pre);
        let cd = parse_challenge_data(&scrambled).unwrap();
        assert_eq!(cd["globalName"], "GLOB");
        assert_eq!(cd["program"], "PROG");
        assert_eq!(
            cd["interpreterJavascript"]["privateDoNotAccessOrElseSafeScriptWrappedValue"],
            "INTERP_JS"
        );
    }

    #[test]
    fn integrity_token_parses() {
        // "AAA" (3 base64 chars → padded) decodes to bytes; ttl comes through.
        let (tok, ttl) = parse_integrity_token_data(r#"["SGVsbG8", 43200, "extra"]"#).unwrap();
        assert_eq!(tok, b"Hello");
        assert_eq!(ttl, 43200);
    }

    #[test]
    fn base64url_no_pad_is_url_safe() {
        // 0xFF 0xFF 0xFE → standard "///+" → url-safe no-pad "___-"
        assert_eq!(b64url_encode_no_pad(&[0xFF, 0xFF, 0xFE]), "___-");
        // Round-trips with the loose decoder.
        assert_eq!(b64_decode_loose("___-").unwrap(), vec![0xFF, 0xFF, 0xFE]);
    }

    #[test]
    fn js_array_literal() {
        assert_eq!(js_byte_array(&[0, 1, 255]), "[0,1,255]");
    }

    #[test]
    fn loose_decode_handles_urlsafe_and_missing_pad() {
        assert_eq!(b64_decode_loose("SGVsbG8").unwrap(), b"Hello"); // missing padding
        assert_eq!(b64_decode_loose("___-").unwrap(), vec![0xFF, 0xFF, 0xFE]); // url-safe chars
    }
}
