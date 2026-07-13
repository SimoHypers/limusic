# Limusic

<div align="center">
  <img src="/src-tauri/icons/128x128@2x.png" alt="Limusic Logo">
</div>

A native desktop client for YouTube Music, built with Tauri (Rust + SvelteKit).

Limusic talks directly to YouTube's internal API and plays audio through libmpv.
No Electron, no bundled browser runtime, no backend server, no ads in the audio.
It started as a desktop rebuild of the playback engine behind
[Metrolist](https://github.com/mostafaalagamy/Metrolist), an Android YouTube Music
client, and grew from there.

## Features

- Search and browse: songs, albums, artists, playlists, and the YTM home feed
- Sign in with your YouTube Music account, either through an in-app Google login
  or by pasting a cookie
- Your library: playlists, liked songs, and write actions (like, add to playlist,
  create/rename/delete playlists, subscribe)
- Gapless playback with loudness normalization, powered by libmpv
- Queue with radio/automix continuation, restored across restarts
- OS media keys and now-playing integration (MPRIS on Linux, SMTC on Windows)
- Discord Rich Presence (off by default, toggle in Settings)
- Listen Together: synced listening rooms over a small self-hosted relay
- Self-updating builds (AppImage on Linux, setup.exe on Windows)

## Install

Download from [Releases](https://github.com/SimoHypers/limusic/releases):

| Platform | File | Notes |
|---|---|---|
| Linux | `.AppImage` | Self-updating, libmpv bundled |
| Linux (Fedora/RHEL) | `.rpm` | Needs `mpv-libs` installed (`sudo dnf install mpv-libs`) |
| Windows | `-setup.exe` | Self-updating |
| Windows | `.msi` | Plain installer, no auto-update |
| macOS | none yet | Build from source, see [docs/BUILD-PLATFORMS.md](docs/BUILD-PLATFORMS.md) |

## Listen Together

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

## Building from source

Fedora:

```bash
sudo dnf install mpv-libs mpv-libs-devel webkit2gtk4.1-devel \
  gcc gcc-c++ make openssl-devel librsvg2-devel
cd ui && pnpm install && cd ..
cargo tauri build
```

Windows and macOS instructions live in [docs/BUILD-PLATFORMS.md](docs/BUILD-PLATFORMS.md).

## How it works, briefly

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

## Disclaimer

This project is not affiliated with, funded, authorized, endorsed by, or in
any way associated with YouTube, Google LLC, or any of their affiliates and
subsidiaries.

All trademarks, service marks, and intellectual property rights referenced in
this project belong to their respective owners.

## License

[GPL-3.0](LICENSE)
