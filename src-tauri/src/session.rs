//! Login webview (context/15 Path A). Opens a visible Google sign-in window with a spoofed desktop
//! UA, watches for the redirect back to music.youtube.com, captures the resulting cookies, and
//! feeds them through the **same** sign-in path as cookie-paste (`AppState::sign_in`).
//!
//! Persistent (non-incognito) on purpose: the webview keeps its own Google session, so a later
//! re-login is one click with no password/paste — the real fix for KI-2 (cookie staleness), where
//! Google's short-lived `__Secure-*SIDTS` cookies rotate and a pasted cookie eventually stops
//! authenticating.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::webview::cookie::Cookie;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::state::{AppState, SignInOutcome};

const LOGIN_LABEL: &str = "login";
/// The hidden twin of [`LOGIN_LABEL`], used by [`refresh_session`]. Its own label so a refresh can
/// never collide with a sign-in window the user has open.
const REFRESH_LABEL: &str = "login-refresh";
/// Where the refresh webview goes. YTM itself, not the sign-in page: an already-signed-in jar
/// needs rotating, not a login.
const REFRESH_URL: &str = "https://music.youtube.com/";
/// How long to wait for that page to load before giving up on this attempt.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(45);
/// Google rotates roughly hourly, so anything under this is a burst of 401s from one dead cookie,
/// not a second thing to fix.
const REFRESH_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// WebKitGTK and WKWebView are WebKit engines, so a macOS Safari UA is the most
/// internally-consistent spoof and the least likely to trip Google's "this browser may not be
/// secure" block. **Tune here** if Google rejects it — this is the fragile part (context/15 Path A).
#[cfg(not(target_os = "windows"))]
const LOGIN_UA: Option<&str> = Some(
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
     (KHTML, like Gecko) Version/17.4.1 Safari/605.1.15",
);

/// Windows is WebView2, a Chromium engine, and overriding its UA does not touch the client hints it
/// sends: `Sec-CH-UA` still announces Edge and `Sec-CH-UA-Platform` still says Windows. A macOS
/// Safari UA therefore contradicts the request it rides on, and Google answers the login with
/// "This browser or app may not be secure" (#152). WebView2's own default UA is Edge's, which
/// agrees with those hints, so there is nothing to spoof here.
#[cfg(target_os = "windows")]
const LOGIN_UA: Option<&str> = None;

/// Google sign-in with `continue` back to YTM, so a successful login redirects to music.youtube.com
/// (our completion signal).
const LOGIN_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&continue=https://music.youtube.com/";

/// Open the login webview. Returns immediately; sign-in completes asynchronously (the UI learns via
/// the `auth-changed` event, or `login-error` on failure).
pub fn open_login(app: AppHandle, state: Arc<AppState>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    // When the webview lands on music.youtube.com, capture cookies + sign in. Runs off the
    // event-handler thread because reading the cookie store can deadlock when called synchronously
    // from inside a page-load callback.
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while rx.recv().await.is_some() {
                // The redirect that lands us here sets the youtube cookies; they may appear a beat
                // after the page finishes, so poll briefly.
                for _ in 0..6 {
                    let cookie = read_login_cookies(&app, LOGIN_LABEL).await;
                    if innertube::cookie_sapisid(&cookie).is_some() {
                        match state.sign_in(cookie).await {
                            Ok(SignInOutcome::Complete) => {
                                let _ = app.emit("login-done", ());
                            }
                            // The authenticated cookie is saved, but the account remains
                            // deliberately unfinished until the main-window picker selects a
                            // server-issued delegated identity.
                            Ok(SignInOutcome::SelectionRequired) => {}
                            Err(e) => {
                                let _ = app.emit("login-error", e);
                            }
                        }
                        close_login(&app);
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                // Landed on music.youtube.com but not authenticated yet — keep watching.
            }
        });
    }

    // Window creation must happen on the main thread (GTK).
    let app2 = app.clone();
    let dispatched = app.run_on_main_thread(move || {
        // Reclaim the label if a prior login window is still around.
        if let Some(w) = app2.get_webview_window(LOGIN_LABEL) {
            let _ = w.destroy();
        }
        let Ok(url) = tauri::Url::parse(LOGIN_URL) else { return };
        let builder = WebviewWindowBuilder::new(&app2, LOGIN_LABEL, WebviewUrl::External(url))
            .title("Sign in to YouTube Music")
            .inner_size(480.0, 720.0)
            .on_page_load(move |_w, payload| {
                if matches!(payload.event(), PageLoadEvent::Finished)
                    && payload.url().host_str() == Some("music.youtube.com")
                {
                    let _ = tx.send(());
                }
            });
        let builder = match LOGIN_UA {
            Some(ua) => builder.user_agent(ua),
            None => builder,
        };
        let res = builder.build();
        if let Err(e) = res {
            let _ = app2.emit("login-error", format!("Couldn't open the sign-in window: {e}"));
        }
    });
    if let Err(e) = dispatched {
        let _ = app.emit("login-error", format!("Couldn't open the sign-in window: {e}"));
    }
}

/// Re-mint the stored cookie from the login webview's own Google session, with no user
/// interaction. Issue #165 / KI-2.
///
/// The exported `Cookie` header is a snapshot; Google rotates `__Secure-*SIDTS` out from under it
/// and eventually rejects it, at which point every account-scoped call 401s and the UI's "Try
/// again" can only repeat the same dead request. But the webview jar is *not* a snapshot: it is
/// persistent and still holds the long-lived `__Secure-1PSID`, so loading music.youtube.com in it
/// gets fresh rotated cookies exactly the way opening the site in a browser does. Then we export
/// them again through the ordinary sign-in path.
///
/// Nothing is emitted on failure: the user is already looking at "sign in again", and a webview
/// session that has genuinely expired leaves them exactly where they were.
pub async fn refresh_session(app: AppHandle, state: Arc<AppState>) {
    if !state.it.is_logged_in() || !claim_refresh() {
        return;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let app2 = app.clone();
    let dispatched = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(REFRESH_LABEL) {
            let _ = w.destroy();
        }
        let Ok(url) = tauri::Url::parse(REFRESH_URL) else { return };
        // Not 1x1 like the cipher harness: this is a real page, and YouTube's own startup is what
        // makes Google hand out the rotated cookies.
        let builder = WebviewWindowBuilder::new(&app2, REFRESH_LABEL, WebviewUrl::External(url))
            .title("YouTube Music")
            .visible(false)
            .inner_size(1024.0, 768.0)
            .skip_taskbar(true)
            .decorations(false)
            .focused(false);
        let builder = match LOGIN_UA {
            Some(ua) => builder.user_agent(ua),
            None => builder,
        };
        let builder = builder.on_page_load(move |_w, payload| {
            if matches!(payload.event(), PageLoadEvent::Finished)
                && payload.url().host_str() == Some("music.youtube.com")
            {
                let _ = tx.send(());
            }
        });
        if let Err(e) = builder.build() {
            tracing::warn!(error = %e, "could not open the session-refresh webview");
        }
    });
    if dispatched.is_err() {
        return;
    }

    // `None` here is the build having failed (the sender was dropped), not a slow page.
    if !matches!(tokio::time::timeout(REFRESH_TIMEOUT, rx.recv()).await, Ok(Some(()))) {
        tracing::debug!("session refresh: the webview never reached music.youtube.com");
        close(&app, REFRESH_LABEL);
        return;
    }

    // Landing on the page and *having rotated* are not the same instant, so poll. Signing in with
    // a jar identical to the one we already hold would only spend two requests to be told the same
    // thing, so wait for a different one.
    let current = state.it.cookie().unwrap_or_default();
    for _ in 0..8 {
        let cookie = read_login_cookies(&app, REFRESH_LABEL).await;
        if innertube::cookie_sapisid(&cookie).is_some() && !same_jar(&cookie, &current) {
            match state.sign_in(cookie).await {
                Ok(_) => tracing::info!("re-minted the login session from the login webview"),
                Err(error) => tracing::warn!(%error, "could not re-mint the login session"),
            }
            close(&app, REFRESH_LABEL);
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    tracing::debug!("session refresh: the login webview had nothing fresher to offer");
    close(&app, REFRESH_LABEL);
}

/// Same cookies, whatever order they came out in. The stored header is rebuilt by two different
/// paths (the webview export sorts, the transport's rotation merge appends), so comparing the
/// strings would report a change on every launch.
fn same_jar(a: &str, b: &str) -> bool {
    let pairs = |s: &str| {
        let mut v: Vec<String> = s.split(';').map(|kv| kv.trim().to_owned()).collect();
        v.sort();
        v
    };
    pairs(a) == pairs(b)
}

/// One refresh at a time, and not more than one per [`REFRESH_COOLDOWN`]: a stale session throws
/// off a burst of 401s (nine of them in the report), and each is a signal to heal.
fn claim_refresh() -> bool {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    if last.is_some_and(|t| t.elapsed() < REFRESH_COOLDOWN) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

/// Merge the youtube-domain cookies into a `Cookie` header string. Reads the platform cookie store
/// (HttpOnly + secure included), matching what a browser sends to music.youtube.com.
///
/// Hops to the main thread: both backends drive their platform event loop while they wait for the
/// store (`gtk::main_iteration` on WebKitGTK, `NSRunLoop::mainRunLoop` on WKWebView), so they are
/// written to be called from the thread that owns it.
async fn read_login_cookies(app: &AppHandle, label: &'static str) -> String {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app2 = app.clone();
    if app
        .run_on_main_thread(move || {
            let _ = tx.send(youtube_cookies(&app2, label));
        })
        .is_err()
    {
        return String::new();
    }
    rx.await.unwrap_or_default()
}

fn youtube_cookies(app: &AppHandle, label: &str) -> String {
    let Some(wv) = app.get_webview_window(label) else { return String::new() };
    let Ok(cookies) = wv.cookies() else { return String::new() };
    youtube_cookie_header(cookies)
}

/// Domain-match by hand rather than with `cookies_for_url`: WKWebView's implementation compares the
/// cookie's domain to the URL's host with `==`, so YouTube's `.youtube.com` cookies never match
/// music.youtube.com and macOS got an empty jar (no SAPISID, so sign-in gave up silently).
/// WebKitGTK matches domains properly, which is why Linux never saw it.
///
/// Anything outside youtube.com is dropped, google.com cookies included: this becomes a `Cookie`
/// header sent to YouTube, and a cookie without a domain we recognise doesn't belong in it.
fn youtube_cookie_header(mut cookies: Vec<Cookie<'static>>) -> String {
    // `Cookie::domain()` has already stripped the leading dot. Sorting means the most specific
    // domain is inserted last and so wins a name collision, the way a browser resolves one.
    cookies.sort_by_key(|c| c.domain().unwrap_or_default().len());
    let mut jar = std::collections::BTreeMap::new();
    for c in cookies {
        let domain = c.domain().unwrap_or_default();
        if domain == "youtube.com" || domain.ends_with(".youtube.com") {
            jar.insert(c.name().to_string(), c.value().to_string());
        }
    }
    jar.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(s: &str) -> Cookie<'static> {
        Cookie::parse(s.to_string()).unwrap()
    }

    #[test]
    fn keeps_the_youtube_jar_and_drops_everything_else() {
        // `.youtube.com` is where the auth cookies actually live, and the domain WKWebView refuses
        // to match against music.youtube.com.
        let header = youtube_cookie_header(vec![
            cookie("SAPISID=abc; Domain=.youtube.com"),
            cookie("SID=def; Domain=.youtube.com"),
            cookie("VISITOR_INFO1_LIVE=xyz; Domain=music.youtube.com"),
            cookie("SAPISID=notthisone; Domain=.google.com"),
            cookie("nodomain=1"),
        ]);
        assert_eq!(header, "SAPISID=abc; SID=def; VISITOR_INFO1_LIVE=xyz");
        // The check open_login gates on: no SAPISID means sign-in silently gives up.
        assert_eq!(innertube::cookie_sapisid(&header), Some("abc"));
    }

    #[test]
    fn the_most_specific_domain_wins_a_name_collision() {
        let header = youtube_cookie_header(vec![
            cookie("PREF=broad; Domain=.youtube.com"),
            cookie("PREF=specific; Domain=music.youtube.com"),
        ]);
        assert_eq!(header, "PREF=specific");
    }

    #[test]
    fn a_reordered_jar_is_not_a_fresher_one() {
        // The webview export sorts and the transport's rotation merge appends, so the same
        // cookies routinely arrive in a different order. Calling that "changed" would re-sign-in
        // on every launch; missing a real rotation would leave the session dead.
        assert!(same_jar("SAPISID=a; SID=b", "SID=b; SAPISID=a"));
        assert!(!same_jar("SAPISID=a; __Secure-3PSIDTS=new", "SAPISID=a; __Secure-3PSIDTS=old"));
    }
}

fn close_login(app: &AppHandle) {
    close(app, LOGIN_LABEL)
}

fn close(app: &AppHandle, label: &'static str) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(label) {
            let _ = w.destroy();
        }
    });
}
