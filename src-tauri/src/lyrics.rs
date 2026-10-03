//! Lyrics fetching.
//!
//! Every provider sits in one list the user orders and switches off in Settings (`lyrics_providers`,
//! read by `provider_order`). A fresh install asks them in `PROVIDERS` order:
//!
//! 1. **Boidu** (`lyrics-api.boidu.dev`, the Better Lyrics API): Apple's TTML with word timings,
//!    only for songs its cache already holds.
//! 2. **LyricsPlus** (YouLyPlus' backend): Apple Music lyrics as KPOE JSON, word-timed, with
//!    Apple's own romanization. Checked against our length, since its search strays.
//! 3. **LRCLIB**: `/api/get` (exact), then `/api/search` (fuzzy, ±5s). Free, no key, best coverage.
//! 4. **YouTube Music**: `next(videoId)` → lyrics browseId → mobile-client browse
//!    (`timedLyricsData`), else the plain text with YouTube's attribution.
//! 5. **SimpMusic**: community lyrics filed by videoId, so timed to the exact upload playing.
//! 6. **Netease / QQ / Kugou**: synced LRC, plus translations from Netease. Search hits are matched
//!    on length (`best_by_duration`); these catalogues rank remixes next to originals.
//!
//! Synced beats plain: the first synced answer down the list wins, and only when none is synced
//! does the first plain one. Each provider returns its own best (synced if it has it), so the
//! plain pass costs no second request.
//!
//! Results are cached in SQLite (`lyrics_cache`): hits forever, "no lyrics" verdicts for 24h.
//! A run where every provider merely *errored* (offline) caches nothing, so lyrics come back
//! when the network does. A song's source picked by hand in the lyrics footer (`choose_source`),
//! or its timing nudged (`set_offset`), is `pinned` and outlives a change to the order.
//! Everything is best-effort: a lyrics failure is never a user error.

use std::time::Duration;

use innertube::NextResult;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// How long a cached "no lyrics found" verdict suppresses refetching.
const MISS_TTL_SECS: i64 = 24 * 3600;

const LRCLIB_ROOT: &str = "https://lrclib.net/api";

/// Every provider, in the order a fresh install asks them. The user's order lives in the
/// `lyrics_providers` setting; this is the default and the list of what exists.
pub const PROVIDERS: [&str; 8] =
    ["boidu", "lyricsplus", "lrclib", "youtube", "simpmusic", "netease", "qq", "kugou"];

/// The attribution a provider's lyrics carry (`Lyrics::source`), also its name in Settings.
pub fn provider_name(id: &str) -> &'static str {
    match id {
        "boidu" => "Boidu",
        "lyricsplus" => "LyricsPlus",
        "lrclib" => "LRCLIB",
        "youtube" => "YouTube Music",
        "simpmusic" => "SimpMusic",
        "netease" => "Netease Cloud Music",
        "qq" => "QQ Music",
        "kugou" => "Kugou",
        _ => "",
    }
}

/// The `lyrics_providers` setting as `(id, on)` in the user's order: ids top to bottom, `-id` for
/// one switched off. Whatever it doesn't name (all of them on a fresh install, or a provider a
/// later release adds) follows in the default order, switched on.
pub fn provider_order(setting: Option<&str>) -> Vec<(&'static str, bool)> {
    let mut out: Vec<(&'static str, bool)> = Vec::new();
    for tok in setting.unwrap_or_default().split(',').map(str::trim) {
        let (id, on) = tok.strip_prefix('-').map_or((tok, true), |id| (id, false));
        if let Some(&p) = PROVIDERS.iter().find(|p| **p == id) {
            if !out.iter().any(|(q, _)| *q == p) {
                out.push((p, on));
            }
        }
    }
    for p in PROVIDERS {
        if !out.iter().any(|(q, _)| *q == p) {
            out.push((p, true));
        }
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub on: bool,
}

/// The provider list for Settings and the source picker, in the user's order.
pub fn providers(state: &AppState) -> Vec<ProviderInfo> {
    provider_order(state.db.get_setting("lyrics_providers").as_deref())
        .into_iter()
        .map(|(id, on)| ProviderInfo { id, name: provider_name(id), on })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricWord {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// One display line. `time_ms` present ⇔ the line is synced (a plain-lyrics response has none).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LyricLine {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time_ms: Option<u64>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<LyricWord>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
    /// Latin-script reading of `text` (#202). Apple's arrives with the lyrics and is cached with
    /// them; the local engine's is filled on every answer by `romanize::fill`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub romanized: Option<String>,
    /// Apple's reading is word-timed like the line, so it can sweep with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub romanized_words: Option<Vec<LyricWord>>,
}

impl LyricLine {
    pub fn simple(time_ms: Option<u64>, text: String) -> Self {
        Self { time_ms, text, ..Default::default() }
    }
}

/// What the UI gets (and what `lyrics_cache` stores as JSON).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lyrics {
    /// Attribution shown in the panel footer ("LRCLIB", "Source: Musixmatch", …).
    pub source: String,
    /// The `PROVIDERS` id that answered, so the source picker can tick it.
    #[serde(default)]
    pub provider: String,
    pub synced: bool,
    #[serde(default)]
    pub instrumental: bool,
    pub lines: Vec<LyricLine>,
    /// Picked by hand, or its timing nudged: kept when the provider order changes.
    #[serde(default)]
    pub pinned: bool,
    /// Timing nudge for this song in ms, positive = lyrics later.
    #[serde(default)]
    pub offset_ms: i64,
}

pub struct LyricsRequest {
    pub video_id: String,
    pub title: String,
    pub artists: String,
    pub album: Option<String>,
    /// Track length in seconds (mpv's), tightens LRCLIB matching. `None`/0 when unknown yet.
    pub duration: Option<f64>,
}

/// Entry point for the `get_lyrics` command, romanized. `source: None` is the song's lyrics (cached,
/// or down the provider list). `Some(id)` asks that one provider alone and caches nothing: the
/// source picker's preview, which also works for a provider switched off in Settings.
pub async fn get_lyrics(
    state: &AppState,
    req: LyricsRequest,
    source: Option<String>,
) -> Result<Option<Lyrics>, String> {
    let lyrics = match source {
        Some(id) => preview(state, req, &id).await?,
        None => cached_or_fetched(state, req).await,
    };
    Ok(lyrics.map(|mut l| {
        crate::romanize::fill(&mut l);
        l
    }))
}

/// The source picker's choice for one song. `Some(id)`: that provider's lyrics, kept for the song
/// (a miss leaves what was there). `None`: forget the choice and let the list decide again.
pub async fn choose_source(
    state: &AppState,
    req: LyricsRequest,
    source: Option<String>,
) -> Option<Lyrics> {
    let video_id = req.video_id.clone();
    let mut lyrics = match source {
        Some(id) => {
            let mut l = preview(state, req, &id).await.ok().flatten()?;
            l.pinned = true;
            if let Ok(json) = serde_json::to_string(&l) {
                state.db.put_lyrics(&video_id, Some(&json), now_secs());
            }
            l
        }
        None => {
            state.db.delete_lyrics(&video_id);
            cached_or_fetched(state, req).await?
        }
    };
    crate::romanize::fill(&mut lyrics);
    Some(lyrics)
}

/// Nudge a song's timing. Pins what it applies to, so the nudge isn't lost to a reorder that would
/// fetch lyrics timed differently. Only a cached song can keep one; the view still applies it.
pub fn set_offset(state: &AppState, video_id: &str, offset_ms: i64) {
    let Some(Some(json)) = state.db.get_lyrics(video_id, now_secs(), MISS_TTL_SECS) else {
        return;
    };
    let Ok(mut l) = serde_json::from_str::<Lyrics>(&json) else { return };
    l.offset_ms = offset_ms;
    l.pinned = true;
    if let Ok(json) = serde_json::to_string(&l) {
        state.db.put_lyrics(video_id, Some(&json), now_secs());
    }
}

async fn cached_or_fetched(state: &AppState, req: LyricsRequest) -> Option<Lyrics> {
    let now = now_secs();
    let video_id = req.video_id.clone();
    if let Some(cached) = state.db.get_lyrics(&video_id, now, MISS_TTL_SECS) {
        return cached.and_then(|json| serde_json::from_str(&json).ok());
    }
    let (lyrics, cacheable) = fetch(state, req).await;
    if cacheable {
        let json = lyrics.as_ref().and_then(|l| serde_json::to_string(l).ok());
        state.db.put_lyrics(&video_id, json.as_deref(), now);
    }
    lyrics
}

/// One provider on its own, uncached. `next()` only when that provider needs it (YouTube's browseId)
/// or the length is unknown: the picker previews every provider at once, and they'd all ask.
async fn preview(
    state: &AppState,
    mut req: LyricsRequest,
    id: &str,
) -> Result<Option<Lyrics>, String> {
    let next = if id == "youtube" || req.duration.is_none() {
        resolve(state, &mut req).await
    } else {
        Ok(None)
    };
    ask(id, state, &req, &next).await
}

/// `next()` for the track, filling in what the request lacks. It carries the lyrics browseId AND,
/// via its seed item, the exact length of the cut this videoId plays. The queue item often has no
/// duration (card plays; stream-cache replays skip /player entirely), and duration is what keeps
/// the fuzzy providers from matching a differently-timed cut, so resolve it here where it's always
/// available. Same for the album: a play from search has none, and Boidu's cache and LRCLIB's exact
/// match both key on it.
///
/// `Ok(None)` for a local file: its duration came off the file itself, and YouTube has nothing for
/// it. `Err` when `next()` failed, which is not the same as YouTube having no lyrics.
async fn resolve(state: &AppState, req: &mut LyricsRequest) -> Result<Option<NextResult>, String> {
    if crate::local::is_local_song(&req.video_id) {
        return Ok(None);
    }
    let client = state.clients.get(innertube::METADATA_CLIENT).ok_or("no metadata client")?;
    let next = state.it.next(client, Some(&req.video_id), None).await.map_err(|e| e.to_string())?;
    let seed = next.items.iter().find(|i| i.video_id == req.video_id);
    if req.duration.is_none() {
        req.duration = seed.and_then(|i| duration_str_secs(i.duration.as_deref()?));
    }
    if req.album.is_none() {
        req.album = seed.and_then(|i| i.album.clone());
    }
    Ok(Some(next))
}

/// Ask one provider. Its lyrics come back tagged with its id. `Err` is transport trouble (or an
/// answer we couldn't read), never "no lyrics", so it can't cache a miss.
async fn ask(
    id: &str,
    state: &AppState,
    req: &LyricsRequest,
    next: &Result<Option<NextResult>, String>,
) -> Result<Option<Lyrics>, String> {
    let e = |e: reqwest::Error| e.to_string();
    let hit = match id {
        "boidu" => boidu_get(req).await.map_err(e),
        "lyricsplus" => lyricsplus_get(req).await.map_err(e),
        "lrclib" => lrclib(req).await.map_err(e),
        "youtube" => youtube_get(state, next).await,
        "simpmusic" => simpmusic_get(req).await.map_err(e),
        "netease" => netease_get(req).await.map_err(e),
        "qq" => qqmusic_get(req).await.map_err(e),
        "kugou" => kugou_get(req).await.map_err(e),
        _ => Ok(None),
    }?;
    Ok(hit.map(|l| Lyrics { provider: id.to_owned(), ..l }))
}

/// Down the user's list. Second value: cache the outcome. A hit only when the track's length was
/// known (the fuzzy providers land on wrong *cuts* without it, lyrics seconds off the audio) or the
/// provider matched the video itself; a miss only when some provider answered rather than merely
/// erroring (offline must not poison the cache with a 24h "no lyrics").
async fn fetch(state: &AppState, mut req: LyricsRequest) -> (Option<Lyrics>, bool) {
    let next = resolve(state, &mut req).await;
    if let Err(e) = &next {
        tracing::debug!(error = %e, "lyrics: next() failed");
    }
    let req = &req;
    let cacheable = |id: &str| req.duration.is_some() || matches!(id, "youtube" | "simpmusic");

    let mut definitive = false;
    let mut plain: Option<Lyrics> = None;
    for (id, on) in provider_order(state.db.get_setting("lyrics_providers").as_deref()) {
        if !on {
            continue;
        }
        match ask(id, state, req, &next).await {
            Ok(Some(l)) if l.synced || l.instrumental => return (Some(l), cacheable(id)),
            Ok(hit) => {
                definitive = true;
                plain = plain.or(hit);
            }
            Err(e) => tracing::debug!(provider = id, error = %e, "lyrics: provider failed"),
        }
    }
    match plain {
        Some(l) => {
            let cache = cacheable(&l.provider);
            (Some(l), cache)
        }
        None => (None, definitive),
    }
}

/// YouTube Music's own lyrics: the timed ones its mobile app shows, else the plain text under
/// YouTube's attribution ("Source: Musixmatch"). Region-licensed, so often absent.
async fn youtube_get(
    state: &AppState,
    next: &Result<Option<NextResult>, String>,
) -> Result<Option<Lyrics>, String> {
    // A next() answer with no lyrics tab IS "YouTube has no lyrics".
    let bid = match next {
        Ok(n) => n.as_ref().and_then(|n| n.lyrics_browse_id.clone()),
        Err(e) => return Err(e.clone()),
    };
    let Some(bid) = bid else { return Ok(None) };
    if let Some(client) = state.clients.get(innertube::LYRICS_TIMED_CLIENT) {
        match state.it.lyrics_timed(client, &bid).await {
            Ok(lines) if !lines.is_empty() => {
                return Ok(Some(Lyrics {
                    source: "YouTube Music".into(),
                    synced: true,
                    lines: lines
                        .into_iter()
                        .map(|l| LyricLine::simple(Some(l.time_ms), l.text))
                        .collect(),
                    ..Default::default()
                }));
            }
            Ok(_) => {}
            Err(e) => tracing::debug!(error = %e, "lyrics: timed browse failed"),
        }
    }
    let client = state.clients.get(innertube::METADATA_CLIENT).ok_or("no metadata client")?;
    let plain = state.it.lyrics_plain(client, &bid).await.map_err(|e| e.to_string())?;
    Ok(plain.and_then(|p| {
        let source = p.footer.unwrap_or_else(|| "YouTube Music".into());
        plain_from_text(Some(&p.text), &source)
    }))
}

// --- LRCLIB (https://lrclib.net/docs) -------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrclibTrack {
    #[serde(default)]
    instrumental: bool,
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    synced_lyrics: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

impl LrclibTrack {
    fn has_synced(&self) -> bool {
        self.synced_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
}

/// LRCLIB asks integrations to identify themselves via User-Agent.
const LRCLIB_UA: &str =
    concat!("Limusic v", env!("CARGO_PKG_VERSION"), " (https://github.com/SimoHypers/limusic)");

/// A GET to LRCLIB, carrying the two things this API wants from us: who we are, and a bound on how
/// long we will wait. Both used to be baked into a client of our own.
fn get(url: String) -> reqwest::RequestBuilder {
    crate::http::client().get(url).header("User-Agent", LRCLIB_UA).timeout(Duration::from_secs(15))
}

/// LRCLIB as one provider: the exact match, then the fuzzy search for synced lyrics, then whichever
/// plain text either found. Transport trouble only when both requests failed.
async fn lrclib(req: &LyricsRequest) -> Result<Option<Lyrics>, reqwest::Error> {
    let exact = lrclib_get(req).await;
    if let Ok(Some(l)) = exact.as_ref().map(|t| t.as_ref().and_then(lrclib_to_lyrics)) {
        if l.synced || l.instrumental {
            return Ok(Some(l));
        }
    }
    let searched = lrclib_search(req).await;
    if let Ok(Some(l)) = searched.as_ref().map(|t| t.as_ref().and_then(lrclib_to_lyrics)) {
        if l.synced {
            return Ok(Some(l));
        }
    }
    let plain = |r: &Result<Option<LrclibTrack>, reqwest::Error>| match r {
        Ok(Some(t)) => lrclib_to_lyrics(t),
        _ => None,
    };
    match (exact, searched) {
        (Err(e), Err(_)) => Err(e),
        (exact, searched) => Ok(plain(&exact).or_else(|| plain(&searched))),
    }
}

/// `/api/get`: exact signature match. `Ok(None)` = definitive "not in LRCLIB" (404);
/// `Err` = transport trouble (don't cache a negative off it).
async fn lrclib_get(req: &LyricsRequest) -> Result<Option<LrclibTrack>, reqwest::Error> {
    let mut q: Vec<(&str, String)> =
        vec![("track_name", req.title.clone()), ("artist_name", req.artists.clone())];
    if let Some(album) = &req.album {
        q.push(("album_name", album.clone()));
    }
    if let Some(d) = req.duration.filter(|d| *d > 0.0) {
        q.push(("duration", format!("{}", d.round() as i64)));
    }
    let resp = get(format!("{LRCLIB_ROOT}/get")).query(&q).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(resp.error_for_status()?.json().await?))
}

/// `/api/search`: fuzzy fallback. The title and artist fields first, then free text when those find
/// nothing synced (#329): the tidied title with the artist, then without. Free text wants every
/// word to match, so one stray word sinks it, and YouTube supplies plenty. A lyric video arrives as
/// "Bad Apple／ Lizz Robinett (English Cover) | Lyrics/Lyric Video [English]" with the uploading
/// channel ("Lyrics Radio") as the artist; only the bare "Bad Apple／ Lizz Robinett" finds it. An
/// artist in native script (容祖兒) also misses the synced copies filed under the romanized name.
/// Free text only with a known length: it is the looser match, and the ±5s window is what keeps it
/// on the right song.
async fn lrclib_search(req: &LyricsRequest) -> Result<Option<LrclibTrack>, reqwest::Error> {
    let fields = [("track_name", req.title.as_str()), ("artist_name", req.artists.as_str())];
    let mut hit = lrclib_search_by(req, &fields).await?;
    let title = search_title(&req.title);
    if hit.as_ref().is_some_and(LrclibTrack::has_synced)
        || req.duration.is_none_or(|d| d <= 0.0)
        || title.is_empty()
    {
        return Ok(hit);
    }
    for q in [format!("{title} {}", req.artists), title] {
        match lrclib_search_by(req, &[("q", q.as_str())]).await {
            Ok(Some(t)) if t.has_synced() => return Ok(Some(t)),
            Ok(free) => hit = hit.or(free),
            Err(_) => {}
        }
    }
    Ok(hit)
}

/// A YouTube title without what no lyrics catalogue files a song under: everything after a `|`
/// and every bracketed aside, "(Official Video)", "[English]", "【MV】" alike.
fn search_title(title: &str) -> String {
    let title = title.split('|').next().unwrap_or_default();
    let mut depth = 0u32;
    let mut out = String::new();
    for c in title.chars() {
        match c {
            '(' | '[' | '【' | '（' => depth += 1,
            ')' | ']' | '】' | '）' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One `/api/search` call. Prefers a synced candidate whose duration is within ±5s of ours (when
/// known); returns the best or `Ok(None)`.
async fn lrclib_search_by(
    req: &LyricsRequest,
    q: &[(&str, &str)],
) -> Result<Option<LrclibTrack>, reqwest::Error> {
    let list: Vec<LrclibTrack> = get(format!("{LRCLIB_ROOT}/search"))
        .query(q)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let ours = req.duration.filter(|d| *d > 0.0);
    // Distance from our track's length; unknown-length candidates rank last but aren't excluded.
    let dist = |t: &LrclibTrack| match (ours, t.duration) {
        (Some(a), Some(b)) => (a - b).abs(),
        _ => f64::INFINITY,
    };
    let close = |t: &LrclibTrack| ours.is_none() || dist(t) <= 5.0;
    // Prefer the synced candidate whose duration is CLOSEST to ours — LRCLIB carries multiple
    // cuts of popular tracks, and a 4s-different cut plays lyrics 4s off the audio.
    let mut best_synced: Option<(f64, LrclibTrack)> = None;
    let mut best_plain: Option<LrclibTrack> = None;
    for t in list {
        if !close(&t) {
            continue;
        }
        if t.has_synced() {
            let d = dist(&t);
            if best_synced.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best_synced = Some((d, t));
            }
        } else if best_plain.is_none() {
            best_plain = Some(t);
        }
    }
    Ok(best_synced.map(|(_, t)| t).or(best_plain))
}

/// Best `Lyrics` an LRCLIB track yields: instrumental > synced > plain > nothing.
fn lrclib_to_lyrics(t: &LrclibTrack) -> Option<Lyrics> {
    if t.instrumental {
        return Some(Lyrics { source: "LRCLIB".into(), instrumental: true, ..Default::default() });
    }
    if let Some(lrc) = t.synced_lyrics.as_deref().filter(|s| !s.trim().is_empty()) {
        let lines = parse_lrc(lrc);
        if !lines.is_empty() {
            return Some(Lyrics {
                source: "LRCLIB".into(),
                synced: true,
                lines,
                ..Default::default()
            });
        }
    }
    plain_from_text(t.plain_lyrics.as_deref(), "LRCLIB")
}

/// Plain text → un-timed lines (blank lines kept as stanza breaks).
fn plain_from_text(text: Option<&str>, source: &str) -> Option<Lyrics> {
    let text = text?.trim();
    if text.is_empty() {
        return None;
    }
    Some(Lyrics {
        source: source.to_owned(),
        lines: text.lines().map(|l| LyricLine::simple(None, l.trim_end().to_owned())).collect(),
        ..Default::default()
    })
}

/// A provider's parsed lines as a result, or `None` when there was nothing to show.
///
/// `synced` is derived from the lines rather than asserted by the caller. TTML without `begin`
/// attributes, and JSON items carrying text but no time, both parse to real lines with no cue.
/// Declaring those synced puts the UI in its synced view, where no line ever highlights (none has
/// a cue to pass) and clicking one to seek does nothing: lyrics that look broken, rather than
/// lyrics that read as plain text.
fn from_parsed(source: &str, lines: Vec<LyricLine>) -> Option<Lyrics> {
    if lines.is_empty() {
        return None;
    }
    Some(Lyrics {
        source: source.to_owned(),
        // Any cue at all: an LRC with untimed credit or stanza lines is still a synced lyric.
        synced: lines.iter().any(|l| l.time_ms.is_some()),
        lines,
        ..Default::default()
    })
}

// --- LRC parsing ----------------------------------------------------------------------------

/// Parse LRC text (`[mm:ss.xx] line`) into sorted lines. Handles multiple timestamps per line
/// (`[t1][t2]text` — the line repeats at both cues) and skips metadata tags (`[ar:…]`).
/// Timestamped empty lines are kept: they're instrumental gaps the UI can show as such.
fn parse_lrc(lrc: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    for raw in lrc.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some(end) = after.find(']') else { break };
            match parse_lrc_time(&after[..end]) {
                Some(ms) => {
                    times.push(ms);
                    rest = after[end + 1..].trim_start();
                }
                // Not a timestamp: a metadata tag ([ar:…] — no times yet, line skipped) or
                // bracketed lyric text ("[Chorus]" — keep it as the line's text).
                None => break,
            }
        }
        for &ms in &times {
            out.push(LyricLine::simple(Some(ms), rest.to_owned()));
        }
    }
    out.sort_by_key(|l| l.time_ms);
    out
}

/// `mm:ss`, `mm:ss.xx`, or `mm:ss.xxx` → milliseconds.
fn parse_lrc_time(tag: &str) -> Option<u64> {
    let (m, rest) = tag.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let (s, frac) = match rest.split_once('.') {
        Some((s, f)) => (s, Some(f)),
        None => (rest, None),
    };
    let s: u64 = s.trim().parse().ok()?;
    let ms = match frac {
        Some(f) => {
            let digits: String = f.chars().filter(char::is_ascii_digit).take(3).collect();
            let val: u64 = digits.parse().ok()?;
            match digits.len() {
                1 => val * 100,
                2 => val * 10,
                _ => val,
            }
        }
        None => 0,
    };
    Some((m * 60 + s) * 1000 + ms)
}

/// `"3:21"` / `"1:02:03"` → seconds.
pub(crate) fn duration_str_secs(s: &str) -> Option<f64> {
    let mut total: u64 = 0;
    for part in s.split(':') {
        total = total * 60 + part.trim().parse::<u64>().ok()?;
    }
    (total > 0).then_some(total as f64)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// --- Other providers ------------------------------------------------------------------------

/// A provider's JSON answer, bounded at 8s. Transport trouble and an unreadable body are both
/// `Err`: neither is the provider saying it has no lyrics.
async fn get_json(rb: reqwest::RequestBuilder) -> Result<serde_json::Value, reqwest::Error> {
    rb.timeout(Duration::from_secs(8)).send().await?.error_for_status()?.json().await
}

/// Boidu provider (boidu.dev / Better Lyrics API)
///
/// Without an API key (none are being issued) Boidu answers only from its cache and 401s a miss.
/// The cache key is title, artist, album and duration: with no duration it never hits, and a track
/// may be cached with its album or without one ("Spring Day" only with, NewJeans' "Ditto" only
/// without). So the album form first, then the bare one, the second only after a 401.
async fn boidu_get(req: &LyricsRequest) -> Result<Option<Lyrics>, reqwest::Error> {
    let Some(d) = req.duration.filter(|d| *d > 0.0) else {
        return Ok(None);
    };
    let base: Vec<(&str, String)> = vec![
        ("s", req.title.clone()),
        ("a", req.artists.clone()),
        ("d", format!("{}", d.round() as i64)),
    ];
    let forms = if req.album.is_some() { 2 } else { 1 };

    let url = "https://lyrics-api.boidu.dev/getLyrics";
    tracing::debug!(title = %req.title, artist = %req.artists, "lyrics: querying Boidu provider");
    let mut resp = serde_json::Value::Null;
    for album in [req.album.as_ref(), None].into_iter().take(forms) {
        let mut q = base.clone();
        if let Some(al) = album {
            q.push(("al", al.clone()));
        }
        let r = crate::http::client()
            .get(url)
            .query(&q)
            .header("User-Agent", LRCLIB_UA)
            .timeout(Duration::from_secs(8))
            .send()
            .await?;
        if r.status() == reqwest::StatusCode::UNAUTHORIZED {
            continue; // not cached under this key
        }
        resp = r.error_for_status()?.json().await?;
        break;
    }

    let lrc_str = resp
        .get("ttml")
        .or_else(|| resp.get("syncedLyrics"))
        .or_else(|| resp.get("lyrics"))
        .or_else(|| resp.get("lrc"))
        .and_then(|v| {
            if let Some(s) = v.as_str() {
                if !s.trim().is_empty() {
                    return Some(s.to_string());
                }
            }
            if v.is_array() {
                return serde_json::to_string(v).ok();
            }
            None
        });

    let hit = lrc_str.and_then(|lrc| from_parsed("Boidu", parse_lrc_or_ttml(&lrc)));
    match &hit {
        Some(l) => tracing::debug!(count = l.lines.len(), synced = l.synced, "lyrics: Boidu hit"),
        None => tracing::debug!("lyrics: Boidu returned no lines"),
    }
    Ok(hit)
}

/// LyricsPlus, the backend of YouLyPlus (#46): Apple Music's word-timed lyrics as KPOE JSON, for
/// far more songs than Boidu's cache holds.
// ponytail: one mirror. Of the six listed in #46 only this one answered on 2026-09-27 (the rest
// were dead, disabled or rate-limited); when it goes, point this at the next live one.
const LYRICSPLUS_URL: &str = "https://lyricsplus.binimum.org/v2/lyrics/get";

#[derive(Debug, Deserialize)]
struct Kpoe {
    /// "Word" when the syllables carry their own timings, "Line" when only the lines do.
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    metadata: KpoeMeta,
    #[serde(default)]
    lyrics: Vec<KpoeLine>,
}

#[derive(Debug, Default, Deserialize)]
struct KpoeMeta {
    /// "3:53.713": the length of the song it matched.
    #[serde(rename = "totalDuration")]
    total_duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KpoeLine {
    time: f64,
    duration: f64,
    #[serde(default)]
    text: String,
    #[serde(default)]
    syllabus: Vec<KpoeSyllable>,
    /// Apple's reading, timed like the line (`{"lang": "ko-Latn", "text", "syllabus"}`).
    transliteration: Option<KpoeReading>,
}

#[derive(Debug, Deserialize)]
struct KpoeReading {
    #[serde(default)]
    text: String,
    #[serde(default)]
    syllabus: Vec<KpoeSyllable>,
}

#[derive(Debug, Deserialize)]
struct KpoeSyllable {
    time: f64,
    duration: f64,
    #[serde(default)]
    text: String,
}

async fn lyricsplus_get(req: &LyricsRequest) -> Result<Option<Lyrics>, reqwest::Error> {
    let title = search_title(&req.title);
    let mut q = vec![
        ("title", if title.is_empty() { req.title.clone() } else { title }),
        ("artist", req.artists.clone()),
    ];
    if let Some(album) = &req.album {
        q.push(("album", album.clone()));
    }
    if let Some(d) = req.duration.filter(|d| *d > 0.0) {
        q.push(("duration", format!("{}", d.round() as i64)));
    }
    let resp = crate::http::client()
        .get(LYRICSPLUS_URL)
        .query(&q)
        .header("User-Agent", LRCLIB_UA)
        .timeout(Duration::from_secs(8))
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(kpoe_to_lyrics(resp.error_for_status()?.json().await?, req.duration))
}

/// KPOE into our lines, or `None` when it matched some other song. Its search strays: 我的驕傲 by
/// 容祖兒 (188s) came back as 跩跩, a 146s track off the album of that name. So the length has to
/// agree, within the tolerance the other catalogues get; without a length on either side it stands.
fn kpoe_to_lyrics(k: Kpoe, ours: Option<f64>) -> Option<Lyrics> {
    let theirs = k.metadata.total_duration.as_deref().and_then(parse_ttml_time);
    if let (Some(a), Some(b)) = (ours.filter(|d| *d > 0.0), theirs) {
        if (a - b as f64 / 1000.0).abs() > MATCH_TOLERANCE_SECS {
            return None;
        }
    }
    let word_timed = k.kind == "Word";
    let words = |s: &[KpoeSyllable]| {
        (word_timed && !s.is_empty()).then(|| {
            s.iter()
                .map(|w| LyricWord {
                    text: w.text.clone(),
                    start_ms: w.time as u64,
                    end_ms: (w.time + w.duration) as u64,
                })
                .collect()
        })
    };
    let lines = k
        .lyrics
        .into_iter()
        .map(|l| {
            // Apple repeats a Latin line verbatim as its own reading.
            let reading = l
                .transliteration
                .filter(|r| !r.text.trim().is_empty() && r.text.trim() != l.text.trim());
            LyricLine {
                time_ms: Some(l.time as u64),
                end_time_ms: Some((l.time + l.duration) as u64),
                words: words(&l.syllabus),
                romanized_words: reading.as_ref().and_then(|r| words(&r.syllabus)),
                romanized: reading.map(|r| r.text),
                text: l.text,
                ..Default::default()
            }
        })
        .collect();
    from_parsed("LyricsPlus", lines)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimpMusicTrack {
    #[serde(default)]
    plain_lyric: Option<String>,
    #[serde(default)]
    synced_lyrics: Option<String>,
    /// Enhanced LRC, word-timed. Empty on most.
    #[serde(default)]
    rich_sync_lyrics: Option<String>,
    #[serde(default)]
    vote: i64,
}

/// SimpMusic's community library, filed by YouTube videoId (#46). The one provider matched on the
/// upload itself rather than on title and length, so its timings are for the cut actually playing,
/// a music video's intro included. Community-written, hence below the curated ones by default.
async fn simpmusic_get(req: &LyricsRequest) -> Result<Option<Lyrics>, reqwest::Error> {
    if crate::local::is_local_song(&req.video_id) {
        return Ok(None);
    }
    #[derive(Deserialize)]
    struct Resp {
        data: Option<Vec<SimpMusicTrack>>,
    }
    let resp = crate::http::client()
        .get(format!("https://api-lyrics.simpmusic.org/v1/{}", req.video_id))
        .header("User-Agent", LRCLIB_UA)
        .timeout(Duration::from_secs(8))
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let mut tracks = resp.error_for_status()?.json::<Resp>().await?.data.unwrap_or_default();
    // Several submissions can compete for one video: the best voted first.
    tracks.sort_by_key(|t| std::cmp::Reverse(t.vote));
    let synced = tracks.iter().find_map(|t| {
        [&t.rich_sync_lyrics, &t.synced_lyrics]
            .into_iter()
            .flatten()
            .find_map(|lrc| from_parsed("SimpMusic", parse_elrc(lrc)).filter(|l| l.synced))
    });
    Ok(synced.or_else(|| {
        tracks.iter().find_map(|t| plain_from_text(t.plain_lyric.as_deref(), "SimpMusic"))
    }))
}

/// How far a search hit's length may sit from the track we're actually playing. Same tolerance the
/// LRCLIB search above uses, for the same reason.
const MATCH_TOLERANCE_SECS: f64 = 5.0;

/// Pick the search hit closest in length to what we're playing, rejecting anything further off than
/// `MATCH_TOLERANCE_SECS`.
///
/// The three providers below rank remixes, live cuts and radio edits right next to the original
/// (Kugou's top hit for "Shape of You" is a 263s edit of a 233s song, and Netease ranks a 231s
/// remix second), so taking whatever came back first plays lyrics seconds out of step with the
/// audio. Closest-match rather than first-within-tolerance matters: the remix is often inside the
/// window too, and only the distance separates it from the real cut.
///
/// With no length on our side there is nothing to check, so the first hit stands. A candidate whose
/// own length is missing ranks last but is not dropped — if a provider renames the field we want
/// degraded matching, not a provider that silently returns nothing.
fn best_by_duration<T>(
    ours: Option<f64>,
    cands: &[T],
    secs: impl Fn(&T) -> Option<f64>,
) -> Option<&T> {
    let Some(ours) = ours.filter(|d| *d > 0.0) else {
        return cands.first();
    };
    cands
        .iter()
        .map(|c| (secs(c).map_or(f64::INFINITY, |d| (d - ours).abs()), c))
        .filter(|(d, _)| *d <= MATCH_TOLERANCE_SECS || d.is_infinite())
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| c)
}

/// Netease Cloud Music provider (supports LRC, word timestamps, & translations)
async fn netease_get(req: &LyricsRequest) -> Result<Option<Lyrics>, reqwest::Error> {
    let query = format!("{} {}", req.title, req.artists);
    // POST `/api/search/get`, not GET `/api/search/get/web`: the latter now answers with an
    // encrypted hex blob instead of JSON, which parsed to "no hit" and left this provider dead.
    let resp = get_json(
        crate::http::client()
            .post("https://music.163.com/api/search/get")
            .form(&[("s", query.as_str()), ("type", "1"), ("limit", "5"), ("offset", "0")])
            .header("User-Agent", LRCLIB_UA)
            .header("Referer", "https://music.163.com/"),
    )
    .await?;

    let songs = resp
        .pointer("/result/songs")
        .and_then(|v| v.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default();
    // Netease reports track length in milliseconds.
    let hit =
        best_by_duration(req.duration, songs, |s| Some(s.get("duration")?.as_f64()? / 1000.0));
    let Some(id) = hit.and_then(|s| s.get("id")).and_then(|v| v.as_u64()) else {
        return Ok(None);
    };

    let l_resp = get_json(
        crate::http::client()
            .get(format!("https://music.163.com/api/song/lyric?id={id}&lv=1&kv=1&tv=-1"))
            .header("User-Agent", LRCLIB_UA)
            .header("Referer", "https://music.163.com/"),
    )
    .await?;

    let lrc_str = l_resp.pointer("/lrc/lyric").and_then(|v| v.as_str());
    let klyric_str = l_resp.pointer("/klyric/lyric").and_then(|v| v.as_str());
    let tlyric_str = l_resp.pointer("/tlyric/lyric").and_then(|v| v.as_str());

    if let Some(lrc) = lrc_str {
        let mut lines = parse_lrc_or_ttml(lrc);
        if let Some(klrc) = klyric_str {
            let klines = parse_lrc_or_ttml(klrc);
            lines = lrc_mux(lines, klines);
        }
        if let Some(tlrc) = tlyric_str {
            let tlines = parse_lrc(tlrc);
            for l in &mut lines {
                if let Some(t_time) = l.time_ms {
                    if let Some(tl) = tlines.iter().find(|t| t.time_ms == Some(t_time)) {
                        if !tl.text.trim().is_empty() {
                            l.translation = Some(tl.text.clone());
                        }
                    }
                }
            }
        }
        return Ok(from_parsed("Netease Cloud Music", lines));
    }
    Ok(None)
}

/// QQ Music provider. `client_search_cp`, the search this used to call, answers 500 to everything
/// (seen 2026-09-27), and the desktop search now wants a signed request. The search box's
/// suggestions still answer, but carry no lengths, so one batch song-detail call fetches those for
/// the duration match.
async fn qqmusic_get(req: &LyricsRequest) -> Result<Option<Lyrics>, reqwest::Error> {
    let qq = |url: String| {
        crate::http::client()
            .get(url)
            .header("User-Agent", LRCLIB_UA)
            .header("Referer", "https://y.qq.com/")
    };
    let query = format!("{} {}", req.title, req.artists);
    let found = get_json(qq(format!(
        "https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg?format=json&key={}",
        urlencoding::encode(&query)
    )))
    .await?;
    let mids: Vec<&str> = found
        .pointer("/data/song/itemlist")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("mid")?.as_str())
        .collect();
    if mids.is_empty() {
        return Ok(None);
    }
    let detail = get_json(qq(format!(
        "https://c.y.qq.com/v8/fcg-bin/fcg_play_single_song.fcg?format=json&songmid={}",
        mids.join(",")
    )))
    .await?;
    let songs = detail.get("data").and_then(|v| v.as_array()).map(|v| v.as_slice());
    // QQ reports track length in whole seconds, as `interval`.
    let hit =
        best_by_duration(req.duration, songs.unwrap_or_default(), |s| s.get("interval")?.as_f64());
    let Some(mid) = hit.and_then(|s| s.get("mid")).and_then(|v| v.as_str()) else {
        return Ok(None);
    };

    let l_resp = get_json(qq(format!(
        "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid={mid}&format=json&nobase64=1"
    )))
    .await?;

    let mut lyric_raw = l_resp.get("lyric").and_then(|v| v.as_str()).unwrap_or("");
    let decoded;
    if let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, lyric_raw)
    {
        if let Ok(s) = String::from_utf8(bytes) {
            decoded = s;
            lyric_raw = &decoded;
        }
    }
    Ok(from_parsed("QQ Music", parse_lrc_or_ttml(lyric_raw)))
}

/// Kugou provider
async fn kugou_get(req: &LyricsRequest) -> Result<Option<Lyrics>, reqwest::Error> {
    let query = format!("{} {}", req.title, req.artists);
    let resp = get_json(crate::http::client().get(format!(
        "https://songsearch.kugou.com/song_search_v2?keyword={}&page=1&pagesize=5",
        urlencoding::encode(&query)
    )))
    .await?;

    let songs = resp
        .pointer("/data/lists")
        .and_then(|v| v.as_array())
        .map(|v| v.as_slice())
        .unwrap_or_default();
    // Kugou reports track length in whole seconds, as `Duration`.
    let hit = best_by_duration(req.duration, songs, |s| s.get("Duration")?.as_f64());
    let Some(h) = hit.and_then(|s| s.get("FileHash")).and_then(|v| v.as_str()) else {
        return Ok(None);
    };

    // `hash=`, not `h=`: the latter is not a parameter this endpoint knows, so it answered
    // "paramter_error: empty hash and keyword" for every track and the provider never returned
    // anything at all.
    let krc_resp = get_json(
        crate::http::client()
            .get(format!("https://krcs.kugou.com/search?ver=1&man=yes&client=mobi&hash={h}")),
    )
    .await?;

    let id = krc_resp.pointer("/candidates/0/id").and_then(|v| v.as_str());
    let accesskey = krc_resp.pointer("/candidates/0/accesskey").and_then(|v| v.as_str());
    let (Some(id_str), Some(key_str)) = (id, accesskey) else {
        return Ok(None);
    };

    let dl_resp = get_json(crate::http::client().get(format!(
        "https://lyrics.kugou.com/download?ver=1&client=pc&id={id_str}&accesskey={key_str}&fmt=lrc"
    )))
    .await?;

    let b64_content = dl_resp.get("content").and_then(|v| v.as_str()).unwrap_or("");
    if let Ok(bytes) =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64_content)
    {
        if let Ok(lrc_str) = String::from_utf8(bytes) {
            return Ok(from_parsed("Kugou", parse_lrc_or_ttml(&lrc_str)));
        }
    }
    Ok(None)
}

// --- TTML / AAML / eLRC Parsing & LRCMux ----------------------------------------------------

fn parse_time_val(v: &serde_json::Value) -> Option<u64> {
    if let Some(f) = v.as_f64() {
        if f < 500.0 {
            Some((f * 1000.0) as u64)
        } else {
            Some(f as u64)
        }
    } else if let Some(u) = v.as_u64() {
        if u < 500 {
            Some(u * 1000)
        } else {
            Some(u)
        }
    } else if let Some(s) = v.as_str() {
        if let Ok(f) = s.parse::<f64>() {
            if f < 500.0 {
                Some((f * 1000.0) as u64)
            } else {
                Some(f as u64)
            }
        } else {
            None
        }
    } else {
        None
    }
}

fn parse_lrc_or_ttml(text: &str) -> Vec<LyricLine> {
    let trimmed = text.trim();

    // 1. JSON Array / KPOE / LyricsPlus format
    if (trimmed.starts_with('[') || trimmed.starts_with('{'))
        && (trimmed.contains("\"text\"")
            || trimmed.contains("\"time\"")
            || trimmed.contains("\"words\"")
            || trimmed.contains("\"start\"")
            || trimmed.contains("\"startTime\""))
    {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let mut out = Vec::new();
            let arr_opt = val
                .as_array()
                .or_else(|| val.get("lyrics").and_then(|v| v.as_array()))
                .or_else(|| val.get("lines").and_then(|v| v.as_array()))
                .or_else(|| val.get("element").and_then(|v| v.as_array()));
            if let Some(arr) = arr_opt {
                for item in arr {
                    let line_text = item
                        .get("text")
                        .or_else(|| item.get("words"))
                        .or_else(|| item.get("line"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let time_val = item
                        .get("time")
                        .or_else(|| item.get("startTime"))
                        .or_else(|| item.get("start"))
                        .or_else(|| item.get("t"))
                        .and_then(parse_time_val);

                    // Parse inner word array if present
                    let mut words = Vec::new();
                    if let Some(w_arr) = item.get("words").and_then(|v| v.as_array()) {
                        for w in w_arr {
                            let w_text = w
                                .get("text")
                                .or_else(|| w.get("word"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let w_start = w
                                .get("startTime")
                                .or_else(|| w.get("start"))
                                .or_else(|| w.get("time"))
                                .and_then(parse_time_val)
                                .or(time_val);
                            let w_end = w
                                .get("endTime")
                                .or_else(|| w.get("end"))
                                .and_then(parse_time_val)
                                .or_else(|| w_start.map(|s| s + 500));
                            if let (Some(b), Some(e)) = (w_start, w_end) {
                                words.push(LyricWord { text: w_text, start_ms: b, end_ms: e });
                            }
                        }
                    }

                    if !line_text.is_empty() || time_val.is_some() {
                        out.push(LyricLine {
                            time_ms: time_val,
                            end_time_ms: None,
                            text: line_text,
                            words: if !words.is_empty() { Some(words) } else { None },
                            ..Default::default()
                        });
                    }
                }
                if !out.is_empty() {
                    out.sort_by_key(|l| l.time_ms);
                    return out;
                }
            }
        }
    }

    // 2. TTML / AAML XML
    if trimmed.starts_with('<') || trimmed.contains("<p ") || trimmed.contains("<tt") {
        let ttml_lines = parse_ttml_aaml(trimmed);
        if !ttml_lines.is_empty() {
            return ttml_lines;
        }
    }

    // 3. LRC / eLRC
    parse_elrc(text)
}

/// TTML and Apple Music AAML XML parser
fn parse_ttml_aaml(xml: &str) -> Vec<LyricLine> {
    let readings = ttml_transliterations(xml);
    let mut lines = Vec::new();
    let mut pos = 0;
    while let Some(p_start) = xml[pos..].find("<p") {
        let abs_p_start = pos + p_start;
        let Some(p_tag_end) = xml[abs_p_start..].find('>') else {
            break;
        };
        let abs_p_tag_end = abs_p_start + p_tag_end;
        let p_tag_str = &xml[abs_p_start..abs_p_tag_end + 1];

        let Some(p_close) = xml[abs_p_tag_end..].find("</p>") else {
            break;
        };
        let abs_p_close = abs_p_tag_end + p_close;
        let inner_str = &xml[abs_p_tag_end + 1..abs_p_close];

        pos = abs_p_close + 4;

        let line_begin = parse_xml_attr(p_tag_str, "begin").and_then(|s| parse_ttml_time(&s));
        let line_end = parse_xml_attr(p_tag_str, "end").and_then(|s| parse_ttml_time(&s));
        let (full_text, words) = parse_ttml_spans(inner_str, line_begin, line_end);

        if !full_text.is_empty() || line_begin.is_some() {
            let reading = parse_xml_attr(p_tag_str, "itunes:key")
                .and_then(|k| readings.iter().find(|(key, ..)| *key == k))
                // Apple repeats a Latin line verbatim as its own reading.
                .filter(|(_, text, _)| *text != full_text);
            lines.push(LyricLine {
                time_ms: line_begin,
                end_time_ms: line_end,
                text: full_text,
                words,
                romanized: reading.map(|(_, text, _)| text.clone()),
                romanized_words: reading.and_then(|(.., w)| w.clone()),
                ..Default::default()
            });
        }
    }
    lines.sort_by_key(|l| l.time_ms);
    lines
}

/// The text of one `<p>` (or `<text>`) and its timed `<span>`s. Text between spans, the spaces
/// that separate words, is appended to the word before it.
fn parse_ttml_spans(
    inner_str: &str,
    line_begin: Option<u64>,
    line_end: Option<u64>,
) -> (String, Option<Vec<LyricWord>>) {
    let mut words: Vec<LyricWord> = Vec::new();
    let mut span_pos = 0;
    let mut plain_text_buf = String::new();

    while let Some(s_start) = inner_str[span_pos..].find("<span") {
        let abs_s_start = span_pos + s_start;
        let Some(s_tag_end) = inner_str[abs_s_start..].find('>') else {
            break;
        };
        let abs_s_tag_end = abs_s_start + s_tag_end;
        let s_tag_str = &inner_str[abs_s_start..abs_s_tag_end + 1];

        let before = strip_xml_tags(&inner_str[span_pos..abs_s_start]);
        if !before.is_empty() {
            plain_text_buf.push_str(&before);
            if let Some(last_w) = words.last_mut() {
                last_w.text.push_str(&before);
            }
        }

        let Some(s_close) = inner_str[abs_s_tag_end..].find("</span>") else {
            break;
        };
        let abs_s_close = abs_s_tag_end + s_close;
        let w_text = strip_xml_tags(&inner_str[abs_s_tag_end + 1..abs_s_close]);

        let w_begin =
            parse_xml_attr(s_tag_str, "begin").and_then(|s| parse_ttml_time(&s)).or(line_begin);
        let w_end = parse_xml_attr(s_tag_str, "end").and_then(|s| parse_ttml_time(&s)).or(line_end);

        if let (Some(b), Some(e)) = (w_begin, w_end) {
            if !w_text.is_empty() {
                words.push(LyricWord { text: w_text.clone(), start_ms: b, end_ms: e });
            }
        }
        plain_text_buf.push_str(&w_text);
        span_pos = abs_s_close + 7;
    }

    if span_pos < inner_str.len() {
        plain_text_buf.push_str(&strip_xml_tags(&inner_str[span_pos..]));
    }

    (plain_text_buf.trim().to_string(), (!words.is_empty()).then_some(words))
}

/// Apple's pronunciation lines (#202), keyed by the `itunes:key` of the line they read:
/// `<transliteration xml:lang="ja-Latn"><text for="L1"><span begin=…>yume</span> …</text>`.
/// Human-written and timed to the same syllables as the original, so they beat anything
/// `romanize` can produce. Empty when the TTML has none, which is most of them.
fn ttml_transliterations(xml: &str) -> Vec<(String, String, Option<Vec<LyricWord>>)> {
    let Some(start) = xml.find("<transliteration ") else {
        return Vec::new();
    };
    let block = &xml[start..];
    let block = &block[..block.find("</transliteration>").unwrap_or(block.len())];
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(t) = block[pos..].find("<text ") {
        let tag_start = pos + t;
        let Some(tag_len) = block[tag_start..].find('>') else { break };
        let tag_end = tag_start + tag_len;
        let Some(close) = block[tag_end..].find("</text>") else { break };
        let inner = &block[tag_end + 1..tag_end + close];
        pos = tag_end + close;
        if let Some(key) = parse_xml_attr(&block[tag_start..=tag_end], "for") {
            let (text, words) = parse_ttml_spans(inner, None, None);
            if !text.is_empty() {
                out.push((key, text, words));
            }
        }
    }
    out
}

/// Enhanced LRC: `[00:10.50]<00:10.50>Hello <00:11.20>world <00:12.00>`. An inline tag is when the
/// word after it starts, so a word runs to the next tag, and text before the first tag starts with
/// the line. The last word ends at a closing tag when there is one, else at the next line's cue,
/// held to a few seconds so an instrumental break doesn't stretch it across the gap.
fn parse_elrc(lrc: &str) -> Vec<LyricLine> {
    /// How long a last word with nothing to end it may run.
    const LAST_WORD_MAX_MS: u64 = 3000;
    let mut lines = parse_lrc(lrc);
    for line in &mut lines {
        let Some(start) = line.time_ms.filter(|_| line.text.contains('<')) else { continue };
        // (start, text) runs, split at every tag that reads as a time.
        let mut runs: Vec<(u64, String)> = vec![(start, String::new())];
        let mut rest = line.text.as_str();
        while let Some(open) = rest.find('<') {
            let run = &mut runs.last_mut().unwrap().1;
            run.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let tag = after.find('>').and_then(|end| Some((end, parse_lrc_time(&after[..end])?)));
            match tag {
                Some((end, ms)) => {
                    runs.push((ms, String::new()));
                    rest = &after[end + 1..];
                }
                None => {
                    run.push('<');
                    rest = after;
                }
            }
        }
        runs.last_mut().unwrap().1.push_str(rest);
        if runs.len() == 1 {
            continue; // no timing tags, just a `<` in the text
        }
        let words: Vec<LyricWord> = runs
            .iter()
            .enumerate()
            .filter(|(_, (_, text))| !text.is_empty())
            .map(|(i, (ms, text))| LyricWord {
                text: text.clone(),
                start_ms: *ms,
                end_ms: runs.get(i + 1).map_or(u64::MAX, |r| r.0),
            })
            .collect();
        line.text = words.iter().map(|w| w.text.as_str()).collect::<String>().trim().to_owned();
        line.words = (!words.is_empty()).then_some(words);
    }
    for i in 0..lines.len() {
        let next = lines.get(i + 1).and_then(|l| l.time_ms).unwrap_or(u64::MAX);
        if let Some(w) = lines[i].words.as_mut().and_then(|w| w.last_mut()) {
            if w.end_ms == u64::MAX {
                w.end_ms = next.min(w.start_ms + LAST_WORD_MAX_MS).max(w.start_ms);
            }
        }
    }
    lines
}

/// LRCMux multiplexer: merges line lyrics with word timing or translations
fn lrc_mux(mut primary: Vec<LyricLine>, word_source: Vec<LyricLine>) -> Vec<LyricLine> {
    if word_source.is_empty() {
        return primary;
    }
    for p in &mut primary {
        let Some(p_time) = p.time_ms else {
            continue;
        };
        let best = word_source.iter().find(|ws| {
            if let Some(ws_time) = ws.time_ms {
                (p_time as i64 - ws_time as i64).abs() <= 800
            } else {
                false
            }
        });
        if let Some(ws) = best {
            if p.words.is_none() && ws.words.is_some() {
                p.words = ws.words.clone();
            }
            if p.translation.is_none() && ws.translation.is_some() {
                p.translation = ws.translation.clone();
            }
            if p.end_time_ms.is_none() && ws.end_time_ms.is_some() {
                p.end_time_ms = ws.end_time_ms;
            }
        }
    }
    primary
}

fn parse_ttml_time(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix("ms") {
        return rest.parse::<u64>().ok();
    }
    if let Some(rest) = s.strip_suffix('s') {
        let secs: f64 = rest.parse().ok()?;
        return Some((secs * 1000.0) as u64);
    }
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 3 {
            let h: u64 = parts[0].parse().ok()?;
            let m: u64 = parts[1].parse().ok()?;
            let secs: f64 = parts[2].parse().ok()?;
            return Some((h * 3600 + m * 60) * 1000 + (secs * 1000.0) as u64);
        } else if parts.len() == 2 {
            let m: u64 = parts[0].parse().ok()?;
            let secs: f64 = parts[1].parse().ok()?;
            return Some(m * 60 * 1000 + (secs * 1000.0) as u64);
        }
    }
    let secs: f64 = s.parse().ok()?;
    Some((secs * 1000.0) as u64)
}

fn parse_xml_attr(tag: &str, attr: &str) -> Option<String> {
    let pattern = format!("{attr}=\"");
    if let Some(idx) = tag.find(&pattern) {
        let start = idx + pattern.len();
        let end = tag[start..].find('"')?;
        return Some(tag[start..start + end].to_string());
    }
    let pattern_single = format!("{attr}='");
    if let Some(idx) = tag.find(&pattern_single) {
        let start = idx + pattern_single.len();
        let end = tag[start..].find('\'')?;
        return Some(tag[start..start + end].to_string());
    }
    None
}

fn strip_xml_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_lrc() {
        let lrc = "[ar:Fleetwood Mac]\n[00:27.93] Listen to the wind blow\n[00:31.16] Watch the sun rise\n";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time_ms, Some(27930));
        assert_eq!(lines[0].text, "Listen to the wind blow");
        assert_eq!(lines[1].time_ms, Some(31160));
    }

    #[test]
    fn multi_timestamp_line_repeats() {
        let lines = parse_lrc("[00:10.00][01:10.00]la la la");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time_ms, Some(10000));
        assert_eq!(lines[1].time_ms, Some(70000));
        assert!(lines.iter().all(|l| l.text == "la la la"));
    }

    #[test]
    fn keeps_bracketed_lyric_text_and_gap_lines() {
        let lines = parse_lrc("[00:05.5][Chorus] yeah\n[00:20.123]\n[00:30] plain seconds");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].time_ms, Some(5500));
        assert_eq!(lines[0].text, "[Chorus] yeah");
        assert_eq!(lines[1].time_ms, Some(20123));
        assert_eq!(lines[1].text, "");
        assert_eq!(lines[2].time_ms, Some(30000));
    }

    #[test]
    fn plain_text_splits_lines() {
        let l = plain_from_text(Some("one\ntwo\n\nthree"), "LRCLIB").unwrap();
        assert!(!l.synced);
        assert_eq!(l.lines.len(), 4);
        assert_eq!(l.lines[2].text, "");
    }

    #[test]
    fn parses_ttml_aaml_word_timestamps() {
        let xml = r#"<tt><p begin="00:10.500" end="00:14.200"><span begin="00:10.500" end="00:11.200">Hello </span><span begin="00:11.200" end="00:12.100">world </span></p></tt>"#;
        let lines = parse_ttml_aaml(xml);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].time_ms, Some(10500));
        assert_eq!(lines[0].end_time_ms, Some(14200));
        assert_eq!(lines[0].text, "Hello world");
        let words = lines[0].words.as_ref().unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Hello ");
        assert_eq!(words[0].start_ms, 10500);
        assert_eq!(words[0].end_ms, 11200);
    }

    /// Apple's TTML as Boidu serves it (trimmed from "Lemon"): the reading sits in the head,
    /// keyed to the line by `itunes:key`, and a Latin line is repeated verbatim.
    #[test]
    fn keeps_apple_transliterations() {
        let xml = r#"<tt xmlns:itunes="http://music.apple.com/lyric-ttml-internal" xml:lang="ja"><head><metadata><iTunesMetadata><transliterations><transliteration xml:lang="ja-Latn"><text for="L1"><span begin="1.241" end="1.635" xmlns="http://www.w3.org/ns/ttml">yume</span> <span begin="1.635" end="2.152" xmlns="http://www.w3.org/ns/ttml">nara</span></text><text for="L2"><span begin="3.0" end="4.0">Hey</span></text></transliteration></transliterations></iTunesMetadata></metadata></head><body><div><p begin="1.241" end="2.152" itunes:key="L1"><span begin="1.241" end="1.635">夢</span><span begin="1.635" end="2.152">なら</span></p><p begin="3.0" end="4.0" itunes:key="L2"><span begin="3.0" end="4.0">Hey</span></p></div></body></tt>"#;
        let lines = parse_ttml_aaml(xml);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "夢なら");
        assert_eq!(lines[0].romanized.as_deref(), Some("yume nara"));
        let words = lines[0].romanized_words.as_ref().unwrap();
        assert_eq!((words[0].text.as_str(), words[0].start_ms), ("yume ", 1241));
        assert_eq!(lines[1].romanized, None);
    }

    /// A tag starts the word after it. Reading it as the end shifted the whole sweep a word early.
    #[test]
    fn parses_elrc_inline_word_timestamps() {
        let lrc = "[00:10.50]<00:10.50>Hello <00:11.20>world <00:12.00>\n[00:20.00]<00:20.00>next";
        let lines = parse_elrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time_ms, Some(10500));
        assert_eq!(lines[0].text, "Hello world");
        let span = |w: &LyricWord| (w.text.clone(), w.start_ms, w.end_ms);
        let words: Vec<_> = lines[0].words.as_ref().unwrap().iter().map(span).collect();
        assert_eq!(words, [("Hello ".into(), 10500, 11200), ("world ".into(), 11200, 12000)]);
        // No closing tag: to the next line's cue, but never across a long break.
        let last = &lines[1].words.as_ref().unwrap()[0];
        assert_eq!((last.start_ms, last.end_ms), (20000, 23000));
    }

    #[test]
    fn elrc_keeps_non_ascii_text_intact() {
        // Text before the first tag starts with the line, and multi-byte text survives the split.
        let lines = parse_elrc("[00:12.00]私は<00:12.50>歌う\n[00:13.00]次");
        assert_eq!(lines[0].text, "私は歌う");
        let words = lines[0].words.as_ref().unwrap();
        assert_eq!(
            (words[0].text.as_str(), words[0].start_ms, words[0].end_ms),
            ("私は", 12000, 12500)
        );
        assert_eq!(
            (words[1].text.as_str(), words[1].start_ms, words[1].end_ms),
            ("歌う", 12500, 13000)
        );
        // A `<` that isn't a time stays text.
        assert_eq!(parse_elrc("[00:01.00]a <3 b")[0].text, "a <3 b");
    }

    #[test]
    fn from_parsed_derives_synced_from_the_lines() {
        // TTML with no `begin` parses to real lines carrying no cue. Declaring those synced is
        // what put the UI in a synced view whose highlight could never move.
        let untimed = parse_lrc_or_ttml("<tt><body><div><p>no timing here</p></div></body></tt>");
        assert!(!untimed.is_empty());
        assert!(!from_parsed("X", untimed).unwrap().synced);

        // Same for a JSON payload whose items carry text but no time.
        let json = parse_lrc_or_ttml(r#"[{"text":"one"},{"text":"two"}]"#);
        assert!(!json.is_empty());
        assert!(!from_parsed("X", json).unwrap().synced);

        let timed = parse_lrc_or_ttml("[00:01.00]one\n[00:02.00]two");
        assert!(from_parsed("X", timed).unwrap().synced);

        assert!(from_parsed("X", Vec::new()).is_none());
    }

    /// Real Kugou/Netease search shapes: the original is not first, and a remix sits inside the
    /// tolerance window, so only closest-match picks the right cut.
    #[test]
    fn best_by_duration_skips_remixes_and_wrong_cuts() {
        let secs = |t: &(f64, &str)| Some(t.0);
        // Kugou's actual top hit for "Shape of You" is a 263s edit of a 233s song.
        let kugou = [(263.0, "wrong cut"), (251.0, "dj edit"), (233.0, "original")];
        assert_eq!(best_by_duration(Some(233.0), &kugou, secs).unwrap().1, "original");
        // Netease ranks a 231s remix second; both are within 5s, distance breaks the tie.
        let netease = [(233.7, "original"), (231.2, "stormzy remix")];
        assert_eq!(best_by_duration(Some(233.0), &netease, secs).unwrap().1, "original");
        // Nothing close enough beats a wrong answer.
        assert!(best_by_duration(Some(233.0), &kugou[..2], secs).is_none());
        // No length on our side: nothing to check, first hit stands.
        assert_eq!(best_by_duration(None, &kugou, secs).unwrap().1, "wrong cut");
        // A hit with no length of its own still gets used rather than silently dropped.
        let unknown = [(0.0, "no duration")];
        let none = |_: &(f64, &str)| None;
        assert_eq!(best_by_duration(Some(233.0), &unknown, none).unwrap().1, "no duration");
    }

    #[test]
    fn provider_order_keeps_the_users_order_and_adds_new_ones() {
        let ids = |o: Vec<(&str, bool)>| {
            o.iter()
                .map(|(id, on)| format!("{}{id}", if *on { "" } else { "-" }))
                .collect::<Vec<_>>()
                .join(",")
        };
        assert_eq!(ids(provider_order(None)), PROVIDERS.join(","));
        // Unknown and repeated ids are dropped; the ones it doesn't name follow, switched on.
        assert_eq!(
            ids(provider_order(Some("lrclib, -boidu,gone,lrclib"))),
            "lrclib,-boidu,lyricsplus,youtube,simpmusic,netease,qq,kugou"
        );
    }

    /// LyricsPlus' shape, trimmed from "Spring Day": word-timed, with Apple's reading.
    #[test]
    fn kpoe_keeps_words_and_readings_and_drops_other_songs() {
        let json = r#"{"type":"Word","metadata":{"totalDuration":"4:34.000"},"lyrics":[
            {"time":21682,"duration":1297,"text":"보고 싶다","syllabus":[{"time":21682,"duration":241,"text":"보"},{"time":21923,"duration":183,"text":"고 "}],
             "transliteration":{"text":"bogo sipda","syllabus":[{"time":21682,"duration":424,"text":"bogo "}]}},
            {"time":30000,"duration":1000,"text":"Hey","syllabus":[],"transliteration":{"text":"Hey","syllabus":[]}}]}"#;
        let k = || serde_json::from_str::<Kpoe>(json).unwrap();
        let l = kpoe_to_lyrics(k(), Some(274.0)).unwrap();
        assert!(l.synced);
        assert_eq!(l.source, "LyricsPlus");
        let first = &l.lines[0];
        assert_eq!((first.time_ms, first.end_time_ms), (Some(21682), Some(22979)));
        let w = &first.words.as_ref().unwrap()[1];
        assert_eq!((w.text.as_str(), w.start_ms, w.end_ms), ("고 ", 21923, 22106));
        assert_eq!(first.romanized.as_deref(), Some("bogo sipda"));
        assert!(first.romanized_words.is_some());
        // A Latin line repeated as its own reading is no reading.
        assert_eq!(l.lines[1].romanized, None);
        // Its search strays onto other songs off the same album: the length has to agree.
        assert!(kpoe_to_lyrics(k(), Some(188.0)).is_none());
        assert!(kpoe_to_lyrics(k(), None).is_some());
    }

    #[test]
    fn search_title_drops_asides_and_suffixes() {
        assert_eq!(
            search_title(
                "Bad Apple／ Lizz Robinett (English Cover) | Lyrics/Lyric Video [English]"
            ),
            "Bad Apple／ Lizz Robinett"
        );
        assert_eq!(search_title("【MV】 我的驕傲 （Official Video）"), "我的驕傲");
        assert_eq!(search_title("Shape of You"), "Shape of You");
        assert_eq!(search_title("(Intro)"), "");
    }

    #[test]
    fn lrc_mux_combines_lines_and_word_sources() {
        let primary = vec![LyricLine::simple(Some(10000), "Hello world".into())];
        let word_source = vec![LyricLine {
            time_ms: Some(10100),
            end_time_ms: Some(14000),
            text: "Hello world".into(),
            words: Some(vec![LyricWord { text: "Hello ".into(), start_ms: 10100, end_ms: 12000 }]),
            translation: Some("Halo dunia".into()),
            ..Default::default()
        }];
        let muxed = lrc_mux(primary, word_source);
        assert_eq!(muxed.len(), 1);
        assert!(muxed[0].words.is_some());
        assert_eq!(muxed[0].translation.as_deref(), Some("Halo dunia"));
    }

    /// Are the external providers still alive? Hits them all for real, so it is NOT in the default
    /// run (context/17: network tests are opt-in, or `cargo test` fails offline):
    ///   cargo test -p limusic-app --lib -- --ignored --nocapture
    ///
    /// This exists because a provider that is *broken* and a provider that simply *has no lyrics
    /// for this track* both return `Ok(None)`, and nothing else in the chain can tell them apart:
    /// each one just falls through to the next. Netease and Kugou both shipped in PR #13 querying
    /// endpoints that answered an error for every track, and stayed unnoticed for exactly that
    /// reason. Run this after touching a provider, and whenever lyrics quietly get worse.
    ///
    /// **Read the output, don't just trust the pass.** It fails only when *every* provider is
    /// silent, because a single "no hit" is not proof of breakage: these are third-party
    /// catalogues, they drop tracks, and Kugou in particular throttles by IP and answers
    /// `total: 0` to everything for a while rather than returning an error. A provider that is
    /// genuinely dead prints "no hit" on every track you try, run after run.
    #[tokio::test]
    #[ignore = "hits the live lyrics APIs"]
    async fn providers_are_alive() {
        let req = LyricsRequest {
            video_id: "test".into(),
            title: "Shape of You".into(),
            artists: "Ed Sheeran".into(),
            album: None,
            duration: Some(233.0),
        };
        // The official music video, which SimpMusic files lyrics under.
        let video = LyricsRequest {
            video_id: "JGwWNGJdvx8".into(),
            title: req.title.clone(),
            artists: req.artists.clone(),
            album: None,
            duration: Some(264.0),
        };
        let mut alive = 0;
        for (name, hit) in [
            ("Boidu", boidu_get(&req).await),
            ("LyricsPlus", lyricsplus_get(&req).await),
            ("SimpMusic", simpmusic_get(&video).await),
            ("Netease Cloud Music", netease_get(&req).await),
            ("QQ Music", qqmusic_get(&req).await),
            ("Kugou", kugou_get(&req).await),
        ] {
            match hit {
                Ok(Some(l)) => {
                    let words = l.lines.iter().any(|l| l.words.is_some());
                    println!("{name}: {} lines, synced={}, words={words}", l.lines.len(), l.synced);
                    assert_eq!(l.source, name);
                    assert!(!l.lines.is_empty());
                    alive += 1;
                }
                Ok(None) => println!("{name}: NO HIT"),
                Err(e) => println!("{name}: ERROR {e}"),
            }
        }
        assert!(alive > 0, "no lyrics from any provider (offline?)");

        // Boidu is the only provider carrying per-word timings, and the karaoke sweep renders
        // nothing without them. Checked here rather than in its own test: a second live test runs
        // concurrently with this one, and the added latency alone was enough to trip another
        // provider's 8s timeout and fail the run.
        let boidu = boidu_get(&req).await.unwrap().expect("Boidu hit");
        assert!(boidu.lines.iter().any(|l| l.words.is_some()));

        // #329: neither title matches an LRCLIB track name, so only the free-text retries find
        // them. The second is what YouTube Music actually sends for a fan lyric video.
        for (title, artists) in [
            ("Bad Apple!! (Lizz Robinett English Cover)", "Lizz Robinett"),
            (
                "Bad Apple／ Lizz Robinett (English Cover) | Lyrics/Lyric Video [English]",
                "Lyrics Radio",
            ),
        ] {
            let cover = LyricsRequest {
                video_id: "test".into(),
                title: title.into(),
                artists: artists.into(),
                album: None,
                duration: Some(283.0),
            };
            let hit = lrclib_search(&cover).await.unwrap().expect("LRCLIB free-text hit");
            assert!(hit.has_synced(), "{title}");
        }
    }
}
