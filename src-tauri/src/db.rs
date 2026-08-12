//! Local SQLite state. context/11 §state. `rusqlite` (bundled) behind a Mutex — one file, low
//! write volume, no async pool needed (plan decision).

use std::sync::Mutex;

use rusqlite::Connection;

pub struct Db(Mutex<Connection>);

/// Unix seconds. Lives here because every wall-clock value in the app is a column in this file
/// (`expires_at`, `played_at`, `fetched_at`) or something stored alongside them.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A cached stream URL with its expiry. Never a source of truth — purely a latency cache.
pub struct CachedStream {
    pub url: String,
    pub itag: i64,
    pub expires_at: i64,
    /// Raw `loudnessDb` (main-client metadata) so a cache-hit replay still normalizes loudness.
    pub loudness_db: Option<f64>,
}

impl Db {
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        // This file is a cache plus a little UI state, and it is written on every volume nudge,
        // pause, track change and queue edit. WAL keeps those off a full rollback journal, and
        // `synchronous=NORMAL` drops the fsync per commit: a power cut can lose the last few
        // seconds of "what was playing", which is the correct trade for a music player, and no
        // crash of ours can corrupt the file either way. `journal_mode` answers with a row, so it
        // is a query rather than a `pragma_update`.
        let _ = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0));
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS stream_url_cache (
                video_id    TEXT PRIMARY KEY,
                url         TEXT NOT NULL,
                itag        INTEGER NOT NULL,
                expires_at  INTEGER NOT NULL,
                loudness_db REAL
            );
            CREATE TABLE IF NOT EXISTS lyrics_cache (
                video_id   TEXT PRIMARY KEY,
                lyrics     TEXT,
                fetched_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS plays (
                id        INTEGER PRIMARY KEY,
                video_id  TEXT NOT NULL,
                played_at INTEGER NOT NULL,
                song_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS plays_played_at ON plays(played_at);
            CREATE TABLE IF NOT EXISTS local_tracks (
                path          TEXT PRIMARY KEY,
                title         TEXT NOT NULL,
                artist        TEXT NOT NULL,
                album         TEXT NOT NULL,
                album_key     TEXT NOT NULL,
                track_no      INTEGER NOT NULL,
                duration_secs INTEGER NOT NULL,
                cover         TEXT,
                mtime         INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS local_tracks_album ON local_tracks(album_key);
            "#,
        )?;
        // Migrate pre-Phase-4 DBs that predate the loudness_db column. Errors ("duplicate column")
        // on fresh DBs are expected and ignored — the cache is disposable anyway.
        let _ = conn.execute(
            "ALTER TABLE stream_url_cache ADD COLUMN loudness_db REAL",
            [],
        );
        // Local files are no longer recorded as plays (see `AppState::on_position`), but 0.3.1
        // recorded them for a while, so clear out anything already sitting in On Repeat's table.
        let _ = conn.execute("DELETE FROM plays WHERE video_id LIKE 'LOCAL:%'", []);
        // Sweep dead stream URLs here as well as on write. `put_stream` only runs on a cache miss,
        // so a session spent replaying cached tracks never triggers one, and the backlog that
        // built up before anything pruned at all (1803 rows, 1772 of them expired, on a real
        // install) would sit there until it happened to.
        let _ = conn.execute(
            "DELETE FROM stream_url_cache WHERE expires_at <= ?1",
            [now_secs()],
        );
        Ok(Db(Mutex::new(conn)))
    }

    // --- settings ---------------------------------------------------------------------------

    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .ok()
    }

    pub fn set_setting(&self, key: &str, value: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        );
    }

    pub fn delete_setting(&self, key: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute("DELETE FROM settings WHERE key = ?1", [key]);
    }

    pub fn all_settings(&self) -> Vec<(String, String)> {
        let conn = self.0.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn.prepare("SELECT key, value FROM settings") {
            if let Ok(rows) = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
                out.extend(rows.flatten());
            }
        }
        out
    }

    // --- stream url cache -------------------------------------------------------------------

    /// Return the cached URL only if still valid (`expires_at` in the future). context/11.
    pub fn get_stream(&self, video_id: &str, now: i64) -> Option<CachedStream> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT url, itag, expires_at, loudness_db FROM stream_url_cache WHERE video_id = ?1 AND expires_at > ?2",
            rusqlite::params![video_id, now],
            |r| {
                Ok(CachedStream {
                    url: r.get(0)?,
                    itag: r.get(1)?,
                    expires_at: r.get(2)?,
                    loudness_db: r.get(3)?,
                })
            },
        )
        .ok()
    }

    /// Drop a cached URL (e.g. it 403'd on the real GET). context/06 §2.
    pub fn evict_stream(&self, video_id: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "DELETE FROM stream_url_cache WHERE video_id = ?1",
            [video_id],
        );
    }

    /// Cache one resolved URL, and drop every entry that has already expired.
    ///
    /// The prune rides along with the insert (same shape as [`Db::record_play`]) because nothing
    /// else ever deleted a dead row: `get_stream` filters them out but leaves them, so the table
    /// only ever grew. Measured on a real install before this: 1803 rows / 2.5 MB, nearly all of
    /// them URLs that expired hours or weeks ago.
    pub fn put_stream(
        &self,
        video_id: &str,
        url: &str,
        itag: i64,
        expires_at: i64,
        loudness_db: Option<f64>,
        now: i64,
    ) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO stream_url_cache(video_id, url, itag, expires_at, loudness_db) VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(video_id) DO UPDATE SET url = excluded.url, itag = excluded.itag, expires_at = excluded.expires_at, loudness_db = excluded.loudness_db",
            rusqlite::params![video_id, url, itag, expires_at, loudness_db],
        );
        let _ = conn.execute("DELETE FROM stream_url_cache WHERE expires_at <= ?1", [now]);
    }

    /// Wipe the whole URL cache (settings "Clear caches"). context/11.
    pub fn clear_stream_cache(&self) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute("DELETE FROM stream_url_cache", []);
        let _ = conn.execute("DELETE FROM lyrics_cache", []);
    }

    // --- lyrics cache -----------------------------------------------------------------------

    /// Cached lyrics JSON for a track. `Some(None)` = a cached "no lyrics" verdict (NULL row),
    /// still valid; misses expire after `miss_ttl` secs while hits live forever.
    pub fn get_lyrics(&self, video_id: &str, now: i64, miss_ttl: i64) -> Option<Option<String>> {
        let conn = self.0.lock().unwrap();
        let (lyrics, fetched_at): (Option<String>, i64) = conn
            .query_row(
                "SELECT lyrics, fetched_at FROM lyrics_cache WHERE video_id = ?1",
                [video_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()?;
        if lyrics.is_none() && now - fetched_at > miss_ttl {
            return None; // stale negative result → refetch
        }
        Some(lyrics)
    }

    /// `lyrics = None` records a "no lyrics found" verdict.
    pub fn put_lyrics(&self, video_id: &str, lyrics: Option<&str>, now: i64) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO lyrics_cache(video_id, lyrics, fetched_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(video_id) DO UPDATE SET lyrics = excluded.lyrics, fetched_at = excluded.fetched_at",
            rusqlite::params![video_id, lyrics, now],
        );
    }

    // --- play history (the On Repeat playlist) ------------------------------------------------

    /// Record one completed play and drop everything that has fallen out of the window, so the
    /// table stays bounded at roughly a month of listening whether or not anyone opens the
    /// playlist. `song_json` is the serialized `SongItem`, kept per row so the playlist can be
    /// rebuilt without asking YouTube for metadata it already gave us.
    pub fn record_play(&self, video_id: &str, song_json: &str, now: i64, window: i64) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO plays(video_id, played_at, song_json) VALUES(?1, ?2, ?3)",
            rusqlite::params![video_id, now, song_json],
        );
        let _ = conn.execute("DELETE FROM plays WHERE played_at < ?1", [now - window]);
    }

    /// The most-played songs since `since`, as `(song_json, play_count)` ranked by plays and then
    /// by recency. Each row's JSON comes from that song's latest play: SQLite resolves a bare
    /// column against the row matching the single `max()` in the query.
    pub fn top_plays(&self, since: i64, limit: usize) -> Vec<(String, i64)> {
        let conn = self.0.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT song_json, COUNT(*) AS plays, MAX(played_at) AS last FROM plays
             WHERE played_at >= ?1
             GROUP BY video_id
             ORDER BY plays DESC, last DESC
             LIMIT ?2",
        ) {
            if let Ok(rows) = stmt.query_map(rusqlite::params![since, limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            }) {
                out.extend(rows.flatten());
            }
        }
        out
    }

    // --- local music library (local.rs) -------------------------------------------------------

    /// Every known file with its recorded mtime — the scanner re-reads tags only where it differs.
    pub fn local_mtimes(&self) -> std::collections::HashMap<String, i64> {
        let conn = self.0.lock().unwrap();
        let mut out = std::collections::HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT path, mtime FROM local_tracks") {
            if let Ok(rows) = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) {
                out.extend(rows.flatten());
            }
        }
        out
    }

    /// Upsert a batch in one transaction. SQLite fsyncs per statement otherwise, which is the
    /// difference between a first scan taking a second and taking minutes.
    pub fn put_local_tracks(&self, tracks: &[LocalTrack]) {
        if tracks.is_empty() {
            return;
        }
        let mut conn = self.0.lock().unwrap();
        let Ok(tx) = conn.transaction() else { return };
        for t in tracks {
            let _ = tx.execute(
                LOCAL_TRACK_UPSERT,
                rusqlite::params![
                    t.path,
                    t.title,
                    t.artist,
                    t.album,
                    t.album_key,
                    t.track_no,
                    t.duration_secs,
                    t.cover,
                    t.mtime
                ],
            );
        }
        let _ = tx.commit();
    }

    /// Forget files that are no longer on disk (the user deleted or moved them).
    pub fn delete_local_tracks(&self, paths: &[String]) {
        if paths.is_empty() {
            return;
        }
        let mut conn = self.0.lock().unwrap();
        let Ok(tx) = conn.transaction() else { return };
        for p in paths {
            let _ = tx.execute("DELETE FROM local_tracks WHERE path = ?1", [p]);
        }
        let _ = tx.commit();
    }

    /// All tracks, or one album's, in album order. ponytail: loads the whole table — a personal
    /// collection is thousands of rows, so paging it would buy nothing.
    pub fn local_tracks(&self, album_key: Option<&str>) -> Vec<LocalTrack> {
        let conn = self.0.lock().unwrap();
        let sql =
            "SELECT path, title, artist, album, album_key, track_no, duration_secs, cover, mtime
                   FROM local_tracks {WHERE} ORDER BY album, track_no, title";
        let sql = sql.replace(
            "{WHERE}",
            if album_key.is_some() {
                "WHERE album_key = ?1"
            } else {
                ""
            },
        );
        let mut out = Vec::new();
        let row = |r: &rusqlite::Row| {
            Ok(LocalTrack {
                path: r.get(0)?,
                title: r.get(1)?,
                artist: r.get(2)?,
                album: r.get(3)?,
                album_key: r.get(4)?,
                track_no: r.get(5)?,
                duration_secs: r.get(6)?,
                cover: r.get(7)?,
                mtime: r.get(8)?,
            })
        };
        if let Ok(mut stmt) = conn.prepare(&sql) {
            let rows = match album_key {
                Some(k) => stmt.query_map([k], row),
                None => stmt.query_map([], row),
            };
            if let Ok(rows) = rows {
                out.extend(rows.flatten());
            }
        }
        out
    }
}

const LOCAL_TRACK_UPSERT: &str =
    "INSERT INTO local_tracks(path, title, artist, album, album_key, track_no, duration_secs, cover, mtime)
     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
     ON CONFLICT(path) DO UPDATE SET title = excluded.title, artist = excluded.artist,
        album = excluded.album, album_key = excluded.album_key, track_no = excluded.track_no,
        duration_secs = excluded.duration_secs, cover = excluded.cover, mtime = excluded.mtime";

/// One file in the local library. Tag data as read at scan time; `mtime` is the change detector.
#[derive(Debug, Clone)]
pub struct LocalTrack {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Stable, human-readable album id fragment (`artist--album`, sanitized). See `local.rs`.
    pub album_key: String,
    pub track_no: i64,
    pub duration_secs: i64,
    /// Absolute path to the cover image (extracted or found next to the files).
    pub cover: Option<String>,
    pub mtime: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn top_plays_ranks_by_count_then_recency_and_carries_the_latest_metadata() {
        let d = db();
        // "a" twice, "b" three times, "c" once but most recently, "old" outside the window.
        for (id, json, at) in [
            ("old", "{\"old\":1}", 100),
            ("a", "{\"a\":1}", 1_000),
            ("a", "{\"a\":2}", 1_100),
            ("b", "{\"b\":1}", 1_000),
            ("b", "{\"b\":2}", 1_050),
            ("b", "{\"b\":3}", 1_060),
            ("c", "{\"c\":1}", 2_000),
        ] {
            // A window wide enough that inserting doesn't prune what the next row needs; the
            // "old" row is excluded by `since` below instead.
            d.record_play(id, json, at, 10_000);
        }

        let top = d.top_plays(900, 20);
        assert_eq!(
            top,
            vec![
                ("{\"b\":3}".into(), 3), // most plays
                ("{\"a\":2}".into(), 2), // latest json wins for a song, not the first
                ("{\"c\":1}".into(), 1), // ties on count break toward the recent play
            ],
            "'old' is outside the window and must not appear"
        );
        assert_eq!(d.top_plays(900, 2).len(), 2, "limit applies");
    }

    #[test]
    fn opening_the_db_clears_local_files_out_of_on_repeat() {
        // 0.3.1 counted local plays before On Repeat excluded them; opening the db drops the rows.
        let path = std::env::temp_dir().join("limusic-plays-purge-test.sqlite");
        std::fs::remove_file(&path).ok();
        {
            let d = Db::open(&path).unwrap();
            // Piggybacking on the one file-backed test: `journal_mode` answers with a row, so
            // setting it via `pragma_update` would silently do nothing (and `:memory:` cannot be
            // WAL at all, which is why this can't live in its own in-memory test).
            let mode: String =
                d.0.lock()
                    .unwrap()
                    .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                    .unwrap();
            assert_eq!(mode, "wal");
            d.record_play("LOCAL:/music/a.mp3", "{\"local\":1}", 1_000, 10_000);
            d.record_play("dQw4w9WgXcQ", "{\"yt\":1}", 1_000, 10_000);
            assert_eq!(d.top_plays(0, 20).len(), 2, "both were recorded");
        }
        let d = Db::open(&path).unwrap();
        assert_eq!(
            d.top_plays(0, 20),
            vec![("{\"yt\":1}".to_string(), 1)],
            "only the YouTube play survives"
        );
        drop(d);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn put_stream_drops_entries_that_have_already_expired() {
        let d = db();
        d.put_stream("stale", "https://x/1", 251, 1_000, None, 900);
        d.put_stream("live", "https://x/2", 251, 9_000, None, 900);
        assert!(
            d.get_stream("stale", 900).is_some(),
            "not expired yet at t=900"
        );

        // t=2000: "stale" expired at 1_000, so writing anything now sweeps it.
        d.put_stream("fresh", "https://x/3", 251, 8_000, None, 2_000);
        assert!(d.get_stream("stale", 2_000).is_none());
        assert!(
            d.get_stream("live", 2_000).is_some(),
            "unexpired rows survive the sweep"
        );
        assert!(
            d.get_stream("fresh", 2_000).is_some(),
            "the row just written survives it"
        );
    }

    #[test]
    fn record_play_prunes_outside_the_window() {
        let d = db();
        d.record_play("stale", "{}", 1_000, 60);
        d.record_play("fresh", "{}", 5_000, 60); // prunes anything before 4_940
        assert_eq!(d.top_plays(0, 20), vec![("{}".to_string(), 1)]);
    }
}

// Queue persistence lives in the `settings` KV as a JSON blob (`queue_json`) + `queue_position`,
// so restore round-trips the full SongItem losslessly via serde (context/11 §state).
