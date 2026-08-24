//! PoToken / BotGuard generator (context/04, context/13).
//!
//! Two tokens per session/video (context/04): a session token (minted from visitorData, cached in
//! Rust with its TTL — [`PoTokenGenerator::get_session_po_token`]) → the `/player` request body; a
//! per-video streaming token (minted from videoId, lazy —
//! [`PoTokenGenerator::get_streaming_po_token`]) → `&pot=` on the stream URL. Everything is wrapped
//! so a failure or timeout returns `None` and the orchestrator falls through to the non-PoToken
//! clients (graceful degradation — context/06 §5).
//!
//! BotGuard itself runs in [`botguard`], on `rustypipe-botguard` (deno_core + JSDOM), **not** in a
//! hidden webview. That is the 2026-08-25 fix for KI-1: googlevideo rejects every token a real
//! browser engine mints, whatever origin, engine, user agent or settings it is given, because
//! `/GenerateIT` hands that attestation an integrity token of the wrong class. See `botguard.rs`
//! and `progress/KNOWN-ISSUES.md` KI-1.

mod botguard;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::db::{now_secs, Db};
use crate::http::WEB_UA;

/// Overall budget for a full bootstrap. Generous because it covers up to `MAX_BOOTSTRAPS` BotGuard
/// solves (each one a `/Create`, a VM run and a `/GenerateIT`, measured at ~0.8s) while it hunts
/// for the accepted integrity-token class.
const MINT_BUDGET: Duration = Duration::from_secs(60);
/// Budget for one mint on an already-bootstrapped runtime. Pure JS, no network.
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
/// Safety margin before the integrity token's TTL (context/04 §GenerateIT).
const EXPIRY_MARGIN: Duration = Duration::from_secs(10 * 60);

struct Minter {
    session_id: String,
    expires_at: Instant,
    inner: botguard::Minter,
    last_used: Instant,
}

impl Minter {
    fn valid_for(&self, session_id: &str) -> bool {
        self.session_id == session_id && Instant::now() < self.expires_at
    }
}

/// Where the session token is persisted between runs. Internal: not in `UI_SETTINGS`, so the
/// webview can neither read nor write it.
const SESSION_TOKEN_KEY: &str = "potoken_session";

/// Cached session token (context/04: minted from visitorData, ~12h TTL). Lives OUTSIDE the
/// [`Minter`] so the mint-and-drop idle teardown doesn't force a full BotGuard bootstrap on the
/// next track start just to re-learn a string we already had.
///
/// Also survives the process, in `settings`. Google's own `/GenerateIT` says how long it is good
/// for (43200s, ~12h), and re-minting means standing up a V8 isolate and solving BotGuard several
/// times over to re-learn a string we were told stays valid until tomorrow. `expires_at` is
/// therefore wall-clock, not an `Instant`, which cannot outlive the process.
#[derive(serde::Serialize, serde::Deserialize)]
struct SessionToken {
    session_id: String,
    token: String,
    expires_at: i64,
}

impl SessionToken {
    fn valid_for(&self, session_id: &str, now: i64) -> bool {
        self.session_id == session_id && now < self.expires_at
    }
}

pub struct PoTokenGenerator {
    db: Arc<Db>,
    minter: Mutex<Option<Minter>>,
    /// Session token cache — see [`SessionToken`].
    session_token: Mutex<Option<SessionToken>>,
    /// Latched once the JS runtime proves unusable — thereafter always degrade to the non-PoToken
    /// clients (context/04 §BadWebViewException).
    runtime_bad: AtomicBool,
}

impl PoTokenGenerator {
    pub fn new(db: Arc<Db>) -> Self {
        // A token stored by a previous run is as good as one minted now, right up to its expiry.
        // A wrong-session or expired one is simply never returned by `cached_session_token`, so it
        // costs nothing to load it optimistically and let the normal validity check reject it.
        let stored: Option<SessionToken> =
            db.get_setting(SESSION_TOKEN_KEY).and_then(|raw| serde_json::from_str(&raw).ok());
        if let Some(t) = &stored {
            tracing::debug!(
                expires_in = t.expires_at - now_secs(),
                "loaded stored PoToken session"
            );
        }
        PoTokenGenerator {
            db,
            minter: Mutex::new(None),
            session_token: Mutex::new(stored),
            runtime_bad: AtomicBool::new(false),
        }
    }

    /// Forget the session token, in memory and on disk.
    ///
    /// Called when a web-client stream is rejected (the orchestrator's off-hot-path self-heal).
    /// Without this a token that Google stopped honouring early would be replayed for the rest of
    /// its nominal 12 hours instead of only until the next launch.
    pub async fn invalidate_session_token(&self) {
        *self.session_token.lock().await = None;
        self.db.delete_setting(SESSION_TOKEN_KEY);
    }

    /// Session token for the `/player` request body (context/04). Cheap when cached; otherwise
    /// performs the full bootstrap (and leaves the minter warm for streaming-token mints).
    pub async fn get_session_po_token(&self, visitor_data: &str) -> Option<String> {
        if self.runtime_bad.load(Ordering::SeqCst) {
            return None;
        }
        if let Some(token) = self.cached_session_token(visitor_data).await {
            return Some(token);
        }
        match timeout(MINT_BUDGET, self.ensure_minter(visitor_data)).await {
            Ok(Ok(_guard)) => self.cached_session_token(visitor_data).await,
            Ok(Err(e)) => {
                self.on_failure("session", &e).await;
                None
            }
            Err(_) => {
                tracing::warn!("PoToken session mint timed out — degrading");
                self.teardown().await;
                None
            }
        }
    }

    /// Per-video streaming token for the `&pot=` URL param (context/04). Builds/reuses the
    /// minter; call ONLY when a web-client stream URL actually resolved (post-decipher).
    pub async fn get_streaming_po_token(
        &self,
        video_id: &str,
        visitor_data: &str,
    ) -> Option<String> {
        if self.runtime_bad.load(Ordering::SeqCst) {
            return None;
        }
        match timeout(MINT_BUDGET, self.mint_streaming(video_id, visitor_data)).await {
            Ok(Ok(pot)) => Some(pot),
            Ok(Err(e)) => {
                self.on_failure(video_id, &e).await;
                None
            }
            Err(_) => {
                tracing::warn!(video_id, "PoToken streaming mint timed out — degrading");
                self.teardown().await;
                None
            }
        }
    }

    async fn on_failure(&self, what: &str, e: &botguard::Error) {
        tracing::warn!(what, error = %e, "PoToken mint failed — degrading");
        if matches!(e, botguard::Error::Fatal(_)) {
            self.runtime_bad.store(true, Ordering::SeqCst);
        }
        self.teardown().await;
    }

    async fn cached_session_token(&self, visitor_data: &str) -> Option<String> {
        let now = now_secs();
        self.session_token
            .lock()
            .await
            .as_ref()
            .filter(|t| t.valid_for(visitor_data, now))
            .map(|t| t.token.clone())
    }

    /// Ensure `self.minter` holds a valid minter for `visitor_data`, (re)building it (which also
    /// refreshes the cached session token) if needed. Returns the locked guard so callers needing
    /// the minter (streaming mint) can keep using it without a re-lock race.
    async fn ensure_minter<'a>(
        &'a self,
        visitor_data: &str,
    ) -> Result<tokio::sync::MutexGuard<'a, Option<Minter>>, botguard::Error> {
        let mut guard = self.minter.lock().await;
        if !guard.as_ref().is_some_and(|m| m.valid_for(visitor_data)) {
            *guard = Some(self.create_minter(visitor_data).await?);
        }
        Ok(guard)
    }

    /// Per-video token (identifier = videoId). One retry with a fresh minter on failure.
    async fn mint_streaming(
        &self,
        video_id: &str,
        visitor_data: &str,
    ) -> Result<String, botguard::Error> {
        let mut guard = self.ensure_minter(visitor_data).await?;
        let minter = guard.as_mut().expect("minter present");
        minter.last_used = Instant::now();
        match timeout(CALL_TIMEOUT, minter.inner.mint(video_id)).await {
            Ok(Ok(pot)) => Ok(pot),
            other => {
                let why = match other {
                    Ok(Err(e)) => e.to_string(),
                    _ => "mint timed out".to_owned(),
                };
                tracing::debug!(error = why, "per-video mint failed, rebuilding minter once");
                let fresh = self.create_minter(visitor_data).await?;
                let pot = timeout(CALL_TIMEOUT, fresh.inner.mint(video_id))
                    .await
                    .map_err(|_| botguard::Error::Transient("mint timed out".into()))??;
                *guard = Some(fresh);
                Ok(pot)
            }
        }
    }

    /// Full bootstrap: a BotGuard runtime that landed in the accepted integrity-token class, plus
    /// the session token it minted on the way (see [`botguard::Minter::spawn`]).
    async fn create_minter(&self, session_id: &str) -> Result<Minter, botguard::Error> {
        let b = botguard::Minter::spawn(WEB_UA.to_owned(), session_id.to_owned()).await?;
        // One lifetime, two clocks: the minter never outlives this process, so its expiry stays an
        // `Instant`, while the session token is written to disk and needs wall-clock.
        let good_for = Duration::from_secs(b.lifetime_secs).saturating_sub(EXPIRY_MARGIN);

        let token = SessionToken {
            session_id: session_id.to_owned(),
            token: b.session_token,
            expires_at: now_secs() + good_for.as_secs() as i64,
        };
        if let Ok(json) = serde_json::to_string(&token) {
            self.db.set_setting(SESSION_TOKEN_KEY, &json);
        }
        *self.session_token.lock().await = Some(token);

        Ok(Minter {
            session_id: session_id.to_owned(),
            expires_at: Instant::now() + good_for,
            inner: b.minter,
            last_used: Instant::now(),
        })
    }

    /// Warm the minter for `visitor_data` (context/04 §startup). Non-fatal.
    pub async fn prewarm(&self, visitor_data: &str) {
        if self.runtime_bad.load(Ordering::SeqCst) {
            return;
        }
        // The only reason to prewarm is to have the session token ready before the first /player
        // call, and one stored by a previous run is exactly that. The per-video streaming path
        // builds its own minter on demand if it ever needs one, so there is nothing else to warm.
        if self.cached_session_token(visitor_data).await.is_some() {
            tracing::info!("PoToken session token still valid — skipping the BotGuard bootstrap");
            return;
        }
        let mut guard = self.minter.lock().await;
        if guard.as_ref().is_some_and(|m| m.valid_for(visitor_data)) {
            return;
        }
        match timeout(MINT_BUDGET, self.create_minter(visitor_data)).await {
            Ok(Ok(m)) => *guard = Some(m),
            Ok(Err(e)) => tracing::warn!(error = %e, "PoToken prewarm failed"),
            Err(_) => tracing::warn!("PoToken prewarm timed out"),
        }
    }

    /// Drop the runtime if it's been idle longer than `idle` — the mint-and-drop memory policy
    /// (Phase-0 decision): keep it warm while the queue mints, drop it when idle. Dropping the
    /// minter ends its thread and frees the V8 isolate.
    // ponytail: called from a periodic task in lib.rs; no self-spawned monitor.
    pub async fn teardown_if_idle(&self, idle: Duration) {
        let mut guard = self.minter.lock().await;
        if guard.as_ref().is_some_and(|m| m.last_used.elapsed() >= idle) {
            *guard = None;
            tracing::debug!("BotGuard runtime torn down (idle)");
        }
    }

    async fn teardown(&self) {
        *self.minter.lock().await = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(session_id: &str, expires_at: i64) -> SessionToken {
        SessionToken { session_id: session_id.to_owned(), token: "tok".to_owned(), expires_at }
    }

    #[test]
    fn session_token_valid_for_matching_id_and_future_expiry() {
        assert!(token("vd123", 1_000).valid_for("vd123", 900));
    }

    #[test]
    fn session_token_invalid_for_wrong_id() {
        // Signing in or out changes visitorData, so a token minted for the old one must not be
        // replayed against the new one.
        assert!(!token("vd123", 1_000).valid_for("other", 900));
    }

    #[test]
    fn session_token_invalid_when_expired() {
        assert!(!token("vd123", 1_000).valid_for("vd123", 1_000), "expiry is exclusive");
        assert!(!token("vd123", 1_000).valid_for("vd123", 1_001));
    }

    #[test]
    fn session_token_survives_a_round_trip_through_settings() {
        // What is stored between runs. A shape change here silently turns every launch back into
        // a full BotGuard bootstrap (the load is a best-effort `ok()`), so pin it.
        let json = serde_json::to_string(&token("vd123", 4_000)).unwrap();
        let back: SessionToken = serde_json::from_str(&json).unwrap();
        assert!(back.valid_for("vd123", 3_999));
        assert_eq!(back.token, "tok");

        // Anything unreadable must read as "no token" rather than panic the constructor.
        assert!(serde_json::from_str::<SessionToken>("{}").is_err());
        assert!(serde_json::from_str::<SessionToken>("not json").is_err());
    }
}
