//! `FunctionNameExtractor` (context/05 §2) — find, inside a fetched `player.js`, the
//! signature-timestamp and the *names* of the signature-decipher and `n`-transform functions.
//!
//! Because the cipher webview runs YouTube's OWN `player.js`, we only need to NAME the entry
//! functions, not reimplement their bodies. Modern (2025+) player.js is Q-array obfuscated, so
//! regex name-finding is best-effort and can false-match in ~2.8 MB of code (context/05). The
//! reliable path is the config table ([`super::config`], `isHardcoded`); regex is the fallback,
//! and a webview brute-force ([`build_injection`] + the harness discovery script) is the net for
//! the `n` function. STS extraction, by contrast, is a simple and reliable literal search.

use regex::Regex;

/// The marker that closes player.js's IIFE — our export-injection point (verified present in the
/// live player.js). context/05 §CipherWebView injection.
pub const IIFE_TAIL: &str = "})(_yt_player);";

/// Extract the `signatureTimestamp` that must accompany the `/player` request (context/03, 05).
pub fn extract_sts(player_js: &str) -> Option<i32> {
    // Tolerate both `signatureTimestamp:20632` (raw) and `"sts":19999` (JSON-ish).
    let re = Regex::new(r#"(?:signatureTimestamp|sts)"?\s*[:=]\s*(\d{4,})"#).ok()?;
    re.captures(player_js)?.get(1)?.as_str().parse().ok()
}

/// Best-effort name of the signature-decipher function. context/05.
///
/// NB: the `regex` crate has no backreferences, so these patterns can't require the split/join to
/// use the same variable — they match the shape loosely. A wrong hit self-heals via a 403 →
/// `on_stream_rejected`; the config table is the reliable path.
pub fn find_sig_fn(player_js: &str) -> Option<String> {
    let patterns = [
        r#"\b([a-zA-Z0-9$_]{2,})=function\([a-zA-Z0-9$_]\)\{\s*[a-zA-Z0-9$_]=[a-zA-Z0-9$_]\.split\(""\)"#,
        r#"\b([a-zA-Z0-9$_]{2,})=function\([a-zA-Z0-9$_]\)\{var [a-zA-Z0-9$_]=[a-zA-Z0-9$_]\.split\(""\)"#,
        r#"(?:["']signature["']|\bsig)\s*[,:=]\s*([a-zA-Z0-9$_]{2,})\("#,
    ];
    first_capture(player_js, &patterns)
}

/// Best-effort name of the `n`-throttling-transform function. context/05. Frequently unresolvable
/// statically on obfuscated players → `None`, and the webview brute-force takes over.
pub fn find_n_fn(player_js: &str) -> Option<String> {
    let patterns = [
        r#"[a-zA-Z0-9$_]=([a-zA-Z0-9$_]{2,})(?:\[\d+\])?\([a-zA-Z0-9$_]\)[;,][a-zA-Z0-9$_.]{0,20}\.set\("n""#,
        r#"\.get\("n"\)\)&&\([a-zA-Z0-9$_]=([a-zA-Z0-9$_]{2,})[\(\[]"#,
        r#"\b([a-zA-Z0-9$_]{2,})=function\([a-zA-Z0-9$_]\)\{var [a-zA-Z0-9$_]=[a-zA-Z0-9$_]\.split\(""\),[a-zA-Z0-9$_]=\[\]"#,
    ];
    first_capture(player_js, &patterns)
}

fn first_capture(js: &str, patterns: &[&str]) -> Option<String> {
    for p in patterns {
        if let Ok(re) = Regex::new(p) {
            if let Some(c) = re.captures(js) {
                if let Some(m) = c.get(1) {
                    return Some(m.as_str().to_owned());
                }
            }
        }
    }
    None
}

/// Turn `player.js` into a self-exporting script: append, inside the IIFE, exports that expose the
/// sig/n entry functions on `window` so the harness can call them. A `None` name is left for the
/// harness's brute-force discovery to fill in. context/05 §injection.
pub fn build_injection(player_js: &str, sig_fn: Option<&str>, n_fn: Option<&str>) -> String {
    let mut exports = String::from(";");
    if let Some(sig) = sig_fn {
        exports.push_str(&format!(
            "try{{window._cipherSigFunc=function(s){{return {sig}(s);}};}}catch(e){{}}"
        ));
    }
    if let Some(n) = n_fn {
        exports.push_str(&format!(
            "try{{window._nTransformFunc=function(n){{return {n}(n);}};}}catch(e){{}}"
        ));
    }
    exports.push_str(&format!("{IIFE_TAIL}"));
    // Replace only the final IIFE close so the exports live inside the player closure's scope.
    match player_js.rfind(IIFE_TAIL) {
        Some(idx) => {
            let mut out = String::with_capacity(player_js.len() + exports.len());
            out.push_str(&player_js[..idx]);
            out.push_str(&exports);
            out.push_str(&player_js[idx + IIFE_TAIL.len()..]);
            out
        }
        // No known tail (unexpected player shape) — append exports and hope globals are reachable.
        None => format!("{player_js}\n{exports}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sts_extracted() {
        assert_eq!(extract_sts("abc signatureTimestamp:20632 def"), Some(20632));
        assert_eq!(extract_sts(r#"...,"sts":19999,..."#), Some(19999));
        assert_eq!(extract_sts("no timestamp here"), None);
    }

    #[test]
    fn sig_fn_classic_definition() {
        let js = r#"var x=1;Dz=function(a){a=a.split("");Xu.reverse(a,1);return a.join("")};"#;
        assert_eq!(find_sig_fn(js).as_deref(), Some("Dz"));
    }

    #[test]
    fn n_fn_set_pattern() {
        // Positive match doubles as a compile guard: a pattern with an unsupported construct (e.g.
        // a backreference) is silently skipped, so a real capture proves it compiled AND matched.
        let js = r#"c=d.get("n"))&&(d=Gx[0](c),e.set("n",d)"#;
        assert_eq!(find_n_fn(js).as_deref(), Some("Gx"));
    }

    #[test]
    fn injection_lands_inside_iife() {
        let js = "var _yt_player={};(function(g){g.foo=1;})(_yt_player);";
        let out = build_injection(js, Some("Dz"), Some("En"));
        assert!(out.contains("window._cipherSigFunc=function(s){return Dz(s);}"));
        assert!(out.contains("window._nTransformFunc=function(n){return En(n);}"));
        // Exports sit before the (single) IIFE tail, and the tail still closes the script.
        assert!(out.ends_with(IIFE_TAIL));
        let export_at = out.find("window._cipherSigFunc").unwrap();
        assert!(export_at < out.rfind(IIFE_TAIL).unwrap());
    }

    #[test]
    fn injection_skips_unknown_names() {
        let js = "(function(g){})(_yt_player);";
        let out = build_injection(js, None, None);
        assert!(!out.contains("_cipherSigFunc"));
        assert!(!out.contains("_nTransformFunc"));
        assert!(out.ends_with(IIFE_TAIL));
    }
}
