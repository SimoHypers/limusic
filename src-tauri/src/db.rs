//! Local SQLite state. context/11 §state. `rusqlite` (bundled) behind a Mutex — one file, low
//! write volume, no async pool needed (plan decision).

use std::sync::Mutex;

use rusqlite::Connection;

pub struct Db(Mutex<Connection>);

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
            "#,
        )?;
        // Migrate pre-Phase-4 DBs that predate the loudness_db column. Errors ("duplicate column")
        // on fresh DBs are expected and ignored — the cache is disposable anyway.
        let _ = conn.execute("ALTER TABLE stream_url_cache ADD COLUMN loudness_db REAL", []);
        Ok(Db(Mutex::new(conn)))
    }

    // --- settings ---------------------------------------------------------------------------

    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
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
        let _ = conn.execute("DELETE FROM stream_url_cache WHERE video_id = ?1", [video_id]);
    }

    pub fn put_stream(
        &self,
        video_id: &str,
        url: &str,
        itag: i64,
        expires_at: i64,
        loudness_db: Option<f64>,
    ) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO stream_url_cache(video_id, url, itag, expires_at, loudness_db) VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(video_id) DO UPDATE SET url = excluded.url, itag = excluded.itag, expires_at = excluded.expires_at, loudness_db = excluded.loudness_db",
            rusqlite::params![video_id, url, itag, expires_at, loudness_db],
        );
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
}

// Queue persistence lives in the `settings` KV as a JSON blob (`queue_json`) + `queue_position`,
// so restore round-trips the full SongItem losslessly via serde (context/11 §state).
