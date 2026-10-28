//! Durable per-provider listens. Credentials remain in the worker; only their fingerprint is stored.

use md5::{Digest, Md5};
use rusqlite::{params, Connection};

const RETENTION: i64 = 14 * 24 * 60 * 60;
const MAX_ATTEMPTS: u32 = 20;

pub(crate) struct Enrollment {
    pub provider: &'static str,
    account: String,
    persisted: bool,
}

impl Enrollment {
    pub(crate) fn new(provider: &'static str, credential: &str) -> Self {
        Self { provider, account: account_key(provider, credential), persisted: false }
    }

    pub(crate) fn enqueue(
        &mut self,
        db: &crate::db::Db,
        credential: Option<&str>,
        event: &str,
        payload: &str,
        listened_at: i64,
        now: i64,
    ) -> rusqlite::Result<()> {
        // A timer may already have submitted and deleted this play's outbox row.
        if self.persisted {
            return Ok(());
        }
        if credential.is_some_and(|key| account_key(self.provider, key) == self.account) {
            db.enqueue_scrobble(self.provider, &self.account, event, payload, listened_at, now)?;
            self.persisted = true;
        }
        Ok(())
    }
}

pub(crate) struct Pending {
    pub id: i64,
    pub provider: String,
    pub account: String,
    pub payload: String,
    pub attempts: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    Accepted,
    Retry,
    Permanent,
}

/// This is an identity comparison of high-entropy tokens, never an authentication credential.
pub(crate) fn account_key(provider: &str, credential: &str) -> String {
    format!("{:x}", Md5::digest(format!("limusic-scrobble:{provider}\0{credential}").as_bytes()))
}

pub(crate) fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS scrobble_outbox (
        id INTEGER PRIMARY KEY,
        provider TEXT NOT NULL,
        account TEXT NOT NULL,
        event TEXT NOT NULL,
        payload TEXT NOT NULL,
        listened_at INTEGER NOT NULL,
        attempts INTEGER NOT NULL DEFAULT 0,
        next_attempt INTEGER NOT NULL,
        failed INTEGER NOT NULL DEFAULT 0,
        UNIQUE(provider, event)
    );",
    )
}

pub(crate) fn enqueue(
    conn: &mut Connection,
    provider: &str,
    account: &str,
    event: &str,
    payload: &str,
    listened_at: i64,
    now: i64,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM scrobble_outbox WHERE listened_at < ?1", [now - RETENTION])?;
    tx.execute("INSERT OR IGNORE INTO scrobble_outbox(provider,account,event,payload,listened_at,next_attempt) VALUES(?1,?2,?3,?4,?5,?6)", params![provider,account,event,payload,listened_at,now])?;
    tx.execute("DELETE FROM scrobble_outbox WHERE id IN (SELECT id FROM scrobble_outbox ORDER BY failed DESC, listened_at, id LIMIT (SELECT MAX(COUNT(*) - 10000, 0) FROM scrobble_outbox))", [])?;
    tx.commit()
}

pub(crate) fn due(conn: &Connection, now: i64) -> rusqlite::Result<Vec<Pending>> {
    conn.execute("DELETE FROM scrobble_outbox WHERE listened_at < ?1", [now - RETENTION])?;
    let mut query = conn.prepare("SELECT id,provider,account,payload,attempts FROM scrobble_outbox WHERE failed=0 AND next_attempt<=?1 AND id IN (SELECT MIN(id) FROM scrobble_outbox WHERE failed=0 GROUP BY provider) ORDER BY id")?;
    let rows = query.query_map([now], |row| {
        Ok(Pending {
            id: row.get(0)?,
            provider: row.get(1)?,
            account: row.get(2)?,
            payload: row.get(3)?,
            attempts: row.get(4)?,
        })
    })?;
    rows.collect()
}

pub(crate) fn finish(
    conn: &Connection,
    id: i64,
    retry: Option<(u32, i64)>,
    failed: bool,
) -> rusqlite::Result<()> {
    if failed {
        conn.execute("UPDATE scrobble_outbox SET failed=1 WHERE id=?1", [id])?;
    } else if let Some((attempts, next_attempt)) = retry {
        conn.execute(
            "UPDATE scrobble_outbox SET attempts=?2,next_attempt=?3 WHERE id=?1",
            params![id, attempts, next_attempt],
        )?;
    } else {
        conn.execute("DELETE FROM scrobble_outbox WHERE id=?1", [id])?;
    }
    Ok(())
}

pub(crate) fn counts(conn: &Connection, provider: &str) -> rusqlite::Result<(usize, usize)> {
    conn.query_row("SELECT COALESCE(SUM(failed=0),0),COALESCE(SUM(failed<>0),0) FROM scrobble_outbox WHERE provider=?1", [provider], |row| Ok((row.get(0)?, row.get(1)?)))
}

pub(crate) fn discard(
    conn: &Connection,
    provider: &str,
    account: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM scrobble_outbox WHERE provider=?1 AND (?2 IS NULL OR account<>?2)",
        params![provider, account],
    )?;
    Ok(())
}

pub(crate) async fn drain<F, Fut>(
    db: &crate::db::Db,
    accounts: &[(&str, &str)],
    now: i64,
    mut send: F,
) -> rusqlite::Result<()>
where
    F: FnMut(Pending, String) -> Fut,
    Fut: std::future::Future<Output = Delivery>,
{
    for pending in db.due_scrobbles(now)? {
        let Some((_, credential)) =
            accounts.iter().find(|(provider, _)| *provider == pending.provider)
        else {
            continue;
        };
        if account_key(&pending.provider, credential) != pending.account {
            db.finish_scrobble(pending.id, None, false)?;
            continue;
        }
        let (id, attempts) = (pending.id, pending.attempts + 1);
        match send(pending, (*credential).to_owned()).await {
            Delivery::Accepted => db.finish_scrobble(id, None, false)?,
            Delivery::Permanent => db.finish_scrobble(id, None, true)?,
            Delivery::Retry if attempts >= MAX_ATTEMPTS => db.finish_scrobble(id, None, true)?,
            Delivery::Retry => {
                let wait = (30_i64 << (attempts - 1).min(10)).min(6 * 60 * 60);
                db.finish_scrobble(id, Some((attempts, now + wait)), false)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use std::path::Path;

    #[tokio::test]
    async fn retrying_a_partial_enqueue_never_replays_a_provider_already_drained_by_the_timer() {
        let path = std::env::temp_dir()
            .join(format!("limusic-partial-enqueue-{}.sqlite", rand::random::<u64>()));
        let db = Db::open(&path).unwrap();
        let injected_failure = Connection::open(&path).unwrap();
        injected_failure.execute_batch("CREATE TRIGGER reject_listenbrainz_enqueue BEFORE INSERT ON scrobble_outbox WHEN NEW.provider='listenbrainz' BEGIN SELECT RAISE(FAIL, 'injected write failure'); END;").unwrap();
        let accounts = [("lastfm", "session-a"), ("listenbrainz", "token-b")];
        let mut enrolled: Vec<_> =
            accounts.iter().map(|(provider, key)| Enrollment::new(provider, key)).collect();
        assert!(enrolled[0].enqueue(&db, Some("session-a"), "play", "{}", 100, 100).is_ok());
        assert!(enrolled[1].enqueue(&db, Some("token-b"), "play", "{}", 100, 100).is_err());

        // The paused worker's timer can submit and delete Last.fm's row before another Position.
        let mut requests = Vec::new();
        drain(&db, &accounts, 115, |pending, _| {
            requests.push(pending.provider);
            std::future::ready(Delivery::Accepted)
        })
        .await
        .unwrap();
        assert_eq!(requests, ["lastfm"]);
        assert_eq!(db.scrobble_counts("lastfm"), (0, 0));

        injected_failure.execute_batch("DROP TRIGGER reject_listenbrainz_enqueue").unwrap();
        for (enrollment, (_, credential)) in enrolled.iter_mut().zip(accounts) {
            enrollment.enqueue(&db, Some(credential), "play", "{}", 100, 116).unwrap();
        }
        drain(&db, &accounts, 116, |pending, _| {
            requests.push(pending.provider);
            std::future::ready(Delivery::Accepted)
        })
        .await
        .unwrap();
        assert_eq!(requests, ["lastfm", "listenbrainz"]);
        assert_eq!(db.scrobble_counts("lastfm"), (0, 0));
        assert_eq!(db.scrobble_counts("listenbrainz"), (0, 0));
        drop(injected_failure);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn an_unpersisted_enrollment_cannot_move_to_new_credentials() {
        let db = Db::open(Path::new(":memory:")).unwrap();
        let mut enrollment = Enrollment::new("listenbrainz", "old-token");
        enrollment.enqueue(&db, Some("new-token"), "play", "{}", 100, 100).unwrap();
        assert_eq!(db.scrobble_counts("listenbrainz"), (0, 0));
    }

    #[test]
    fn queue_deduplicates_each_provider_play_and_expires_old_listens() {
        let db = Db::open(Path::new(":memory:")).unwrap();
        let account = account_key("lastfm", "a-secret-that-must-not-be-stored");
        for _ in 0..2 {
            db.enqueue_scrobble("lastfm", &account, "one-play", "{\"artist\":\"Band\"}", 100, 100)
                .unwrap();
        }
        assert_eq!(db.scrobble_counts("lastfm"), (1, 0));
        let pending = db.due_scrobbles(100).unwrap().pop().unwrap();
        assert_ne!(pending.account, "a-secret-that-must-not-be-stored");
        assert_eq!(pending.payload, "{\"artist\":\"Band\"}");
        assert!(db.due_scrobbles(100 + 14 * 24 * 60 * 60 + 1).unwrap().is_empty());
        assert_eq!(db.scrobble_counts("lastfm"), (0, 0));
    }

    #[test]
    fn queue_cap_discards_failed_rows_before_pending_listens() {
        let mut conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn.execute_batch("WITH RECURSIVE plays(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM plays WHERE n<10000) INSERT INTO scrobble_outbox(provider,account,event,payload,listened_at,next_attempt,failed) SELECT 'lastfm','account',n,'{}',100,100,n=5 FROM plays;").unwrap();
        enqueue(&mut conn, "listenbrainz", "other", "new", "{}", 100, 100).unwrap();
        let total: usize =
            conn.query_row("SELECT COUNT(*) FROM scrobble_outbox", [], |row| row.get(0)).unwrap();
        assert_eq!(total, 10_000);
        assert_eq!(counts(&conn, "lastfm").unwrap(), (9999, 0));
        assert_eq!(counts(&conn, "listenbrainz").unwrap(), (1, 0));
    }

    #[tokio::test]
    async fn a_transient_provider_failure_survives_restart_without_replaying_the_successful_provider(
    ) {
        let path =
            std::env::temp_dir().join(format!("limusic-outbox-{}.sqlite", rand::random::<u64>()));
        let accounts = [("lastfm", "session-a"), ("listenbrainz", "token-b")];
        let now = 1_800_000_000;
        {
            let db = Db::open(&path).unwrap();
            for (provider, credential) in accounts {
                db.enqueue_scrobble(
                    provider,
                    &account_key(provider, credential),
                    "play-1",
                    "{}",
                    now,
                    now,
                )
                .unwrap();
            }
            let mut requests = Vec::new();
            drain(&db, &accounts, now, |pending, _| {
                requests.push(pending.provider.clone());
                std::future::ready(if pending.provider == "lastfm" {
                    Delivery::Accepted
                } else {
                    Delivery::Retry
                })
            })
            .await
            .unwrap();
            assert_eq!(requests, ["lastfm", "listenbrainz"]);
            assert_eq!(db.scrobble_counts("lastfm"), (0, 0));
            assert_eq!(db.scrobble_counts("listenbrainz"), (1, 0));
        }
        let db = Db::open(&path).unwrap();
        let mut requests = Vec::new();
        drain(&db, &accounts, now + 31, |pending, _| {
            requests.push(pending.provider);
            std::future::ready(Delivery::Accepted)
        })
        .await
        .unwrap();
        assert_eq!(requests, ["listenbrainz"]);
        assert_eq!(db.scrobble_counts("listenbrainz"), (0, 0));
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn permanent_rejections_are_retained_as_failed_without_retrying() {
        let db = Db::open(Path::new(":memory:")).unwrap();
        db.enqueue_scrobble("lastfm", &account_key("lastfm", "key"), "play", "{}", 100, 100)
            .unwrap();
        let mut requests = 0;
        for now in [100, 200, 10_000] {
            drain(&db, &[("lastfm", "key")], now, |_, _| {
                requests += 1;
                std::future::ready(Delivery::Permanent)
            })
            .await
            .unwrap();
        }
        assert_eq!(requests, 1);
        assert_eq!(db.scrobble_counts("lastfm"), (0, 1));
    }

    #[tokio::test]
    async fn changed_credentials_never_receive_the_previous_accounts_pending_listens() {
        let db = Db::open(Path::new(":memory:")).unwrap();
        db.enqueue_scrobble(
            "listenbrainz",
            &account_key("listenbrainz", "old-token"),
            "play",
            "{}",
            100,
            100,
        )
        .unwrap();
        let mut requests = 0;
        drain(&db, &[("listenbrainz", "new-token")], 100, |_, _| {
            requests += 1;
            std::future::ready(Delivery::Accepted)
        })
        .await
        .unwrap();
        assert_eq!(requests, 0);
        assert_eq!(db.scrobble_counts("listenbrainz"), (0, 0));
    }

    #[tokio::test]
    async fn repeated_transient_failures_are_bounded_and_backoff_blocks_later_listens() {
        let db = Db::open(Path::new(":memory:")).unwrap();
        let account = account_key("lastfm", "key");
        db.enqueue_scrobble("lastfm", &account, "first", "{}", 100, 100).unwrap();
        db.enqueue_scrobble("lastfm", &account, "second", "{}", 101, 101).unwrap();
        let mut requests = 0;
        let mut now = 101;
        for _ in 0..20 {
            drain(&db, &[("lastfm", "key")], now, |pending, _| {
                assert_eq!(pending.payload, "{}");
                requests += 1;
                std::future::ready(Delivery::Retry)
            })
            .await
            .unwrap();
            assert!(
                db.due_scrobbles(now + 1).unwrap().is_empty()
                    || db.scrobble_counts("lastfm").1 == 1
            );
            now += 21_601;
        }
        assert_eq!(requests, 20);
        assert_eq!(db.scrobble_counts("lastfm"), (1, 1));
    }
}
