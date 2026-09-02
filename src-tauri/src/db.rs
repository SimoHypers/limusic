//! Local SQLite state. context/11 §state. `rusqlite` (bundled) behind a Mutex — one file, low
//! write volume, no async pool needed (plan decision).

use std::sync::Mutex;

use rusqlite::Connection;
use sha1::{Digest, Sha1};

pub struct Db(Mutex<Connection>);

/// The stored-account key (multi-account support): a stable per-Google-account identifier derived
/// from the long-lived `SAPISID` cookie value — the one piece of the jar Google does not rotate.
/// Versioned SHA-1, hex-encoded — deliberately not `DefaultHasher`, whose output Rust does not
/// guarantee to stay stable across releases (a persisted key must survive toolchain upgrades).
/// The SAPISID itself never appears in the key the UI gets to see.
pub fn account_key(session_cookie: &str) -> String {
    let mut digest = Sha1::new();
    digest.update(b"limusic-google-account-v1");
    digest.update(innertube::cookie_sapisid(session_cookie).unwrap_or_default().as_bytes());
    format!("ga-{}", hex::encode(digest.finalize()))
}

/// One saved Google account. Canonical multi-account state; the `settings` rows `session_cookie`,
/// `selected_identity_json`, `data_sync_id`, `account_json` and `visitor_data` remain as
/// projections of the *active* account, so everything that read them before (startup bootstrap in
/// lib.rs, `account_snapshot`, the channel switcher, legacy fallbacks) keeps working unchanged.
#[derive(Debug, Clone)]
pub struct StoredAccount {
    pub id: String,
    pub session_cookie: String,
    pub data_sync_id: Option<String>,
    pub selected_identity_json: Option<String>,
    pub account_json: Option<String>,
    pub visitor_data: Option<String>,
    pub added_at: i64,
}

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
    /// YouTube's `musicVideoType` verdict, cached for the same reason: a hit skips `/player`, and
    /// without it a replay inside the cache window can't tell the player view whether the track
    /// has a music video. `None` on rows written before the column existed.
    pub is_video: Option<bool>,
    /// The watch-history ping (`videostatsPlaybackUrl` + the client that was issued it), cached for
    /// the same reason again: a hit skips `/player`, and without it every replay inside the cache
    /// window went unregistered. That included plenty of *first* listens, because the gapless
    /// lookahead caches the next track and a non-gapless advance then re-resolves it. Issue #83.
    pub ping_url: Option<String>,
    pub ping_client: Option<String>,
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
                loudness_db REAL,
                is_video    INTEGER,
                ping_url    TEXT,
                ping_client TEXT
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
                album_artist  TEXT,
                track_no      INTEGER NOT NULL,
                duration_secs INTEGER NOT NULL,
                cover         TEXT,
                mtime         INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS local_tracks_album ON local_tracks(album_key);
            CREATE TABLE IF NOT EXISTS playlist_track (
                playlist_id TEXT NOT NULL,
                video_id    TEXT NOT NULL,
                PRIMARY KEY (playlist_id, video_id)
            ) WITHOUT ROWID;
            CREATE INDEX IF NOT EXISTS playlist_track_video ON playlist_track(video_id);
            CREATE TABLE IF NOT EXISTS accounts (
                id                     TEXT PRIMARY KEY,
                session_cookie         TEXT NOT NULL,
                data_sync_id           TEXT,
                selected_identity_json TEXT,
                account_json           TEXT,
                visitor_data           TEXT,
                added_at               INTEGER NOT NULL
            );
            "#,
        )?;
        // Migrate pre-Phase-4 DBs that predate the loudness_db column. Errors ("duplicate column")
        // on fresh DBs are expected and ignored — the cache is disposable anyway.
        let _ = conn.execute("ALTER TABLE stream_url_cache ADD COLUMN loudness_db REAL", []);
        // Same for the AlbumArtist tag, which older scans read but never stored. The SCAN_VERSION
        // bump in local.rs is what refills it; this only makes the column exist.
        let _ = conn.execute("ALTER TABLE local_tracks ADD COLUMN album_artist TEXT", []);
        // Same one-shot for the music-video verdict, except the rows that predate it have to go:
        // a cache hit skips `/player`, so a NULL there reads as "no music video" for as long as
        // the URL lives (hours). `execute` succeeds only on the launch that adds the column, so
        // this wipes the stale rows once. The cache is disposable; the next play refills it.
        if conn.execute("ALTER TABLE stream_url_cache ADD COLUMN is_video INTEGER", []).is_ok() {
            let _ = conn.execute("DELETE FROM stream_url_cache", []);
        }
        // The watch-history ping, added for the same reason (issue #83). No wipe: a NULL here just
        // means that one replay goes unregistered, which is exactly what every row did before.
        let _ = conn.execute("ALTER TABLE stream_url_cache ADD COLUMN ping_url TEXT", []);
        let _ = conn.execute("ALTER TABLE stream_url_cache ADD COLUMN ping_client TEXT", []);
        // Local files are no longer recorded as plays (see `AppState::on_position`), but 0.3.1
        // recorded them for a while, so clear out anything already sitting in On Repeat's table.
        let _ = conn.execute("DELETE FROM plays WHERE video_id LIKE 'LOCAL:%'", []);
        // Sweep dead stream URLs here as well as on write. `put_stream` only runs on a cache miss,
        // so a session spent replaying cached tracks never triggers one, and the backlog that
        // built up before anything pruned at all (1803 rows, 1772 of them expired, on a real
        // install) would sit there until it happened to.
        let _ = conn.execute("DELETE FROM stream_url_cache WHERE expires_at <= ?1", [now_secs()]);
        // One-time migration of the pre-multi-account single session into `accounts`. The legacy
        // settings rows stay in place as projections of the active account (see `StoredAccount`).
        let legacy_cookie = conn
            .query_row("SELECT value FROM settings WHERE key = 'session_cookie'", [], |r| {
                r.get::<_, String>(0)
            })
            .ok();
        if let Some(cookie) = legacy_cookie {
            let existing: i64 =
                conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0)).unwrap_or(0);
            if existing == 0 && innertube::cookie_sapisid(&cookie).is_some() {
                let get = |key: &str| -> Option<String> {
                    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                        r.get(0)
                    })
                    .ok()
                };
                let id = account_key(&cookie);
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO accounts(id, session_cookie, data_sync_id, \
                     selected_identity_json, account_json, visitor_data, added_at) \
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        id,
                        cookie,
                        get("data_sync_id"),
                        get("selected_identity_json"),
                        get("account_json"),
                        get("visitor_data"),
                        now_secs()
                    ],
                );
                let _ = conn.execute(
                    "INSERT INTO settings(key, value) VALUES('active_account', ?1) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [id],
                );
            }
        }
        // Rekey rows still carrying pre-sha1 (DefaultHasher) account ids: recompute the stable
        // key from each row's own cookie and move both the row and `active_account` onto it.
        // Idempotent — a row already on its stable key is left alone. The statement must be
        // dropped before the updates (rusqlite borrows the connection through it).
        let stale_rows: Vec<(String, String)> = {
            let mut stmt = conn.prepare("SELECT id, session_cookie FROM accounts").ok();
            let mut rows = Vec::new();
            if let Some(stmt) = stmt.as_mut() {
                if let Ok(found) = stmt.query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                }) {
                    rows.extend(found.flatten());
                }
            }
            rows
        };
        for (old_id, cookie) in stale_rows {
            let new_id = account_key(&cookie);
            if new_id != old_id {
                let _ = conn.execute(
                    "UPDATE accounts SET id = ?1 WHERE id = ?2",
                    [&new_id, &old_id],
                );
                let _ = conn.execute(
                    "UPDATE settings SET value = ?1 \
                     WHERE key = 'active_account' AND value = ?2",
                    [&new_id, &old_id],
                );
            }
        }
        Ok(Db(Mutex::new(conn)))
    }

    // --- settings ---------------------------------------------------------------------------

    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).ok()
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

    /// Persist the canonical selected identity and its two legacy projections atomically. Older
    /// releases still read `data_sync_id` / `account_json`; keeping all three in one transaction
    /// prevents a restart from pairing one channel's request delegation with another's display.
    pub fn set_auth_identity(
        &self,
        session_cookie: &str,
        selected_json: &str,
        data_sync_id: Option<&str>,
        account_json: &str,
    ) -> rusqlite::Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO settings(key, value) VALUES('session_cookie', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [session_cookie],
        )?;
        tx.execute(
            "INSERT INTO settings(key, value) VALUES('selected_identity_json', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [selected_json],
        )?;
        if let Some(id) = data_sync_id {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES('data_sync_id', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [id],
            )?;
        } else {
            tx.execute("DELETE FROM settings WHERE key = 'data_sync_id'", [])?;
        }
        tx.execute(
            "INSERT INTO settings(key, value) VALUES('account_json', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [account_json],
        )?;
        tx.execute("DELETE FROM settings WHERE key = 'account_selection_pending'", [])?;
        tx.commit()
    }

    /// Persist an authenticated cookie while deliberately leaving the account unfinished. Keeping
    /// the marker and removal of stale identity projections in the same transaction means a crash
    /// during the required picker cannot restart into YouTube's default channel silently.
    pub fn set_pending_auth_selection(&self, session_cookie: &str) -> rusqlite::Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO settings(key, value) VALUES('session_cookie', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [session_cookie],
        )?;
        for key in ["selected_identity_json", "data_sync_id", "account_json"] {
            tx.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        }
        tx.execute(
            "INSERT INTO settings(key, value) VALUES('account_selection_pending', 'true')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        tx.commit()
    }

    pub fn clear_auth_identity(&self) -> rusqlite::Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        for key in
            ["selected_identity_json", "data_sync_id", "account_json", "account_selection_pending"]
        {
            tx.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        }
        tx.commit()
    }

    // --- saved Google accounts (multi-account) ------------------------------------------------

    /// Insert or refresh a saved account. `added_at` is deliberately not updated on conflict: a
    /// re-login refreshes the cookie and identity, not the account's place in the list order.
    pub fn upsert_account(&self, account: &StoredAccount) -> rusqlite::Result<()> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts(id, session_cookie, data_sync_id, selected_identity_json, \
             account_json, visitor_data, added_at) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(id) DO UPDATE SET \
                session_cookie = excluded.session_cookie, \
                data_sync_id = excluded.data_sync_id, \
                selected_identity_json = excluded.selected_identity_json, \
                account_json = excluded.account_json, \
                visitor_data = excluded.visitor_data",
            rusqlite::params![
                account.id,
                account.session_cookie,
                account.data_sync_id,
                account.selected_identity_json,
                account.account_json,
                account.visitor_data,
                account.added_at
            ],
        )?;
        Ok(())
    }

    /// Saved accounts, oldest first. No raw auth material leaves this function's callers except
    /// back into the transport; the UI only ever sees display fields plus the opaque id.
    pub fn list_accounts(&self) -> Vec<StoredAccount> {
        let conn = self.0.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, session_cookie, data_sync_id, selected_identity_json, account_json, \
             visitor_data, added_at FROM accounts ORDER BY added_at",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok(StoredAccount {
                    id: r.get(0)?,
                    session_cookie: r.get(1)?,
                    data_sync_id: r.get(2)?,
                    selected_identity_json: r.get(3)?,
                    account_json: r.get(4)?,
                    visitor_data: r.get(5)?,
                    added_at: r.get(6)?,
                })
            }) {
                out.extend(rows.flatten());
            }
        }
        out
    }

    pub fn get_account(&self, id: &str) -> Option<StoredAccount> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT id, session_cookie, data_sync_id, selected_identity_json, account_json, \
             visitor_data, added_at FROM accounts WHERE id = ?1",
            [id],
            |r| {
                Ok(StoredAccount {
                    id: r.get(0)?,
                    session_cookie: r.get(1)?,
                    data_sync_id: r.get(2)?,
                    selected_identity_json: r.get(3)?,
                    account_json: r.get(4)?,
                    visitor_data: r.get(5)?,
                    added_at: r.get(6)?,
                })
            },
        )
        .ok()
    }

    pub fn remove_account(&self, id: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute("DELETE FROM accounts WHERE id = ?1", [id]);
    }

    /// Write a saved account's fields into the active-account `settings` projections and flip the
    /// `active_account` pointer, atomically (used when switching to a saved account), so a crash
    /// mid-switch can't restart into projections for one account and a pointer for another.
    /// Clears `account_selection_pending`: a completed account needs no channel pick.
    pub fn restore_account(&self, account: &StoredAccount) -> rusqlite::Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO settings(key, value) VALUES('session_cookie', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [&account.session_cookie],
        )?;
        for (key, value) in [
            ("data_sync_id", account.data_sync_id.as_deref()),
            ("selected_identity_json", account.selected_identity_json.as_deref()),
            ("account_json", account.account_json.as_deref()),
            ("visitor_data", account.visitor_data.as_deref()),
        ] {
            match value {
                Some(value) => {
                    tx.execute(
                        "INSERT INTO settings(key, value) VALUES(?1, ?2)
                         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        [key, value],
                    )?;
                }
                None => {
                    tx.execute("DELETE FROM settings WHERE key = ?1", [key])?;
                }
            }
        }
        tx.execute(
            "INSERT INTO settings(key, value) VALUES('active_account', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [&account.id],
        )?;
        tx.execute("DELETE FROM settings WHERE key = 'account_selection_pending'", [])?;
        tx.commit()
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
            "SELECT url, itag, expires_at, loudness_db, is_video, ping_url, ping_client FROM stream_url_cache WHERE video_id = ?1 AND expires_at > ?2",
            rusqlite::params![video_id, now],
            |r| {
                Ok(CachedStream {
                    url: r.get(0)?,
                    itag: r.get(1)?,
                    expires_at: r.get(2)?,
                    loudness_db: r.get(3)?,
                    is_video: r.get(4)?,
                    ping_url: r.get(5)?,
                    ping_client: r.get(6)?,
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

    /// Cache one resolved URL, and drop every entry that has already expired.
    ///
    /// The prune rides along with the insert (same shape as [`Db::record_play`]) because nothing
    /// else ever deleted a dead row: `get_stream` filters them out but leaves them, so the table
    /// only ever grew. Measured on a real install before this: 1803 rows / 2.5 MB, nearly all of
    /// them URLs that expired hours or weeks ago.
    pub fn put_stream(&self, video_id: &str, row: &CachedStream, now: i64) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO stream_url_cache(video_id, url, itag, expires_at, loudness_db, is_video, ping_url, ping_client) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(video_id) DO UPDATE SET url = excluded.url, itag = excluded.itag, expires_at = excluded.expires_at, loudness_db = excluded.loudness_db, is_video = excluded.is_video, ping_url = excluded.ping_url, ping_client = excluded.ping_client",
            rusqlite::params![
                video_id,
                row.url,
                row.itag,
                row.expires_at,
                row.loudness_db,
                row.is_video,
                row.ping_url,
                row.ping_client
            ],
        );
        let _ = conn.execute("DELETE FROM stream_url_cache WHERE expires_at <= ?1", [now]);
    }

    /// Wipe the whole URL cache (settings "Clear caches"). context/11.
    pub fn clear_stream_cache(&self) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute("DELETE FROM stream_url_cache", []);
        let _ = conn.execute("DELETE FROM lyrics_cache", []);
    }

    /// Drop cached lyrics only, leaving stream URLs alone. Changing which providers are allowed
    /// has to invalidate what earlier ones already answered, or the setting appears to do nothing
    /// on every track whose lyrics were already fetched (cache hits never expire).
    pub fn clear_lyrics_cache(&self) {
        let conn = self.0.lock().unwrap();
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
            if let Ok(rows) = stmt
                .query_map(rusqlite::params![since, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))
            {
                out.extend(rows.flatten());
            }
        }
        out
    }

    /// Play count per videoId since `since`. [`Db::top_plays`] answers "what are my 20 most played
    /// songs"; this answers "how many times have I played each of these", which is what sorting an
    /// arbitrary playlist by plays needs. Same table, so the same trailing window applies.
    pub fn play_counts(&self, since: i64) -> Vec<(String, i64)> {
        let conn = self.0.lock().unwrap();
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn
            .prepare("SELECT video_id, COUNT(*) FROM plays WHERE played_at >= ?1 GROUP BY video_id")
        {
            if let Ok(rows) = stmt.query_map([since], |r| Ok((r.get(0)?, r.get(1)?))) {
                out.extend(rows.flatten());
            }
        }
        out
    }

    // --- playlist membership index (which of your playlists hold a track) ----------------------
    // Populated by `commands::sync_playlist_index`, which walks the library's owned playlists.
    // Nothing here talks to YouTube; it is the answer, cached, so a track list can draw the
    // "saved" mark on its first row instead of after a round-trip per song.

    /// Replace one playlist's tracks. Delete-then-insert, not an upsert: a removal made on another
    /// device only disappears if the rows the crawl no longer saw go away with it.
    pub fn set_playlist_tracks(&self, playlist_id: &str, video_ids: &[String]) {
        let mut conn = self.0.lock().unwrap();
        let Ok(tx) = conn.transaction() else { return };
        let _ = tx.execute("DELETE FROM playlist_track WHERE playlist_id = ?1", [playlist_id]);
        for video_id in video_ids {
            let _ = tx.execute(
                "INSERT OR IGNORE INTO playlist_track(playlist_id, video_id) VALUES(?1, ?2)",
                [playlist_id, video_id.as_str()],
            );
        }
        let _ = tx.commit();
    }

    /// One track added to one playlist, so an add made here shows its mark without a re-crawl.
    pub fn add_playlist_track(&self, playlist_id: &str, video_id: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT OR IGNORE INTO playlist_track(playlist_id, video_id) VALUES(?1, ?2)",
            [playlist_id, video_id],
        );
    }

    pub fn remove_playlist_track(&self, playlist_id: &str, video_id: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "DELETE FROM playlist_track WHERE playlist_id = ?1 AND video_id = ?2",
            [playlist_id, video_id],
        );
    }

    pub fn forget_playlist(&self, playlist_id: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute("DELETE FROM playlist_track WHERE playlist_id = ?1", [playlist_id]);
    }

    /// Drop every playlist the crawl no longer saw: deleted, unsaved, or no longer owned. An
    /// empty list means nothing was indexed, which is the same thing as an empty index.
    pub fn retain_playlists(&self, keep: &[String]) {
        let conn = self.0.lock().unwrap();
        if keep.is_empty() {
            let _ = conn.execute("DELETE FROM playlist_track", []);
            return;
        }
        let holes = vec!["?"; keep.len()].join(",");
        let params = rusqlite::params_from_iter(keep.iter());
        let _ = conn.execute(
            &format!("DELETE FROM playlist_track WHERE playlist_id NOT IN ({holes})"),
            params,
        );
    }

    /// videoId → the playlists holding it. ponytail: the whole table in one go, like
    /// `local_tracks`, since an owned-playlist library is thousands of rows and the UI needs random
    /// access to it on every row it draws.
    pub fn playlist_memberships(&self) -> std::collections::HashMap<String, Vec<String>> {
        let conn = self.0.lock().unwrap();
        let mut out: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        if let Ok(mut stmt) = conn.prepare("SELECT video_id, playlist_id FROM playlist_track") {
            if let Ok(rows) =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            {
                for (video_id, playlist_id) in rows.flatten() {
                    out.entry(video_id).or_default().push(playlist_id);
                }
            }
        }
        out
    }

    /// The index is per-account, so signing out or switching channel empties it.
    pub fn clear_playlist_index(&self) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute("DELETE FROM playlist_track", []);
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
                    t.album_artist,
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
            "SELECT path, title, artist, album, album_key, album_artist, track_no, duration_secs, cover, mtime
                   FROM local_tracks {WHERE} ORDER BY album, track_no, title";
        let sql =
            sql.replace("{WHERE}", if album_key.is_some() { "WHERE album_key = ?1" } else { "" });
        let mut out = Vec::new();
        let row = |r: &rusqlite::Row| {
            Ok(LocalTrack {
                path: r.get(0)?,
                title: r.get(1)?,
                artist: r.get(2)?,
                album: r.get(3)?,
                album_key: r.get(4)?,
                album_artist: r.get(5)?,
                track_no: r.get(6)?,
                duration_secs: r.get(7)?,
                cover: r.get(8)?,
                mtime: r.get(9)?,
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
    "INSERT INTO local_tracks(path, title, artist, album, album_key, album_artist, track_no, duration_secs, cover, mtime)
     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
     ON CONFLICT(path) DO UPDATE SET title = excluded.title, artist = excluded.artist,
        album = excluded.album, album_key = excluded.album_key,
        album_artist = excluded.album_artist, track_no = excluded.track_no,
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
    /// The AlbumArtist tag, when the file has one. What an album is credited to, even when its
    /// tracks name different performers.
    pub album_artist: Option<String>,
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

        // Same rows through `play_counts`: every song, not a top N, and no metadata.
        let mut counts = d.play_counts(900);
        counts.sort();
        assert_eq!(counts, vec![("a".into(), 2), ("b".into(), 3), ("c".into(), 1)]);
        assert!(
            d.play_counts(1_500) == vec![("c".into(), 1)],
            "`since` cuts the same way it does for top_plays"
        );
    }

    #[test]
    fn playlist_index_replaces_patches_and_prunes() {
        let d = db();
        d.set_playlist_tracks("VL1", &["a".into(), "b".into()]);
        d.set_playlist_tracks("VL2", &["b".into()]);

        let m = d.playlist_memberships();
        assert_eq!(m["a"], vec!["VL1"]);
        let mut b = m["b"].clone();
        b.sort();
        assert_eq!(b, vec!["VL1", "VL2"], "one track can sit in several playlists");

        // A re-crawl is the whole list, so a track it no longer saw has to disappear with it.
        d.set_playlist_tracks("VL1", &["a".into()]);
        assert_eq!(d.playlist_memberships()["b"], vec!["VL2"]);

        // Single-track patches, the path an add or a remove made inside the app takes.
        d.add_playlist_track("VL2", "a");
        d.add_playlist_track("VL2", "a"); // idempotent: the index may already know
        let mut a = d.playlist_memberships()["a"].clone();
        a.sort();
        assert_eq!(a, vec!["VL1", "VL2"]);
        d.remove_playlist_track("VL2", "a");
        assert_eq!(d.playlist_memberships()["a"], vec!["VL1"]);

        d.forget_playlist("VL2");
        assert!(!d.playlist_memberships().contains_key("b"), "VL2 held b alone");

        // Retain keeps the named playlists and drops everything else, including on an empty list.
        d.set_playlist_tracks("VL3", &["c".into()]);
        d.retain_playlists(&["VL3".into()]);
        assert_eq!(d.playlist_memberships().keys().collect::<Vec<_>>(), vec!["c"]);
        d.retain_playlists(&[]);
        assert!(d.playlist_memberships().is_empty());
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
                d.0.lock().unwrap().query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
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

    /// A cache row with only the fields a test cares about; the rest are the boring defaults.
    fn row(url: &str, expires_at: i64) -> CachedStream {
        CachedStream {
            url: url.to_owned(),
            itag: 251,
            expires_at,
            loudness_db: None,
            is_video: None,
            ping_url: None,
            ping_client: None,
        }
    }

    #[test]
    fn put_stream_drops_entries_that_have_already_expired() {
        let d = db();
        d.put_stream("stale", &row("https://x/1", 1_000), 900);
        d.put_stream("live", &row("https://x/2", 9_000), 900);
        assert!(d.get_stream("stale", 900).is_some(), "not expired yet at t=900");

        // t=2000: "stale" expired at 1_000, so writing anything now sweeps it.
        d.put_stream("fresh", &row("https://x/3", 8_000), 2_000);
        assert!(d.get_stream("stale", 2_000).is_none());
        assert!(d.get_stream("live", 2_000).is_some(), "unexpired rows survive the sweep");
        assert!(d.get_stream("fresh", 2_000).is_some(), "the row just written survives it");
    }

    /// A cache hit skips `/player`, so the music-video verdict has to survive the round trip or
    /// the player view can't tell whether to load the video for a track played twice in a session.
    #[test]
    fn put_stream_round_trips_the_music_video_verdict() {
        let d = db();
        d.put_stream(
            "mv",
            &CachedStream { is_video: Some(true), ..row("https://x/1", 9_000) },
            900,
        );
        d.put_stream(
            "song",
            &CachedStream { is_video: Some(false), ..row("https://x/2", 9_000) },
            900,
        );
        d.put_stream("unknown", &row("https://x/3", 9_000), 900);
        assert_eq!(d.get_stream("mv", 900).unwrap().is_video, Some(true));
        assert_eq!(d.get_stream("song", 900).unwrap().is_video, Some(false));
        assert_eq!(d.get_stream("unknown", 900).unwrap().is_video, None);
    }

    /// The watch-history ping has to survive the cache the same way (issue #83): a hit skips
    /// `/player`, and the gapless lookahead means a track's *first* play is often a cache hit.
    #[test]
    fn put_stream_round_trips_the_watch_history_ping() {
        let d = db();
        d.put_stream(
            "pinged",
            &CachedStream {
                ping_url: Some("https://s.youtube.com/api/stats/playback?docid=x".to_owned()),
                ping_client: Some("ANDROID_VR_1_65_10".to_owned()),
                ..row("https://x/1", 9_000)
            },
            900,
        );
        d.put_stream("unpinged", &row("https://x/2", 9_000), 900);

        let hit = d.get_stream("pinged", 900).unwrap();
        assert_eq!(
            hit.ping_url.as_deref(),
            Some("https://s.youtube.com/api/stats/playback?docid=x")
        );
        assert_eq!(hit.ping_client.as_deref(), Some("ANDROID_VR_1_65_10"));

        let none = d.get_stream("unpinged", 900).unwrap();
        assert!(none.ping_url.is_none() && none.ping_client.is_none());
    }

    #[test]
    fn record_play_prunes_outside_the_window() {
        let d = db();
        d.record_play("stale", "{}", 1_000, 60);
        d.record_play("fresh", "{}", 5_000, 60); // prunes anything before 4_940
        assert_eq!(d.top_plays(0, 20), vec![("{}".to_string(), 1)]);
    }

    /// A re-login refreshes an account's row without resetting its place in the list, and a
    /// cookie from a different Google account lands in its own row keyed on its SAPISID.
    #[test]
    fn accounts_round_trip_refresh_and_remove() {
        let d = db();
        let a = StoredAccount {
            id: account_key("SAPISID=aaa"),
            session_cookie: "SAPISID=aaa".into(),
            data_sync_id: Some("channel-a".into()),
            selected_identity_json: Some(r#"{"data_sync_id":"channel-a"}"#.into()),
            account_json: Some(r#"{"name":"A"}"#.into()),
            visitor_data: Some("vd-a".into()),
            added_at: 100,
        };
        d.upsert_account(&a).unwrap();
        assert_ne!(a.id, account_key("SAPISID=bbb"), "keys follow the Google account");

        d.upsert_account(&StoredAccount {
            id: account_key("SAPISID=bbb"),
            session_cookie: "SAPISID=bbb".into(),
            data_sync_id: Some("channel-b".into()),
            selected_identity_json: Some(r#"{"data_sync_id":"channel-b"}"#.into()),
            account_json: Some(r#"{"name":"B"}"#.into()),
            visitor_data: None,
            added_at: 200,
        })
        .unwrap();
        assert_eq!(d.list_accounts().len(), 2);

        // Same Google account re-login: refreshes the row, keeps its original `added_at`.
        d.upsert_account(&StoredAccount {
            id: a.id.clone(),
            session_cookie: "SAPISID=aaa; __Secure-3PSID=new".into(),
            data_sync_id: Some("channel-a".into()),
            selected_identity_json: Some(r#"{"data_sync_id":"channel-a"}"#.into()),
            account_json: Some(r#"{"name":"A renamed"}"#.into()),
            visitor_data: Some("vd-a2".into()),
            added_at: 999,
        })
        .unwrap();
        let accounts = d.list_accounts();
        assert_eq!(accounts.len(), 2);
        let refreshed = accounts.iter().find(|acc| acc.id == a.id).unwrap();
        assert_eq!(refreshed.session_cookie, "SAPISID=aaa; __Secure-3PSID=new");
        assert_eq!(refreshed.account_json.as_deref(), Some(r#"{"name":"A renamed"}"#));
        assert_eq!(refreshed.added_at, 100, "re-login must not reorder the list");
        assert_eq!(d.get_account(&a.id).unwrap().visitor_data.as_deref(), Some("vd-a2"));

        d.remove_account(&a.id);
        assert!(d.get_account(&a.id).is_none());
        assert_eq!(d.list_accounts().len(), 1);
    }

    /// Switching to a saved account flips the `active_account` pointer in the same transaction as
    /// the restored projections, so a crash between them can't leave the pointer disagreeing with
    /// the projections a restart will actually use.
    #[test]
    fn restore_account_moves_the_active_pointer_with_the_projections() {
        let d = db();
        let account = StoredAccount {
            id: account_key("SAPISID=bbb"),
            session_cookie: "SAPISID=bbb".into(),
            data_sync_id: Some("channel-b".into()),
            selected_identity_json: Some(r#"{"data_sync_id":"channel-b"}"#.into()),
            account_json: Some(r#"{"name":"B"}"#.into()),
            visitor_data: Some("vd-b".into()),
            added_at: 200,
        };
        d.upsert_account(&account).unwrap();
        d.set_setting("active_account", &account_key("SAPISID=aaa"));

        d.restore_account(&account).unwrap();
        assert_eq!(
            d.get_setting("active_account").as_deref(),
            Some(account.id.as_str()),
            "the pointer flips alongside the restored projections"
        );
        assert_eq!(d.get_setting("session_cookie").as_deref(), Some("SAPISID=bbb"));
        assert_eq!(d.get_setting("data_sync_id").as_deref(), Some("channel-b"));
        assert_eq!(
            d.get_setting("selected_identity_json").as_deref(),
            Some(r#"{"data_sync_id":"channel-b"}"#)
        );
        assert_eq!(d.get_setting("account_json").as_deref(), Some(r#"{"name":"B"}"#));
        assert_eq!(d.get_setting("visitor_data").as_deref(), Some("vd-b"));
        assert_eq!(d.get_setting("account_selection_pending"), None);
    }

    /// Databases written before multi-account migrate their single session into `accounts` once.
    #[test]
    fn opening_the_db_migrates_the_legacy_session() {
        let path = std::env::temp_dir().join("limusic-accounts-migration-test.sqlite");
        std::fs::remove_file(&path).ok();
        {
            let d = Db::open(&path).unwrap();
            d.set_setting("session_cookie", "SAPISID=legacy");
            d.set_setting("data_sync_id", "channel-x");
            d.set_setting("account_json", r#"{"name":"Legacy"}"#);
            d.set_setting("visitor_data", "vd-x");
        }
        {
            let d = Db::open(&path).unwrap();
            let accounts = d.list_accounts();
            assert_eq!(accounts.len(), 1);
            assert_eq!(accounts[0].session_cookie, "SAPISID=legacy");
            assert_eq!(accounts[0].data_sync_id.as_deref(), Some("channel-x"));
            assert_eq!(
                d.get_setting("active_account").as_deref(),
                Some(accounts[0].id.as_str()),
                "the migrated session is the active account"
            );
        }
        std::fs::remove_file(&path).ok();
    }

    /// Rows saved under the old DefaultHasher-based keys are rekeyed onto the stable sha1 keys
    /// when the db opens, and `active_account` follows the move.
    #[test]
    fn opening_the_db_rekeys_accounts_onto_stable_keys() {
        let path = std::env::temp_dir().join("limusic-accounts-rekey-test.sqlite");
        std::fs::remove_file(&path).ok();
        {
            let d = Db::open(&path).unwrap();
            let conn = d.0.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts(id, session_cookie, data_sync_id, selected_identity_json, \
                 account_json, visitor_data, added_at) \
                 VALUES('ga-olddefault123', 'SAPISID=legacy2', NULL, NULL, NULL, NULL, 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO settings(key, value) VALUES('active_account', 'ga-olddefault123')",
                [],
            )
            .unwrap();
        }
        {
            let d = Db::open(&path).unwrap();
            let accounts = d.list_accounts();
            assert_eq!(accounts.len(), 1);
            let stable = account_key("SAPISID=legacy2");
            assert_eq!(accounts[0].id, stable, "stale keys are recomputed from the cookie");
            assert_eq!(d.get_setting("active_account").as_deref(), Some(stable.as_str()));
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn auth_identity_projections_are_updated_and_cleared_together() {
        let d = db();
        d.set_auth_identity(
            "SAPISID=cookie-a",
            r#"{"data_sync_id":"channel-a"}"#,
            Some("channel-a"),
            r#"{"name":"Channel A"}"#,
        )
        .unwrap();
        assert_eq!(d.get_setting("data_sync_id").as_deref(), Some("channel-a"));
        assert_eq!(
            d.get_setting("selected_identity_json").as_deref(),
            Some(r#"{"data_sync_id":"channel-a"}"#)
        );
        assert_eq!(d.get_setting("account_json").as_deref(), Some(r#"{"name":"Channel A"}"#));
        assert_eq!(d.get_setting("session_cookie").as_deref(), Some("SAPISID=cookie-a"));

        d.set_pending_auth_selection("SAPISID=cookie-b").unwrap();
        assert_eq!(d.get_setting("session_cookie").as_deref(), Some("SAPISID=cookie-b"));
        assert_eq!(d.get_setting("selected_identity_json"), None);
        assert_eq!(d.get_setting("data_sync_id"), None);
        assert_eq!(d.get_setting("account_json"), None);
        assert_eq!(d.get_setting("account_selection_pending").as_deref(), Some("true"));

        d.set_auth_identity(
            "SAPISID=cookie-b",
            r#"{"data_sync_id":null}"#,
            None,
            r#"{"name":"Single channel"}"#,
        )
        .unwrap();
        assert_eq!(d.get_setting("data_sync_id"), None, "a stale delegated id must be deleted");
        assert_eq!(d.get_setting("account_selection_pending"), None);

        d.clear_auth_identity().unwrap();
        assert_eq!(d.get_setting("selected_identity_json"), None);
        assert_eq!(d.get_setting("data_sync_id"), None);
        assert_eq!(d.get_setting("account_json"), None);
    }
}

// Queue persistence lives in the `settings` KV as a JSON blob (`queue_json`) + `queue_position`,
// so restore round-trips the full SongItem losslessly via serde (context/11 §state).
