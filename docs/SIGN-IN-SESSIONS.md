# Sign-in sessions: where the data lives

Everything below is per-OS under Tauri's app-data dir for identifier `com.limusic.desktop`
(`tauri.conf.json`), created in `lib.rs` via `app.path().app_data_dir()`:

| OS | Directory |
|---|---|
| Windows | `%APPDATA%\com.limusic.desktop\` |
| Linux | `~/.local/share/com.limusic.desktop/` |
| macOS | `~/Library/Application Support/com.limusic.desktop/` |

## 1. SQLite: `limusic.sqlite`

The canonical store of your saved Google accounts and the active session. Schema: `src-tauri/src/db.rs`.

### `accounts` table

One row per saved Google account.

| Column | Contents |
|---|---|
| `id` | Opaque account key: `ga-<sha1>` over the cookie's long-lived `SAPISID` value (`db::account_key`). Stable across Rust releases, and the SAPISID itself is not recoverable from it. |
| `session_cookie` | The full `Cookie:` header captured at sign-in (SAPISID, SID, `__Secure-*`, ...): the actual login credential. |
| `data_sync_id` | Server-issued delegated identity of the selected YouTube channel within this Google account. |
| `selected_identity_json` | Canonical account model: name, handle, email, thumbnail, channelId, data_sync_id. `NULL` means the login authenticated but a multi-channel pick is still pending. |
| `account_json` | The JSON shape the UI consumes (`signedIn`, `name`, ...). |
| `visitor_data` | Login-bound visitorData for this account. |
| `added_at` | Unix seconds; the account menu lists oldest first (re-logins do not reorder). |

### `settings` rows: active-account projections

The active account is projected into these keys so the startup bootstrap in `lib.rs`,
`AppState::account_snapshot`, and the channel switcher work unchanged:

| Key | Contents |
|---|---|
| `active_account` | `id` of the active account; absent means signed out (guest). |
| `session_cookie` | Cookie of the active account only. |
| `selected_identity_json`, `data_sync_id`, `account_json` | Active account's identity, written together (`Db::set_auth_identity`). |
| `account_selection_pending` | `"true"` while a multi-channel login awaits a pick. |
| `visitor_data` | Active session's visitorData (anonymous bootstrap id until first login). |

Databases from before multi-account are migrated once on open (`Db::open`): the legacy
`session_cookie` row becomes an `accounts` row and `active_account` is set.

## 2. Webview cookie store: the login window's own Google session

The sign-in webview (`src-tauri/src/session.rs`) is persistent (non-incognito) on purpose. Its
cookies live in the OS webview profile data *next to* the app data dir, not in the SQLite file:

- **Windows (WebView2):** user-data folder under `%LOCALAPPDATA%\com.limusic.desktop\EBWebView\`
- **Linux (WebKitGTK):** `~/.local/share/com.limusic.desktop/` webkit data (or XDG cache)
- **macOS (WKWebView):** inside `~/Library/Application Support/com.limusic.desktop/` WebKit data

This is why a re-login is one click with no password/paste, and why deleting `limusic.sqlite`
alone does not sign the webview out of Google. Limusic never reads this store as state:
`session.rs` copies the youtube-domain cookies out of it into a `Cookie` header and stores that
copy in SQLite.

It holds **one** Google session at a time, and **Add account** replaces it: that flow deletes every
google.com/youtube.com cookie from the store first (webview-side only, the SQLite rows are
untouched) so the fresh sign-in lands on the account the user actually entered. Two consequences:
a later plain "Sign in with Google" asks for a password again, and the automatic session healing
below can only re-mint whichever account the webview currently holds.

## 3. What never leaves Rust

Auth material (`session_cookie`, `selected_identity_json`, `data_sync_id`, `account_json`,
`visitor_data`, `account_selection_pending`, `active_account`) is excluded from the `get_settings`
whitelist (`commands.rs::UI_SETTINGS`), so the renderer cannot read or overwrite it. The UI only
ever sees display fields plus opaque selectors (`id`, `selectionKey`).

## Flow summary

- **Sign in** (`login_webview`): ServiceLogin webview, redirect to music.youtube.com, cookies
  captured, validated via `account_menu`, `accounts` row upserted (keyed on SAPISID, so a new
  Google account adds a row and a known one refreshes its own), `active_account` set,
  `auth-changed` emitted.
- **Session freshness** (#165 / KI-2): Google rotates its short-lived tokens
  (`__Secure-*SIDTS`) on authenticated responses. The innertube transport merges every
  `Set-Cookie` into its jar and raises `cookie_changed`; the app writes the rotated jar back to
  the `session_cookie` projection *and* the account's own row, so a switch away and back, or a
  restart, never revives a dead cookie. A half-hourly `account_menu` ping keeps the jar rolling
  while the app is idle, since rotation only happens on requests the app actually makes. If
  YouTube rejects the session outright, `refresh_session` re-mints it from the login webview, but
  only when that webview still holds the same Google account, so healing can never move the app to
  a different account behind the user's back.
- **Add account** (`login_webview` with `add_account: true`): webview signed out of Google, then
  Google's `accounts.google.com/AddSession` screen. If the cookie store cannot be cleared the flow
  stops with a `login-error` rather than signing in as the account already there.
- **Switch account** (`switch_google_account`): stored cookie validated with `account_menu` first,
  then projections and `active_account` swap atomically, playlist index forgotten, `auth-changed`
  emitted. A row saved mid multi-channel sign-in reopens the required channel picker.
- **Sign out** (`sign_out`): drops to guest. The account stays in the saved list, one click from
  coming back.
- **Remove account** (`remove_google_account`): deletes the account's row, and drops to guest if it
  was the active one. Other saved accounts stay listed and switchable.
- **Channel switch** (`switch_account`): unchanged, picks a YouTube channel *within* the active
  Google account.
