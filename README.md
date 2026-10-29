<div align="center">

<img src="./assets/docs/limusic-github-image.png" alt="Limusic Banner" width="100%">

# Limusic

**A native desktop YouTube Music client. Rust + Tauri, ad-free, no Electron.**

<p align="center">
  <a href="https://github.com/SimoHypers/limusic/releases/latest"><img alt="GitHub Downloads" src="https://img.shields.io/github/downloads/SimoHypers/limusic/total?style=for-the-badge&label=DOWNLOADS&color=a4c400"></a>
  <a href="https://github.com/SimoHypers/limusic/releases/latest"><img alt="GitHub Release" src="https://img.shields.io/github/v/release/SimoHypers/limusic?display_name=release&style=for-the-badge&color=a10935"></a>
  <img alt="License" src="https://img.shields.io/github/license/SimoHypers/limusic?style=for-the-badge&color=1881cc">
  <a href="https://hosted.weblate.org/engage/limusic/"><img alt="Translation status" src="https://img.shields.io/weblate/progress/limusic?server=https%3A%2F%2Fhosted.weblate.org&style=for-the-badge&label=TRANSLATED&color=6a3fb5"></a>
  <a href="https://simohypers.github.io/limusic/"><img alt="Website" src="https://img.shields.io/badge/WEBSITE-limusic-e5486e?style=for-the-badge"></a>
  <a href="https://ko-fi.com/simohypers"><img alt="Support on Ko-fi" src="https://img.shields.io/badge/KO--FI-support-ff5e5b?style=for-the-badge&logo=kofi&logoColor=white"></a>
  <br>
  <img alt="Linux" src="https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black">
  <img alt="Windows" src="https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logoColor=white">
  <img alt="macOS" src="https://img.shields.io/badge/macOS-000000?style=for-the-badge&logo=apple&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri_2-24C8D8?style=for-the-badge&logo=tauri&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white">
</p>

**Limusic** talks directly to YouTube's internal API and plays audio through libmpv: no bundled
browser runtime, no backend server, no ads in the audio. It started as a desktop rebuild of the
playback engine behind [Metrolist](https://github.com/mostafaalagamy/Metrolist), an Android
YouTube Music client, and grew from there.

</div>

---

## Features

- **Ad-free playback**: streams come straight from YouTube's API, ads never do
- **Search & browse**: songs, albums, artists, playlists and the YTM home feed, with results previewing as you type
- **Sign in** with your YouTube Music account: in-app Google login or cookie-paste, several accounts at once with switching between them
- **Your library**: playlists, liked songs, saved albums and artists, your uploads, and write actions (like, add to playlist, create/edit/delete playlists including cover art, subscribe, save to library)
- **History**: everything you have played, in YouTube Music's own day buckets
- **Gapless playback** with loudness normalization, powered by libmpv
- **Tempo, pitch and reverb**: adjust playback speed and pitch, add reverb with independent dry/wet controls, and save custom presets on this device.
- **Queue** with radio/automix continuation, drag to reorder, restored across restarts
- **Synced lyrics**: side panel with auto-scroll and click-to-jump, word by word where the source has the timings, with translations under each line
- **Music videos**: optional, the video plays where the artwork sits, with the same gapless audio behind it
- **Mini Player and theater mode**: shrink to a strip that keeps playing, or go fullscreen with cover and lyrics side by side
- **Local Music**: play your own files, with all metadata still intact
- **Last.fm scrobbling**: connect once from the title bar, every play is scrobbled
- **ListenBrainz scrobbling**: paste a user token in the Scrobbling settings, or point it at a self-hosted ListenBrainz-compatible server such as Koito
- **Discord Rich Presence**: artwork, live progress bar, one click to toggle
- **OS media keys** and now-playing integration (MPRIS on Linux, SMTC on Windows, plus playback buttons on the Windows taskbar preview)
- **System tray**: close the window, keep the music; play/pause and skip from the tray, optional start-on-login
- **Listen Together**: synced listening rooms over a small self-hosted relay
- **Keyboard and mouse**: `Ctrl+K` searches from anywhere, `Ctrl+H` lists every shortcut, right-click menus throughout, `Ctrl` and the wheel zooms the interface
- **Eighteen languages**: English, German, Spanish, French, Indonesian, Italian, Japanese, Korean, Polish, Brazilian Portuguese, Romanian, Russian, Tamil, Turkish, Ukrainian, Vietnamese, and Simplified and Traditional Chinese, with more in progress on [Weblate](https://hosted.weblate.org/projects/limusic/)
- **Self-updating builds** (AppImage on Linux, setup.exe on Windows, .app on macOS)
- **Make it yours**: accent palettes, custom colors, your own fonts, corner roundness, a custom app icon, and an adaptive theme that recolors the app from the playing cover

---

## Screenshots

<table>
  <tr>
    <td><img src="website/src/assets/screen-playlist.webp" alt="A playlist in Limusic"></td>
    <td><img src="website/src/assets/screen-lyrics.webp" alt="Word-by-word synced lyrics"></td>
  </tr>
  <tr>
    <td><img src="website/src/assets/screen-album.webp" alt="An album page, colors adapted to the cover"></td>
    <td><img src="website/src/assets/screen-video.webp" alt="A music video playing with lyrics alongside"></td>
  </tr>
</table>

---

<h2 align="center">Download & Install</h2>

<p align="center">
  <a href="https://github.com/SimoHypers/limusic/releases/latest">
    <img src="https://img.shields.io/badge/GitHub_Releases-100000?style=for-the-badge&logo=github&logoColor=white" height="40">
  </a>
</p>

| Platform | File | Notes |
|---|---|---|
| Windows | `-setup.exe` | Self-updating |
| Windows | `.msi` | Plain installer, no auto-update |
| Linux | `.AppImage` | Self-updating, libmpv bundled. Needs glibc 2.39+ (Ubuntu 24.04+, Debian 13+, Fedora 40+) |
| Linux (Ubuntu/Debian) | `.deb` | No self-update. Needs Ubuntu 24.04+ / Debian 13+; apt pulls libmpv and webkit2gtk in for you |
| Linux (Fedora/RHEL) | `.rpm` | Needs `mpv-libs` installed (`sudo dnf install mpv-libs`). No updates, redownload each release |
| macOS (Apple Silicon) | `.dmg` | Self-updating. Unsigned, so the first launch needs `xattr -dr com.apple.quarantine /Applications/limusic.app` |
| macOS (Intel) | none | Build from source, see [docs/BUILD-PLATFORMS.md](docs/BUILD-PLATFORMS.md) |

Community-maintained repositories, packaged and updated by their maintainers rather than by this project:

| Platform | Source | Notes |
|---|---|---|
| Linux (Arch) | [AUR](https://aur.archlinux.org/packages/limusic-bin) | `yay -S limusic-bin`. Maintained by [@xiryuudev](https://github.com/xiryuudev), updates through pacman |
| Linux (Fedora COPR) | [COPR](https://copr.fedorainfracloud.org/coprs/oguzkarayemis/limusic/) | `sudo dnf copr enable oguzkarayemis/limusic` then `sudo dnf install limusic`. Maintained by [@oguzkarayemis](https://github.com/oguzkarayemis), updates through dnf |
| Linux (openSUSE Tumbleweed) | [OBS](https://build.opensuse.org/package/show/home:itachi_re/limusic) | `sudo zypper ar -p 100 https://download.opensuse.org/repositories/home:/itachi_re/openSUSE_Tumbleweed/home:itachi_re.repo` then `sudo zypper install limusic`. Maintained by [@itachi-re](https://github.com/itachi-re), updates through zypper. The repo carries the maintainer's other packages too, so `-p 100` keeps it below the distro repos |

---

## Scrobbling & Discord

Last.fm lives in the title bar, next to the window controls, and ListenBrainz connects from
Settings ▸ Scrobbling. Both get the same cleaned-up track (the tab's preview shows what each
play counts as).

- **Last.fm**: click the Last.fm mark, approve Limusic in the browser tab that
  opens, and you're connected for good. Tracks scrobble at the halfway point (or
  four minutes, whichever comes first), which is Last.fm's own rule. Click again
  to see the account or disconnect.
- **ListenBrainz**: open the Scrobbling settings, paste the user token from
  [listenbrainz.org/settings](https://listenbrainz.org/settings/) and connect.
  Plays submit as `playing_now` on start and one `single` listen at the same
  halfway-or-four-minutes point. A new MetaBrainz account has to verify its email first, or
  ListenBrainz turns every listen away (the Scrobbling tab says so). The server field defaults
  to the official service; to use a self-hosted ListenBrainz-compatible server such as
  [Koito](https://koito.io), enter its API base (for Koito,
  `https://your-server/apis/listenbrainz`) and connect with that server's user token.
- **Discord**: click the Discord mark to toggle Rich Presence. Green dot means
  it's live. The card shows the track, artist, album art, and a progress bar, and
  it disappears when you pause.

Building from source? Last.fm needs your own API credentials, and they are not in
the repo. Get a key at [last.fm/api/account/create](https://www.last.fm/api/account/create)
and put it in `src-tauri/lastfm.keys`:

```
LIMUSIC_LASTFM_API_KEY=your_key
LIMUSIC_LASTFM_API_SECRET=your_secret
```

Without that file everything else still builds and runs; the Last.fm button just
reports that it isn't configured.

---

## Lyrics

Open the panel with the microphone button in the player bar, next to the queue
button. It takes the same side of the window as the queue, so opening one closes
the other.

Lyrics come from [Boidu](https://boidu.dev) first, then
[LRCLIB](https://lrclib.net), then YouTube Music's own timed lyrics, then
Netease, QQ Music and Kugou, falling back to plain un-timed text when nobody has
a synced version. Matching is keyed on the track's exact length, because popular
songs exist as several cuts and the wrong one drifts a few seconds out. Results
are cached locally, so replaying a track is instant.

Boidu is the only source with per-word timings, which is what lets a line
highlight word by word as it's sung. It goes first for that reason, which also
means it is asked about every track you play. Turn it off in **Settings ->
Playback -> Word-by-word lyrics** and the other sources still provide
line-by-line lyrics. Netease additionally supplies translations, shown under
each line where it has them.

Note that YouTube Music's lyrics are licensed per region and are missing
entirely in some countries. Where that's the case, LRCLIB does all the work.

---

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

---

## Translations

Limusic is translated on [Weblate](https://hosted.weblate.org/engage/limusic/),
who host it free for libre projects.

<a href="https://hosted.weblate.org/engage/limusic/">
  <img src="https://hosted.weblate.org/widget/limusic/ui/multi-auto.svg" alt="Translation status">
</a>

English, Spanish, French, Turkish, Brazilian Portuguese and Indonesian ship in
the app today.
The badge above shows everything else in flight.

**Translate on Weblate, not in a pull request.** Weblate keeps its own copy of
the catalogs, so a hand-edited `fr.json` merged here puts the two out of sync
and the next batch of real translations arrives as a merge conflict. Weblate
also shows you the English original beside each string, flags translations that
went stale when the English changed, checks that placeholders like `{count}`
survived, and opens the pull request for you. Anything untranslated falls back
to English in the app, so partial work is safe to submit.

`en.json` is the exception: it changes by hand, in whichever pull request
changes the UI. Switching a finished language on in the picker takes a small
code change too, see [CONTRIBUTING.md](CONTRIBUTING.md#translations).

---

## Building from Source

Fedora:

```bash
sudo dnf install mpv-libs mpv-libs-devel webkit2gtk4.1-devel \
  gcc gcc-c++ make openssl-devel librsvg2-devel
cd ui && pnpm install && cd ..
cargo tauri build
```

Windows and macOS instructions live in [docs/BUILD-PLATFORMS.md](docs/BUILD-PLATFORMS.md).

---

## Releasing (this fork)

This fork ships its own signed builds from GitHub Actions. The full runbook is
[docs/RELEASING-FROM-ACTIONS.md](docs/RELEASING-FROM-ACTIONS.md); this section is the
checklist to read before touching a release.

**The setup, so you do not have to rediscover it**

| Thing | Where it lives | Why it matters |
| --- | --- | --- |
| Signing private key | `~/.tauri/limusic-enhanced.key` and repo secret `TAURI_SIGNING_PRIVATE_KEY` | Signs every updater artifact. **Back it up.** Lose it and every installed copy refuses all future updates; there is no recovery but a manual reinstall |
| Signing public key | `src-tauri/tauri.conf.json` → `plugins.updater.pubkey` | Must match the private key. Changing it invalidates updates for existing installs |
| Updater endpoint | `src-tauri/tauri.conf.json` → `plugins.updater.endpoints` | This fork's `releases/latest/download/latest.json` |
| Beta endpoint | `src-tauri/src/commands.rs` → `BETA_MANIFEST` | This fork's `releases/download/beta/latest.json` |
| `beta` release | GitHub release tagged `beta`, marked prerelease | Permanent pointer that RC installs read. **Never delete it** |
| `Create release` workflow | `.github/workflows/release.yml` | Bumps, tags, publishes, and dispatches the three platform builds |

**Cutting a release**

1. Wait for upstream to tag the version you are shipping and merge it, so the number means
   the same thing in both repos.
2. Actions → **Create release** → Run workflow with `dry_run: true`. It validates the
   version and prints the plan without changing anything.
3. Re-run with `dry_run: false`. It bumps `Cargo.toml`, `Cargo.lock` and
   `src-tauri/tauri.conf.json`, commits to `master`, pushes the tag, creates the release
   (not Latest), and dispatches Linux, Windows and macOS.

From a terminal:

```bash
gh workflow run "Create release" --repo kiranojhanp/limusic-enhanced --ref master \
  -f version=1.4.0 -f dry_run=true
gh workflow run "Create release" --repo kiranojhanp/limusic-enhanced --ref master \
  -f version=1.4.0 -f dry_run=false
```

**Rules that are easy to forget**

- **Continue upstream's version numbers.** Never restart at `1.0.0`: the updater compares
  semver, so a lower number is never offered to an installed copy.
- **Never re-release a tag.** Bump instead; the workflow refuses an existing tag or release.
- **Release candidates never become Latest.** `1.4.0-rc.5` lands on the `beta` channel only.
  `/releases/latest/download/latest.json` keeps resolving to the last stable release, and
  returns 404 until this fork ships its first stable.
- **Only all three platforms flip `latest`.** If one fails, the release stays un-Latest and
  installed clients keep seeing the previous complete release. Re-dispatch just the failed
  one:

  ```bash
  gh workflow run "Linux release binaries" --repo kiranojhanp/limusic-enhanced \
    --ref master -f tag=v1.4.0
  ```
- **The `.rpm` is not built in Actions.** It needs Fedora; build it separately if you want one.
- **`RELEASE_TOKEN`** is only needed once `master` has branch protection, because
  `GITHUB_TOKEN` cannot push past it.

**Checking a release**

```bash
gh release view v1.4.0 --repo kiranojhanp/limusic-enhanced
curl -sL https://github.com/kiranojhanp/limusic-enhanced/releases/latest/download/latest.json | jq
curl -sL https://github.com/kiranojhanp/limusic-enhanced/releases/download/beta/latest.json | jq
```

A complete release has a `.sig` next to every updater artifact (AppImage, setup.exe,
`.app.tar.gz`) and all three platform keys in `latest.json`.

---

## How It Works, Briefly

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

## Star History

<a href="https://www.star-history.com/?repos=simohypers%2Flimusic&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=simohypers/limusic&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=simohypers/limusic&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=simohypers/limusic&type=date&legend=top-left" />
 </picture>
</a>

---

## Support

Limusic is free and stays free. If it earned a coffee,
[ko-fi.com/simohypers](https://ko-fi.com/simohypers) is where to leave one.

---

## Disclaimer

This project is not affiliated with, funded, authorized, endorsed by, or in
any way associated with YouTube, Google LLC, or any of their affiliates and
subsidiaries.

All trademarks, service marks, and intellectual property rights referenced in
this project belong to their respective owners.

---

## License

[GPL-3.0](LICENSE)
