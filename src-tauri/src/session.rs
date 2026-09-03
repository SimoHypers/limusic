//! Login webview (context/15 Path A). Opens a visible Google sign-in window with a spoofed desktop
//! UA, watches for the redirect back to music.youtube.com, captures the resulting cookies, and
//! feeds them through the **same** sign-in path as cookie-paste (`AppState::sign_in`).
//!
//! Persistent (non-incognito) on purpose: the webview keeps its own Google session, so a later
//! re-login is one click with no password/paste — the real fix for KI-2 (cookie staleness), where
//! Google's short-lived `__Secure-*SIDTS` cookies rotate and a pasted cookie eventually stops
//! authenticating.

use std::sync::Arc;
use std::time::Duration;

use tauri::webview::cookie::Cookie;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::state::{AppState, SignInOutcome};

const LOGIN_LABEL: &str = "login";

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

/// Google's own "Add account" entry point (the one their account picker's "Add account" links
/// to). Unlike ServiceLogin it presents the fresh sign-in screen even when the webview already
/// holds a Google session, which is what lets a second account be added without signing the
/// first one out of the webview. Verified: it renders the identifier screen with no session.
const ADD_ACCOUNT_URL: &str =
    "https://accounts.google.com/AddSession?service=youtube&continue=https://music.youtube.com/";

/// Open the login webview. Returns immediately; sign-in completes asynchronously (the UI learns via
/// the `auth-changed` event, or `login-error` on failure). `add_account` picks the AddSession
/// flow so an additional Google account can be added while another is signed in.
pub fn open_login(app: AppHandle, state: Arc<AppState>, add_account: bool) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    // When the webview lands on music.youtube.com, capture cookies + sign in. Runs off the
    // event-handler thread because reading the cookie store can deadlock when called synchronously
    // from inside a page-load callback.
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while rx.recv().await.is_some() {
                // The redirect that lands us here sets the youtube cookies; they may appear a beat
                // after the page finishes, and the YTM app the window loads keeps setting more of
                // them (Google's rotating `__Secure-*SIDTS` tokens arrive via its first requests).
                // Wait for a SAPISID-bearing jar and keep reading while it still changes; the
                // moment two consecutive reads agree, the redirect is settled and that jar is the
                // one to sign in with. No fixed 4-second tail: the transport rolls any remaining
                // tokens itself on its first authenticated responses (KI-2).
                let mut last: Option<String> = None;
                let mut settled: Option<String> = None;
                for _ in 0..8 {
                    let cookie = read_login_cookies(&app).await;
                    if innertube::cookie_sapisid(&cookie).is_some() {
                        if last.as_deref() == Some(cookie.as_str()) {
                            settled = Some(cookie);
                            break;
                        }
                        last = Some(cookie);
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                if let Some(cookie) = settled.or(last) {
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
        // Add-account flow: sign the *webview* out of Google first, so the fresh sign-in is the
        // webview's only session and music.youtube.com lands on the new account instead of
        // auto-continuing into whichever one it was already holding. The app's own saved sessions
        // live in SQLite (`accounts`), not in this cookie store — they are never touched here.
        if add_account {
            clear_google_webview_cookies(&app2);
        }
        let url = if add_account { ADD_ACCOUNT_URL } else { LOGIN_URL };
        let Ok(url) = tauri::Url::parse(url) else { return };
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

/// Delete every google.com / youtube.com cookie from the webview's cookie store (one shared store
/// for all of the app's webviews). Only the webview-side Google session is lost: the app's saved
/// accounts are SQLite rows in `Db`, and the main window never sends YouTube cookies itself.
/// Reads through the main window's webview — always alive, and the store is profile-wide. That
/// sharing is verified on WebView2; if a WebKitGTK build ever shows the old wrong-account
/// behaviour again, this is the first suspect.
fn clear_google_webview_cookies(app: &AppHandle) {
    let Some(wv) = app.get_webview_window("main") else { return };
    let Ok(cookies) = wv.cookies() else { return };
    let mut cleared = 0;
    for cookie in cookies {
        let domain = cookie.domain().unwrap_or_default();
        let google_or_youtube = domain == "google.com"
            || domain.ends_with(".google.com")
            || domain == "youtube.com"
            || domain.ends_with(".youtube.com");
        if google_or_youtube && wv.delete_cookie(cookie).is_ok() {
            cleared += 1;
        }
    }
    tracing::info!(cleared, "cleared the webview's Google session for add-account sign-in");
}

/// Merge the youtube-domain cookies into a `Cookie` header string. Reads the platform cookie store
/// (HttpOnly + secure included), matching what a browser sends to music.youtube.com.
///
/// Hops to the main thread: both backends drive their platform event loop while they wait for the
/// store (`gtk::main_iteration` on WebKitGTK, `NSRunLoop::mainRunLoop` on WKWebView), so they are
/// written to be called from the thread that owns it.
async fn read_login_cookies(app: &AppHandle) -> String {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app2 = app.clone();
    if app
        .run_on_main_thread(move || {
            let _ = tx.send(youtube_cookies(&app2));
        })
        .is_err()
    {
        return String::new();
    }
    rx.await.unwrap_or_default()
}

fn youtube_cookies(app: &AppHandle) -> String {
    let Some(wv) = app.get_webview_window(LOGIN_LABEL) else { return String::new() };
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
}

fn close_login(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(LOGIN_LABEL) {
            let _ = w.destroy();
        }
    });
}
