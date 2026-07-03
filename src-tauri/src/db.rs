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
                video_id   TEXT PRIMARY KEY,
                url        TEXT NOT NULL,
                itag       INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS queue (
                position  INTEGER PRIMARY KEY,
                video_id  TEXT NOT NULL,
                title     TEXT,
                artists   TEXT,
                duration  TEXT,
                thumbnail TEXT
            );
            "#,
        )?;
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
            "SELECT url, itag, expires_at FROM stream_url_cache WHERE video_id = ?1 AND expires_at > ?2",
            rusqlite::params![video_id, now],
            |r| Ok(CachedStream { url: r.get(0)?, itag: r.get(1)?, expires_at: r.get(2)? }),
        )
        .ok()
    }

    pub fn put_stream(&self, video_id: &str, url: &str, itag: i64, expires_at: i64) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO stream_url_cache(video_id, url, itag, expires_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(video_id) DO UPDATE SET url = excluded.url, itag = excluded.itag, expires_at = excluded.expires_at",
            rusqlite::params![video_id, url, itag, expires_at],
        );
    }

    // --- queue persistence ------------------------------------------------------------------

    pub fn save_queue(&self, items: &[QueueRow]) {
        let mut conn = self.0.lock().unwrap();
        let tx = match conn.transaction() {
            Ok(tx) => tx,
            Err(_) => return,
        };
        let _ = tx.execute("DELETE FROM queue", []);
        for (i, item) in items.iter().enumerate() {
            let _ = tx.execute(
                "INSERT INTO queue(position, video_id, title, artists, duration, thumbnail)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    i as i64,
                    item.video_id,
                    item.title,
                    item.artists,
                    item.duration,
                    item.thumbnail
                ],
            );
        }
        let _ = tx.commit();
    }

    // ponytail: queue-restore-on-startup seam. Written by save_queue every play; not read back
    // until Phase 2 wires "resume last session" in lib.rs setup. Keep — the write half is live.
    #[allow(dead_code)]
    pub fn load_queue(&self) -> Vec<QueueRow> {
        let conn = self.0.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT video_id, title, artists, duration, thumbnail FROM queue ORDER BY position",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok(QueueRow {
                    video_id: r.get(0)?,
                    title: r.get(1)?,
                    artists: r.get(2)?,
                    duration: r.get(3)?,
                    thumbnail: r.get(4)?,
                })
            }) {
                out.extend(rows.flatten());
            }
        }
        out
    }
}

#[derive(Clone, serde::Serialize)]
pub struct QueueRow {
    pub video_id: String,
    pub title: Option<String>,
    pub artists: Option<String>,
    pub duration: Option<String>,
    pub thumbnail: Option<String>,
}
