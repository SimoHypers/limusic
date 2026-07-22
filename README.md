<div align="center">

<img src="./assets/docs/limusic-github-image.png" alt="Limusic Banner" width="100%">

# 🍋 Limusic

**A native desktop YouTube Music client — Rust + Tauri, ad-free, no Electron.**

<p align="center">
  <a href="https://github.com/SimoHypers/limusic/releases/latest"><img alt="GitHub Downloads" src="https://img.shields.io/github/downloads/SimoHypers/limusic/total?style=for-the-badge&label=DOWNLOADS&color=a4c400"></a>
  <a href="https://github.com/SimoHypers/limusic/releases/latest"><img alt="GitHub Release" src="https://img.shields.io/github/v/release/SimoHypers/limusic?display_name=release&style=for-the-badge&color=a10935"></a>
  <img alt="License" src="https://img.shields.io/github/license/SimoHypers/limusic?style=for-the-badge&color=1881cc">
  <br>
  <img src="https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black">
  <img src="https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logoColor=white">
  <img src="https://img.shields.io/badge/Tauri_2-24C8D8?style=for-the-badge&logo=tauri&logoColor=white">
  <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white">
</p>

**Limusic** talks directly to YouTube's internal API and plays audio through libmpv — no bundled
browser runtime, no backend server, no ads in the audio. It started as a desktop rebuild of the
playback engine behind [Metrolist](https://github.com/mostafaalagamy/Metrolist), an Android
YouTube Music client, and grew from there. 🎵

</div>

---

## ✨ Features

- 🚫 **Ad-free playback** — streams come straight from YouTube's API, ads never do
- 🔎 **Search & browse** — songs, albums, artists, playlists, and the YTM home feed
- 🔑 **Sign in** with your YouTube Music account: in-app Google login or cookie-paste
- 📚 **Your library** — playlists, liked songs, and write actions (like, add to playlist, create/rename/delete playlists, subscribe)
- 🎧 **Gapless playback** with loudness normalization, powered by libmpv
- 📜 **Queue** with radio/automix continuation, restored across restarts
- 🎤 **Synced lyrics** — line-by-line side panel with auto-scroll and click-to-jump
- 📊 **Last.fm scrobbling** — connect once from the title bar, every play is scrobbled
- 🎮 **Discord Rich Presence** — artwork, live progress bar, one click to toggle
- ⌨️ **OS media keys** and now-playing integration (MPRIS on Linux, SMTC on Windows)
- 🔔 **System tray** — close the window, keep the music; play/pause and skip from the tray, optional start-on-login
- 👥 **Listen Together** — synced listening rooms over a small self-hosted relay
- 🔄 **Self-updating builds** (AppImage on Linux, setup.exe on Windows)

---

<h2 align="center">📥 Download & Install</h2>

<p align="center">
  <a href="https://github.com/SimoHypers/limusic/releases/latest">
    <img src="https://img.shields.io/badge/GitHub_Releases-100000?style=for-the-badge&logo=github&logoColor=white" height="40">
  </a>
</p>

| Platform | File | Notes |
|---|---|---|
| Linux | `.AppImage` | Self-updating, libmpv bundled |
| Linux (Fedora/RHEL) | `.rpm` | Needs `mpv-libs` installed (`sudo dnf install mpv-libs`) |
| Windows | `-setup.exe` | Self-updating |
| Windows | `.msi` | Plain installer, no auto-update |
| macOS | none yet | Build from source, see [docs/BUILD-PLATFORMS.md](docs/BUILD-PLATFORMS.md) |

---

## 📊 Scrobbling & Discord

Both live in the title bar, next to the window controls.

- **Last.fm** — click the Last.fm mark, approve Limusic in the browser tab that
  opens, and you're connected for good. Tracks scrobble at the halfway point (or
  four minutes, whichever comes first), which is Last.fm's own rule. Click again
  to see the account or disconnect.
- **Discord** — click the Discord mark to toggle Rich Presence. Green dot means
  it's live. The card shows the track, artist, album art, and a progress bar, and
  it disappears when you pause.

Building from source? Last.fm needs your own API credentials — they're not in the
repo. Get a key at [last.fm/api/account/create](https://www.last.fm/api/account/create)
and put it in `src-tauri/lastfm.keys`:

```
LIMUSIC_LASTFM_API_KEY=your_key
LIMUSIC_LASTFM_API_SECRET=your_secret
```

Without that file everything else still builds and runs; the Last.fm button just
reports that it isn't configured.

---

## 🎤 Lyrics

Open the panel with the microphone button in the player bar, next to the queue
button. It takes the same side of the window as the queue, so opening one closes
the other.

Lyrics come from [LRCLIB](https://lrclib.net) first, then YouTube Music's own
timed lyrics, falling back to plain un-timed text when nobody has a synced
version. Matching is keyed on the track's exact length, because popular songs
exist as several cuts and the wrong one drifts a few seconds out. Results are
cached locally, so replaying a track is instant.

Note that YouTube Music's lyrics are licensed per region and are missing
entirely in some countries — where that's the case, LRCLIB does all the work.

---

## 👥 Listen Together

Synced listening with friends. Everyone streams their own audio from YouTube;
the room only relays play/pause, seeks, track changes and the queue. One person
hosts the relay:

```bash
cargo run -p sync-server        # plain WebSocket on 0.0.0.0:8080
```

Front it with something that terminates TLS (Tailscale Funnel, Cloudflare
Tunnel), then paste the `wss://` URL into the Listen Together panel in the app.
Rooms have join codes and the host approves every join and every track
suggestion.

---

## 🔧 Building from Source

Fedora:

```bash
sudo dnf install mpv-libs mpv-libs-devel webkit2gtk4.1-devel \
  gcc gcc-c++ make openssl-devel librsvg2-devel
cd ui && pnpm install && cd ..
cargo tauri build
```

Windows and macOS instructions live in [docs/BUILD-PLATFORMS.md](docs/BUILD-PLATFORMS.md).

---

## ⚙️ How It Works, Briefly

- A pure Rust crate speaks YouTube's InnerTube API, impersonating several
  official client identities and falling back between them when one fails.
- YouTube's stream URLs are protected by obfuscated JavaScript (the signature
  cipher and the `n` parameter) and by BotGuard attestation. Limusic runs that
  JavaScript where it expects to run, in a real webview, hidden, and never lets
  any of it touch the UI process.
- Audio goes through libmpv: gapless transitions, an on-disk cache, and
  loudness normalization from YouTube's own metadata.
- The UI is a SvelteKit SPA that only ever talks to the Rust core. It never
  contacts YouTube itself.

---

## ⚠️ Disclaimer

This project is not affiliated with, funded, authorized, endorsed by, or in
any way associated with YouTube, Google LLC, or any of their affiliates and
subsidiaries.

All trademarks, service marks, and intellectual property rights referenced in
this project belong to their respective owners.

---

## 📄 License

[GPL-3.0](LICENSE)
