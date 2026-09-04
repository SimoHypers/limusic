# Sign-in sessions: where the data lives

Everything below is per-OS under Tauri's app-data dir for identifier `com.limusic.desktop`
(`tauri.conf.json`), created in `lib.rs` via `app.path().app_data_dir()`:

| OS | Directory |
|---|---|
| Windows | `%APPDATA%\com.limusic.desktop\` |
| Linux | `~/.local/share/com.limusic.desktop/` |
| macOS | `~/Library/Application Support/com.limusic.desktop/` |

## 1. SQLite — `limusic.sqlite`

The canonical store of your saved Google accounts and the active session. Schema: `src-tauri/src/db.rs`.

### `accounts` table (multi-account, canonical)

| Column | Contents |
|---|---|
| `id` | Opaque account key: `ga-<sha1>` (versioned SHA-1, hex-encoded) over the cookie's long-lived `SAPISID` value (`db::account_key`). One row per Google account. The digest is deliberately not `DefaultHasher`, whose output Rust does not guarantee across releases. |
| `session_cookie` | The full `Cookie:` header captured at sign-in (SAPISID, SID, `__Secure-*`, …) — the actual login credential. |
| `data_sync_id` | Server-issued delegated identity of the selected YouTube channel within this Google account. |
| `selected_identity_json` | Canonical account model: name, handle, email, thumbnail, channelId, data_sync_id. `NULL` = login authenticated but a multi-channel pick is still pending. |
| `account_json` | The JSON shape the UI consumes (`signedIn`, `name`, …). |
| `visitor_data` | Login-bound visitorData for this account. |
| `added_at` | Unix seconds; the account menu lists oldest first (re-logins don't reorder). |

### `settings` rows (active-account projections + legacy keys)

The active account is projected into these keys so the startup bootstrap in `lib.rs`,
`AppState::account_snapshot`, and the channel switcher keep working unchanged:

| Key | Contents |
|---|---|
| `active_account` | `id` of the active account; absent = signed out (guest). |
| `session_cookie` | Cookie of the active account only. |
| `selected_identity_json` | Active account's identity model. |
| `data_sync_id`, `account_json` | Legacy projections, written atomically with the above (`Db::set_auth_identity`). |
| `account_selection_pending` | `"true"` while a multi-channel login awaits a pick. |
| `visitor_data` | Active session's visitorData (anonymous bootstrap id until first login). |

Databases from before multi-account are migrated once on open (`Db::open`): the legacy
`session_cookie` row becomes an `accounts` row and `active_account` is set. Rows still carrying a
pre-sha1 account key are rekeyed on open too, with `active_account` following the move.

## 2. Webview cookie store — the login window's own Google session

The sign-in webview (`src-tauri/src/session.rs`) is persistent (non-incognito) on purpose. Its
cookies live in the OS webview profile data *next to* the app data dir, not in the SQLite file:

- **Windows (WebView2):** user-data folder under `%LOCALAPPDATA%\com.limusic.desktop\EBWebView\`
- **Linux (WebKitGTK):** `~/.local/share/com.limusic.desktop/` webkit data (or XDG cache)
- **macOS (WKWebView):** inside `~/Library/Application Support/com.limusic.desktop/` WebKit data

This is why a re-login is one click with no password/paste, and why deleting `limusic.sqlite`
alone does not sign the webview out of Google. Limusic itself never reads this store as state —
`session.rs` copies the youtube-domain cookies out of it into the Cookie header after each login
and then stores that copy in SQLite. The one deliberate exception: the **Add account** flow first
deletes every google.com/youtube.com cookie from this store (webview-side sign-out only — the
SQLite sessions are untouched), so a fresh sign-in lands on the account just entered instead of
auto-continuing into whatever session the webview was already holding.

## 3. What never leaves Rust

Auth material (`session_cookie`, `selected_identity_json`, `data_sync_id`, `account_json`,
`visitor_data`, `account_selection_pending`) is excluded from the `get_settings` whitelist
(`commands.rs::UI_SETTINGS`), so the renderer can't read or overwrite it. The UI only ever sees
display fields plus opaque selectors (`id`, `selectionKey`).

## Flow summary

- **Sign in** (`login_webview`): ServiceLogin webview → redirect to music.youtube.com → cookies
  captured (polling for a few seconds so the jar includes the rotation tokens the YTM page's first
  requests set) → validated via `account_menu` → `accounts` row upserted (keyed on SAPISID; a new
  Google account adds a row, the same one refreshes its row) → `active_account` set → `auth-changed`.
- **Session freshness**: Google rotates its short-lived tokens (`__Secure-*SIDTS` and friends) on
  authenticated responses. The innertube transport merges every `Set-Cookie` into its jar, and a
  60-second timer in the app writes the rotated jar back to the active account's row (and the
  `session_cookie` projection), so switching away and back or restarting never revives a dead
  cookie. An account only needs a fresh sign-in again if it sat unused long enough for Google to
  expire it entirely, or after sign-out/removal.
- **Add account** (`login_webview` with `add_account: true`): first the *webview* is signed out of
  Google — every google.com/youtube.com cookie is deleted from its shared cookie store, leaving the
  app's saved accounts in SQLite untouched. Then Google's `accounts.google.com/AddSession` screen
  opens, so the account the user signs in with becomes the webview's only session and the redirect
  back to music.youtube.com lands on it. (As a fallback, if the captured cookie still matches a
  saved account, the webview is bounced once through `accounts.google.com/AccountChooser`.)
- **Switch account** (`switch_google_account`): stored cookie validated with `account_menu`
  first, then projections + `active_account` swap atomically, playlist index forgotten,
  `auth-changed` emitted. A row saved mid multi-channel sign-in reopens the required channel picker.
- **Sign out / remove** (`sign_out`, `remove_google_account`): deletes the account's row; if it
  was active, drops to guest. Other saved accounts remain listed.
- **Channel switch** (`switch_account`): unchanged — picks a YouTube channel *within* the active
  Google account.
