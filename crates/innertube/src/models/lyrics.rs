//! Lyrics browse parsing. The lyrics-tab browseId (from `next`, see `metadata::parse_next`)
//! is a plain `browse` call, but the response shape depends on the client:
//!
//! - **Mobile clients** (IOS_MUSIC) return line-synced lyrics: an element-renderer tree holding
//!   `timedLyricsData: [{ lyricLine, cueRange: { startTimeMilliseconds, … } }, …]` — the same
//!   real-time lyrics the YT Music app shows.
//! - **WEB_REMIX** returns plain text: `musicDescriptionShelfRenderer` with the lyrics in
//!   `description` and the attribution ("Source: Musixmatch") in `footer`.

use serde_json::Value;

use super::metadata::{find_all, runs_text};

/// One line of synced lyrics. `time_ms` is the line's start cue.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedLyricLine {
    pub time_ms: u64,
    pub text: String,
}

/// Un-timed lyrics text + the attribution footer, when present.
#[derive(Debug, Clone, PartialEq)]
pub struct PlainLyrics {
    pub text: String,
    pub footer: Option<String>,
}

/// Pull the synced lines out of a mobile-client lyrics browse response. Empty when the track has
/// no timed lyrics (the response degrades to a plain-text description or an error message).
pub fn parse_lyrics_timed(root: &Value) -> Vec<TimedLyricLine> {
    let mut out = Vec::new();
    for data in find_all(root, "timedLyricsData") {
        let Some(entries) = data.as_array() else { continue };
        for e in entries {
            // Credit/footer entries carry no cueRange — skip them, keep only real cue lines.
            let Some(text) = e.get("lyricLine").and_then(Value::as_str) else { continue };
            let Some(ms) = e
                .get("cueRange")
                .and_then(|c| c.get("startTimeMilliseconds"))
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            out.push(TimedLyricLine { time_ms: ms, text: text.to_owned() });
        }
        if !out.is_empty() {
            break; // first populated block is the lyrics; don't merge duplicates
        }
    }
    out
}

/// Pull plain lyrics out of a WEB_REMIX lyrics browse response. `None` when the track has none
/// (the shelf is absent or empty).
pub fn parse_lyrics_plain(root: &Value) -> Option<PlainLyrics> {
    let shelf = find_all(root, "musicDescriptionShelfRenderer").into_iter().next()?;
    let text = runs_text(shelf.get("description"))?;
    let footer = runs_text(shelf.get("footer"));
    Some(PlainLyrics { text, footer })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_timed_lines_and_skips_credits() {
        let root = json!({ "contents": { "elementRenderer": { "model": { "timedLyricsModel": {
            "lyricsData": { "timedLyricsData": [
                { "lyricLine": "First line", "cueRange": { "startTimeMilliseconds": "1200", "endTimeMilliseconds": "3400" } },
                { "lyricLine": "Second line", "cueRange": { "startTimeMilliseconds": "3400", "endTimeMilliseconds": "5000" } },
                { "lyricLine": "Source: Musixmatch" }
            ] }
        } } } } });
        let lines = parse_lyrics_timed(&root);
        assert_eq!(
            lines,
            vec![
                TimedLyricLine { time_ms: 1200, text: "First line".into() },
                TimedLyricLine { time_ms: 3400, text: "Second line".into() },
            ]
        );
    }

    #[test]
    fn timed_empty_when_absent() {
        assert!(parse_lyrics_timed(&json!({ "contents": {} })).is_empty());
    }

    #[test]
    fn parses_plain_shelf_with_footer() {
        let root = json!({ "contents": { "sectionListRenderer": { "contents": [
            { "musicDescriptionShelfRenderer": {
                "description": { "runs": [{ "text": "Verse one\nVerse two" }] },
                "footer": { "runs": [{ "text": "Source: Musixmatch" }] }
            } }
        ] } } });
        let p = parse_lyrics_plain(&root).unwrap();
        assert_eq!(p.text, "Verse one\nVerse two");
        assert_eq!(p.footer.as_deref(), Some("Source: Musixmatch"));
    }

    #[test]
    fn plain_none_when_absent() {
        assert!(parse_lyrics_plain(&json!({ "contents": {} })).is_none());
    }
}
