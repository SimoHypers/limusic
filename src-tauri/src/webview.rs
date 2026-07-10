//! Shared hidden-webview bridge for the cipher + PoToken JS runtimes. context/13.
//!
//! Mechanics pinned against the installed tauri 2.11 / wry 0.55 source:
//! - **Rust→JS→Rust:** [`WebviewWindow::eval_with_callback`] delivers the JSON of a *synchronous*
//!   JS value. wry's WebKitGTK `run_javascript` does NOT await promises, so synchronous calls
//!   (cipher sig/n) round-trip in one shot; async work (BotGuard) stores its result into a JS
//!   global that we poll ([`Bridge::call_async`]).
//! - **Handles:** fetched fresh via `get_webview_window` per call and dropped before any `.await`,
//!   so we never hold a non-`Send` handle across a suspension point.
//! - **Lifecycle:** create/destroy run on the main thread via `run_on_main_thread` (GTK requires
//!   it). Callers must invoke `create` from a spawned task, not from `setup()` — the event loop
//!   isn't pumping yet during setup and the closure would never run.
//! - **Harness HTML:** loaded as a `data:text/html` URL (CustomProtocol). The app CSP is `null`
//!   so nothing is injected; BotGuard's dynamic eval runs unhindered (proven in botguard-spike).

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("webview '{0}' does not exist")]
    Gone(String),
    #[error("webview eval failed: {0}")]
    Eval(String),
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    /// Uncaught JS error in the harness → treat the webview as bad (context/04 BadWebViewException).
    #[error("webview reported a JS error: {0}")]
    BadWebview(String),
    #[error("build failed: {0}")]
    Build(String),
}

/// A handle to one hidden webview, identified by its Tauri window label.
#[derive(Clone)]
pub struct Bridge {
    app: AppHandle,
    label: String,
}

impl Bridge {
    /// Create a hidden webview from a raw HTML harness + an initialization script (runs before the
    /// page's own scripts). Resolves once the page has finished loading (Tauri `on_page_load`) AND
    /// a probe confirms JS↔Rust round-trips. On any failure the just-built window is destroyed so
    /// its label can be reused (no orphan "already exists").
    pub async fn create(
        app: &AppHandle,
        label: &str,
        harness_html: &str,
        init_script: &str,
    ) -> Result<Bridge, Error> {
        // Reclaim the label if a prior attempt left an orphan (or a concurrent build raced us).
        destroy_and_wait(app, label).await;

        // Preamble every harness gets: uncaught-error capture (for BadWebview) + a slots bag.
        let init = format!(
            "window.__jserr=null;window.__slots={{}};\
             window.addEventListener('error',function(e){{window.__jserr=String((e&&e.message)||e);}});\
             window.onunhandledrejection=function(e){{window.__jserr=String((e.reason&&e.reason.message)||e.reason);}};\n{init_script}"
        );
        let data_url = format!("data:text/html,{}", urlencoding::encode(harness_html));
        let url = tauri::Url::parse(&data_url).map_err(|e| Error::Build(e.to_string()))?;

        // Page-load signal from the runtime (`on_page_load` → Finished). It's the fast path on
        // WebKitGTK; on WebView2 it is unreliable for the initial `data:` harness (loaded as
        // NavigateToString, whose NavigationCompleted often never fires — WebView2Feedback #998),
        // so it's an accelerator here, not the gate. The real readiness gate is an eval round-trip.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let ready_slot = Arc::new(StdMutex::new(Some(ready_tx)));

        let (built_tx, built_rx) = tokio::sync::oneshot::channel();
        let app2 = app.clone();
        let label2 = label.to_string();
        let ready_slot2 = ready_slot.clone();
        app.run_on_main_thread(move || {
            let res = WebviewWindowBuilder::new(&app2, label2, WebviewUrl::CustomProtocol(url))
                .visible(false)
                .inner_size(1.0, 1.0)
                .skip_taskbar(true)
                .decorations(false)
                .focused(false)
                .initialization_script(init)
                .on_page_load(move |_wv, payload| {
                    if matches!(payload.event(), PageLoadEvent::Finished) {
                        if let Some(tx) = ready_slot2.lock().unwrap().take() {
                            let _ = tx.send(());
                        }
                    }
                })
                .build();
            let _ = built_tx.send(res.map(|_| ()).map_err(|e| e.to_string()));
        })
        .map_err(|e| Error::Build(e.to_string()))?;

        built_rx
            .await
            .map_err(|_| Error::Build("main-thread create dropped".into()))?
            .map_err(Error::Build)?;

        let bridge = Bridge { app: app.clone(), label: label.to_string() };

        // Give the page-load event a brief chance (WebKitGTK fires it in ~tens of ms, so no evals
        // are issued before load there); if it doesn't arrive, fall through to probing JS directly
        // — a hidden WebView2 runs JS even when NavigationCompleted never fires. This is the fix
        // that makes the cipher/PoToken webviews work on Windows.
        tokio::select! {
            _ = ready_rx => tracing::info!(label, "webview page loaded"),
            _ = tokio::time::sleep(Duration::from_secs(1)) =>
                tracing::debug!(label, "no page-load event within 1s — probing JS directly"),
        }

        // Real readiness gate: poll until JS confirms our harness document is actually loaded.
        // `location.protocol==='data:'` is true only once the `data:` harness is live — it stays
        // false on the `about:blank` a fresh WebView2 sits on, so (unlike a plain `1+1`, which would
        // pass on `about:blank` too) this proves BOTH the JS↔Rust round-trip works AND the right
        // document loaded. WebView2 misses the load *event*, not the load itself; on WebKitGTK the
        // same data URL is loaded via `load_uri`, so the check holds there too (no Linux regression).
        // Short per-attempt timeout so an eval whose callback is dropped pre-load (WebKitGTK quirk)
        // retries instead of stalling. On timeout the window exists but is unusable — destroy it.
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let probe = bridge
                .eval_json("location.protocol==='data:'".into(), Duration::from_millis(800))
                .await;
            if matches!(probe, Ok(Value::Bool(true))) {
                tracing::info!(label, "webview bridge OK — harness loaded, eval round-trips");
                return Ok(bridge);
            }
            if Instant::now() >= deadline {
                tracing::error!(label, "webview never became ready — harness never loaded / JS never round-tripped");
                let _ = bridge.destroy();
                return Err(Error::Timeout(Duration::from_secs(12)));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    /// True if the underlying webview window still exists.
    pub fn exists(&self) -> bool {
        self.app.get_webview_window(&self.label).is_some()
    }

    /// Fire-and-forget JS (no result awaited). Used to kick off async work.
    pub fn eval(&self, js: &str) -> Result<(), Error> {
        let wv = self
            .app
            .get_webview_window(&self.label)
            .ok_or_else(|| Error::Gone(self.label.clone()))?;
        wv.eval(js).map_err(|e| Error::Eval(e.to_string()))
    }

    /// Evaluate a **synchronous** JS expression and return the JSON of its value. `Value::Null`
    /// when the expression is null/undefined.
    pub async fn eval_json(&self, js: String, timeout: Duration) -> Result<Value, Error> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let slot: Arc<StdMutex<Option<_>>> = Arc::new(StdMutex::new(Some(tx)));
        {
            let wv = self
                .app
                .get_webview_window(&self.label)
                .ok_or_else(|| Error::Gone(self.label.clone()))?;
            let slot2 = slot.clone();
            wv.eval_with_callback(js, move |json| {
                if let Some(tx) = slot2.lock().unwrap().take() {
                    let _ = tx.send(json);
                }
            })
            .map_err(|e| Error::Eval(e.to_string()))?;
            // wv dropped here — never held across the await below.
        }
        let raw = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| Error::Timeout(timeout))?
            .map_err(|_| Error::Eval("callback dropped".into()))?;
        if raw.is_empty() || raw == "null" {
            return Ok(Value::Null);
        }
        serde_json::from_str(&raw).map_err(|e| Error::Eval(format!("bad json {raw:?}: {e}")))
    }

    /// Poll a synchronous JS expression until it evaluates non-null, or time out. Also fails fast
    /// if the harness recorded an uncaught JS error. An individual eval that times out or errors is
    /// treated as "not ready yet" — only the outer `timeout` aborts the poll.
    async fn poll_json(&self, expr: &str, timeout: Duration) -> Result<Value, Error> {
        let deadline = Instant::now() + timeout;
        let step = Duration::from_millis(100);
        loop {
            match self.eval_json(format!("({expr})"), Duration::from_secs(3)).await {
                Ok(v) if !v.is_null() => return Ok(v),
                Ok(_) => {} // null → still pending
                Err(Error::Gone(l)) => return Err(Error::Gone(l)), // window died — stop
                Err(_) => {} // a transient eval timeout/error → keep polling until the deadline
            }
            if let Ok(Some(msg)) = self
                .eval_json("window.__jserr||null".into(), Duration::from_secs(3))
                .await
                .map(|v| v.as_str().map(str::to_owned))
            {
                return Err(Error::BadWebview(msg));
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout(timeout));
            }
            tokio::time::sleep(step).await;
        }
    }

    /// Run an **async** JS expression: `await (expr)`, capturing the resolved value or error into a
    /// slot we then poll. This is how BotGuard's promise-returning functions bridge back to Rust.
    pub async fn call_async(&self, expr: &str, timeout: Duration) -> Result<Value, Error> {
        let id = next_call_id();
        let kick = format!(
            "(async()=>{{try{{window.__slots['{id}']={{ok:1,v:await ({expr})}};}}\
             catch(e){{window.__slots['{id}']={{ok:0,e:String((e&&e.message)||e)}};}}}})();"
        );
        self.eval(&kick)?;
        let slot = self.poll_json(&format!("window.__slots['{id}']||null"), timeout).await?;
        // Free the slot regardless of outcome.
        let _ = self.eval(&format!("delete window.__slots['{id}'];"));
        if slot.get("ok").and_then(Value::as_i64) == Some(1) {
            Ok(slot.get("v").cloned().unwrap_or(Value::Null))
        } else {
            Err(Error::Eval(
                slot.get("e").and_then(Value::as_str).unwrap_or("async call failed").to_owned(),
            ))
        }
    }

    /// Destroy the webview (on the main thread). Idempotent.
    pub fn destroy(&self) -> Result<(), Error> {
        if let Some(wv) = self.app.get_webview_window(&self.label) {
            let _ = self.app.run_on_main_thread(move || {
                let _ = wv.destroy();
            });
        }
        Ok(())
    }
}

fn next_call_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// Destroy any existing webview with `label` and wait until its label is actually free. `destroy`
/// dispatches to the event loop, so we poll `get_webview_window` until it's `None` (or give up).
/// Prevents the "a webview with label X already exists" collision on a re-create.
pub(crate) async fn destroy_and_wait(app: &AppHandle, label: &str) {
    let Some(wv) = app.get_webview_window(label) else { return };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let _ = app.run_on_main_thread(move || {
        let _ = wv.destroy();
        let _ = tx.send(());
    });
    let _ = rx.await;
    for _ in 0..40 {
        if app.get_webview_window(label).is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tracing::warn!(label, "webview label still present after destroy — create may collide");
}
