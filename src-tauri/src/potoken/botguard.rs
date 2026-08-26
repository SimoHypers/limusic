//! BotGuard minting on a dedicated thread, under `rustypipe-botguard` (deno_core + JSDOM).
//!
//! **Not a webview, deliberately.** googlevideo honours the pots built from a ~62-byte
//! `/GenerateIT` integrity token and rejects the ones built from a 65-66 byte token, and which
//! class you get is decided by the BotGuard snapshot. A minimal-DOM runtime lands in the accepted
//! class roughly one bootstrap in three; every real browser engine measured 0 in 25 (WebKitGTK,
//! headless Chromium, headed GPU Chromium, including an exact reproduction of Metrolist's Android
//! recipe). So we mint here instead, and re-roll the bootstrap until the class is right, which the
//! pot's own length tells us before it is ever attached to a URL. Evidence and the harness:
//! `progress/KNOWN-ISSUES.md` KI-1, with the harness in `progress/active/webremix-403-harness/`.
//!
//! `deno_core::JsRuntime` is `!Send`, so the runtime lives on its own thread and is driven over a
//! channel. Dropping the [`Minter`] closes that channel, which ends the thread and frees the V8
//! isolate: that is the whole teardown path, there is nothing else to shut down.

use std::sync::mpsc::{self, Sender};
use std::thread;

use base64::Engine;
use rustypipe_botguard::{Botguard, Error as BgError};

/// Bytes the minter wraps around `identifier + integrity token`. Measured with identifiers 11 and
/// ~520 bytes long: `pot_len == ident_len + integrity_token_len + 14`, in both token classes.
const POT_OVERHEAD: usize = 14;
/// Largest integrity token whose pots googlevideo accepts. The two observed classes are 61-62
/// (accepted) and 65-66 (rejected) with nothing in between, so the threshold sits in the gap.
const ACCEPTED_MAX_IT: usize = 63;
/// How many bootstraps to spend looking for the accepted class. At the measured ~1-in-3 rate this
/// misses about once in 25 launches; a miss keeps the last runtime and mints from it anyway.
const MAX_BOOTSTRAPS: usize = 8;
/// Identifier the class check mints against: plain ASCII, so its decoded byte length is its char
/// length and the integrity-token arithmetic is exact. visitorData is not usable for this — it
/// carries percent escapes, and reading the class off it came out a byte adrift of the truth.
const CLASS_PROBE: &str = "limusicprobe";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No JS runtime can be stood up at all: the thread would not spawn, or tokio would not build.
    /// Nothing will fix that at runtime, so the caller latches and degrades permanently (context/04
    /// §BadWebViewException, same policy as the old webview path). Constructed only in [`spawn`].
    #[error("botguard runtime: {0}")]
    Fatal(String),
    /// Network trouble, or a challenge we could not solve. Worth trying again on the next track.
    #[error("botguard: {0}")]
    Transient(String),
}

/// Everything BotGuard itself can fail at is transient, `BgError::Js` included: a JS error here
/// means the program YouTube shipped today threw, which is the case that fixes itself the next time
/// the program is downloaded. Latching on it (as the webview path did, where a JS error really did
/// mean a broken engine) left the app degraded to the non-PoToken clients until it was restarted,
/// and this app runs for days.
fn classify(e: BgError) -> Error {
    Error::Transient(e.to_string())
}

enum Cmd {
    Mint { ident: String, reply: tokio::sync::oneshot::Sender<Result<String, Error>> },
}

/// A live BotGuard runtime. Clone-free on purpose: one owner, and dropping it kills the thread.
pub struct Minter {
    tx: Sender<Cmd>,
}

/// What a successful bootstrap yields. The session token comes out of the bootstrap because the
/// class check needs a real mint anyway, so it costs nothing to keep it.
pub struct Bootstrap {
    pub minter: Minter,
    pub session_token: String,
    pub lifetime_secs: u64,
}

impl Minter {
    /// Stand up a BotGuard runtime bound to `session_ident` (visitorData), re-rolling the whole
    /// bootstrap while it lands in the rejected integrity-token class (see [`bootstrap`]).
    pub async fn spawn(user_agent: String, session_ident: String) -> Result<Bootstrap, Error> {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

        thread::Builder::new()
            .name("botguard".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(Error::Fatal(e.to_string())));
                        return;
                    }
                };
                let mut bg = match rt.block_on(bootstrap(&user_agent, &session_ident)) {
                    Ok((bg, token, lifetime_secs)) => {
                        if ready_tx.send(Ok((token, lifetime_secs))).is_err() {
                            return; // caller gave up (timeout) — don't hold a V8 isolate for nobody
                        }
                        bg
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                // Ends when the `Minter` is dropped and the channel closes.
                while let Ok(Cmd::Mint { ident, reply }) = rx.recv() {
                    let _ = reply.send(rt.block_on(bg.mint_token(&ident)).map_err(classify));
                }
            })
            .map_err(|e| Error::Fatal(e.to_string()))?;

        let (session_token, lifetime_secs) = ready_rx
            .await
            .map_err(|_| Error::Fatal("botguard thread stopped during bootstrap".into()))??;
        Ok(Bootstrap { minter: Minter { tx }, session_token, lifetime_secs })
    }

    /// Mint one PoToken for `ident` (a videoId, for the `&pot=` URL parameter).
    pub async fn mint(&self, ident: &str) -> Result<String, Error> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Cmd::Mint { ident: ident.to_owned(), reply })
            .map_err(|_| Error::Transient("botguard thread gone".into()))?;
        rx.await.map_err(|_| Error::Transient("botguard thread gone".into()))?
    }
}

/// `Botguard::init` + one session mint, repeated until the integrity token is in the class
/// googlevideo honours, or `MAX_BOOTSTRAPS` tries in which case the last runtime is used anyway.
///
/// No snapshot path is passed. `rustypipe-botguard` caches a solved runtime in a process-wide
/// `OnceLock`, so a snapshot would freeze whichever class it was written with for the rest of the
/// process and make this loop unable to re-roll.
// ponytail: one bootstrap per token lifetime (~12h) is cheap enough to not want the snapshot back.
async fn bootstrap(
    user_agent: &str,
    session_ident: &str,
) -> Result<(Botguard, String, u64), Error> {
    let mut last = None;
    for attempt in 1..=MAX_BOOTSTRAPS {
        // Drop the previous attempt's runtime before building the next: v8 asserts isolates are
        // dropped in reverse creation order (v8-130 isolate.rs:1666) and panics the whole thread
        // if two overlap, which kills the minter for the rest of the process.
        drop(last.take());
        let mut bg = Botguard::builder().user_agent(user_agent).init().await.map_err(classify)?;
        // Session token first, on a fresh minter, before any other identifier — Metrolist enforces
        // that ordering under a mutex and there is no reason to find out the hard way why.
        let token = bg.mint_token(session_ident).await.map_err(classify)?;
        let it =
            integrity_token_len(&bg.mint_token(CLASS_PROBE).await.map_err(classify)?, CLASS_PROBE);
        let lifetime = u64::from(bg.lifetime());
        if it.is_some_and(|n| n <= ACCEPTED_MAX_IT) {
            tracing::info!(
                attempt,
                it_bytes = it,
                ttl = lifetime,
                "PoToken minter ready (accepted integrity-token class)"
            );
            return Ok((bg, token, lifetime));
        }
        tracing::debug!(
            attempt,
            it_bytes = it,
            "botguard bootstrap in the rejected class, re-rolling"
        );
        last = Some((bg, token, lifetime, it));
    }
    // Keep the last one rather than degrade with nothing. A rejected-class token costs one HEAD
    // (the orchestrator validates every URL and falls through cleanly), and it might work: the
    // 61-62 / 65-66 byte split is one measurement session's, so if YouTube moves the wrapper by a
    // byte then every mint misclassifies and this warn is the only thing that says so.
    let (bg, token, lifetime, it) = last.expect("MAX_BOOTSTRAPS >= 1");
    tracing::warn!(
        it_bytes = it,
        ttl = lifetime,
        "no accepted-class integrity token in {MAX_BOOTSTRAPS} bootstraps, using a rejected-class \
         token (expect 403s; if this keeps happening the class arithmetic has drifted)"
    );
    Ok((bg, token, lifetime))
}

/// Recover the `/GenerateIT` integrity token's size from a minted pot, which is the only way to see
/// which class the runtime landed in. `None` if the pot does not parse or is implausibly short.
///
/// `mint_token` percent-decodes the identifier before binding it, so the length that counts is the
/// decoded one (visitorData routinely arrives with `%3D` padding).
fn integrity_token_len(pot: &str, ident: &str) -> Option<usize> {
    // `mint_token` returns padded base64url; the harness scripts print it unpadded. Take either.
    let bytes =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(pot.trim_end_matches('=')).ok()?;
    let ident_len = urlencoding::decode(ident).map_or(ident.len(), |d| d.len());
    bytes.len().checked_sub(ident_len + POT_OVERHEAD)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real tokens from `progress/active/webremix-403-harness`, both HEAD-tested against one live
    /// WEB_REMIX stream URL. If this arithmetic drifts, every mint is classified wrong and
    /// WEB_REMIX silently stops resolving again, so pin it to measured bytes.
    #[test]
    fn integrity_token_len_recovers_the_class() {
        // videoId-bound (11 chars), integrity token 62 bytes -> 87 byte pot -> HEAD 200.
        let accepted = "MlXqPDpAlDMkCOoCacp97o6hlNiTzyEqbbG19_M6xd4CsWjs_-As_87AhhsJc8P0TBrOBz-NyIZOkBW27jzq9IdgG3iWL8UIP2Z1mH_re9kA4X-E0VNg";
        // Same shape, integrity token 66 bytes -> 91 byte pot -> HEAD 403.
        let rejected = "MlnHbcTsx94ZOMcvkb-5qM8WsZx58c29FKoGcMaRe-7qyQbSAGmxZKrMAr-_WcwQ3Ayn6QcZJUyQwkDM71r6lMts9NLt3mH9kVN6Ni7xo7et0t-naMX8Jore2A";
        assert_eq!(integrity_token_len(accepted, "PtHEr7siapo"), Some(62));
        assert_eq!(integrity_token_len(rejected, "PtHEr7siapo"), Some(66));
        assert!(integrity_token_len(accepted, "PtHEr7siapo").is_some_and(|n| n <= ACCEPTED_MAX_IT));
        assert!(!integrity_token_len(rejected, "PtHEr7siapo").is_some_and(|n| n <= ACCEPTED_MAX_IT));
    }

    /// A JS error is "YouTube shipped new BotGuard today", not "this machine cannot run JS". If it
    /// classifies as `Fatal` again, `PoTokenGenerator` latches `runtime_bad` and the app stops
    /// sending PoTokens until it is restarted.
    #[test]
    fn js_errors_are_transient() {
        assert!(matches!(classify(BgError::Js("boom".into())), Error::Transient(_)));
        assert!(matches!(classify(BgError::InvalidPoToken("nope".into())), Error::Transient(_)));
    }

    #[test]
    fn integrity_token_len_rejects_garbage() {
        assert_eq!(integrity_token_len("not base64!!", "PtHEr7siapo"), None);
        assert_eq!(integrity_token_len("MlU=", "PtHEr7siapo"), None, "shorter than the overhead");
    }

    #[test]
    fn integrity_token_len_uses_the_decoded_identifier() {
        // visitorData carries `%3D` padding; binding uses the decoded bytes, so length must too.
        assert_eq!(
            integrity_token_len("MlXqPDpAlDMkCOoCacp97o6hlNiTzyEqbbG19_M6xd4CsWjs_-As_87AhhsJc8P0TBrOBz-NyIZOkBW27jzq9IdgG3iWL8UIP2Z1mH_re9kA4X-E0VNg", "abcdefgh%3D"),
            Some(64),
            "%3D is one byte, not three"
        );
    }
}

/// Live smoke test: stand up a real BotGuard runtime and mint a real pot. Hits YouTube, so it is
/// `#[ignore]`d and never runs in the default suite (CLAUDE.md: the extraction integration test is
/// invoked explicitly). Prints the pot so it can be HEAD-tested with
/// `progress/active/webremix-403-harness/pot_head_test.py`.
///
///   cargo test -p limusic-app botguard_mints_an_accepted_token -- --ignored --nocapture
#[cfg(test)]
mod live {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    #[ignore = "hits live YouTube"]
    async fn botguard_mints_an_accepted_token() {
        let visitor = std::env::var("LIMUSIC_VISITOR_DATA")
            .expect("set LIMUSIC_VISITOR_DATA to the app's visitor_data setting");
        let video_id = std::env::var("LIMUSIC_VIDEO_ID").unwrap_or_else(|_| "PtHEr7siapo".into());

        let t0 = Instant::now();
        let b = Minter::spawn(crate::http::WEB_UA.to_owned(), visitor.clone())
            .await
            .expect("bootstrap");
        let boot = t0.elapsed();
        let t1 = Instant::now();
        let pot = b.minter.mint(&video_id).await.expect("mint");
        let it = integrity_token_len(&pot, &video_id);
        println!(
            "bootstrap {boot:?} (ttl {}s) | mint {:?} it {it:?}\npot {pot}",
            b.lifetime_secs,
            t1.elapsed()
        );
        assert!(it.is_some_and(|n| n <= ACCEPTED_MAX_IT), "mint fell out of the accepted class");

        // The idle teardown drops the minter, which ends its thread and its V8 isolate; the next
        // track start builds a fresh one. Prove a second isolate can be created in the same process
        // after the first is gone, because that is what `teardown_if_idle` does all day.
        drop(b);
        let again = Minter::spawn(crate::http::WEB_UA.to_owned(), visitor)
            .await
            .expect("second bootstrap after teardown");
        let pot2 = again.minter.mint(&video_id).await.expect("mint after teardown");
        println!("after teardown: it {:?}", integrity_token_len(&pot2, &video_id));
        assert!(integrity_token_len(&pot2, &video_id).is_some_and(|n| n <= ACCEPTED_MAX_IT));
    }

    /// The orchestrator's self-heal drops the cached session token and leaves the minter alive, so
    /// the next `/player` call has to mint another one off that live runtime. Lives here rather
    /// than beside `PoTokenGenerator` because only a real runtime proves it.
    ///
    ///   cargo test -p limusic-app session_token_is_reminted -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "hits live YouTube"]
    async fn session_token_is_reminted_after_invalidation() {
        let visitor = std::env::var("LIMUSIC_VISITOR_DATA")
            .expect("set LIMUSIC_VISITOR_DATA to the app's visitor_data setting");
        let db =
            std::sync::Arc::new(crate::db::Db::open(std::path::Path::new(":memory:")).unwrap());
        let g = crate::potoken::PoTokenGenerator::new(db);

        assert!(g.get_session_po_token(&visitor).await.is_some(), "first mint");
        g.invalidate_session_token().await;
        let t1 = Instant::now();
        assert!(
            g.get_session_po_token(&visitor).await.is_some(),
            "no session token after invalidation: the minter is alive but nothing re-mints"
        );
        // A re-mint on a warm runtime is pure JS; a bootstrap is ~0.8s and up. If this creeps up,
        // the warm path is being missed and every self-heal costs a bootstrap on the hot path.
        println!("re-mint after invalidation: {:?}", t1.elapsed());
    }
}
