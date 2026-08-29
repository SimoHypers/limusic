//! The text a user hands over when something breaks: what this machine is, plus the tail of
//! `limusic.log` with the secrets taken out.
//!
//! A report used to cost a round of questions (version, distro, how they installed it) and then
//! walking someone to `~/.local/share/limusic/limusic.log` by hand. This is that conversation,
//! precomputed, behind one button in Settings ▸ About.
//!
//! Redaction is the part that is not allowed to be lazy. The blob is written to be pasted into a
//! public GitHub issue, and the log carries cookies, PoTokens, signed googlevideo URLs and the
//! user's IP. The rules in [`redact`] are deliberately over-eager: losing a detail costs one
//! follow-up question, leaking a SAPISID costs the user their account.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use tauri::{AppHandle, Manager};

use crate::db::Db;

/// Ceiling on the whole report. A GitHub issue comment tops out at 65,536 characters and the point
/// of this is that it can be pasted into one.
const MAX_CHARS: usize = 60_000;

/// A current log shorter than this is a fresh launch, so the run that actually broke is the
/// previous one — include it too rather than handing back six lines of startup. Covers the common
/// "it broke, I restarted, then I reported it".
const FRESH_LOG_BYTES: u64 = 4096;

/// Env vars worth knowing about in a report. Anything whose value can carry a credential is
/// reported as `set` instead of printed.
const ENV_KEYS: &[&str] = &[
    "RUST_LOG",
    "LIMUSIC_PROXY",
    "LIMUSIC_DISABLED_CLIENTS",
    "LIMUSIC_FORCE_GPU",
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "__NV_DISABLE_EXPLICIT_SYNC",
    "GDK_BACKEND",
    "APPIMAGE",
];

/// Env vars printed as `set`, never by value.
const SECRET_ENV_KEYS: &[&str] = &["LIMUSIC_PROXY", "LIMUSIC_COOKIE", "LIMUSIC_VISITOR_DATA"];

/// Environment header + redacted log tail, capped at [`MAX_CHARS`].
pub fn report(app: &AppHandle, db: &Db) -> String {
    let mut out = String::new();
    out.push_str(
        "# Limusic diagnostics. Paste this into your bug report.\n\
         # Cookies, tokens, signed URLs, file paths and IP addresses have been removed.\n\n",
    );
    header(&mut out, app, db);

    let dir = app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir());
    let budget = MAX_CHARS.saturating_sub(out.chars().count() + 64);
    let log = redact(&log_text(&dir, budget));
    out.push_str("\n--- log ---\n");
    // Redaction can grow the text (a short token becomes `<redacted>`), so trim again, from the
    // front: the lines nearest the failure are the last ones.
    push_tail_chars(&mut out, &log, budget);
    out
}

/// Just the environment block, for prefilling the `system` field of the GitHub bug form. Short
/// enough to travel in a URL, unlike [`report`].
pub fn summary(app: &AppHandle, db: &Db) -> String {
    let mut out = String::new();
    header(&mut out, app, db);
    out
}

fn header(out: &mut String, app: &AppHandle, db: &Db) {
    let _ = writeln!(
        out,
        "Limusic {} ({} {}, {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        install_kind(app),
    );

    #[cfg(target_os = "linux")]
    {
        // ponytail: distro, kernel and session read straight out of /etc and /proc, no dependency.
        // Windows and macOS get the os/arch line above and nothing more; add `tauri-plugin-os` if
        // a report ever hinges on their build number.
        let distro = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines().find_map(|l| {
                    l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())
                })
            })
            .unwrap_or_else(|| "unknown distro".into());
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        let _ = writeln!(
            out,
            "System: {distro}, kernel {}, {} session on {}",
            kernel.trim(),
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "?".into()),
            std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "?".into()),
        );
        // SAFETY: three parameterless getters returning compile-time constants from libwebkit.
        let (major, minor, micro) = unsafe {
            (
                webkit2gtk::ffi::webkit_get_major_version(),
                webkit2gtk::ffi::webkit_get_minor_version(),
                webkit2gtk::ffi::webkit_get_micro_version(),
            )
        };
        let _ = writeln!(
            out,
            "WebKitGTK: {major}.{minor}.{micro}, NVIDIA: {}",
            yes_no(Path::new("/dev/nvidiactl").exists()),
        );
    }

    let disabled = db.get_setting("disabled_clients").unwrap_or_default();
    let _ = writeln!(
        out,
        "Signed in: {} | Proxy: {} | Quality: {} | Music videos: {} | Disabled clients: {}",
        yes_no(db.get_setting("session_cookie").is_some_and(|c| !c.is_empty())),
        yes_no(db.get_setting("proxy").is_some_and(|p| !p.is_empty())),
        db.get_setting("quality").unwrap_or_else(|| "HIGH".into()),
        yes_no(db.get_setting("music_videos").as_deref() == Some("true")),
        if disabled.is_empty() { "none".into() } else { disabled },
    );

    let env: Vec<String> = ENV_KEYS
        .iter()
        .filter_map(|k| {
            let raw = std::env::var(k).ok()?;
            Some(if SECRET_ENV_KEYS.contains(k) {
                format!("{k}=set")
            } else {
                format!("{k}={raw}")
            })
        })
        .collect();
    if !env.is_empty() {
        let _ = writeln!(out, "Env: {}", env.join(", "));
    }
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// How this copy was installed, which decides whether the in-app updater can do anything and how
/// the user should update. Mirrors [`crate::commands::can_self_update`]'s reasoning.
fn install_kind(app: &AppHandle) -> &'static str {
    if cfg!(debug_assertions) {
        return "dev build";
    }
    #[cfg(target_os = "linux")]
    {
        if app.env().appimage.is_some() {
            return "AppImage";
        }
        if std::env::current_exe().is_ok_and(|p| p.starts_with("/usr")) {
            return "system package";
        }
    }
    let _ = app;
    "installed"
}

/// The log to include: this run, preceded by the previous one when this run has barely started.
fn log_text(dir: &Path, budget: usize) -> String {
    let current = dir.join("limusic.log");
    let previous = dir.join("limusic.log.1");
    let mut text = String::new();
    if std::fs::metadata(&current).map(|m| m.len()).unwrap_or(0) < FRESH_LOG_BYTES {
        if let Some(t) = tail(&previous, budget / 2) {
            text.push_str("=== previous run (limusic.log.1) ===\n");
            text.push_str(&t);
            text.push_str("\n=== this run (limusic.log) ===\n");
        }
    }
    let left = budget.saturating_sub(text.len());
    text.push_str(&tail(&current, left).unwrap_or_else(|| "(no log file)".into()));
    text
}

/// Last `budget` bytes of a file, starting at a line boundary (which is also a UTF-8 boundary).
fn tail(path: &Path, budget: usize) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let cut = bytes.len().saturating_sub(budget);
    let start = if cut == 0 {
        0
    } else {
        bytes[cut..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |i| cut + i + 1)
    };
    Some(String::from_utf8_lossy(&bytes[start..]).into_owned())
}

/// Append at most `budget` characters of `text`, keeping the end.
fn push_tail_chars(out: &mut String, text: &str, budget: usize) {
    let total = text.chars().count();
    if total <= budget {
        out.push_str(text);
        return;
    }
    out.push_str("(older lines trimmed)\n");
    out.extend(text.chars().skip(total - budget));
}

/// Strip everything a public issue must not carry. Over-eager on purpose, in this order:
///
/// 1. the home directory, which carries the user's account name, becomes `~`;
/// 2. known secret-bearing keys lose their value, however short it is (a `SAPISID` is under 40
///    characters, so rule 4 alone would miss it);
/// 3. every URL loses its query string, which is where `pot`, `sig`, `ip` and `ei` live;
/// 4. any run of 40+ token characters is assumed to be a credential;
/// 5. IPv4 addresses go, except loopback (the video proxy's own address is worth keeping).
fn redact(text: &str) -> String {
    static SECRET_KV: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            // The leading class rules out a Rust module path: `app_lib::potoken: …` is a tracing
            // target, not a `token: value` pair, and eating the rest of that line loses the message.
            r#"(?im)(^|[^\w:])(cookie|authorization|sapisidhash|sapisid|hsid|ssid|apisid|psid|sid|__secure[a-z0-9_-]*|potoken|po_token|pot|signature|sig|api_sig|session_key|session_token|sk|token|visitor_?data|data_?sync_?id|api_?key)\s*[:=]\s*"?([^\s",;&)]+)"#,
        )
        .unwrap()
    });
    static URL_QUERY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(https?://[^\s"'<>]+?)\?[^\s"'<>]*"#).unwrap());
    static LONG_TOKEN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[A-Za-z0-9_\-]{40,}").unwrap());
    static IPV4: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").unwrap());

    let mut text = text.to_string();
    for var in ["HOME", "USERPROFILE"] {
        if let Ok(home) = std::env::var(var) {
            if home.len() > 1 {
                text = text.replace(&home, "~");
            }
        }
    }
    let text = SECRET_KV.replace_all(&text, "${1}${2}=<redacted>");
    let text = URL_QUERY.replace_all(&text, "${1}?<redacted>");
    let text = IPV4.replace_all(&text, |c: &regex::Captures| {
        let ip = &c[0];
        if ip.starts_with("127.") || ip.starts_with("0.") {
            ip.to_string()
        } else {
            "<ip>".to_string()
        }
    });
    LONG_TOKEN.replace_all(&text, "<redacted>").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_strips_what_a_public_issue_must_not_carry() {
        let log = concat!(
            "cookie: SAPISID=aB3dEfGhIjKlMnOpQr; __Secure-3PAPISID=zZ9\n",
            "GET https://rr5---sn-abc.googlevideo.com/videoplayback?pot=AbCdEf&ip=203.0.113.9&sig=xyz\n",
            "visitorData=CgtabcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123456789\n",
            "video_id=dQw4w9WgXcQ proxy at 127.0.0.1:8080\n",
            "INFO app_lib::potoken: session token still valid, skipping the bootstrap\n",
        );
        let out = redact(log);
        for leaked in ["aB3dEfGhIjKlMnOpQr", "AbCdEf", "203.0.113.9", "zZ9", "0123456789"] {
            assert!(!out.contains(leaked), "{leaked} survived redaction:\n{out}");
        }
        // Still readable: the loopback proxy, the video id and the host stay.
        assert!(out.contains("dQw4w9WgXcQ"), "{out}");
        assert!(out.contains("127.0.0.1"), "{out}");
        assert!(out.contains("googlevideo.com/videoplayback"), "{out}");
        // A tracing target that merely ends in a secret-ish word keeps its message.
        assert!(out.contains("app_lib::potoken: session token still valid"), "{out}");
    }

    #[test]
    fn tail_keeps_the_end_and_never_splits_a_line() {
        let dir = std::env::temp_dir().join("limusic-diag-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.log");
        std::fs::write(&path, "first line\nsecond line\nthird line\n").unwrap();
        let out = tail(&path, 20).unwrap();
        assert!(out.starts_with("second line") || out.starts_with("third line"), "{out}");
        assert!(out.ends_with("third line\n"), "{out}");
        assert_eq!(tail(&path, 10_000).unwrap(), "first line\nsecond line\nthird line\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn push_tail_chars_respects_the_budget() {
        let mut out = String::new();
        push_tail_chars(&mut out, "abcdefghij", 4);
        assert!(out.ends_with("ghij"), "{out}");
        let mut out = String::new();
        push_tail_chars(&mut out, "abc", 40);
        assert_eq!(out, "abc");
    }
}
